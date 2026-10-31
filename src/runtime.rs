use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::App;
use crate::layout::{Insets, Layout, Measured, Placed, flatten, place, walk};
use crate::render::{
    Color, Offscreen, Outcome, Rect, Renderer, Scene, Spans, damage, render_offscreen,
};
use crate::text::Text;
use crate::widget::{Element, Event, EventCx, Key, PaintCx, State};

pub(crate) struct Runtime<A: App> {
    app: A,
    root: Element<A::Msg>,
    /// Pre-order ids of `root`, so frames that keep the tree skip `flatten`.
    ids: Vec<u64>,
    states: HashMap<u64, State>,
    text: Text,
    layout: Layout,
    /// Taffy's last result; reused until the tree or viewport changes.
    measured: Vec<Measured>,
    placed: Vec<Placed>,
    captured: Option<u64>,
    /// Last painted frame, diffed against the next one for damage.
    prev: Option<(Scene, Spans)>,
    /// Animation frame due after an idle one; idle frames don't block on
    /// vsync, so they're paced by this timer instead.
    next_frame: Option<Instant>,
    /// Earliest `request_frame_at` from the last paint.
    wake: Option<Instant>,
    /// A widget asked for placement to rerun before the next paint.
    place: bool,
    /// Fixed frame time for tests; the wall clock otherwise.
    clock: Option<Instant>,
    /// Hidden windows don't animate; presents would be throttled anyway.
    occluded: bool,
    focused: Option<u64>,
    mods: ModifiersState,
    instance: Option<wgpu::Instance>,
    renderer: Option<Renderer>,
    viewport: (f32, f32),
    cursor: (f32, f32),
    touch: Option<u64>,
    clipboard: Clipboard,
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
        let mut states = HashMap::new();
        let ids = flat_ids(&root, &mut states);
        Runtime {
            app,
            root,
            ids,
            states,
            text: Text::default(),
            layout: Layout::default(),
            measured: Vec::new(),
            placed: Vec::new(),
            captured: None,
            prev: None,
            next_frame: None,
            wake: None,
            place: false,
            clock: None,
            occluded: false,
            focused: None,
            mods: ModifiersState::empty(),
            instance: None,
            renderer: None,
            viewport: (0.0, 0.0),
            cursor: (0.0, 0.0),
            touch: None,
            clipboard: Clipboard::default(),
            stats: Stats::default(),
            insets: [0.0; 4],
            #[cfg(target_os = "android")]
            android: None,
        }
    }

    pub fn run(mut self, event_loop: EventLoop<()>) {
        event_loop.run_app(&mut self).expect("event loop");
    }

    fn now(&self) -> Instant {
        self.clock.unwrap_or_else(Instant::now)
    }

    fn scale(&self) -> f32 {
        self.renderer
            .as_ref()
            .map_or(1.0, |r| r.window().scale_factor() as f32)
    }

    /// The only way `root` changes, so `ids` always matches it.
    fn set_root(&mut self, root: Element<A::Msg>) {
        self.root = root;
        self.ids = flat_ids(&self.root, &mut self.states);
    }

    fn relayout(&mut self) {
        let nodes = walk(&self.root, &self.ids);
        self.measured = self
            .layout
            .measure(&nodes, &self.text, self.viewport, self.insets);
        let now = self.now();
        self.placed = place(&nodes, &self.measured, &mut self.states, now);
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
        self.request_redraw();
    }

    /// Redraw for changes that keep every size (hover, press, scroll, caret):
    /// only placement reruns.
    fn reposition(&mut self) {
        let nodes = walk(&self.root, &self.ids);
        if nodes.len() != self.measured.len() {
            return self.redraw();
        }
        let now = self.now();
        self.placed = place(&nodes, &self.measured, &mut self.states, now);
        self.request_redraw();
    }

    fn request_redraw(&self) {
        if let Some(r) = &self.renderer {
            r.window().request_redraw();
        }
    }

    fn dispatch(&mut self, ev: Event) {
        let nodes = walk(&self.root, &self.ids);
        if nodes.len() != self.placed.len() {
            return;
        }
        let mut msgs = Vec::new();
        let mut updates = Vec::new();
        let mut copied = None;
        let mut redraw = false;
        let now = self.now();
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
                now,
                rect: p.rect,
                clip: p.clip,
                content: p.content,
                focused: self.focused == Some(n.id),
                shift: self.mods.shift_key(),
                state: states.entry(n.id).or_default(),
                text: &self.text,
                msgs: &mut msgs,
                updates: &mut updates,
                copied: &mut copied,
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
        if let Some(t) = copied {
            self.clipboard.set(t);
        }
        // A tap that no widget claims for focus blurs the focused one.
        if matches!(ev, Event::PointerDown { .. }) {
            redraw |= self.set_focus(focus);
        }

        let rebuild = !msgs.is_empty() || !updates.is_empty();
        if rebuild {
            for (id, f) in updates {
                f(self.states.entry(id).or_default());
            }
            for m in msgs {
                self.app.update(m);
            }
            // ponytail: any update rebuilds the whole tree; rebuild only dirty stateful subtrees if views get expensive.
            self.set_root(self.app.view());
            let live: std::collections::HashSet<u64> = self.ids.iter().copied().collect();
            self.states.retain(|id, _| live.contains(id));
            if self.focused.is_some_and(|id| !live.contains(&id)) {
                self.set_focus(None);
            }
        }
        if rebuild {
            self.redraw();
        } else if redraw {
            self.reposition();
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
        if !frame
            && !self.occluded
            && let Some(t) = self.wake
        {
            self.next_frame = Some(t);
        }
        self.prev = Some((scene, spans));
    }

    fn scene(&mut self, frame: &mut bool) -> (Scene, Spans) {
        let nodes = walk(&self.root, &self.ids);
        let now = self.now();
        if std::mem::take(&mut self.place) && nodes.len() == self.measured.len() {
            self.placed = place(&nodes, &self.measured, &mut self.states, now);
        }
        self.wake = None;
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
        self.text.outlines.begin_frame();
        let mut spans = Vec::with_capacity(nodes.len());
        for (n, p) in nodes.iter().zip(&self.placed) {
            let start = scene.len();
            n.w.paint(&mut PaintCx {
                now,
                rect: p.rect,
                clip: p.clip,
                focused: self.focused == Some(n.id),
                state: self.states.entry(n.id).or_default(),
                scene: &mut scene,
                text: &mut self.text,
                frame: &mut *frame,
                wake: &mut self.wake,
                place: &mut self.place,
            });
            spans.push((n.id, start..scene.len()));
        }
        self.text.end_frame();
        (scene, spans)
    }
}

