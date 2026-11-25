//! Decoded frames to RGBA textures dfui draws as bitmaps: Y'CbCr to R'G'B',
//! and for HDR films either the HDR signal for an HDR screen or a tone map to SDR.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use dfui::wgpu;

use crate::{Color, Frame, Matrix, Planes, Transfer};

/// Y'CbCr (each sample over its code range, 0..1) to R'G'B' rows:
/// `rgb[i] = dot(m[i].xyz, yuv) + m[i].w`.
fn matrix(c: &Color) -> [[f32; 4]; 3] {
    let (kr, kb) = match c.matrix {
        Matrix::Bt601 => (0.299, 0.114),
        Matrix::Bt709 => (0.2126, 0.0722),
        Matrix::Bt2020 => (0.2627, 0.0593),
    };
    let kg = 1.0 - kr - kb;
    let max = ((1u32 << c.bits) - 1) as f32;
    // Limited range is 16..235 (luma) and 16..240 (chroma) at 8 bits, scaled up for more.
    let q = (1u32 << (c.bits - 8)) as f32;
    let (ys, yo, cs, c0) = match c.full_range {
        true => (1.0, 0.0, 1.0, (1u32 << (c.bits - 1)) as f32 / max),
        false => (
            max / (219.0 * q),
            -16.0 / 219.0,
            max / (224.0 * q),
            128.0 * q / max,
        ),
    };
    let (rv, gu, gv, bu) = (
        2.0 * (1.0 - kr) * cs,
        -2.0 * kb * (1.0 - kb) / kg * cs,
        -2.0 * kr * (1.0 - kr) / kg * cs,
        2.0 * (1.0 - kb) * cs,
    );
    [
        [ys, 0.0, rv, yo - rv * c0],
        [ys, gu, gv, yo - (gu + gv) * c0],
        [ys, bu, 0.0, yo - bu * c0],
    ]
}

/// Linear BT.2020 to linear BT.709 rows (ITU-R BT.2087); colors outside BT.709 go negative.
const BT2020_TO_BT709: [[f32; 4]; 3] = [
    [1.6605, -0.5876, -0.0728, 0.0],
    [-0.1246, 1.1329, -0.0083, 0.0],
    [-0.0182, -0.1006, 1.1187, 0.0],
];
const IDENTITY: [[f32; 4]; 3] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

/// The peak to tone-map from when a film doesn't say: what most are mastered for.
const DEFAULT_PEAK: f32 = 1000.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    yuv: [[f32; 4]; 3],
    gamut: [[f32; 4]; 3],
    /// Plane samples times this are code over the code range: P010 keeps 10 bits high in 16.
    scale: f32,
    /// 0 SDR, 1 PQ, 2 HLG.
    transfer: u32,
    /// 1 writes HDR for an HDR screen, 0 tone-maps to SDR.
    hdr: u32,
    /// Nits the tone map starts from.
    peak: f32,
}

/// A converted frame; its texture goes back to the pool once every clone drops.
#[derive(Clone)]
pub struct Shown {
    pub pts: f64,
    pub bitmap: dfui::Bitmap,
    /// Display width over height.
    pub aspect: f32,
    _texture: Arc<Tex>,
}

/// See dfui's `View`: stops `Send`/`Sync` checks from recursing through wgpu.
pub(crate) struct Tex(pub(crate) wgpu::Texture);

// SAFETY: wgpu's handles are Send + Sync on native targets.
unsafe impl Send for Tex {}
unsafe impl Sync for Tex {}

pub(crate) struct Converter {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
    /// One per output format, made when first needed.
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    /// Planes for frames in memory, kept while the size and format hold.
    planes: Option<(wgpu::Texture, wgpu::Texture)>,
    pool: Vec<Arc<Tex>>,
    /// Dmabuf import failed once, so the decoder should hand over copies.
    pub import_failed: bool,
}

const SHADER: &str = r#"
struct P { r: vec4f, g: vec4f, b: vec4f, g0: vec4f, g1: vec4f, g2: vec4f,
           scale: f32, transfer: u32, hdr: u32, peak: f32 }
@group(0) @binding(0) var ty: texture_2d<f32>;
@group(0) @binding(1) var tuv: texture_2d<f32>;
@group(0) @binding(2) var s: sampler;
@group(0) @binding(3) var<uniform> p: P;

