use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::App;
use crate::layout::{Insets, Placed, flatten, layout};
use crate::render::{Renderer, Scene, render_offscreen};
use crate::text::Text;
use crate::widget::{Element, Event, EventCx, PaintCx, State};

pub(crate) struct Runtime<A: App> {
    app: A,
    root: Element<A::Msg>,
    states: HashMap<u64, State>,
    text: Text,
    placed: Vec<Placed>,
    captured: Option<u64>,
    instance: Option<wgpu::Instance>,
    renderer: Option<Renderer>,
    viewport: (f32, f32),
    cursor: (f32, f32),
    touch: Option<u64>,
    stats: Stats,
    insets: Insets,
    #[cfg(target_os = "android")]
    pub(crate) android: Option<crate::AndroidApp>,
}

/// Rolling frame timings, logged every couple of seconds.
struct Stats {
    since: Instant,
    frames: u32,
    cpu: Duration,
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            since: Instant::now(),
            frames: 0,
            cpu: Duration::ZERO,
        }
    }
}

impl Stats {
    fn frame(&mut self, cpu: Duration, gpu_ms: Option<f32>, instances: usize) {
        self.frames += 1;
        self.cpu += cpu;
        let window = self.since.elapsed();
        if window >= Duration::from_secs(2) {
            log::info!(
                "fps {:.1}  cpu {:.2}ms  gpu {}  instances {instances}",
                self.frames as f32 / window.as_secs_f32(),
                self.cpu.as_secs_f32() * 1e3 / self.frames as f32,
                gpu_ms.map_or("n/a".into(), |g| format!("{g:.2}ms")),
            );
            *self = Stats::default();
        }
    }
}

impl<A: App> Runtime<A> {
    pub fn new(app: A) -> Self {
        let root = app.view();
        Runtime {
            app,
            root,
            states: HashMap::new(),
            text: Text::default(),
            placed: Vec::new(),
            captured: None,
            instance: None,
            renderer: None,
            viewport: (0.0, 0.0),
            cursor: (0.0, 0.0),
            touch: None,
            stats: Stats::default(),
            insets: [0.0; 4],
            #[cfg(target_os = "android")]
            android: None,
        }
    }

    pub fn run(mut self, event_loop: EventLoop<()>) {
        event_loop.run_app(&mut self).expect("event loop");
    }

    fn scale(&self) -> f32 {
        self.renderer
            .as_ref()
            .map_or(1.0, |r| r.window().scale_factor() as f32)
    }

    fn relayout(&mut self) {
        let nodes = flatten(&self.root, &mut self.states);
        self.placed = layout(
            &nodes,
            &mut self.states,
            &self.text,
            self.viewport,
            self.insets,
        );
    }

    /// System bars cover part of an edge-to-edge window; the content rect is what's left.
    #[cfg(target_os = "android")]
    fn update_insets(&mut self) {
        let (Some(a), Some(r)) = (&self.android, &self.renderer) else {
            return;
        };
        let s = r.window().scale_factor() as f32;
        self.insets = crate::android::insets(a).map(|v| v as f32 / s);
        log::info!("insets {:?}", self.insets);
    }

    #[cfg(not(target_os = "android"))]
    fn update_insets(&mut self) {}

    fn redraw(&mut self) {
        self.relayout();
        if let Some(r) = &self.renderer {
            r.window().request_redraw();
        }
    }

    fn dispatch(&mut self, ev: Event) {
        let nodes = flatten(&self.root, &mut self.states);
        if nodes.len() != self.placed.len() {
            return;
        }
        let mut msgs = Vec::new();
        let mut updates = Vec::new();
        let mut redraw = false;
        let targets: Vec<usize> = match self.captured {
            Some(id) => nodes.iter().position(|n| n.id == id).into_iter().collect(),
            // Reverse pre-order: descendants and later (topmost) siblings first.
            None => (0..nodes.len()).rev().collect(),
        };
        for i in targets {
            // ponytail: every event visits every node; index by rect if trees get huge.
            let p = self.placed[i];
            let mut cx = EventCx {
                rect: p.rect,
                clip: p.clip,
                content: p.content,
                state: self.states.entry(nodes[i].id).or_default(),
                msgs: &mut msgs,
                updates: &mut updates,
                capture: false,
                stop: false,
                redraw: false,
            };
            nodes[i].w.event(&ev, &mut cx);
            let (capture, stop) = (cx.capture, cx.stop);
            redraw |= cx.redraw;
            if capture && self.captured.is_none() {
                self.captured = Some(nodes[i].id);
                for (j, n) in nodes.iter().enumerate().filter(|&(j, _)| j != i) {
                    let p = self.placed[j];
                    let mut cx = EventCx {
                        rect: p.rect,
                        clip: p.clip,
                        content: p.content,
                        state: self.states.entry(n.id).or_default(),
                        msgs: &mut msgs,
                        updates: &mut updates,
                        capture: false,
                        stop: false,
                        redraw: false,
                    };
                    n.w.event(&Event::Cancel, &mut cx);
                    redraw |= cx.redraw;
                }
                break;
            }
            if stop {
                break;
            }
        }
        if matches!(ev, Event::PointerUp { .. }) {
            self.captured = None;
        }

        if !msgs.is_empty() || !updates.is_empty() {
            for (id, f) in updates {
                f(self.states.entry(id).or_default());
            }
            for m in msgs {
                self.app.update(m);
            }
            // ponytail: any update rebuilds the whole tree; rebuild only dirty stateful subtrees if views get expensive.
            self.root = self.app.view();
            let live: std::collections::HashSet<u64> = flatten(&self.root, &mut self.states)
                .iter()
                .map(|n| n.id)
                .collect();
            self.states.retain(|id, _| live.contains(id));
            redraw = true;
        }
        if redraw {
            self.redraw();
        }
    }

