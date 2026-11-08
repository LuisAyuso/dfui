use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::text::Text;

use super::*;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 2],
    _pad: [f32; 2],
}

/// Device-level state shared by window and offscreen rendering.
pub(crate) struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instances: wgpu::Buffer,
    outline_layout: wgpu::BindGroupLayout,
    outline_group: wgpu::BindGroup,
    curves: wgpu::Buffer,
    bands: wgpu::Buffer,
    image_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Bound for instances that sample no bitmap.
    no_image: wgpu::BindGroup,
    /// Uploaded bitmaps by id; an entry goes once its bitmap is dropped.
    textures: HashMap<u32, (Weak<BitmapData>, wgpu::BindGroup)>,
    /// `curves`/`bands` hold nothing yet; `Outlines::dirty` only tracks
    /// uploads to whichever device came before, e.g. across an Android resume.
    empty: bool,
    timer: Option<GpuTimer>,
}

/// Measures GPU time of the render pass with timestamp queries, one sample in flight.
struct GpuTimer {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    busy: bool,
    mapped: Arc<AtomicBool>,
    last_ns: f32,
}

impl GpuTimer {
    fn new(device: &wgpu::Device) -> GpuTimer {
        let buf = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 16,
                usage,
                mapped_at_creation: false,
            })
        };
        GpuTimer {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame timer"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: buf(
                "timer resolve",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
            read: buf(
                "timer read",
                wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            ),
            busy: false,
            mapped: Arc::new(AtomicBool::new(false)),
            last_ns: 0.0,
        }
    }
}

fn storage(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size.max(256).next_power_of_two(),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn outline_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    curves: &wgpu::Buffer,
    bands: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("outlines"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: curves.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: bands.as_entire_binding(),
            },
        ],
    })
}