// SDR reference white, nits (ITU-R BT.2408): 1.0 in dfui's canvas.
const WHITE: f32 = 203.0;

struct V { @builtin(position) pos: vec4f, @location(0) uv: vec2f }

@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return V(vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}

// SMPTE ST 2084: signal to nits, and back.
fn pq_eotf(e: f32) -> f32 {
    let p = pow(e, 1.0 / 78.84375);
    return 10000.0 * pow(max(p - 0.8359375, 0.0) / (18.8515625 - 18.6875 * p), 1.0 / 0.1593017578125);
}
fn pq_oetf(nits: f32) -> f32 {
    let y = pow(clamp(nits / 10000.0, 0.0, 1.0), 0.1593017578125);
    return pow((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y), 78.84375);
}

// HLG signal to display nits on a nominal 1000-nit screen (BT.2100 OOTF, gamma 1.2).
fn hlg_nits(e: vec3f) -> vec3f {
    let a = 0.17883277;
    let lin = select((exp((e - 0.55991073) / a) + 0.28466892) / 12.0, e * e / 3.0, e <= vec3f(0.5));
    let y = dot(lin, vec3f(0.2627, 0.6780, 0.0593));
    return 1000.0 * pow(max(y, 1e-6), 0.2) * lin;
}

// ITU-R BT.2390 EETF: `nits` from a film peaking at `peak` into 0..WHITE, keeping the
// darks and midtones as they are and rolling the highlights off in PQ space.
fn eetf(nits: f32, peak: f32) -> f32 {
    if peak <= WHITE {
        return nits;
    }
    let top = pq_oetf(peak);
    let e1 = pq_oetf(nits) / top;
    let goal = pq_oetf(WHITE) / top;
    let ks = 1.5 * goal - 0.5;
    var e2 = e1;
    if e1 > ks {
        let t = (e1 - ks) / (1.0 - ks);
        let t2 = t * t;
        let t3 = t2 * t;
        e2 = (2.0 * t3 - 3.0 * t2 + 1.0) * ks + (t3 - 2.0 * t2 + t) * (1.0 - ks)
            + (-2.0 * t3 + 3.0 * t2) * goal;
    }
    return pq_eotf(min(e2, goal) * top);
}

// sRGB encoding that keeps the sign, as dfui's canvas holds colors outside sRGB.
fn encode(l: vec3f) -> vec3f {
    let a = abs(l);
    return sign(l) * select(1.055 * pow(a, vec3f(1.0 / 2.4)) - 0.055, 12.92 * a, a <= vec3f(0.0031308));
}

fn to709(c: vec3f) -> vec3f {
    return vec3f(dot(p.g0.xyz, c), dot(p.g1.xyz, c), dot(p.g2.xyz, c));
}

@fragment fn fs(v: V) -> @location(0) vec4f {
    let yuv = vec4f(textureSample(ty, s, v.uv).r * p.scale, textureSample(tuv, s, v.uv).rg * p.scale, 1.0);
    let rgb = vec3f(dot(p.r, yuv), dot(p.g, yuv), dot(p.b, yuv));
    if p.transfer == 0u {
        return vec4f(clamp(rgb, vec3f(0.0), vec3f(1.0)), 1.0);
    }
    let e = clamp(rgb, vec3f(0.0), vec3f(1.0));
    var nits: vec3f;
    if p.transfer == 1u {
        nits = vec3f(pq_eotf(e.r), pq_eotf(e.g), pq_eotf(e.b));
    } else {
        nits = hlg_nits(e);
    }
    if p.hdr == 1u {
        return vec4f(encode(to709(nits) / WHITE), 1.0);
    }
    // Tone-map the brightest channel and scale the others with it, so hues hold.
    let m = max(nits.r, max(nits.g, nits.b));
    if m > 0.0 {
        nits *= eetf(m, p.peak) / m;
    }
    return vec4f(encode(clamp(to709(nits) / WHITE, vec3f(0.0), vec3f(1.0))), 1.0);
}
"#;

fn plane(device: &wgpu::Device, (w, h): (u32, u32), format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("video plane"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn write(queue: &wgpu::Queue, t: &wgpu::Texture, data: &[u8], stride: usize) {
    queue.write_texture(
        t.as_image_copy(),
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride as u32),
            rows_per_image: None,
        },
        t.size(),
    );
}

