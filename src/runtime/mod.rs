use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use web_time::Instant;
use winit::event::TouchPhase;
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};

use crate::App;
use crate::layout::{Insets, Layout, Measured, Placed, flatten, order, place, walk, walk_to};
use crate::render::{Color, Outcome, Rect, Renderer, Scene, Spans, damage};
use crate::text::Text;
use crate::widget::{Element, Event, EventCx, Key, Keyboard, PaintCx, State};
use task::{LoopWaker, Tasks};

#[cfg(target_os = "android")]
pub(crate) mod activity;
#[cfg(target_os = "android")]
pub mod android;
mod bench;
mod clipboard;
#[cfg(target_os = "android")]
mod editor;
// TIMERS is already const; clippy misfires on Android's thread_local expansion.
#[allow(clippy::missing_const_for_thread_local)]
pub(crate) mod task;
/// Tells the user why there's no UI, in the page on the web; elsewhere exits.
#[cfg(target_arch = "wasm32")]
fn startup_failed(reason: &str) -> ! {
    if let Some(body) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.body())
    {
        body.set_text_content(Some(&format!("This page needs WebGPU. {reason}")));
    }
    wasm_bindgen::throw_str(reason)
}

#[cfg(not(target_arch = "wasm32"))]
fn startup_failed(reason: &str) -> ! {
    log::error!("{reason}");
    eprintln!("dfui: {reason}");
    std::process::exit(1)
}

#[cfg(test)]
mod tests;
mod winit_app;

pub use bench::{Bench, GpuBench, snapshot};
use clipboard::Clipboard;

type PendingRenderer = Pin<Box<dyn Future<Output = Result<Renderer, String>>>>;

