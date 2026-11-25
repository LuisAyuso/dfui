//! NV12 frames to RGBA textures dfui draws as bitmaps.

use std::sync::Arc;

use dfui::wgpu;

use crate::{Frame, Planes};

/// Y'CbCr to R'G'B' rows: `rgb[i] = dot(m[i].xyz, yuv) + m[i].w`.
fn matrix(bt709: bool, full: bool) -> [[f32; 4]; 3] {
    let (kr, kb) = if bt709 {
        (0.2126, 0.0722)
    } else {
        (0.299, 0.114)
    };
    let kg = 1.0 - kr - kb;
    let (ys, yo, cs) = if full {
        (1.0, 0.0, 1.0)
    } else {
        (255.0 / 219.0, -16.0 / 219.0, 255.0 / 224.0)
    };
    let c = 128.0 / 255.0;
    let (rv, gu, gv, bu) = (
        2.0 * (1.0 - kr) * cs,
        -2.0 * kb * (1.0 - kb) / kg * cs,
        -2.0 * kr * (1.0 - kr) / kg * cs,
        2.0 * (1.0 - kb) * cs,
    );
    [
        [ys, 0.0, rv, yo - rv * c],
        [ys, gu, gv, yo - (gu + gv) * c],
        [ys, bu, 0.0, yo - bu * c],
    ]
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
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    /// Planes for frames in memory, kept while the size holds.
    planes: Option<(wgpu::Texture, wgpu::Texture)>,
    pool: Vec<Arc<Tex>>,
    /// Dmabuf import failed once, so the decoder should hand over copies.
    pub import_failed: bool,
}

const SHADER: &str = r#"
struct P { r: vec4f, g: vec4f, b: vec4f }
@group(0) @binding(0) var ty: texture_2d<f32>;
@group(0) @binding(1) var tuv: texture_2d<f32>;
@group(0) @binding(2) var s: sampler;
@group(0) @binding(3) var<uniform> p: P;

struct V { @builtin(position) pos: vec4f, @location(0) uv: vec2f }

@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
    let uv = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return V(vec4f(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0), uv);
}

@fragment fn fs(v: V) -> @location(0) vec4f {
    let yuv = vec4f(textureSample(ty, s, v.uv).r, textureSample(tuv, s, v.uv).rg, 1.0);
    return vec4f(clamp(vec3f(dot(p.r, yuv), dot(p.g, yuv), dot(p.b, yuv)), vec3f(0.0), vec3f(1.0)), 1.0);
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

impl Converter {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Converter {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nv12"),
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
            label: Some("nv12"),
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nv12"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nv12 matrix"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Converter {
            device,
            queue,
            pipeline,
            layout,
            sampler,
            params,
            planes: None,
            pool: Vec::new(),
            import_failed: false,
        }
    }

    fn output(&mut self, (w, h): (u32, u32)) -> Arc<Tex> {
        let fits = |t: &Tex| t.0.width() == w && t.0.height() == h;
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
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })));
        self.pool.push(t.clone());
        t
    }

    /// Converts `frame` and waits for the GPU, so the decoder can reuse its
    /// memory as soon as this returns. `None` if a dmabuf couldn't be imported.
    pub fn convert(&mut self, frame: &Frame) -> Option<Shown> {
        let (w, h) = (frame.width, frame.height);
        let chroma = (w.div_ceil(2), h.div_ceil(2));
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
                    .is_none_or(|(p, _)| p.width() != w || p.height() != h)
                {
                    self.planes = Some((
                        plane(&self.device, (w, h), wgpu::TextureFormat::R8Unorm),
                        plane(&self.device, chroma, wgpu::TextureFormat::Rg8Unorm),
                    ));
                }
                let (ty, tuv) = self.planes.as_ref().unwrap();
                write(&self.queue, ty, y, *y_stride);
                write(&self.queue, tuv, uv, *uv_stride);
                (ty, tuv)
            }
            #[cfg(target_os = "linux")]
            Planes::Dmabuf(d) => {
                let import = |p: &crate::DmaPlane, size, format| unsafe {
                    dmabuf::import(&self.device, p, size, format)
                };
                imported = match (
                    import(&d.planes[0], (w, h), wgpu::TextureFormat::R8Unorm),
                    import(&d.planes[1], chroma, wgpu::TextureFormat::Rg8Unorm),
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
        let m = matrix(frame.bt709, frame.full_range);
        self.queue
            .write_buffer(&self.params, 0, bytemuck::cast_slice(&m));
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
        let out = self.output((w, h));
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
            pass.set_pipeline(&self.pipeline);
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
    use super::matrix;

    fn rgb(m: [[f32; 4]; 3], yuv: [f32; 3]) -> [f32; 3] {
        m.map(|r| r[0] * yuv[0] + r[1] * yuv[1] + r[2] * yuv[2] + r[3])
    }

    #[test]
    fn limited_range_black_white_and_red() {
        let m = matrix(true, false);
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01);
        let c = 128.0 / 255.0;
        assert!(near(rgb(m, [16.0 / 255.0, c, c]), [0.0; 3]));
        assert!(near(rgb(m, [235.0 / 255.0, c, c]), [1.0; 3]));
        // BT.709 limited-range red: Y 63, Cb 102, Cr 240.
        assert!(near(
            rgb(m, [63.0 / 255.0, 102.0 / 255.0, 240.0 / 255.0]),
            [1.0, 0.0, 0.0]
        ));
        assert!(near(rgb(matrix(false, true), [0.0, c, c]), [0.0; 3]));
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
            required_features: adapter.features() & wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF,
            ..Default::default()
        }))
        .unwrap()
    }

    fn read(device: &wgpu::Device, queue: &wgpu::Queue, t: &wgpu::Texture) -> Vec<u8> {
        let (w, h) = (t.width(), t.height());
        let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
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
            .flat_map(|y| data[(y * row) as usize..][..(w * 4) as usize].to_vec())
            .collect()
    }

    /// The clip's frames in display order, through the converter.
    fn frames(conv: &mut Converter, copy: bool) -> (Vec<Vec<u8>>, bool) {
        let mut r = Reader::new(&include_bytes!("../tests/clip.mp4")[..]).unwrap();
        let (id, avcc) = r
            .tracks
            .iter()
            .find_map(|t| match &t.kind {
                Kind::Video { avcc, .. } => Some((t.id, avcc.clone())),
                _ => None,
            })
            .unwrap();
        let mut dec = Decoder::new(&avcc, 64, 48).unwrap();
        if copy {
            dec.copy_frames();
        }
        let mut out = Vec::new();
        let mut dmabuf = false;
        let mut take = |dec: &mut Decoder, conv: &mut Converter| {
            while let Some(f) = dec.frame() {
                dmabuf |= matches!(f.planes, Planes::Dmabuf(_));
                let s = conv.convert(&f).expect("convert");
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
}