/// The high byte of each little-endian u16: 10-bit samples cut to 8.
fn high_bytes(data: &[u8]) -> Vec<u8> {
    data.as_chunks::<2>().0.iter().map(|s| s[1]).collect()
}

impl Converter {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Converter {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("video"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video"),
            entries: &[
                tex(0),
                tex(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("video params"),
            size: size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Converter {
            device,
            queue,
            shader,
            pipeline_layout,
            pipelines: Vec::new(),
            layout,
            sampler,
            params,
            planes: None,
            pool: Vec::new(),
            import_failed: false,
        }
    }

    fn pipeline(&mut self, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        if let Some((_, p)) = self.pipelines.iter().find(|(f, _)| *f == format) {
            return p.clone();
        }
        let p = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("video"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(format.into())],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines.push((format, p.clone()));
        p
    }

    fn output(&mut self, (w, h): (u32, u32), format: wgpu::TextureFormat) -> Arc<Tex> {
        let fits = |t: &Tex| t.0.width() == w && t.0.height() == h && t.0.format() == format;
        self.pool.retain(|t| fits(t) || Arc::strong_count(t) > 1);
        if let Some(t) = self
            .pool
            .iter()
            .find(|t| Arc::strong_count(t) == 1 && fits(t))
        {
            return t.clone();
        }
        let t = Arc::new(Tex(self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("video frame"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })));
        self.pool.push(t.clone());
        t
    }

    /// Converts `frame` and waits for the GPU, so the decoder can reuse its
    /// memory as soon as this returns. With `hdr` an HDR film keeps its range
    /// for an HDR screen; without it, it's tone-mapped. `None` if a dmabuf
    /// couldn't be imported.
    pub fn convert(&mut self, frame: &Frame, hdr: bool) -> Option<Shown> {
        let (w, h) = (frame.width, frame.height);
        let chroma = (w.div_ceil(2), h.div_ceil(2));
        let mut color = frame.color;
        let deep = color.bits > 8;
        let norm16 = self
            .device
            .features()
            .contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM);
        let (fy, fuv) = match deep && norm16 {
            true => (
                wgpu::TextureFormat::R16Unorm,
                wgpu::TextureFormat::Rg16Unorm,
            ),
            false => (wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm),
        };
        // Imported planes live only as long as this frame.
        #[cfg(target_os = "linux")]
        let imported: (wgpu::Texture, wgpu::Texture);
        let (y, uv) = match &frame.planes {
            Planes::Cpu {
                y,
                y_stride,
                uv,
                uv_stride,
            } => {
                if self
                    .planes
                    .as_ref()
                    .is_none_or(|(p, _)| p.width() != w || p.height() != h || p.format() != fy)
                {
                    self.planes = Some((
                        plane(&self.device, (w, h), fy),
                        plane(&self.device, chroma, fuv),
                    ));
                }
                let (ty, tuv) = self.planes.as_ref().unwrap();
                match deep && !norm16 {
                    // ponytail: no 16-bit textures, so 10-bit films lose 2 bits; fine for SDR screens.
                    true => {
                        write(&self.queue, ty, &high_bytes(y), y_stride / 2);
                        write(&self.queue, tuv, &high_bytes(uv), uv_stride / 2);
                        color.bits = 8;
                    }
                    false => {
                        write(&self.queue, ty, y, *y_stride);
                        write(&self.queue, tuv, uv, *uv_stride);
                    }
                }
                (ty, tuv)
            }
            #[cfg(target_os = "linux")]
            Planes::Dmabuf(d) => {
                if deep && !norm16 {
                    self.import_failed = true;
                    return None;
                }
                let import = |p: &crate::DmaPlane, size, format| unsafe {
                    dmabuf::import(&self.device, p, size, format)
                };
                imported = match (
                    import(&d.planes[0], (w, h), fy),
                    import(&d.planes[1], chroma, fuv),
                ) {
                    (Some(a), Some(b)) => (a, b),
                    _ => {
                        self.import_failed = true;
                        return None;
                    }
                };
                (&imported.0, &imported.1)
            }
        };
        let hdr = hdr && color.transfer != Transfer::Sdr;
        let params = Params {
            yuv: matrix(&color),
            gamut: match color.matrix {
                Matrix::Bt2020 => BT2020_TO_BT709,
                _ => IDENTITY,
            },
            scale: match color.bits {
                8 => 1.0,
                // R16Unorm reads code << 6 over 65535; this makes it code over 1023.
                _ => 65535.0 / (64.0 * 1023.0),
            },
            transfer: match color.transfer {
                Transfer::Sdr => 0,
                Transfer::Pq => 1,
                Transfer::Hlg => 2,
            },
            hdr: hdr as u32,
            peak: match (color.transfer, color.peak) {
                (Transfer::Hlg, _) => 1000.0,
                (_, p) if p > 0.0 => p,
                _ => DEFAULT_PEAK,
            },
        };
        self.queue
            .write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let view = |t: &wgpu::Texture| t.create_view(&Default::default());
        let (vy, vuv) = (view(y), view(uv));
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&vy),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&vuv),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.params.as_entire_binding(),
                },
            ],
        });
        // Half floats only where needed: HDR above SDR white.
        let format = match hdr {
            true => wgpu::TextureFormat::Rgba16Float,
            false => wgpu::TextureFormat::Rgba8Unorm,
        };
        let pipeline = self.pipeline(format);
        let out = self.output((w, h), format);
        let target = view(&out.0);
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        let done = self.queue.submit([enc.finish()]);
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(done),
            timeout: None,
        });
        Some(Shown {
            pts: frame.pts,
            bitmap: dfui::Bitmap::from_texture(&out.0),
            aspect: w as f32 * frame.sar / h.max(1) as f32,
            _texture: out,
        })
    }
}