pub(crate) struct Runtime<A: App> {
    app: A,
    root: Element,
    /// Pre-order ids of `root`, so frames that keep the tree skip `flatten`.
    ids: Vec<u64>,
    /// Paint order of `ids`' indices; layers last.
    order: Vec<usize>,
    /// Per index: one past its last descendant, and its place in `order`.
    ends: Vec<usize>,
    rank: Vec<usize>,
    /// Indices of layer widgets.
    layers: Vec<usize>,
    states: HashMap<u64, State>,
    tasks: Tasks,
    /// Wakes the event loop when a task can progress; a no-op without one.
    waker: Waker,
    loop_waker: Option<Arc<LoopWaker>>,
    text: Text,
    layout: Layout,
    /// Taffy's last result; reused until the tree or viewport changes.
    measured: Vec<Measured>,
    placed: Vec<Placed>,
    captured: Option<u64>,
    /// Nodes (index, id) under the last pointer event and under the press
    /// in progress: they get the next pointer events too, so they see the
    /// pointer leave and keep tracking a drag.
    hovered: Vec<(usize, u64)>,
    pressed: Vec<(usize, u64)>,
    /// Last painted frame, diffed against the next one for damage.
    prev: Option<(Scene, Spans)>,
    /// Animation frame due after an idle one; idle frames don't block on
    /// vsync, so they're paced by this timer instead.
    next_frame: Option<Instant>,
    /// Earliest `request_frame_at` from the last paint.
    wake: Option<Instant>,
    /// A widget asked for placement to rerun before the next paint.
    place: bool,
    /// A widget asked for a rebuild after the last paint.
    rebuild_due: bool,
    /// Hidden windows don't animate; presents would be throttled anyway.
    occluded: bool,
    focused: Option<u64>,
    mods: ModifiersState,
    instance: Option<wgpu::Instance>,
    renderer: Option<Renderer>,
    /// GPU setup started in `resumed`; polled like a task until it resolves.
    pending_renderer: Option<PendingRenderer>,
    viewport: (f32, f32),
    cursor: (f32, f32),
    touch: Option<u64>,
    clipboard: Clipboard,
    stats: Stats,
    insets: Insets,
    #[cfg(target_os = "android")]
    pub(crate) android: Option<crate::AndroidApp>,
    /// Keyboard requests, since tests have no window to send them to.
    #[cfg(test)]
    ime_requests: Vec<Option<Keyboard>>,
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
        let mut tasks = Tasks::new();
        let f = flat_ids(&root, &mut states, &mut tasks);
        Runtime {
            app,
            root,
            ids: f.ids,
            order: f.order,
            ends: f.ends,
            rank: f.rank,
            layers: f.layers,
            states,
            tasks,
            waker: Waker::noop().clone(),
            loop_waker: None,
            text: Text::default(),
            layout: Layout::default(),
            measured: Vec::new(),
            placed: Vec::new(),
            captured: None,
            hovered: Vec::new(),
            pressed: Vec::new(),
            prev: None,
            next_frame: None,
            wake: None,
            place: false,
            rebuild_due: false,
            occluded: false,
            focused: None,
            #[cfg(test)]
            ime_requests: Vec::new(),
            mods: ModifiersState::empty(),
            instance: None,
            renderer: None,
            pending_renderer: None,
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

    fn now(&self) -> Instant {
        task::now()
    }

    fn scale(&self) -> f32 {
        self.renderer.as_ref().map_or(1.0, |r| r.window().scale())
    }

    /// The only way `root` changes, so `ids` always matches it.
    fn set_root(&mut self, root: Element) {
        self.root = root;
        let f = flat_ids(&self.root, &mut self.states, &mut self.tasks);
        (self.ids, self.order, self.ends, self.rank, self.layers) =
            (f.ids, f.order, f.ends, f.rank, f.layers);
    }

    /// Rebuilds the tree from `view`, dropping state and tasks of removed nodes.
    // ponytail: any change rebuilds the whole tree; rebuild only dirty subtrees if views get expensive.
    fn rebuild(&mut self) {
        self.set_root(self.app.view());
        let live: std::collections::HashSet<u64> = self.ids.iter().copied().collect();
        self.states.retain(|id, _| live.contains(id));
        self.tasks.retain(|id, _| live.contains(id));
        if self.focused.is_some_and(|id| !live.contains(&id)) {
            self.set_focus(None);
        }
    }

    /// Runs a rebuild a widget asked for while painting.
    fn rebuild_if_due(&mut self) {
        if std::mem::take(&mut self.rebuild_due) {
            self.rebuild();
            self.drive();
            self.relayout();
        }
    }

    /// A task or the renderer setup can progress.
    fn woken(&mut self) {
        if let Some(w) = &self.loop_waker {
            w.clear();
        }
        self.poll_renderer();
        if self.drive() {
            self.redraw();
        }
    }

    fn poll_renderer(&mut self) {
        let Some(f) = &mut self.pending_renderer else {
            return;
        };
        if let Poll::Ready(r) = f.as_mut().poll(&mut Context::from_waker(&self.waker)) {
            self.pending_renderer = None;
            let r = match r {
                Ok(r) => r,
                Err(e) => startup_failed(&e),
            };
            // Resizes during setup were scaled without the window's factor.
            let ((w, h), s) = (r.window().size(), r.window().scale());
            self.viewport = (w as f32 / s, h as f32 / s);
            self.renderer = Some(r);
            self.update_insets();
            self.redraw();
        }
    }

    /// Polls tasks, rebuilding while they change state (new tasks start in a
    /// rebuild and get polled too). Returns whether anything changed.
    fn drive(&mut self) -> bool {
        let mut changed = false;
        while task::poll(&mut self.tasks, &mut self.states, &self.waker) {
            self.rebuild();
            changed = true;
        }
        changed
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
        let s = r.window().scale();
        self.insets = android::insets(a).map(|v| v as f32 / s);
        log::info!("insets {:?}", self.insets);
    }

    #[cfg(not(target_os = "android"))]
    fn update_insets(&mut self) {}

    /// The window's new size, physical px.
    fn resized(&mut self, w: u32, h: u32) {
        let scale = self.scale();
        if let Some(r) = &mut self.renderer {
            r.resize(w, h);
        }
        self.viewport = (w as f32 / scale, h as f32 / scale);
        self.update_insets();
        self.redraw();
    }

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

    /// Whether a widget stopped the event.
    fn dispatch(&mut self, ev: Event) -> bool {
        if self.ids.len() != self.placed.len() {
            return false;
        }
        let mut updates = Vec::new();
        let mut copied = None;
        let mut redraw = false;
        let now = self.now();
        let mut focus = None;
        let keyboard = matches!(ev, Event::Text(_) | Event::Key(_));
        let captured = self.captured.filter(|_| !matches!(ev, Event::Back));
        let find = |id: u64| self.ids.iter().position(|&n| n == id);
        let targets: Vec<usize> = match (keyboard, captured) {
            (true, _) => self.focused.and_then(find).into_iter().collect(),
            (false, Some(id)) => find(id).into_iter().collect(),
            (false, None) if matches!(ev, Event::Cancel | Event::Back) => {
                self.order.iter().rev().copied().collect()
            }
            // Pointer events go to the nodes under them, the tracked ones and
            // layers (overlays watch for taps anywhere); no other node acts
            // on a pointer that isn't over it.
            (false, None) => {
                let ids = &self.ids;
                // A rebuild can move a node; find it again by id.
                let at = |&(i, id): &(usize, u64)| match ids.get(i) == Some(&id) {
                    true => Some(i),
                    false => ids.iter().position(|&n| n == id),
                };
                let mut t: Vec<usize> = self
                    .hovered
                    .iter()
                    .chain(&self.pressed)
                    .filter_map(at)
                    .collect();
                if let Some((x, y)) = pointer_at(&ev) {
                    // ponytail: scans every placed box; a spatial index if this shows up again.
                    let hits: Vec<(usize, u64)> = (0..ids.len())
                        .filter(|&i| {
                            let p = &self.placed[i];
                            p.rect.contains(x, y) && p.clip.contains(x, y)
                        })
                        .map(|i| (i, ids[i]))
                        .collect();
                    t.extend(hits.iter().map(|h| h.0));
                    if let Event::PointerDown { .. } = ev {
                        self.pressed = hits.clone();
                    }
                    self.hovered = hits;
                }
                t.extend(&self.layers);
                t.retain(|&i| !self.placed[i].inert);
                // Topmost first: layers, then descendants and later siblings.
                t.sort_unstable_by_key(|&i| std::cmp::Reverse(self.rank[i]));
                t.dedup();
                t
            }
        };
        let mut sorted = targets.clone();
        sorted.sort_unstable();
        let widgets = walk_to(&self.root, &self.ends, &sorted);
        let widget = |i: usize| widgets[sorted.binary_search(&i).unwrap()];
        let mut send = |i: usize, ev: &Event, states: &mut HashMap<u64, State>| {
            let (id, p) = (self.ids[i], self.placed[i]);
            let mut cx = EventCx {
                now,
                rect: p.rect,
                clip: p.clip,
                content: p.content,
                focused: self.focused == Some(id),
                shift: self.mods.shift_key(),
                state: states.entry(id).or_default(),
                text: &self.text,
                updates: &mut updates,
                copied: &mut copied,
                capture: false,
                focus: None,
                stop: false,
                redraw: false,
            };
            widget(i).event(ev, &mut cx);
            redraw |= cx.redraw;
            if let Some(k) = cx.focus {
                focus = Some((id, k));
            }
            // A press taken by a capture never reached what it would have focused.
            if cx.capture {
                focus = None;
            }
            (cx.capture, cx.stop)
        };
        let mut stopped = false;
        for &i in &targets {
            let (capture, stop) = send(i, &ev, &mut self.states);
            stopped = stop;
            if capture && self.captured.is_none() {
                self.captured = Some(self.ids[i]);
                // Only nodes that got pointer events can have a gesture to drop.
                for &j in targets.iter().filter(|&&j| j != i) {
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
            self.pressed.clear();
        }
        if matches!(ev, Event::Leave | Event::Cancel) {
            self.hovered.clear();
            self.pressed.clear();
        }
        if let Some(t) = copied {
            self.clipboard.set(t);
        }
        // A tap that no widget claims for focus blurs the focused one.
        if matches!(ev, Event::PointerDown { .. }) {
            redraw |= self.set_focus(focus);
        }

        let rebuild = !updates.is_empty();
        if rebuild {
            for (id, f) in updates {
                f(self.states.entry(id).or_default());
            }
            self.rebuild();
            self.drive();
        }
        if rebuild {
            self.redraw();
        } else if redraw {
            self.reposition();
        }
        stopped
    }

    fn key_pressed(&mut self, key: &WKey, text: Option<&str>) {
        let command = if cfg!(target_os = "macos") {
            self.mods.super_key()
        } else {
            self.mods.control_key()
        };
        let shortcut =
            |k: &str| command && matches!(key, WKey::Character(c) if c.eq_ignore_ascii_case(k));
        let k = match key {
            WKey::Named(NamedKey::Backspace) => Some(Key::Backspace),
            WKey::Named(NamedKey::Delete) => Some(Key::Delete),
            WKey::Named(NamedKey::ArrowLeft) => Some(Key::Left),
            WKey::Named(NamedKey::ArrowRight) => Some(Key::Right),
            WKey::Named(NamedKey::Home) => Some(Key::Home),
            WKey::Named(NamedKey::End) => Some(Key::End),
            WKey::Named(NamedKey::Escape) => {
                self.dispatch(Event::Back);
                return;
            }
            _ if shortcut("a") => Some(Key::SelectAll),
            _ if shortcut("c") => Some(Key::Copy),
            _ if shortcut("x") => Some(Key::Cut),
            _ => None,
        };
        if let Some(k) = k {
            self.dispatch(Event::Key(k));
        } else if shortcut("v") {
            // Pasting is typing the clipboard's text.
            if self.focused.is_some()
                && let Some(t) = self.clipboard.get().map(|t| printable(&t))
                && !t.is_empty()
            {
                self.dispatch(Event::Text(t));
            }
        } else if !(self.mods.control_key() && !self.mods.alt_key() || self.mods.super_key())
            && let Some(t) = text
        {
            let t = printable(t);
            if !t.is_empty() {
                self.dispatch(Event::Text(t));
            }
        }
    }

    /// A finger's touch, logical px.
    fn touch(&mut self, id: u64, phase: TouchPhase, x: f32, y: f32) {
        // ponytail: single-pointer only; multi-touch gestures need per-finger dispatch.
        if self.touch.is_some_and(|t| t != id) {
            return;
        }
        match phase {
            TouchPhase::Started => {
                self.touch = Some(id);
                self.dispatch(Event::PointerDown { x, y });
            }
            TouchPhase::Moved => {
                self.dispatch(Event::PointerMove { x, y });
            }
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

    /// Returns whether focus changed.
    fn set_focus(&mut self, to: Option<(u64, Keyboard)>) -> bool {
        let id = to.map(|(id, _)| id);
        let keyboard = to.map(|(_, k)| k);
        let changed = self.focused != id;
        self.focused = id;
        // ponytail: IME only drives the Android soft keyboard; desktop composition (CJK) needs Ime::Preedit handling.
        // Tapping the focused input again brings back a keyboard the user dismissed.
        if changed || id.is_some() {
            #[cfg(test)]
            self.ime_requests.push(keyboard);
            if let Some(r) = &self.renderer {
                r.window().set_ime(keyboard);
            }
        }
        changed
    }

    fn paint(&mut self) {
        let start = Instant::now();
        let mut frame = false;
        let (scene, spans) = self.scene(&mut frame);
        self.rebuild_if_due();
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
                    let mhz = r.window().refresh_mhz();
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
        let app_theme = self.app.theme();
        let mut themes: Vec<&crate::Theme> = Vec::with_capacity(nodes.len());
        for n in &nodes {
            let inherited = n.parent.map_or(&app_theme, |p| themes[p]);
            let provided = n.w.provide().and_then(|v| v.downcast_ref());
            themes.push(provided.unwrap_or(inherited));
        }
        for &i in &self.order {
            let (n, p) = (&nodes[i], &self.placed[i]);
            let start = scene.len();
            if culled(p.rect, p.clip) {
                spans.push((n.id, start..start));
                continue;
            }
            n.w.paint(&mut PaintCx {
                now,
                rect: p.rect,
                clip: p.clip,
                focused: self.focused == Some(n.id),
                theme: themes[i],
                state: self.states.entry(n.id).or_default(),
                scene: &mut scene,
                text: &mut self.text,
                frame: &mut *frame,
                wake: &mut self.wake,
                place: &mut self.place,
                rebuild: &mut self.rebuild_due,
            });
            spans.push((n.id, start..scene.len()));
        }
        self.text.end_frame();
        (scene, spans)
    }
}

/// How far past its box a widget may draw, e.g. a shadow's blur and offset.
const OVERDRAW: f32 = 48.0;

/// Whether nothing `rect` paints can show through `clip`, so its paint is skipped.
// ponytail: fixed overdraw margin; a per-widget paint bound if something draws farther out.
fn culled(rect: Rect, clip: Rect) -> bool {
    let m = OVERDRAW;
    let grown = Rect::new(rect.x - m, rect.y - m, rect.w + 2.0 * m, rect.h + 2.0 * m);
    let v = grown.intersect(&clip);
    v.w <= 0.0 || v.h <= 0.0
}

fn pointer_at(ev: &Event) -> Option<(f32, f32)> {
    match *ev {
        Event::PointerDown { x, y }
        | Event::PointerMove { x, y }
        | Event::PointerUp { x, y }
        | Event::Wheel { x, y, .. } => Some((x, y)),
        _ => None,
    }
}

fn printable(t: &str) -> String {
    t.chars().filter(|c| !c.is_control()).collect()
}

struct Flat {
    ids: Vec<u64>,
    order: Vec<usize>,
    ends: Vec<usize>,
    rank: Vec<usize>,
    layers: Vec<usize>,
}

fn flat_ids(root: &Element, states: &mut HashMap<u64, State>, tasks: &mut Tasks) -> Flat {
    let nodes = flatten(root, states, tasks);
    let order = order(&nodes);
    let mut rank = vec![0; nodes.len()];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r;
    }
    Flat {
        ids: nodes.iter().map(|n| n.id).collect(),
        ends: nodes.iter().map(|n| n.end).collect(),
        layers: (0..nodes.len())
            .filter(|&i| nodes[i].w.layer().is_some())
            .collect(),
        rank,
        order,
    }
}