/// Uploads `rgba` (premultiplied) as a texture and binds it with `sampler`.
fn image_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    (width, height): (u32, u32),
    rgba: &[u8],
) -> wgpu::BindGroup {
    let texture = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("bitmap"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Not sRGB: samples blend in gamma space like every other color.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        rgba,
    );
    let view = texture.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bitmap"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

impl Gpu {
    async fn new(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> Result<Gpu, String> {
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                // Outline text needs fragment-stage storage buffers (GLES 3.1+/Vulkan/WebGPU).
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits()),
                required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("the GPU can't run dfui: {e}"))?;

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });

        let storage_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let outline_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("outlines"),
            entries: &[storage_entry(0), storage_entry(1)],
        });
        let curves = storage(&device, "curves", 0);
        let bands = storage(&device, "bands", 0);
        let outline_group = outline_group(&device, &outline_layout, &curves, &bands);

        let image_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bitmap"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // ponytail: no mipmaps, so bitmaps drawn far below their size alias; decode at display size.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bitmap"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let no_image = image_group(&device, &queue, &image_layout, &sampler, (1, 1), &[0; 4]);

        let shader = device.create_shader_module(wgpu::include_wgsl!("../shader.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl), Some(&outline_layout), Some(&image_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sdf"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x4, 1 => Float32x4, 2 => Float32x4,
                        3 => Float32x4, 4 => Float32x4, 5 => Float32x4,
                    ],
                })],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let instances = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("instances"),
            contents: &[0; size_of::<Instance>()],
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let timer = device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| GpuTimer::new(&device));
        Ok(Gpu {
            device,
            queue,
            pipeline,
            globals,
            bind_group,
            instances,
            outline_layout,
            outline_group,
            curves,
            bands,
            image_layout,
            sampler,
            no_image,
            textures: HashMap::new(),
            empty: true,
            timer,
        })
    }

    /// Reads back the last timestamp sample if its mapping has completed.
    fn collect_timer(&mut self) {
        let period = self.queue.get_timestamp_period();
        if let Some(t) = &mut self.timer
            && t.mapped.swap(false, Ordering::Acquire)
        {
            if let Ok(data) = t.read.slice(..).get_mapped_range() {
                let ts: &[u64] = bytemuck::cast_slice(&data);
                t.last_ns = ts[1].saturating_sub(ts[0]) as f32 * period;
            }
            t.read.unmap();
            t.busy = false;
        }
    }

    /// `viewport` is in logical pixels; the target's physical size sets the DPI.
    /// With a `scissor` (physical px) only that area is redrawn over the target's
    /// existing contents, so the scene must paint its own background.
    fn draw(
        &mut self,
        target: &wgpu::TextureView,
        viewport: [f32; 2],
        scene: &Scene,
        text: &mut Text,
        scissor: Option<[u32; 4]>,
    ) {
        let o = &mut text.outlines;
        if o.dirty || self.empty {
            let (cb, bb): (&[u8], &[u8]) = (
                bytemuck::cast_slice(&o.curves),
                bytemuck::cast_slice(&o.bands),
            );
            if cb.len() as u64 > self.curves.size() || bb.len() as u64 > self.bands.size() {
                self.curves = storage(&self.device, "curves", cb.len() as u64);
                self.bands = storage(&self.device, "bands", bb.len() as u64);
                self.outline_group = outline_group(
                    &self.device,
                    &self.outline_layout,
                    &self.curves,
                    &self.bands,
                );
            }
            // ponytail: re-uploads all outlines when any glyph is added; append-only writes if fonts get big.
            self.queue.write_buffer(&self.curves, 0, cb);
            self.queue.write_buffer(&self.bands, 0, bb);
            (o.dirty, o.transient_dirty, self.empty) = (false, false, false);
        } else if o.transient_dirty {
            let (c, b) = o.transient_data();
            self.queue
                .write_buffer(&self.curves, 0, bytemuck::cast_slice(c));
            self.queue
                .write_buffer(&self.bands, 0, bytemuck::cast_slice(b));
            o.transient_dirty = false;
        }

        let globals = Globals {
            viewport,
            _pad: [0.0; 2],
        };
        self.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        let bytes: &[u8] = bytemuck::cast_slice(&scene.instances);
        if bytes.len() as u64 > self.instances.size() {
            self.instances = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instances"),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        self.queue.write_buffer(&self.instances, 0, bytes);

        self.textures.retain(|_, (b, _)| b.strong_count() > 0);
        let max = self.device.limits().max_texture_dimension_2d;
        for (_, b) in &scene.bitmaps {
            let d = &b.0;
            self.textures.entry(d.id).or_insert_with(|| {
                let group = if d.width.max(d.height) > max || d.width == 0 || d.height == 0 {
                    log::warn!("bitmap {}x{} exceeds {max}px, not drawn", d.width, d.height);
                    self.no_image.clone()
                } else {
                    image_group(
                        &self.device,
                        &self.queue,
                        &self.image_layout,
                        &self.sampler,
                        (d.width, d.height),
                        &d.rgba,
                    )
                };
                (Arc::downgrade(&b.0), group)
            });
        }

        self.collect_timer();
        let timing = self.timer.as_ref().filter(|t| !t.busy);
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: match scissor {
                            Some(_) => wgpu::LoadOp::Load,
                            None => wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: timing.map(|t| wgpu::RenderPassTimestampWrites {
                    query_set: &t.set,
                    beginning_of_pass_write_index: Some(0),
                    end_of_pass_write_index: Some(1),
                }),
                ..Default::default()
            });
            if let Some([x, y, w, h]) = scissor {
                pass.set_scissor_rect(x, y, w, h);
            }
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &self.outline_group, &[]);
            pass.set_vertex_buffer(0, self.instances.slice(..));
            // One draw per image, and one for each run of other instances
            // between them, so painter's order holds.
            let mut next = 0;
            for (i, b) in &scene.bitmaps {
                let i = *i as u32;
                if i > next {
                    pass.set_bind_group(2, &self.no_image, &[]);
                    pass.draw(0..24, next..i);
                }
                pass.set_bind_group(2, &self.textures[&b.0.id].1, &[]);
                pass.draw(0..24, i..i + 1);
                next = i + 1;
            }
            let n = scene.instances.len() as u32;
            if n > next {
                pass.set_bind_group(2, &self.no_image, &[]);
                pass.draw(0..24, next..n);
            }
        }
        if let Some(t) = timing {
            enc.resolve_query_set(&t.set, 0..2, &t.resolve, 0);
            enc.copy_buffer_to_buffer(&t.resolve, 0, &t.read, 0, 16);
        }
        self.queue.submit([enc.finish()]);
        if let Some(t) = &mut self.timer
            && !t.busy
        {
            t.busy = true;
            let mapped = t.mapped.clone();
            t.read.slice(..).map_async(wgpu::MapMode::Read, move |r| {
                mapped.store(r.is_ok(), Ordering::Release);
            });
        }
        let _ = self.device.poll(wgpu::PollType::Poll);
    }
}

