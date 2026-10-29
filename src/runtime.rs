use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::App;
use crate::layout::{Insets, Placed, flatten, layout};
use crate::render::{
    Color, Offscreen, Outcome, Rect, Renderer, Scene, Spans, damage, render_offscreen,
};
use crate::text::Text;
use crate::widget::{Element, Event, EventCx, Key, PaintCx, State};

pub(crate) struct Runtime<A: App> {
    app: A,
    root: Element<A::Msg>,
    states: HashMap<u64, State>,
    text: Text,
    placed: Vec<Placed>,
    captured: Option<u64>,
    /// Last painted frame, diffed against the next one for damage.
    prev: Option<(Scene, Spans)>,
    /// Animation frame due after an idle one; idle frames don't block on
    /// vsync, so they're paced by this timer instead.
    next_frame: Option<Instant>,
    /// Hidden windows don't animate; presents would be throttled anyway.
    occluded: bool,
    focused: Option<u64>,
    mods: ModifiersState,
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
            prev: None,
            next_frame: None,
            occluded: false,
            focused: None,
            mods: ModifiersState::empty(),
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
        let mut focus = None;
        let keyboard = matches!(ev, Event::Text(_) | Event::Key(_));
        let targets: Vec<usize> = match (keyboard, self.captured) {
            (true, _) => (self.focused)
                .and_then(|id| nodes.iter().position(|n| n.id == id))
                .into_iter()
                .collect(),
            (false, Some(id)) => nodes.iter().position(|n| n.id == id).into_iter().collect(),
            // Reverse pre-order: descendants and later (topmost) siblings first.
            (false, None) => (0..nodes.len()).rev().collect(),
        };
        let mut send = |i: usize, ev: &Event, states: &mut HashMap<u64, State>| {
            let (n, p) = (&nodes[i], self.placed[i]);
            let mut cx = EventCx {
                rect: p.rect,
                clip: p.clip,
                content: p.content,
                focused: self.focused == Some(n.id),
                state: states.entry(n.id).or_default(),
                text: &self.text,
                msgs: &mut msgs,
                updates: &mut updates,
                capture: false,
                focus: false,
                stop: false,
                redraw: false,
            };
            n.w.event(ev, &mut cx);
            redraw |= cx.redraw;
            if cx.focus {
                focus = Some(n.id);
            }
            (cx.capture, cx.stop)
        };
        for i in targets {
            // ponytail: every event visits every node; index by rect if trees get huge.
            let (capture, stop) = send(i, &ev, &mut self.states);
            if capture && self.captured.is_none() {
                self.captured = Some(nodes[i].id);
                for j in (0..nodes.len()).filter(|&j| j != i) {
                    send(j, &Event::Cancel, &mut self.states);
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
        // A tap that no widget claims for focus blurs the focused one.
        if matches!(ev, Event::PointerDown { .. }) {
            redraw |= self.set_focus(focus);
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
            if self.focused.is_some_and(|id| !live.contains(&id)) {
                self.set_focus(None);
            }
            redraw = true;
        }
        if redraw {
            self.redraw();
        }
    }

    /// Returns whether focus changed.
    fn set_focus(&mut self, id: Option<u64>) -> bool {
        if self.focused == id {
            return false;
        }
        self.focused = id;
        // ponytail: IME only drives the Android soft keyboard; desktop composition (CJK) needs Ime::Preedit handling.
        if cfg!(target_os = "android")
            && let Some(r) = &self.renderer
        {
            r.window().set_ime_allowed(id.is_some());
        }
        true
    }

    fn paint(&mut self) {
        let start = Instant::now();
        let mut frame = false;
        let (scene, spans) = self.scene(&mut frame);
        // Empty when nothing changed: the retained frame is reused as is.
        let damage = self
            .prev
            .as_ref()
            .map(|(ps, pn)| damage(ps, pn, &scene, &spans).unwrap_or_default());
        self.next_frame = None;
        if let Some(r) = &mut self.renderer {
            match r.render(&scene, &mut self.text, damage) {
                // Canvas untouched: keep diffing against what it actually holds.
                Outcome::Failed => return,
                Outcome::Idle if frame && !self.occluded => {
                    let hz = r.window().current_monitor();
                    let mhz = hz.and_then(|m| m.refresh_rate_millihertz());
                    let interval = Duration::from_secs_f64(1000.0 / mhz.unwrap_or(60_000) as f64);
                    self.next_frame = Some(Instant::now() + interval);
                }
                Outcome::Idle => {}
                Outcome::Presented(waited) => {
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
        }
        self.prev = Some((scene, spans));
    }

    fn scene(&mut self, frame: &mut bool) -> (Scene, Spans) {
        let nodes = flatten(&self.root, &mut self.states);
        let mut scene = Scene::default();
        // Partial redraws load the previous frame, so damage needs its own background.
        let (w, h) = self.viewport;
        let all = Rect::new(0.0, 0.0, w, h);
        scene.shape(
            all,
            0.0,
            self.app.background(),
            0.0,
            Color::TRANSPARENT,
            all,
        );
        let mut spans = Vec::with_capacity(nodes.len());
        for (n, p) in nodes.iter().zip(&self.placed) {
            let start = scene.len();
            n.w.paint(&mut PaintCx {
                rect: p.rect,
                clip: p.clip,
                focused: self.focused == Some(n.id),
                state: self.states.entry(n.id).or_default(),
                scene: &mut scene,
                text: &mut self.text,
                frame: &mut *frame,
            });
            spans.push((n.id, start..scene.len()));
        }
        self.text.end_frame();
        (scene, spans)
    }
}

/// CPU side of frames for benchmarks: rebuild, layout, paint and damage diff,
/// through the real runtime but without a window or GPU.
#[doc(hidden)]
pub struct Bench<A: App>(Runtime<A>);

impl<A: App> Bench<A> {
    pub fn new(app: A, viewport: (f32, f32)) -> Self {
        let mut rt = Runtime::new(app);
        rt.viewport = viewport;
        rt.relayout();
        let mut b = Bench(rt);
        b.frame();
        b
    }

    /// One full CPU frame as after a state change; returns the instance count.
    pub fn frame(&mut self) -> usize {
        let rt = &mut self.0;
        rt.root = rt.app.view();
        rt.relayout();
        let (scene, spans) = rt.scene(&mut false);
        if let Some((ps, pn)) = &rt.prev {
            std::hint::black_box(damage(ps, pn, &scene, &spans));
        }
        let n = scene.len();
        rt.prev = Some((scene, spans));
        n
    }

    pub fn event(&mut self, ev: Event) {
        self.0.dispatch(ev);
    }
}

/// GPU side of frames for benchmarks: one scene of `app`, drawn offscreen at
/// `size` physical pixels, timed with GPU timestamps where available.
#[doc(hidden)]
pub struct GpuBench {
    scene: Scene,
    text: Text,
    off: Offscreen,
    scale: f32,
}

impl GpuBench {
    pub fn new<A: App>(app: A, size: (u32, u32), scale: f32) -> Self {
        let mut rt = Runtime::new(app);
        rt.viewport = (size.0 as f32 / scale, size.1 as f32 / scale);
        rt.relayout();
        let (scene, _) = rt.scene(&mut false);
        let mut g = GpuBench {
            scene,
            text: rt.text,
            off: Offscreen::new(size),
            scale,
        };
        // Builds and uploads glyph outlines before anything is timed.
        g.draw(None);
        g
    }

    /// Redraws everything, or only `damage` (logical px) as a partial frame.
    pub fn draw(&mut self, damage: Option<Rect>) -> Duration {
        let s = self.scale;
        let scissor = damage.map(|d| {
            let px = |v: f32| (v * s).round() as u32;
            [px(d.x), px(d.y), px(d.w).max(1), px(d.h).max(1)]
        });
        self.off.timed_draw(&self.scene, &mut self.text, s, scissor)
    }
}

/// Renders one frame of `app` offscreen at `size` physical pixels, e.g. for golden tests.
pub fn snapshot<A: App>(app: A, size: (u32, u32), scale: f32) -> image::RgbaImage {
    let mut rt = Runtime::new(app);
    rt.viewport = (size.0 as f32 / scale, size.1 as f32 / scale);
    rt.relayout();
    let (scene, _) = rt.scene(&mut false);
    render_offscreen(&[(&scene, None)], &mut rt.text, size, scale)
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

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        match self.next_frame {
            Some(t) if Instant::now() >= t => {
                self.next_frame = None;
                if let Some(r) = &self.renderer {
                    r.window().request_redraw();
                }
                el.set_control_flow(ControlFlow::Wait);
            }
            Some(t) => el.set_control_flow(ControlFlow::WaitUntil(t)),
            None => el.set_control_flow(ControlFlow::Wait),
        }
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
            WindowEvent::Occluded(o) => {
                self.occluded = o;
                if !o && let Some(r) = &self.renderer {
                    r.window().request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32 / scale, position.y as f32 / scale);
                let (x, y) = self.cursor;
                self.dispatch(Event::PointerMove { x, y });
            }
            WindowEvent::CursorLeft { .. } => self.dispatch(Event::Leave),
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
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let key = match event.logical_key {
                    WKey::Named(NamedKey::Backspace) => Some(Key::Backspace),
                    WKey::Named(NamedKey::Delete) => Some(Key::Delete),
                    WKey::Named(NamedKey::ArrowLeft) => Some(Key::Left),
                    WKey::Named(NamedKey::ArrowRight) => Some(Key::Right),
                    WKey::Named(NamedKey::Home) => Some(Key::Home),
                    WKey::Named(NamedKey::End) => Some(Key::End),
                    _ => None,
                };
                if let Some(k) = key {
                    self.dispatch(Event::Key(k));
                } else if !(self.mods.control_key() && !self.mods.alt_key()
                    || self.mods.super_key())
                    && let Some(t) = event.text
                {
                    let t: String = t.chars().filter(|c| !c.is_control()).collect();
                    if !t.is_empty() {
                        self.dispatch(Event::Text(t));
                    }
                }
            }
            WindowEvent::Ime(Ime::Commit(t)) => self.dispatch(Event::Text(t)),
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
                        self.dispatch(Event::Leave);
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
    use crate::widgets::{Styled, button, column, stateful, text, text_input};

    struct Counter(i32);

    impl App for Counter {
        type Msg = i32;
        fn update(&mut self, d: i32) {
            self.0 += d;
        }
        fn view(&self) -> Element<i32> {
            column([button(text("+")).on_press(1).height(40.0).into()]).into()
        }
        fn background(&self) -> Color {
            if self.0 > 0 {
                Color::BLACK
            } else {
                Color::WHITE
            }
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

    struct Field(String);

    impl App for Field {
        type Msg = String;
        fn update(&mut self, v: String) {
            self.0 = v;
        }
        fn view(&self) -> Element<String> {
            column([text_input(self.0.clone())
                .on_change(|cx, v| cx.emit(v))
                .into()])
            .into()
        }
    }

    #[test]
    fn text_input_edits_only_while_focused() {
        let mut rt = Runtime::new(Field(String::new()));
        rt.viewport = (200.0, 200.0);
        rt.relayout();
        rt.dispatch(Event::Text("x".into()));
        assert_eq!(rt.app.0, "");

        rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
        rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
        for ev in [
            Event::Text("aé".into()),
            Event::Key(Key::Left),
            Event::Text("X".into()),
            Event::Key(Key::Home),
            Event::Key(Key::Delete),
            Event::Key(Key::End),
            Event::Key(Key::Backspace),
        ] {
            rt.dispatch(ev);
        }
        assert_eq!(rt.app.0, "X");

        rt.dispatch(Event::PointerDown { x: 10.0, y: 190.0 });
        rt.dispatch(Event::Text("z".into()));
        assert_eq!(rt.app.0, "X");
    }

    fn damage_after<A: App>(
        rt: &mut Runtime<A>,
        events: impl IntoIterator<Item = Event>,
    ) -> Option<Rect> {
        let (a, na) = rt.scene(&mut false);
        for ev in events {
            rt.dispatch(ev);
        }
        let (b, nb) = rt.scene(&mut false);
        damage(&a, &na, &b, &nb)
    }

    #[test]
    fn interactions_damage_only_the_widget() {
        let mut rt = Runtime::new(Counter(0));
        rt.viewport = (200.0, 200.0);
        rt.relayout();
        let hover = damage_after(&mut rt, [Event::PointerMove { x: 10.0, y: 10.0 }]).unwrap();
        // Button is 40px tall; its shadow reaches a few px further.
        assert!(hover.y + hover.h < 50.0, "{hover:?}");
        assert_eq!(
            damage_after(&mut rt, [Event::PointerMove { x: 12.0, y: 10.0 }]),
            None
        );

        let mut rt = Runtime::new(Field(String::new()));
        rt.viewport = (200.0, 200.0);
        rt.relayout();
        rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
        let field = rt.placed[1].rect;
        let typed = damage_after(&mut rt, [Event::Text("hi".into())]).unwrap();
        assert!(
            typed.h <= field.h + 4.0 && typed.y >= field.y - 2.0,
            "{typed:?} {field:?}"
        );
    }

    #[test]
    fn background_change_damages_everything() {
        let mut rt = Runtime::new(Counter(0));
        rt.viewport = (200.0, 200.0);
        rt.relayout();
        let click = [
            Event::PointerDown { x: 10.0, y: 10.0 },
            Event::PointerUp { x: 10.0, y: 10.0 },
        ];
        let d = damage_after(&mut rt, click).unwrap();
        assert!(d.w >= 200.0 && d.h >= 200.0, "{d:?}");
    }
}
