use std::collections::{HashMap, HashSet};
use std::ops::Range;
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

    fn rgba(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
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

    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let r = (self.x + self.w).max(o.x + o.w);
        let b = (self.y + self.h).max(o.y + o.h);
        Rect::new(x, y, r - x, b - y)
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

pub(crate) enum Outcome {
    /// Drawn and presented; how long acquiring the frame blocked (vsync).
    Presented(std::time::Duration),
    /// Nothing changed, so nothing was presented.
    Idle,
    /// No frame could be acquired; the canvas is untouched.
    Failed,
}

/// Which instances each node painted, by node id.
pub(crate) type Spans = Vec<(u64, Range<usize>)>;

impl Scene {
    pub fn len(&self) -> usize {
        self.instances.len()
    }

    /// Pixels the instances can touch: quad plus AA/blur padding, clipped.
    fn bounds(&self, r: Range<usize>) -> Option<Rect> {
        self.instances[r]
            .iter()
            .map(|i| {
                let [x, y, w, h] = i.rect;
                let glyph = i.params[3] > 0.5;
                let pad = if glyph { 1.0 } else { i.params[2] + 1.0 };
                let [cx, cy, cw, ch] = i.clip;
                Rect::new(x - pad, y - pad, w + 2.0 * pad, h + 2.0 * pad)
                    .intersect(&Rect::new(cx, cy, cw, ch))
            })
            .reduce(|a, b| a.union(&b))
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
            color: fill.rgba(),
            border_color: border_color.rgba(),
            params: [radius, border, 0.0, 0.0],
            clip: [clip.x, clip.y, clip.w, clip.h],
        });
    }

    pub fn shadow(&mut self, r: Rect, radius: f32, s: Shadow, clip: Rect) {
        self.instances.push(Instance {
            rect: [r.x + s.offset.0, r.y + s.offset.1, r.w, r.h],
            uv: [0.0; 4],
            color: s.color.rgba(),
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
            color: color.rgba(),
            border_color: [0.0; 4],
            params: [bands[0], bands[1], bands[2], 1.0],
            clip: [clip.x, clip.y, clip.w, clip.h],
        });
    }
}