/// The platform window a `Renderer` draws into.
pub(crate) trait Host {
    fn surface(
        &self,
        instance: &wgpu::Instance,
    ) -> Result<wgpu::Surface<'static>, wgpu::CreateSurfaceError>;
    /// Physical px.
    fn size(&self) -> (u32, u32);
    fn scale(&self) -> f32;
    fn request_redraw(&self);
    fn pre_present(&self) {}
    fn refresh_mhz(&self) -> Option<u32> {
        None
    }
    /// Shows the given soft keyboard, or hides it.
    fn set_ime(&self, _keyboard: Option<crate::widget::Keyboard>) {}
}

pub(crate) struct Renderer {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    /// Retained frame for partial redraws; swapchain images don't keep their
    /// contents, so we redraw damage here and copy the whole canvas out.
    /// `None` when the surface can't be a copy destination: always full redraws.
    canvas: Option<wgpu::Texture>,
    /// Canvas contents are invalid (new or resized); next frame must be full.
    fresh: bool,
    /// `DFUI_SHOW_DAMAGE` debug overlay: frame counter cycling the tint color.
    show_damage: Option<usize>,
    /// Last presented frame carries the overlay tint; next one must present.
    tinted: bool,
    /// Last, so it's dropped after the surface drawing into it.
    window: Box<dyn Host>,
}

fn canvas(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("canvas"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: config.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &config.view_formats,
    })
}

/// `d` (logical px) rounded out to physical pixels with a 2px margin for AA
/// fringes, within a `size` target; zero-sized when nothing of it is inside.
pub(super) fn scissor(d: Rect, scale: f32, (w, h): (u32, u32)) -> [u32; 4] {
    if d.w <= 0.0 || d.h <= 0.0 {
        return [0; 4];
    }
    let x0 = ((d.x - 2.0) * scale).floor().clamp(0.0, w as f32) as u32;
    let y0 = ((d.y - 2.0) * scale).floor().clamp(0.0, h as f32) as u32;
    let x1 = ((d.x + d.w + 2.0) * scale).ceil().clamp(0.0, w as f32) as u32;
    let y1 = ((d.y + d.h + 2.0) * scale).ceil().clamp(0.0, h as f32) as u32;
    [x0, y0, x1 - x0, y1 - y0]
}