    fn paint(&mut self) {
        let start = Instant::now();
        let mut frame = false;
        let scene = self.scene(&mut frame);
        if let Some(r) = &mut self.renderer {
            let waited = r.render(&scene, &mut self.text);
            self.stats.frame(
                start.elapsed().saturating_sub(waited),
                r.gpu_ms(),
                scene.len(),
            );
            if frame {
                r.window().request_redraw();
            }
        }
    }

    fn scene(&mut self, frame: &mut bool) -> Scene {
        let nodes = flatten(&self.root, &mut self.states);
        let mut scene = Scene::default();
        for (n, p) in nodes.iter().zip(&self.placed) {
            n.w.paint(&mut PaintCx {
                rect: p.rect,
                clip: p.clip,
                state: self.states.entry(n.id).or_default(),
                scene: &mut scene,
                text: &mut self.text,
                frame: &mut *frame,
            });
        }
        scene
    }
}

/// Renders one frame of `app` offscreen at `size` physical pixels, e.g. for golden tests.
pub fn snapshot<A: App>(app: A, size: (u32, u32), scale: f32) -> image::RgbaImage {
    let mut rt = Runtime::new(app);
    rt.viewport = (size.0 as f32 / scale, size.1 as f32 / scale);
    rt.relayout();
    let scene = rt.scene(&mut false);
    render_offscreen(&scene, &mut rt.text, size, scale)
}

impl<A: App> ApplicationHandler for Runtime<A> {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        let window = Arc::new(
            el.create_window(Window::default_attributes().with_title("dfui"))
                .expect("create window"),
        );
        let instance = self.instance.get_or_insert_with(|| {
            let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
                el.owned_display_handle(),
            ));
            if cfg!(target_os = "android") {
                desc.backends = wgpu::Backends::VULKAN;
            }
            wgpu::Instance::new(desc.with_env())
        });
        let size = window.inner_size().to_logical::<f32>(window.scale_factor());
        self.viewport = (size.width, size.height);
        self.renderer = Some(Renderer::new(instance, window));
        self.update_insets();
        self.redraw();
    }

    // Android destroys the native window when backgrounded.
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.renderer = None;
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let scale = self.scale();
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                self.viewport = (size.width as f32 / scale, size.height as f32 / scale);
                self.update_insets();
                self.redraw();
            }
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32 / scale, position.y as f32 / scale);
                let (x, y) = self.cursor;
                self.dispatch(Event::PointerMove { x, y });
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.cursor;
                self.dispatch(match state {
                    ElementState::Pressed => Event::PointerDown { x, y },
                    ElementState::Released => Event::PointerUp { x, y },
                });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 40.0,
                    MouseScrollDelta::PixelDelta(p) => -p.y as f32 / scale,
                };
                let (x, y) = self.cursor;
                self.dispatch(Event::Wheel { x, y, dy });
            }
            WindowEvent::Touch(t) => {
                // ponytail: single-pointer only; multi-touch gestures need per-finger dispatch.
                if self.touch.is_some_and(|id| id != t.id) {
                    return;
                }
                let (x, y) = (t.location.x as f32 / scale, t.location.y as f32 / scale);
                match t.phase {
                    TouchPhase::Started => {
                        self.touch = Some(t.id);
                        self.dispatch(Event::PointerDown { x, y });
                    }
                    TouchPhase::Moved => self.dispatch(Event::PointerMove { x, y }),
                    TouchPhase::Ended => {
                        self.touch = None;
                        self.dispatch(Event::PointerUp { x, y });
                    }
                    TouchPhase::Cancelled => {
                        self.touch = None;
                        self.captured = None;
                        self.dispatch(Event::Cancel);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{Styled, button, column, stateful, text};

    struct Counter(i32);

    impl App for Counter {
        type Msg = i32;
        fn update(&mut self, d: i32) {
            self.0 += d;
        }
        fn view(&self) -> Element<i32> {
            column([button(text("+")).on_press(1).height(40.0).into()]).into()
        }
    }

    #[test]
    fn button_click_emits_msg_only_when_released_inside() {
        let mut rt = Runtime::new(Counter(0));
        rt.viewport = (200.0, 200.0);
        rt.relayout();
        rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
        rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
        assert_eq!(rt.app.0, 1);
        rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
        rt.dispatch(Event::PointerUp { x: 10.0, y: 100.0 });
        assert_eq!(rt.app.0, 1);
        rt.dispatch(Event::PointerDown { x: 10.0, y: 100.0 });
        rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
        assert_eq!(rt.app.0, 1);
    }

    struct Local(i32);

    impl App for Local {
        type Msg = i32;
        fn update(&mut self, d: i32) {
            self.0 += d;
        }
        fn view(&self) -> Element<i32> {
            stateful(|n: &i32, h| {
                button(text(format!("{n}")))
                    .on_click(move |cx| {
                        cx.update(h, |n: &mut i32| *n += 1);
                        cx.emit(10);
                    })
                    .height(40.0)
            })
            .into()
        }
    }

    #[test]
    fn stateful_update_survives_rebuild_alongside_msgs() {
        let mut rt = Runtime::new(Local(0));
        rt.viewport = (200.0, 200.0);
        rt.relayout();
        for _ in 0..2 {
            rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
            rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
        }
        assert_eq!(rt.app.0, 20);
        let id = flatten(&rt.root, &mut rt.states)[0].id;
        assert_eq!(*rt.states.get_mut(&id).unwrap().get::<i32>(), 2);
    }
}
