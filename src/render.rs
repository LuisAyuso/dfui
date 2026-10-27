use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::text::Text;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };

    /// sRGB `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> Color {
        Color {
            r: ((rgb >> 16) & 0xff) as f32 / 255.0,
            g: ((rgb >> 8) & 0xff) as f32 / 255.0,
            b: (rgb & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }

    pub const fn alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }

    fn linear(self) -> [f32; 4] {
        let f = |c: f32| {
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        [f(self.r), f(self.g), f(self.b), self.a]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = (self.x + self.w).min(o.x + o.w);
        let b = (self.y + self.h).min(o.y + o.h);
        Rect::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Shadow {
    pub color: Color,
    pub offset: (f32, f32),
    pub blur: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Instance {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    border_color: [f32; 4],
    params: [f32; 4],
    clip: [f32; 4],
}

/// Flat list of distance-field primitives for one frame, in logical pixels.
#[derive(Default)]
pub struct Scene {
    pub(crate) instances: Vec<Instance>,
}

impl Scene {
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn shape(
        &mut self,
        r: Rect,
        radius: f32,
        fill: Color,
        border: f32,
        border_color: Color,
        clip: Rect,
    ) {
        self.instances.push(Instance {
            rect: [r.x, r.y, r.w, r.h],
            uv: [0.0; 4],
            color: fill.linear(),
            border_color: border_color.linear(),
            params: [radius, border, 0.0, 0.0],
            clip: [clip.x, clip.y, clip.w, clip.h],
        });
    }

    pub fn shadow(&mut self, r: Rect, radius: f32, s: Shadow, clip: Rect) {
        self.instances.push(Instance {
            rect: [r.x + s.offset.0, r.y + s.offset.1, r.w, r.h],
            uv: [0.0; 4],
            color: s.color.linear(),
            border_color: [0.0; 4],
            params: [radius, 0.0, s.blur.max(0.01), 0.0],
            clip: [clip.x, clip.y, clip.w, clip.h],
        });
    }

    pub(crate) fn glyph(
        &mut self,
        r: Rect,
        em: [f32; 4],
        color: Color,
        clip: Rect,
        bands: [f32; 3],
    ) {
        self.instances.push(Instance {
            rect: [r.x, r.y, r.w, r.h],
            uv: em,
            color: color.linear(),
            border_color: [0.0; 4],
            params: [bands[0], bands[1], bands[2], 1.0],
            clip: [clip.x, clip.y, clip.w, clip.h],
        });
    }
}

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

impl Gpu {
    fn new(adapter: &wgpu::Adapter, format: wgpu::TextureFormat) -> Gpu {
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            // Outline text needs fragment-stage storage buffers (GLES 3.1+/Vulkan).
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            ..Default::default()
        }))
        .expect("request device");

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

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl), Some(&outline_layout)],
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
        Gpu {
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
            timer,
        }
    }

    /// `viewport` is in logical pixels; the target's physical size sets the DPI.
    fn draw(
        &mut self,
        target: &wgpu::TextureView,
        viewport: [f32; 2],
        scene: &Scene,
        text: &mut Text,
        clear: Color,
    ) {
        let o = &mut text.outlines;
        if o.dirty {
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
            o.dirty = false;
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
        let timing = self.timer.as_ref().filter(|t| !t.busy);
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let [r, g, b, a] = clear.linear();
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: r as f64,
                            g: g as f64,
                            b: b as f64,
                            a: a as f64,
                        }),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &self.outline_group, &[]);
            pass.set_vertex_buffer(0, self.instances.slice(..));
            pass.draw(0..6, 0..scene.instances.len() as u32);
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

pub(crate) struct Renderer {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: Gpu,
    pub clear: Color,
}

impl Renderer {
    pub fn new(instance: &wgpu::Instance, window: Arc<Window>) -> Renderer {
        let surface = instance
            .create_surface(window.clone())
            .expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no GPU adapter");
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let gpu = Gpu::new(&adapter, format);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&gpu.device, &config);
        Renderer {
            window,
            surface,
            config,
            gpu,
            clear: Color::WHITE,
        }
    }

    pub fn window(&self) -> &Window {
        &self.window
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.config.width = w.max(1);
        self.config.height = h.max(1);
        self.surface.configure(&self.gpu.device, &self.config);
    }

    /// GPU time of the most recently measured frame, if timestamp queries are supported.
    pub fn gpu_ms(&self) -> Option<f32> {
        self.gpu.timer.as_ref().map(|t| t.last_ns / 1e6)
    }

    /// Returns how long acquiring the frame blocked (vsync back-pressure).
    pub fn render(&mut self, scene: &Scene, text: &mut Text) -> std::time::Duration {
        let acquire = std::time::Instant::now();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.gpu.device, &self.config);
                self.window.request_redraw();
                return acquire.elapsed();
            }
            _ => return acquire.elapsed(),
        };
        let waited = acquire.elapsed();
        let scale = self.window.scale_factor() as f32;
        let viewport = [
            self.config.width as f32 / scale,
            self.config.height as f32 / scale,
        ];
        let view = frame.texture.create_view(&Default::default());
        self.gpu.draw(&view, viewport, scene, text, self.clear);
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
        waited
    }
}

/// Renders a scene offscreen and reads it back as sRGB RGBA8.
pub(crate) fn render_offscreen(
    scene: &Scene,
    text: &mut Text,
    size: (u32, u32),
    scale: f32,
) -> image::RgbaImage {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter =
        pollster::block_on(instance.request_adapter(&Default::default())).expect("no GPU adapter");
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let mut gpu = Gpu::new(&adapter, format);
    let (w, h) = size;
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    gpu.draw(
        &target.create_view(&Default::default()),
        [w as f32 / scale, h as f32 / scale],
        scene,
        text,
        Color::WHITE,
    );

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
    let mut img = image::RgbaImage::new(w, h);
    for y in 0..h {
        let src = &data[(y * row) as usize..][..(w * 4) as usize];
        img.as_mut()[(y * w * 4) as usize..][..(w * 4) as usize].copy_from_slice(src);
    }
    img
}