impl Renderer {
    /// Async because adapter and device requests are; on native they resolve
    /// on first poll, on the web they wait for the browser.
    pub async fn new(instance: wgpu::Instance, window: Box<dyn Host>) -> Result<Renderer, String> {
        let surface = window
            .surface(&instance)
            .map_err(|e| format!("can't draw to this window: {e}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|e| format!("no usable GPU: {e}"))?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        // Blend in sRGB space like browsers do; linear blending makes AA edges look thin and stepped.
        let view_format = format.remove_srgb_suffix();
        let gpu = Gpu::new(&adapter, view_format).await?;
        let (width, height) = window.size();
        let copyable = caps.usages.contains(wgpu::TextureUsages::COPY_DST);
        let config = wgpu::SurfaceConfiguration {
            usage: if copyable {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            },
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: if view_format == format {
                vec![]
            } else {
                vec![view_format]
            },
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&gpu.device, &config);
        let canvas = copyable.then(|| canvas(&gpu.device, &config));
        Ok(Renderer {
            surface,
            config,
            gpu,
            canvas,
            fresh: true,
            show_damage: std::env::var_os("DFUI_SHOW_DAMAGE").map(|_| 0),
            tinted: false,
            window,
        })
    }

    pub fn window(&self) -> &dyn Host {
        &*self.window
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.config.width = w.max(1);
        self.config.height = h.max(1);
        self.surface.configure(&self.gpu.device, &self.config);
        if self.canvas.is_some() {
            self.canvas = Some(canvas(&self.gpu.device, &self.config));
            self.fresh = true;
        }
    }

    /// GPU time of the most recently measured frame, if timestamp queries are supported.
    pub fn gpu_ms(&self) -> Option<f32> {
        self.gpu.timer.as_ref().map(|t| t.last_ns / 1e6)
    }

    /// Redraws `damage` (logical px; empty redraws nothing), or everything when
    /// `None`.
    pub fn render(&mut self, scene: &Scene, text: &mut Text, damage: Option<Rect>) -> Outcome {
        let scale = self.window.scale();
        let viewport = [
            self.config.width as f32 / scale,
            self.config.height as f32 / scale,
        ];
        // Content can lie past the window edge; only what's on screen counts.
        let damage = damage.map(|d| d.intersect(&Rect::new(0.0, 0.0, viewport[0], viewport[1])));
        // The window already shows the canvas: nothing to acquire, copy or present.
        let empty = damage.is_some_and(|d| d.w <= 0.0 || d.h <= 0.0);
        if empty && self.canvas.is_some() && !self.fresh && !self.tinted {
            return Outcome::Idle;
        }
        let acquire = web_time::Instant::now();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.gpu.device, &self.config);
                self.window.request_redraw();
                return Outcome::Failed;
            }
            wgpu::CurrentSurfaceTexture::Timeout => {
                self.window.request_redraw();
                return Outcome::Failed;
            }
            _ => return Outcome::Failed,
        };
        let waited = acquire.elapsed();
        let view_desc = wgpu::TextureViewDescriptor {
            format: Some(self.config.format.remove_srgb_suffix()),
            ..Default::default()
        };
        let (w, h) = (self.config.width, self.config.height);
        // Area actually redrawn, physical px; `None` is the whole frame.
        let drawn = match &self.canvas {
            None => {
                let view = frame.texture.create_view(&view_desc);
                self.gpu.draw(&view, viewport, scene, text, None);
                None
            }
            Some(canvas) => {
                let scissor = damage
                    .filter(|_| !self.fresh)
                    .map(|d| scissor(d, scale, (w, h)));
                if scissor.is_none_or(|[_, _, sw, sh]| sw > 0 && sh > 0) {
                    let view = canvas.create_view(&view_desc);
                    self.gpu.draw(&view, viewport, scene, text, scissor);
                }
                let mut enc = self.gpu.device.create_command_encoder(&Default::default());
                enc.copy_texture_to_texture(
                    canvas.as_image_copy(),
                    frame.texture.as_image_copy(),
                    canvas.size(),
                );
                self.gpu.queue.submit([enc.finish()]);
                self.fresh = false;
                scissor
            }
        };
        // Tints the swapchain image only, so the canvas stays clean and each
        // frame shows just its own damage.
        self.tinted = false;
        if let Some(n) = &mut self.show_damage
            && drawn.is_none_or(|[_, _, dw, dh]| dw > 0 && dh > 0)
        {
            const TINTS: [u32; 4] = [0xff3b30, 0x34c759, 0x007aff, 0xffcc00];
            *n += 1;
            let r = drawn.map_or(
                Rect::new(0.0, 0.0, viewport[0], viewport[1]),
                |[x, y, dw, dh]| {
                    Rect::new(
                        x as f32 / scale,
                        y as f32 / scale,
                        dw as f32 / scale,
                        dh as f32 / scale,
                    )
                },
            );
            let mut overlay = Scene::default();
            let tint = Color::hex(TINTS[*n % TINTS.len()]).alpha(0.3);
            overlay.shape(r, 0.0, tint, 0.0, Color::TRANSPARENT, r);
            let view = frame.texture.create_view(&view_desc);
            self.gpu
                .draw(&view, viewport, &overlay, text, Some([0, 0, w, h]));
            // One clean follow-up frame (empty damage) so the tint only flashes.
            // Without a canvas every frame is full, so that would loop.
            if self.canvas.is_some() {
                self.tinted = true;
                self.window.request_redraw();
            }
        }
        self.window.pre_present();
        self.gpu.queue.present(frame);
        Outcome::Presented(waited)
    }
}