#[cfg(target_os = "linux")]
mod dmabuf {
    use std::os::fd::BorrowedFd;

    use dfui::wgpu;
    use wgpu::hal;

    /// One plane of a dmabuf as a texture of `format`, or `None` if Vulkan won't take it.
    pub unsafe fn import(
        device: &wgpu::Device,
        p: &crate::DmaPlane,
        (w, h): (u32, u32),
        format: wgpu::TextureFormat,
    ) -> Option<wgpu::Texture> {
        let fd = unsafe { BorrowedFd::borrow_raw(p.fd) }
            .try_clone_to_owned()
            .ok()?;
        let size = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        let hal_texture = {
            let hal_device = unsafe { device.as_hal::<hal::api::Vulkan>() }?;
            let desc = hal::TextureDescriptor {
                label: Some("video plane"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUses::RESOURCE,
                memory_flags: hal::MemoryFlags::empty(),
                view_formats: vec![],
            };
            match unsafe {
                hal_device.texture_from_dmabuf_fd(fd, &desc, p.modifier, p.pitch, p.offset)
            } {
                Ok(t) => t,
                Err(e) => {
                    log::warn!("dmabuf import failed: {e}");
                    return None;
                }
            }
        };
        Some(unsafe {
            device.create_texture_from_hal::<hal::api::Vulkan>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("video plane"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::TextureUses::RESOURCE,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(m: [[f32; 4]; 3], yuv: [f32; 3]) -> [f32; 3] {
        m.map(|r| r[0] * yuv[0] + r[1] * yuv[1] + r[2] * yuv[2] + r[3])
    }

    #[test]
    fn limited_range_black_white_and_red() {
        let c = |matrix, full_range, bits| Color {
            matrix,
            transfer: Transfer::Sdr,
            full_range,
            bits,
            peak: 0.0,
        };
        let m = matrix(&c(Matrix::Bt709, false, 8));
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01);
        let mid = 128.0 / 255.0;
        assert!(near(rgb(m, [16.0 / 255.0, mid, mid]), [0.0; 3]));
        assert!(near(rgb(m, [235.0 / 255.0, mid, mid]), [1.0; 3]));
        // BT.709 limited-range red: Y 63, Cb 102, Cr 240.
        assert!(near(
            rgb(m, [63.0 / 255.0, 102.0 / 255.0, 240.0 / 255.0]),
            [1.0, 0.0, 0.0]
        ));
        assert!(near(
            rgb(matrix(&c(Matrix::Bt601, true, 8)), [0.0, mid, mid]),
            [0.0; 3]
        ));
        // 10-bit limited range: black 64, white 940, chroma centred on 512.
        let m = matrix(&c(Matrix::Bt2020, false, 10));
        let mid = 512.0 / 1023.0;
        assert!(near(rgb(m, [64.0 / 1023.0, mid, mid]), [0.0; 3]));
        assert!(near(rgb(m, [940.0 / 1023.0, mid, mid]), [1.0; 3]));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod gpu_tests {
    use super::*;
    use crate::mp4::{Kind, Reader};
    use crate::vaapi::Decoder;

    fn device() -> (wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle_from_env()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: adapter.features()
                & (wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF
                    | wgpu::Features::TEXTURE_FORMAT_16BIT_NORM),
            ..Default::default()
        }))
        .unwrap()
    }

    fn read(device: &wgpu::Device, queue: &wgpu::Queue, t: &wgpu::Texture) -> Vec<u8> {
        let (w, h) = (t.width(), t.height());
        let px = t.format().block_copy_size(None).unwrap();
        let row = (w * px).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            t.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            t.size(),
        );
        queue.submit([enc.finish()]);
        buf.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buf.slice(..).get_mapped_range().unwrap();
        (0..h)
            .flat_map(|y| data[(y * row) as usize..][..(w * px) as usize].to_vec())
            .collect()
    }

    /// A converted frame's first pixel's RGB, 0..1 (or past it, for half floats).
    fn first_pixel(conv: &Converter, s: &Shown) -> [f32; 3] {
        let bytes = read(&conv.device, &conv.queue, &s._texture.0);
        match s._texture.0.format() {
            wgpu::TextureFormat::Rgba8Unorm => [0, 1, 2].map(|i| bytes[i] as f32 / 255.0),
            _ => [0, 1, 2].map(|i| half(u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]))),
        }
    }