fn printable(t: &str) -> String {
    t.chars().filter(|c| !c.is_control()).collect()
}

/// The system clipboard on desktop; an in-app one on Android and in tests.
// ponytail: Android could use ClipboardManager over JNI once there's a touch selection UI.
#[derive(Default)]
struct Clipboard {
    #[cfg(not(any(test, target_os = "android")))]
    sys: Option<arboard::Clipboard>,
    #[cfg(any(test, target_os = "android"))]
    text: Option<String>,
}

#[cfg(not(any(test, target_os = "android")))]
impl Clipboard {
    fn sys(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.sys.is_none() {
            self.sys = arboard::Clipboard::new()
                .inspect_err(|e| log::warn!("clipboard unavailable: {e}"))
                .ok();
        }
        self.sys.as_mut()
    }

    fn get(&mut self) -> Option<String> {
        self.sys()?.get_text().ok()
    }

    fn set(&mut self, t: String) {
        if let Some(c) = self.sys()
            && let Err(e) = c.set_text(t)
        {
            log::warn!("clipboard write failed: {e}");
        }
    }
}

#[cfg(any(test, target_os = "android"))]
impl Clipboard {
    fn get(&mut self) -> Option<String> {
        self.text.clone()
    }

    fn set(&mut self, t: String) {
        self.text = Some(t);
    }
}