/// Offscreen render target with its own device, for snapshots and benches.
pub(crate) struct Offscreen {
    gpu: Gpu,
    target: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Offscreen {
    pub(crate) fn new(size: (u32, u32)) -> Offscreen {
        // Vulkan loaders and drivers (NVIDIA's) crash creating instances on
        // several threads at once, as parallel tests do.
        static SETUP: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let gpu = {
            let _setup = SETUP.lock().unwrap_or_else(|e| e.into_inner());
            let instance = wgpu::Instance::new(
                wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
            );
            let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
                .expect("no GPU adapter");
            pollster::block_on(Gpu::new(&adapter, format)).expect("request device")
        };
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        Offscreen { gpu, target, view }
    }

    pub(crate) fn draw(
        &mut self,
        scene: &Scene,
        text: &mut Text,
        scale: f32,
        scissor: Option<[u32; 4]>,
    ) {
        let s = self.target.size();
        let viewport = [s.width as f32 / scale, s.height as f32 / scale];
        self.gpu.draw(&self.view, viewport, scene, text, scissor);
    }

    /// Draws and waits for the GPU; returns the render pass's GPU time from
    /// timestamp queries, or wall time when the adapter lacks them.
    pub(crate) fn timed_draw(
        &mut self,
        scene: &Scene,
        text: &mut Text,
        scale: f32,
        scissor: Option<[u32; 4]>,
    ) -> std::time::Duration {
        let start = web_time::Instant::now();
        self.draw(scene, text, scale, scissor);
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
        let wall = start.elapsed();
        self.gpu.collect_timer();
        match &self.gpu.timer {
            Some(t) => std::time::Duration::from_nanos(t.last_ns as u64),
            None => wall,
        }
    }

    /// Reads the target back as sRGB RGBA8.
    pub(crate) fn read(&self) -> Pixels {
        let gpu = &self.gpu;
        let s = self.target.size();
        let (w, h) = (s.width, s.height);
        let target = &self.target;
        let row = (w * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([enc.finish()]);
        buf.slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.expect("map readback"));
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
        let data = buf.slice(..).get_mapped_range().expect("mapped range");
        let mut rgba = vec![0; (w * h * 4) as usize];
        for y in 0..h {
            let src = &data[(y * row) as usize..][..(w * 4) as usize];
            rgba[(y * w * 4) as usize..][..(w * 4) as usize].copy_from_slice(src);
        }
        Pixels {
            width: w,
            height: h,
            rgba,
        }
    }
}

/// Renders scenes offscreen in order, each optionally scissored, and reads
/// the result back as sRGB RGBA8.
pub(crate) fn render_offscreen(
    passes: &[(&Scene, Option<[u32; 4]>)],
    text: &mut Text,
    size: (u32, u32),
    scale: f32,
) -> Pixels {
    let mut off = Offscreen::new(size);
    for &(scene, scissor) in passes {
        off.draw(scene, text, scale, scissor);
    }
    off.read()
}