/// Area that differs between two painted frames, or `None` if nothing does.
/// Nodes are matched by id, so inserting one doesn't damage its later siblings.
// ponytail: a pure z-order swap with identical instances isn't detected; hash sibling order if that ever matters.
pub(crate) fn damage(
    old: &Scene,
    old_spans: &Spans,
    new: &Scene,
    new_spans: &Spans,
) -> Option<Rect> {
    fn bytes<'a>(s: &'a Scene, r: &Range<usize>) -> &'a [u8] {
        bytemuck::cast_slice(&s.instances[r.clone()])
    }
    let prev: HashMap<u64, &Range<usize>> = old_spans.iter().map(|(id, r)| (*id, r)).collect();
    let mut seen = HashSet::new();
    let mut out: Option<Rect> = None;
    let mut add = |r: Option<Rect>| {
        if let Some(r) = r {
            out = Some(out.map_or(r, |o| o.union(&r)));
        }
    };
    for (id, r) in new_spans {
        seen.insert(*id);
        match prev.get(id) {
            Some(o) if bytes(old, o) == bytes(new, r) => {}
            Some(o) => {
                add(old.bounds((*o).clone()));
                add(new.bounds(r.clone()));
            }
            None => add(new.bounds(r.clone())),
        }
    }
    for (id, r) in old_spans {
        if !seen.contains(id) {
            add(old.bounds(r.clone()));
        }
    }
    out
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
        clear: Color,
        scissor: Option<[u32; 4]>,
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

        self.collect_timer();
        let timing = self.timer.as_ref().filter(|t| !t.busy);
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let [r, g, b, a] = clear.rgba();
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: match scissor {
                            Some(_) => wgpu::LoadOp::Load,
                            None => wgpu::LoadOp::Clear(wgpu::Color {
                                r: r as f64,
                                g: g as f64,
                                b: b as f64,
                                a: a as f64,
                            }),
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
    pub clear: Color,
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
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        // Blend in sRGB space like browsers do; linear blending makes AA edges look thin and stepped.
        let view_format = format.remove_srgb_suffix();
        let gpu = Gpu::new(&adapter, view_format);
        let size = window.inner_size();
        let copyable = caps.usages.contains(wgpu::TextureUsages::COPY_DST);
        let config = wgpu::SurfaceConfiguration {
            usage: if copyable {
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_DST
            } else {
                wgpu::TextureUsages::RENDER_ATTACHMENT
            },
            format,
            width: size.width.max(1),
            height: size.height.max(1),
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
        Renderer {
            window,
            surface,
            config,
            gpu,
            canvas,
            fresh: true,
            show_damage: std::env::var_os("DFUI_SHOW_DAMAGE").map(|_| 0),
            tinted: false,
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
        // The window already shows the canvas: nothing to acquire, copy or present.
        let empty = damage.is_some_and(|d| d.w <= 0.0 || d.h <= 0.0);
        if empty && self.canvas.is_some() && !self.fresh && !self.tinted {
            return Outcome::Idle;
        }
        let acquire = std::time::Instant::now();
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.gpu.device, &self.config);
                self.window.request_redraw();
                return Outcome::Failed;
            }
            _ => return Outcome::Failed,
        };
        let waited = acquire.elapsed();
        let scale = self.window.scale_factor() as f32;
        let viewport = [
            self.config.width as f32 / scale,
            self.config.height as f32 / scale,
        ];
        let view_desc = wgpu::TextureViewDescriptor {
            format: Some(self.config.format.remove_srgb_suffix()),
            ..Default::default()
        };
        let (w, h) = (self.config.width, self.config.height);
        // Area actually redrawn, physical px; `None` is the whole frame.
        let drawn = match &self.canvas {
            None => {
                let view = frame.texture.create_view(&view_desc);
                self.gpu
                    .draw(&view, viewport, scene, text, self.clear, None);
                None
            }
            Some(canvas) => {
                let scissor = damage.filter(|_| !self.fresh).map(|d| {
                    if d.w <= 0.0 || d.h <= 0.0 {
                        return [0; 4];
                    }
                    // Round out to whole pixels; 2px margin covers AA fringes.
                    let x0 = ((d.x - 2.0) * scale).floor().clamp(0.0, w as f32) as u32;
                    let y0 = ((d.y - 2.0) * scale).floor().clamp(0.0, h as f32) as u32;
                    let x1 = ((d.x + d.w + 2.0) * scale).ceil().clamp(0.0, w as f32) as u32;
                    let y1 = ((d.y + d.h + 2.0) * scale).ceil().clamp(0.0, h as f32) as u32;
                    [x0, y0, (x1 - x0).max(1), (y1 - y0).max(1)]
                });
                if scissor.is_none_or(|[_, _, sw, sh]| sw > 0 && sh > 0) {
                    let view = canvas.create_view(&view_desc);
                    self.gpu
                        .draw(&view, viewport, scene, text, self.clear, scissor);
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
            self.gpu.draw(
                &view,
                viewport,
                &overlay,
                text,
                self.clear,
                Some([0, 0, w, h]),
            );
            // One clean follow-up frame (empty damage) so the tint only flashes.
            // Without a canvas every frame is full, so that would loop.
            if self.canvas.is_some() {
                self.tinted = true;
                self.window.request_redraw();
            }
        }
        self.window.pre_present_notify();
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
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
            .expect("no GPU adapter");
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let gpu = Gpu::new(&adapter, format);
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
        self.gpu
            .draw(&self.view, viewport, scene, text, Color::WHITE, scissor);
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
        let start = std::time::Instant::now();
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
    pub(crate) fn read(&self) -> image::RgbaImage {
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
        let mut img = image::RgbaImage::new(w, h);
        for y in 0..h {
            let src = &data[(y * row) as usize..][..(w * 4) as usize];
            img.as_mut()[(y * w * 4) as usize..][..(w * 4) as usize].copy_from_slice(src);
        }
        img
    }
}

/// Renders scenes offscreen in order, each optionally scissored, and reads
/// the result back as sRGB RGBA8.
pub(crate) fn render_offscreen(
    passes: &[(&Scene, Option<[u32; 4]>)],
    text: &mut Text,
    size: (u32, u32),
    scale: f32,
) -> image::RgbaImage {
    let mut off = Offscreen::new(size);
    for &(scene, scissor) in passes {
        off.draw(scene, text, scale, scissor);
    }
    off.read()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(c: Color) -> Scene {
        let mut s = Scene::default();
        let all = Rect::new(0.0, 0.0, 40.0, 40.0);
        s.shape(all, 0.0, c, 0.0, Color::TRANSPARENT, all);
        s
    }

    #[test]
    #[ignore = "needs a GPU"]
    fn scissored_pass_only_touches_damage() {
        let (red, blue) = (solid(Color::hex(0xff0000)), solid(Color::hex(0x0000ff)));
        let passes = [(&red, None), (&blue, Some([10, 10, 20, 20]))];
        let img = render_offscreen(&passes, &mut Text::default(), (40, 40), 1.0);
        assert_eq!(img.get_pixel(5, 5).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(15, 15).0, [0, 0, 255, 255]);
        assert_eq!(img.get_pixel(35, 35).0, [255, 0, 0, 255]);
    }

    const ALL: Rect = Rect::new(-1e4, -1e4, 2e4, 2e4);

    /// One square node per `(id, x, color)`, 10px wide, at y = 0.
    fn frame(nodes: &[(u64, f32, u32)]) -> (Scene, Spans) {
        let mut s = Scene::default();
        let mut spans = Vec::new();
        for &(id, x, c) in nodes {
            let start = s.len();
            let r = Rect::new(x, 0.0, 10.0, 10.0);
            s.shape(r, 0.0, Color::hex(c), 0.0, Color::TRANSPARENT, ALL);
            spans.push((id, start..s.len()));
        }
        (s, spans)
    }

    fn diff(a: &[(u64, f32, u32)], b: &[(u64, f32, u32)]) -> Option<Rect> {
        let ((sa, na), (sb, nb)) = (frame(a), frame(b));
        damage(&sa, &na, &sb, &nb)
    }

    #[test]
    fn damage_tracks_changed_nodes_by_id() {
        let base = [(1, 0.0, 0xff0000), (2, 20.0, 0x00ff00)];
        assert_eq!(diff(&base, &base), None);
        // Recolor: just that node, padded 1px for AA.
        let recolored = [(1, 0.0, 0xff0000), (2, 20.0, 0x0000ff)];
        assert_eq!(
            diff(&base, &recolored),
            Some(Rect::new(19.0, -1.0, 12.0, 12.0))
        );
        // Move: old and new position.
        let moved = [(1, 0.0, 0xff0000), (2, 40.0, 0x00ff00)];
        assert_eq!(diff(&base, &moved), Some(Rect::new(19.0, -1.0, 32.0, 12.0)));
        // Insert before node 2 shifts instance indices but not node 2.
        let inserted = [(1, 0.0, 0xff0000), (3, 60.0, 0x0), (2, 20.0, 0x00ff00)];
        assert_eq!(
            diff(&base, &inserted),
            Some(Rect::new(59.0, -1.0, 12.0, 12.0))
        );
        // Removal damages where the node was.
        assert_eq!(
            diff(&base, &base[..1]),
            Some(Rect::new(19.0, -1.0, 12.0, 12.0))
        );
    }

    #[test]
    fn damage_bounds_include_blur_and_respect_clip() {
        let mut a = Scene::default();
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        let sh = Shadow {
            color: Color::BLACK,
            offset: (0.0, 0.0),
            blur: 4.0,
        };
        a.shadow(r, 0.0, sh, ALL);
        assert_eq!(a.bounds(0..1), Some(Rect::new(-5.0, -5.0, 20.0, 20.0)));
        let mut b = Scene::default();
        b.shape(
            r,
            0.0,
            Color::BLACK,
            0.0,
            Color::TRANSPARENT,
            Rect::new(0.0, 0.0, 4.0, 4.0),
        );
        assert_eq!(b.bounds(0..1), Some(Rect::new(0.0, 0.0, 4.0, 4.0)));
    }
}