fn flat_ids<Msg>(root: &Element<Msg>, states: &mut HashMap<u64, State>) -> Vec<u64> {
    flatten(root, states).iter().map(|n| n.id).collect()
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
        rt.set_root(rt.app.view());
        rt.relayout();
        self.paint()
    }

    /// Applies `msg`, then one full CPU frame.
    pub fn update(&mut self, msg: A::Msg) -> usize {
        self.0.app.update(msg);
        self.frame()
    }

    /// Paint and damage diff only, as after an event that kept the tree.
    pub fn paint(&mut self) -> usize {
        let rt = &mut self.0;
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
                let command = if cfg!(target_os = "macos") {
                    self.mods.super_key()
                } else {
                    self.mods.control_key()
                };
                let shortcut = |k: &str| {
                    command
                        && matches!(&event.logical_key, WKey::Character(c) if c.eq_ignore_ascii_case(k))
                };
                let key = match event.logical_key {
                    WKey::Named(NamedKey::Backspace) => Some(Key::Backspace),
                    WKey::Named(NamedKey::Delete) => Some(Key::Delete),
                    WKey::Named(NamedKey::ArrowLeft) => Some(Key::Left),
                    WKey::Named(NamedKey::ArrowRight) => Some(Key::Right),
                    WKey::Named(NamedKey::Home) => Some(Key::Home),
                    WKey::Named(NamedKey::End) => Some(Key::End),
                    _ if shortcut("a") => Some(Key::SelectAll),
                    _ if shortcut("c") => Some(Key::Copy),
                    _ if shortcut("x") => Some(Key::Cut),
                    _ => None,
                };
                if let Some(k) = key {
                    self.dispatch(Event::Key(k));
                } else if shortcut("v") {
                    // Pasting is typing the clipboard's text.
                    if self.focused.is_some()
                        && let Some(t) = self.clipboard.get().map(|t| printable(&t))
                        && !t.is_empty()
                    {
                        self.dispatch(Event::Text(t));
                    }
                } else if !(self.mods.control_key() && !self.mods.alt_key()
                    || self.mods.super_key())
                    && let Some(t) = event.text
                {
                    let t = printable(&t);
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
    use crate::widgets::{
        Styled, button, column, radio, scroll, segmented, slider, stateful, text, text_input,
        toggle,
    };

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
        let id = rt.ids[0];
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
        let t = *rt.clock.get_or_insert_with(Instant::now);
        let (a, na) = rt.scene(&mut false);
        for ev in events {
            rt.dispatch(ev);
        }
        // Transitions start at the first paint after the event; a second paint
        // a second later shows them settled.
        rt.clock = Some(t + Duration::from_secs(1));
        rt.scene(&mut false);
        rt.clock = Some(t + Duration::from_secs(2));
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

    #[test]
    #[ignore = "needs a GPU"]
    fn new_device_gets_outlines_already_uploaded_elsewhere() {
        let mut rt = Runtime::new(Counter(0));
        rt.viewport = (40.0, 40.0);
        rt.relayout();
        let (scene, _) = rt.scene(&mut false);
        let draw = |text: &mut Text| render_offscreen(&[(&scene, None)], text, (40, 40), 1.0);
        let first = draw(&mut rt.text);
        // A second device, as after an Android resume, with the same `Text`.
        assert_eq!(draw(&mut rt.text), first);
    }

    #[derive(Default)]
    struct Controls {
        on: bool,
        level: f32,
        picked: bool,
        seg: usize,
    }

    #[derive(Clone)]
    enum Ctl {
        On(bool),
        Level(f32),
        Pick,
        Seg(usize),
    }

    impl App for Controls {
        type Msg = Ctl;
        fn update(&mut self, m: Ctl) {
            match m {
                Ctl::On(b) => self.on = b,
                Ctl::Level(v) => self.level = v,
                Ctl::Pick => self.picked = true,
                Ctl::Seg(i) => self.seg = i,
            }
        }
        fn view(&self) -> Element<Ctl> {
            column([
                toggle(self.on)
                    .on_change(|cx, b| cx.emit(Ctl::On(b)))
                    .into(),
                slider(self.level, 0.0..=10.0)
                    .on_change(|cx, v| cx.emit(Ctl::Level(v)))
                    .width(122.0)
                    .into(),
                radio(self.picked, text("pick"))
                    .on_select(|cx| cx.emit(Ctl::Pick))
                    .into(),
                segmented(["a", "b", "c"], self.seg)
                    .on_change(|cx, i| cx.emit(Ctl::Seg(i)))
                    .width(150.0)
                    .into(),
            ])
            .align(taffy::AlignItems::START)
            .into()
        }
    }

    #[test]
    fn controls_report_taps_and_drags() {
        let mut rt = Runtime::new(Controls::default());
        rt.viewport = (300.0, 400.0);
        rt.relayout();
        let center = |rt: &Runtime<Controls>, i: usize| {
            let r = rt.placed[i].rect;
            (r.x + r.w / 2.0, r.y + r.h / 2.0)
        };
        let tap = |rt: &mut Runtime<Controls>, (x, y): (f32, f32)| {
            rt.dispatch(Event::PointerDown { x, y });
            rt.dispatch(Event::PointerUp { x, y });
        };
        // Pre-order: column, toggle, slider, radio, its label, segmented.
        let p = center(&rt, 1);
        tap(&mut rt, p);
        assert!(rt.app.on);
        tap(&mut rt, p);
        assert!(!rt.app.on);

        // 122px wide with a 22px thumb: 100px of travel from x = 11.
        let (_, y) = center(&rt, 2);
        rt.dispatch(Event::PointerDown { x: 61.0, y });
        assert_eq!(rt.app.level, 5.0, "tap jumps to the middle");
        rt.dispatch(Event::PointerMove {
            x: 81.0,
            y: y + 300.0,
        });
        assert_eq!(rt.app.level, 7.0, "captured drag follows x only");
        rt.dispatch(Event::PointerMove { x: 500.0, y });
        rt.dispatch(Event::PointerUp { x: 500.0, y });
        assert_eq!(rt.app.level, 10.0, "clamped to the range");
        rt.dispatch(Event::PointerMove { x: 11.0, y });
        assert_eq!(rt.app.level, 10.0, "released");

        let p = center(&rt, 4);
        tap(&mut rt, p);
        assert!(rt.app.picked, "the label is part of the target");

        let seg = rt.placed[5].rect;
        let y = seg.y + seg.h / 2.0;
        tap(&mut rt, (seg.x + seg.w - 5.0, y));
        assert_eq!(rt.app.seg, 2);
        rt.dispatch(Event::PointerDown { x: seg.x + 5.0, y });
        rt.dispatch(Event::PointerUp {
            x: seg.x + seg.w / 2.0,
            y,
        });
        assert_eq!(rt.app.seg, 2, "released over another option");
    }

    struct Long;

    impl App for Long {
        type Msg = ();
        fn update(&mut self, _: ()) {}
        fn view(&self) -> Element<()> {
            let rows = (0..10).map(|_| column([]).height(50.0).into());
            column([scroll(column(rows)).grow(1.0).into()]).into()
        }
    }

    #[test]
    fn wheel_scroll_moves_content_without_relayout() {
        let mut rt = Runtime::new(Long);
        rt.viewport = (100.0, 200.0);
        rt.relayout();
        let row = rt.placed[3].rect;
        rt.measured.clear();
        rt.dispatch(Event::Wheel {
            x: 10.0,
            y: 10.0,
            dy: 30.0,
        });
        // A cleared taffy result forces a full relayout, which must agree.
        assert_eq!(rt.placed[3].rect.y, row.y - 30.0);
        rt.dispatch(Event::Wheel {
            x: 10.0,
            y: 10.0,
            dy: 30.0,
        });
        assert_eq!(rt.placed[3].rect.y, row.y - 60.0);
    }

    #[test]
    fn transitions_request_frames_until_settled() {
        let mut rt = Runtime::new(Counter(0));
        rt.viewport = (200.0, 200.0);
        rt.relayout();
        let t = Instant::now();
        let frame_at = |ms: u64, rt: &mut Runtime<Counter>| {
            rt.clock = Some(t + Duration::from_millis(ms));
            let mut frame = false;
            rt.scene(&mut frame);
            frame
        };
        assert!(!frame_at(0, &mut rt));
        rt.dispatch(Event::PointerMove { x: 10.0, y: 10.0 });
        assert!(frame_at(10, &mut rt), "hover fade starts");
        assert!(frame_at(60, &mut rt), "still fading");
        assert!(!frame_at(500, &mut rt), "settled, loop stops");
    }

    #[test]
    fn caret_moves_and_deletes_whole_graphemes() {
        let mut rt = Runtime::new(Field(String::new()));
        rt.viewport = (200.0, 200.0);
        rt.relayout();
        rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
        rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
        // "a", "e" + combining acute, a flag (two regional indicators), "b".
        rt.dispatch(Event::Text("ae\u{301}\u{1f1ea}\u{1f1f8}b".into()));
        let keys = |rt: &mut Runtime<Field>, keys: &[Key]| {
            for &k in keys {
                rt.dispatch(Event::Key(k));
            }
        };
        // Left twice from the end skips "b", then the whole flag.
        keys(&mut rt, &[Key::Left, Key::Left]);
        rt.dispatch(Event::Text("|".into()));
        assert_eq!(rt.app.0, "ae\u{301}|\u{1f1ea}\u{1f1f8}b");
        // Backspace removes "|", then "e" together with its accent.
        keys(&mut rt, &[Key::Backspace, Key::Backspace]);
        assert_eq!(rt.app.0, "a\u{1f1ea}\u{1f1f8}b");
        // Delete removes the whole flag.
        keys(&mut rt, &[Key::Delete]);
        assert_eq!(rt.app.0, "ab");
    }

    #[test]
    fn selection_extends_with_shift_and_is_replaced_by_edits() {
        let mut rt = Runtime::new(Field(String::new()));
        rt.viewport = (200.0, 200.0);
        rt.relayout();
        rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
        rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
        rt.dispatch(Event::Text("hello world".into()));
        // Shift+Left x5 selects "world"; typing replaces it.
        rt.mods = ModifiersState::SHIFT;
        for _ in 0..5 {
            rt.dispatch(Event::Key(Key::Left));
        }
        rt.mods = ModifiersState::empty();
        rt.dispatch(Event::Text("there".into()));
        assert_eq!(rt.app.0, "hello there");
        // Shift+Home selects back to the start; Left collapses to it.
        rt.mods = ModifiersState::SHIFT;
        rt.dispatch(Event::Key(Key::Home));
        rt.mods = ModifiersState::empty();
        rt.dispatch(Event::Key(Key::Left));
        rt.dispatch(Event::Text(">".into()));
        assert_eq!(rt.app.0, ">hello there");
        // Select all, then Backspace clears it.
        rt.dispatch(Event::Key(Key::SelectAll));
        rt.dispatch(Event::Key(Key::Backspace));
        assert_eq!(rt.app.0, "");
        // Dragging from the start of new text selects it.
        rt.dispatch(Event::Text("drag".into()));
        rt.dispatch(Event::PointerDown { x: 0.0, y: 10.0 });
        rt.dispatch(Event::PointerMove { x: 190.0, y: 10.0 });
        rt.dispatch(Event::PointerUp { x: 190.0, y: 10.0 });
        rt.dispatch(Event::Text("x".into()));
        assert_eq!(rt.app.0, "x");
    }

    #[test]
    fn copy_and_cut_take_the_selection() {
        let mut rt = Runtime::new(Field(String::new()));
        rt.viewport = (200.0, 200.0);
        rt.relayout();
        rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
        rt.dispatch(Event::PointerUp { x: 10.0, y: 10.0 });
        rt.dispatch(Event::Text("héllo wörld".into()));
        rt.dispatch(Event::Key(Key::Copy));
        assert_eq!(rt.clipboard.get(), None, "nothing selected");
        rt.mods = ModifiersState::SHIFT;
        for _ in 0..5 {
            rt.dispatch(Event::Key(Key::Left));
        }
        rt.mods = ModifiersState::empty();
        rt.dispatch(Event::Key(Key::Cut));
        assert_eq!(rt.clipboard.get().as_deref(), Some("wörld"));
        assert_eq!(rt.app.0, "héllo ");
        rt.dispatch(Event::Key(Key::SelectAll));
        rt.dispatch(Event::Key(Key::Copy));
        assert_eq!(rt.clipboard.get().as_deref(), Some("héllo "));
        assert_eq!(rt.app.0, "héllo ");
    }

    #[test]
    fn caret_blinks_on_timed_wakeups() {
        let mut rt = Runtime::new(Field(String::new()));
        rt.viewport = (200.0, 200.0);
        rt.relayout();
        let t = Instant::now();
        let paint_at = |ms: u64, rt: &mut Runtime<Field>| {
            rt.clock = Some(t + Duration::from_millis(ms));
            let mut frame = false;
            let n = rt.scene(&mut frame).0.len();
            assert!(!frame, "a blink never asks for every frame");
            n
        };
        let idle = paint_at(0, &mut rt);
        assert_eq!(rt.wake, None, "unfocused fields don't wake");
        rt.dispatch(Event::PointerDown { x: 10.0, y: 10.0 });
        let on = paint_at(100, &mut rt);
        assert_eq!(on, idle + 1, "caret shown");
        assert_eq!(rt.wake, Some(t + Duration::from_millis(600)));
        assert_eq!(paint_at(600, &mut rt), idle, "caret hidden");
        assert_eq!(rt.wake, Some(t + Duration::from_millis(1100)));
        // Typing restarts the cycle with the caret shown.
        rt.dispatch(Event::Text("a".into()));
        let typed = paint_at(700, &mut rt);
        assert_eq!(rt.wake, Some(t + Duration::from_millis(1200)));
        assert!(typed > paint_at(1200, &mut rt), "hidden again 500ms later");
    }

    #[test]
    fn released_drag_flings_and_settles_at_the_edge() {
        let mut rt = Runtime::new(Long);
        rt.viewport = (100.0, 200.0);
        rt.relayout();
        let t = Instant::now();
        let at = |ms: u64| t + Duration::from_millis(ms);
        let offset = |rt: &Runtime<Long>| -rt.placed[3].rect.y;
        let paint_at = |ms: u64, rt: &mut Runtime<Long>| {
            rt.clock = Some(at(ms));
            let mut frame = false;
            rt.scene(&mut frame);
            frame
        };
        // Drag up 80px over 64ms: 1250 px/s.
        rt.clock = Some(at(0));
        rt.dispatch(Event::PointerDown { x: 50.0, y: 150.0 });
        for i in 1..=4 {
            rt.clock = Some(at(16 * i));
            let y = 150.0 - 20.0 * i as f32;
            rt.dispatch(Event::PointerMove { x: 50.0, y });
        }
        rt.dispatch(Event::PointerUp { x: 50.0, y: 70.0 });
        assert_eq!(offset(&rt), 80.0);
        assert!(paint_at(64, &mut rt), "fling animates");
        // 80 + 1250·τ·(1 - e^(-0.1/τ)) at 100ms in, τ = 0.35s.
        let want = 80.0 + 1250.0 * 0.35 * (1.0 - (-0.1f32 / 0.35).exp());
        assert!(paint_at(164, &mut rt));
        assert!(
            (offset(&rt) - want).abs() < 0.5,
            "{} vs {want}",
            offset(&rt)
        );
        // Would travel past the end (300): stops there.
        assert!(!paint_at(2000, &mut rt), "settled");
        assert_eq!(offset(&rt), 300.0);

        // A drag that pauses before lifting doesn't fling.
        rt.clock = Some(at(3000));
        rt.dispatch(Event::PointerDown { x: 50.0, y: 50.0 });
        rt.clock = Some(at(3016));
        rt.dispatch(Event::PointerMove { x: 50.0, y: 100.0 });
        rt.clock = Some(at(3300));
        rt.dispatch(Event::PointerUp { x: 50.0, y: 100.0 });
        assert!(!paint_at(3300, &mut rt));
        assert_eq!(offset(&rt), 250.0);
    }
}