    /// An IEEE half float's value (normal numbers and zero, as frames hold).
    fn half(h: u16) -> f32 {
        let (sign, exp, frac) = (h >> 15, (h >> 10) & 0x1f, h & 0x3ff);
        let v = match exp {
            0 => frac as f32 / 1024.0 * 2f32.powi(-14),
            _ => (1.0 + frac as f32 / 1024.0) * 2f32.powi(exp as i32 - 15),
        };
        if sign == 1 { -v } else { v }
    }

    /// An 8x8 HDR10 frame of one grey: limited-range 10-bit PQ code `y`.
    fn grey(y: u16) -> Frame {
        let le = |v: u16, n: usize| (0..n).flat_map(|_| v.to_le_bytes()).collect::<Vec<u8>>();
        Frame {
            pts: 0.0,
            width: 8,
            height: 8,
            sar: 1.0,
            color: Color {
                matrix: Matrix::Bt2020,
                transfer: Transfer::Pq,
                full_range: false,
                bits: 10,
                peak: 1000.0,
            },
            planes: Planes::Cpu {
                y: le(y << 6, 64),
                y_stride: 16,
                uv: le(512 << 6, 32),
                uv_stride: 16,
            },
        }
    }

    /// The limited-range 10-bit code of `nits` in PQ.
    fn pq_code(nits: f64) -> u16 {
        let y = (nits / 10000.0).powf(0.1593017578125);
        let e = ((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y)).powf(78.84375);
        (64.0 + e * 876.0).round() as u16
    }

    /// sRGB encoding of `v` times SDR white.
    fn srgb(v: f32) -> f32 {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn tone_map_keeps_the_darks_and_rolls_highlights_into_sdr_white() {
        let (device, queue) = device();
        let mut conv = Converter::new(device, queue);
        let mut out = |nits: f64| {
            let s = conv.convert(&grey(pq_code(nits)), false).unwrap();
            assert_eq!(s._texture.0.format(), wgpu::TextureFormat::Rgba8Unorm);
            first_pixel(&conv, &s)[1]
        };
        assert!(out(0.0) < 0.01);
        // Below the knee the picture is untouched: 50 nits is 50/203 of SDR white.
        let dark = out(50.0);
        assert!((dark - srgb(50.0 / 203.0)).abs() < 0.01, "{dark}");
        // The mastering peak lands on SDR white, and past it clips there.
        assert!(out(1000.0) > 0.99);
        assert!(out(4000.0) > 0.99);
        let ramp: Vec<f32> = [100.0, 200.0, 400.0, 600.0, 800.0].map(&mut out).to_vec();
        assert!(ramp.windows(2).all(|w| w[0] < w[1]), "monotonic: {ramp:?}");
        // Highlights roll off: 400 nits isn't twice 200 nits' brightness.
        assert!(ramp[2] < 1.0 && ramp[1] > srgb(150.0 / 203.0), "{ramp:?}");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn hdr_output_keeps_the_signal_past_sdr_white() {
        let (device, queue) = device();
        let mut conv = Converter::new(device, queue);
        let mut out = |nits: f64| {
            let s = conv.convert(&grey(pq_code(nits)), true).unwrap();
            assert_eq!(s._texture.0.format(), wgpu::TextureFormat::Rgba16Float);
            first_pixel(&conv, &s)[1]
        };
        let near = |a: f32, b: f32| (a - b).abs() < 0.02 * b.max(0.1);
        assert!(near(out(50.0), srgb(50.0 / 203.0)));
        assert!(near(out(203.0), 1.0));
        // dfui's present pass turns 1.9953 back into 1000 nits.
        let peak = out(1000.0);
        assert!(near(peak, srgb(1000.0 / 203.0)), "{peak}");
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn hdr10_dmabuf_and_copy_convert_alike() {
        let (device, queue) = device();
        let mut conv = Converter::new(device, queue);
        let decode = |copy: bool| -> Vec<Frame> {
            let mut r = Reader::new(&include_bytes!("../tests/hdr10.mp4")[..]).unwrap();
            let config = match &r.tracks[0].kind {
                Kind::Video { config, .. } => config.clone(),
                _ => panic!("no video"),
            };
            let mut dec = Decoder::new(crate::mp4::Codec::Hevc, &config, 256, 144).unwrap();
            if copy {
                dec.copy_frames();
            }
            let mut frames = Vec::new();
            while let Some(s) = r.sample().unwrap() {
                dec.decode(&s.data, s.pts).unwrap();
                frames.extend(std::iter::from_fn(|| dec.frame()));
            }
            frames
        };
        let (direct, copied) = (decode(false), decode(true));
        let dmabuf = matches!(direct[0].planes, Planes::Dmabuf(_));
        eprintln!("zero-copy path used: {dmabuf}");
        for hdr in [false, true] {
            let a = conv.convert(&direct[0], hdr).expect("convert");
            let a = read(&conv.device, &conv.queue, &a._texture.0);
            let b = conv.convert(&copied[0], hdr).expect("convert");
            let b = read(&conv.device, &conv.queue, &b._texture.0);
            assert_eq!(a.len(), b.len());
            let max = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            // A half float's low byte can differ by a rounding step.
            assert!(
                max <= if hdr { 255 } else { 1 },
                "hdr {hdr}: differ by {max}"
            );
            if hdr {
                let px = |v: &[u8]| -> Vec<f32> {
                    v.as_chunks::<2>()
                        .0
                        .iter()
                        .map(|c| half(u16::from_le_bytes(*c)))
                        .collect()
                };
                let worst = px(&a)
                    .iter()
                    .zip(px(&b))
                    .map(|(x, y)| (x - y).abs())
                    .fold(0f32, f32::max);
                assert!(worst < 0.01, "hdr floats differ by {worst}");
            }
        }
    }

    /// The clip's frames in display order, through the converter.
    fn frames(conv: &mut Converter, copy: bool) -> (Vec<Vec<u8>>, bool) {
        let mut r = Reader::new(&include_bytes!("../tests/clip.mp4")[..]).unwrap();
        let (id, avcc) = r
            .tracks
            .iter()
            .find_map(|t| match &t.kind {
                Kind::Video { config, .. } => Some((t.id, config.clone())),
                _ => None,
            })
            .unwrap();
        let mut dec = Decoder::new(crate::mp4::Codec::H264, &avcc, 64, 48).unwrap();
        if copy {
            dec.copy_frames();
        }
        let mut out = Vec::new();
        let mut dmabuf = false;
        let mut take = |dec: &mut Decoder, conv: &mut Converter| {
            while let Some(f) = dec.frame() {
                dmabuf |= matches!(f.planes, Planes::Dmabuf(_));
                let s = conv.convert(&f, false).expect("convert");
                out.push(read(&conv.device, &conv.queue, &s._texture.0));
            }
        };
        while let Some(s) = r.sample().unwrap() {
            if s.track == id {
                dec.decode(&s.data, s.pts).unwrap();
                take(&mut dec, conv);
            }
        }
        dec.finish().unwrap();
        take(&mut dec, conv);
        (out, dmabuf)
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn both_paths_match_ffmpegs_colours() {
        let (device, queue) = device();
        let mut conv = Converter::new(device, queue);
        let (copied, _) = frames(&mut conv, true);
        let (direct, dmabuf) = frames(&mut conv, false);
        eprintln!("zero-copy path used: {dmabuf}");
        assert_eq!((copied.len(), direct.len()), (24, 24));
        let diff = |a: &[u8], b: &[u8]| {
            a.iter()
                .zip(b)
                .map(|(x, y)| x.abs_diff(*y) as u32)
                .max()
                .unwrap()
        };
        for (a, b) in copied.iter().zip(&direct) {
            assert!(diff(a, b) <= 1, "dmabuf and copy differ by {}", diff(a, b));
        }
        // ffmpeg's own conversion of the first frame, where available.
        let Ok(out) = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-i", "tests/clip.mp4", "-frames:v", "1"])
            .args(["-pix_fmt", "rgba", "-f", "rawvideo", "-"])
            .output()
        else {
            return;
        };
        assert_eq!(out.stdout.len(), copied[0].len());
        // Chroma is upsampled differently, which shows only at colour edges; a
        // wrong matrix or range would move every pixel.
        let mut diffs: Vec<u8> = copied[0]
            .iter()
            .zip(&out.stdout)
            .map(|(a, b)| a.abs_diff(*b))
            .collect();
        diffs.sort();
        assert!(
            diffs[diffs.len() / 2] <= 2,
            "median {}",
            diffs[diffs.len() / 2]
        );
    }

    #[test]
    #[ignore = "needs a GPU, and ffmpeg with libplacebo"]
    fn tone_map_matches_libplacebos_bt2390() {
        let Ok(out) = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-init_hw_device", "vulkan", "-i", "tests/hdr10.mp4"])
            .args(["-frames:v", "1", "-vf"])
            .arg(
                "libplacebo=tonemapping=bt.2390:peak_detect=false:gamut_mode=clip:\
                 colorspace=bt709:color_primaries=bt709:color_trc=iec61966-2-1:range=pc:format=rgba",
            )
            .args(["-f", "rawvideo", "-"])
            .output()
        else {
            return;
        };
        if out.stdout.len() != 256 * 144 * 4 {
            eprintln!("no libplacebo: {}", String::from_utf8_lossy(&out.stderr));
            return;
        }
        let (device, queue) = device();
        let mut conv = Converter::new(device, queue);
        let mut r = Reader::new(&include_bytes!("../tests/hdr10.mp4")[..]).unwrap();
        let config = match &r.tracks[0].kind {
            Kind::Video { config, .. } => config.clone(),
            _ => panic!("no video"),
        };
        let mut dec = Decoder::new(crate::mp4::Codec::Hevc, &config, 256, 144).unwrap();
        let frame = loop {
            let s = r.sample().unwrap().unwrap();
            dec.decode(&s.data, s.pts).unwrap();
            if let Some(f) = dec.frame() {
                break f;
            }
        };
        let s = conv.convert(&frame, false).unwrap();
        let ours = read(&conv.device, &conv.queue, &s._texture.0);
        let mut diffs: Vec<u8> = ours
            .iter()
            .zip(&out.stdout)
            .enumerate()
            .filter(|(i, _)| i % 4 != 3)
            .map(|(_, (a, b))| a.abs_diff(*b))
            .collect();
        diffs.sort();
        let at = |q: f32| diffs[((diffs.len() - 1) as f32 * q) as usize];
        eprintln!(
            "vs libplacebo: median {} p90 {} p99 {} max {}",
            at(0.5),
            at(0.9),
            at(0.99),
            at(1.0)
        );
        let mean = |v: &[u8]| v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64;
        eprintln!(
            "mean brightness: ours {:.1} libplacebo {:.1}",
            mean(&ours),
            mean(&out.stdout)
        );
        // Both are BT.2390 from the same 1000-nit peak; they differ in how each
        // applies it (we scale by the brightest channel), not in the overall picture.
        assert!(at(0.5) <= 16, "median {}", at(0.5));
        let (a, b) = (mean(&ours), mean(&out.stdout));
        assert!((a - b).abs() < 0.05 * b, "brightness {a:.1} vs {b:.1}");
    }
}
