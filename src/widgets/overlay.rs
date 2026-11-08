use std::rc::Rc;
use std::time::Duration;

use taffy::{
    AlignItems, Dimension, FlexDirection, JustifyContent, LengthPercentageAuto, Position, Style,
};
use web_time::Instant;

use super::{
    Container, Snapshot, Stateful, Styled, button, column, container, future_builder, row, scroll,
    stateful, text,
};
use crate::anim::{Animated, ease};
use crate::render::{Color, Shadow};
use crate::runtime::task::{self, sleep};
use crate::theme::Role;
use crate::widget::{
    Element, Event, EventCx, Handle, Layer, PaintCx, Placement, Scope, State, Widget,
};

use super::button::Handler;
type OnChange = Rc<dyn Fn(&mut EventCx, usize)>;
type Build = Rc<dyn Fn(Shown) -> Overlay>;

/// Slide and scrim fade, in and out.
const MOTION: Duration = Duration::from_millis(250);

/// Hold on a tooltip's anchor that shows it on touch, and how long it stays
/// after the finger lifts.
const LONG_PRESS: Duration = Duration::from_millis(500);
const LINGER: Duration = Duration::from_millis(1500);

// ponytail: fixed, so a window shorter than this still clips the menu; bound it by the space left if that bites.
const MENU_MAX_HEIGHT: f32 = 320.0;

const SCRIM: Color = Color::BLACK.alpha(0.4);
const SHADOW: Shadow = Shadow {
    color: Color::BLACK.alpha(0.25),
    offset: (0.0, 4.0),
    blur: 16.0,
};

#[derive(Clone, Copy)]
enum Show {
    Always,
    Hover,
    For(Duration),
}

#[derive(Default)]
struct OverlayState {
    hovered: bool,
    /// When a press on a tooltip's anchor started.
    pressed: Option<Instant>,
    /// Until when a long-pressed tooltip stays after release.
    linger: Option<Instant>,
    since: Option<Instant>,
    /// Whether the last placement had it open; only then does it take input.
    shown: bool,
    /// 0 closed, 1 open, for overlays that slide.
    t: Animated<f32>,
}

/// Content floating above the rest of the tree, outside the layout flow.
pub struct Overlay {
    layer: Layer,
    style: Style,
    child: [Element; 1],
    scrim: Option<Color>,
    on_dismiss: Option<Handler>,
    show: Show,
    /// Unit direction the content slides out toward, clearing the clip;
    /// `None` pops.
    slide: Option<(f32, f32)>,
    closing: bool,
}

/// Stops pointer input that hits its content, so whatever reaches the
/// overlay itself landed outside it.
struct Sink {
    child: [Element; 1],
    style: Style,
}

fn layer(layer: Layer, style: Style, sink: Style, child: Element) -> Overlay {
    Overlay {
        layer,
        style,
        child: [Element::new(Sink {
            child: [child],
            style: sink,
        })],
        scrim: None,
        on_dismiss: None,
        show: Show::Always,
        slide: None,
        closing: false,
    }
}

fn column_style() -> Style {
    Style {
        flex_direction: FlexDirection::Column,
        ..Style::default()
    }
}

/// Covers the window; place `child` in it with `align` and `justify`.
pub fn overlay(child: impl Into<Element>) -> Overlay {
    layer(Layer::Window, column_style(), column_style(), child.into())
}

/// Floats `child` just below its parent, at least as wide.
pub fn popup(child: impl Into<Element>) -> Overlay {
    let mut sink = beside(false);
    sink.min_size.width = LengthPercentageAuto::percent(1.0);
    layer(Layer::Parent, Style::default(), sink, child.into())
}

/// Content box 4px past the parent's bottom edge, or its top one if `above`.
fn beside(above: bool) -> Style {
    let (near, far) = (
        LengthPercentageAuto::percent(1.0),
        LengthPercentageAuto::auto(),
    );
    let gap = LengthPercentageAuto::length(4.0);
    let zero = LengthPercentageAuto::length(0.0);
    Style {
        position: Position::Absolute,
        inset: taffy::Rect {
            top: if above { far } else { near },
            bottom: if above { near } else { far },
            left: zero,
            right: far,
        },
        margin: taffy::Rect {
            top: if above { zero } else { gap },
            bottom: if above { gap } else { zero },
            left: zero,
            right: zero,
        },
        size: taffy::Size {
            width: Dimension::max_content(),
            height: Dimension::auto(),
        },
        ..column_style()
    }
}

impl Overlay {
    /// Dims everything under it and blocks its input.
    pub fn modal(mut self, scrim: Color) -> Self {
        self.scrim = Some(scrim);
        self
    }
    /// Runs on a press outside the content, which is consumed, and on
    /// Escape or Android back while it is the topmost overlay.
    pub fn on_dismiss(mut self, f: impl Fn(&mut EventCx) + 'static) -> Self {
        self.on_dismiss = Some(Box::new(f));
        self
    }
}

impl From<Overlay> for Element {
    fn from(w: Overlay) -> Self {
        Element::new(w)
    }
}

impl Styled for Overlay {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

fn pointer(ev: &Event) -> Option<(f32, f32)> {
    match *ev {
        Event::PointerDown { x, y }
        | Event::PointerMove { x, y }
        | Event::PointerUp { x, y }
        | Event::Wheel { x, y, .. } => Some((x, y)),
        _ => None,
    }
}

impl Widget for Overlay {
    fn layer(&self) -> Option<Layer> {
        Some(self.layer)
    }
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element] {
        &self.child
    }
    fn scroll(&self, state: &mut State, at: Placement, now: Instant) -> Option<(f32, f32)> {
        let s = state.get::<OverlayState>();
        let first = s.since.is_none();
        let since = *s.since.get_or_insert(now);
        if s.linger.is_some_and(|u| now >= u) {
            s.linger = None;
        }
        s.shown = !self.closing
            && match self.show {
                Show::Always => true,
                Show::Hover => {
                    let held = s.pressed.map(|t| now >= t + LONG_PRESS);
                    held.unwrap_or(s.hovered) || s.linger.is_some()
                }
                Show::For(d) => now < since + d,
            };
        // Hidden content is scrolled out of the overlay's clip.
        let Some((dx, mut dy)) = self.slide else {
            return (!s.shown).then_some((0.0, 1e6));
        };
        // Flipped above its anchor, a popup unrolls the other way.
        if self.layer == Layer::Parent && at.rest.y < at.anchor.y {
            dy = -dy;
        }
        if first {
            s.t.to(0.0, now, MOTION, ease::out_cubic);
        }
        let target = if s.shown { 1.0 } else { 0.0 };
        s.t.to(target, now, MOTION, ease::out_cubic);
        let k = 1.0 - s.t.get(now);
        // Just clear of the clip still leaves the shadow in it.
        if k >= 1.0 {
            return Some((-dx * 1e6, -dy * 1e6));
        }
        // How far the content travels along the slide to clear the clip.
        let (c, r) = (at.clip, at.rest);
        let reach = |d: f32, lo: f32, hi: f32, start: f32, end: f32| match d {
            d if d > 0.0 => hi - start,
            d if d < 0.0 => end - lo,
            _ => 0.0,
        };
        let w = reach(dx, c.x, c.x + c.w, r.x, r.x + r.w);
        let h = reach(dy, c.y, c.y + c.h, r.y, r.y + r.h);
        (k > 0.0).then_some((-dx * k * w, -dy * k * h))
    }
    fn paint(&self, cx: &mut PaintCx) {
        let now = cx.now;
        let s = cx.state.get::<OverlayState>();
        let (shown, since, moving) = (s.shown, s.since, s.t.running(now));
        let press = s.pressed.map(|t| t + LONG_PRESS).filter(|_| !shown);
        let linger = s.linger.filter(|_| shown);
        let t = match self.slide {
            Some(_) => s.t.get(now),
            None if shown => 1.0,
            None => 0.0,
        };
        let timer = match (self.show, since) {
            (Show::For(d), Some(since)) if shown => Some(since + d),
            (Show::Hover, _) => press.or(linger),
            _ => None,
        };
        if (self.slide.is_some() && moving) || timer.is_some_and(|at| now >= at) {
            cx.request_placement();
        } else if let Some(at) = timer {
            cx.request_frame_at(at);
        }
        if let Some(c) = self.scrim
            && t > 0.0
        {
            cx.fill(cx.clip, 0.0, c.alpha(c.a * t));
        }
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        if let Event::Back = ev {
            if cx.state.get::<OverlayState>().shown {
                if let Some(f) = &self.on_dismiss {
                    f(cx);
                    cx.stop();
                }
                // A modal keeps Back from what's under it.
                if self.scrim.is_some() {
                    cx.stop();
                }
            }
            return;
        }
        let at = pointer(ev);
        if let Show::Hover = self.show {
            let (now, hit) = (cx.now, at.map(|(x, y)| cx.hit(x, y)));
            let s = cx.state.get::<OverlayState>();
            let before = (s.hovered, s.pressed, s.linger);
            match ev {
                Event::PointerDown { .. } if hit == Some(true) => {
                    (s.hovered, s.pressed) = (true, Some(now));
                }
                // The anchor took the press; the Leave after it still comes.
                // ponytail: a touch the system cancels leaves `pressed` set until the next Leave.
                Event::Cancel => {}
                Event::PointerUp { .. } | Event::Leave => {
                    if s.pressed.is_some_and(|t| now >= t + LONG_PRESS) {
                        s.linger = Some(now + LINGER);
                    }
                    s.pressed = None;
                    s.hovered &= !matches!(ev, Event::Leave);
                }
                _ => {
                    if let Some(h) = hit {
                        s.hovered = h;
                        s.pressed = s.pressed.filter(|_| h);
                    }
                }
            }
            if (s.hovered, s.pressed, s.linger) != before {
                cx.request_redraw();
            }
        }
        if at.is_none() || !cx.state.get::<OverlayState>().shown {
            return;
        }
        if let (Event::PointerDown { .. }, Some(f)) = (ev, &self.on_dismiss) {
            f(cx);
            cx.stop();
        }
        if self.scrim.is_some() {
            cx.stop();
        }
    }
}

impl Widget for Sink {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element] {
        &self.child
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        if let Some((x, y)) = pointer(ev)
            && cx.hit(x, y)
        {
            cx.stop();
        }
    }
}

fn card(child: impl Into<Element>) -> Container {
    container(child)
        .bg(Role::Surface)
        .content(Role::Ink)
        .radius(16.0)
        .padding(24.0)
        .shadow(SHADOW)
}

/// Modal card centered over the dimmed window.
pub fn dialog(child: impl Into<Element>) -> Overlay {
    let mut o = overlay(card(child))
        .modal(SCRIM)
        .align(AlignItems::CENTER)
        .justify(JustifyContent::CENTER)
        .padding(24.0);
    o.slide = Some((0.0, 1.0));
    o
}

/// Modal full-height panel along the left edge.
pub fn drawer(child: impl Into<Element>) -> Overlay {
    let panel = card(child).radius(0.0).width(280.0).grow(1.0);
    let mut o = overlay(panel).modal(SCRIM);
    o.style.flex_direction = FlexDirection::Row;
    o.slide = Some((-1.0, 0.0));
    o
}

/// Modal panel along the bottom edge, full width.
pub fn bottom_sheet(child: impl Into<Element>) -> Overlay {
    // Runs a radius past the window's bottom edge, whose clip squares off
    // the bottom corners.
    let mut panel = card(child);
    panel.style_mut().padding.bottom = taffy::LengthPercentage::length(24.0 + 16.0);
    panel.style_mut().margin.bottom = LengthPercentageAuto::length(-16.0);
    let mut o = overlay(panel)
        .modal(SCRIM)
        .justify(JustifyContent::FLEX_END);
    o.slide = Some((0.0, 1.0));
    o
}

/// Short message at the bottom of the window for four seconds after it
/// enters the tree. Shown through `Overlays` it waits for earlier ones to
/// leave and is removed after; in a plain tree give it a new `key` to show
/// it again.
pub fn snackbar(message: impl Into<String>) -> Overlay {
    let bar = container(text(message).size(14.0).color(Role::OnInverse))
        .bg(Role::Inverse)
        .radius(8.0)
        .padding_xy(16.0, 14.0);
    let mut o = overlay(bar)
        .align(AlignItems::CENTER)
        .justify(JustifyContent::FLEX_END)
        .padding(16.0);
    o.show = Show::For(Duration::from_secs(4));
    o.slide = Some((0.0, 1.0));
    o
}

/// Overlays opened from handlers with `Handle<Overlays>::show`, each kept
/// until its exit animation ends.
#[derive(Default)]
pub struct Overlays {
    entries: Vec<Entry>,
    next: u64,
}

struct Entry {
    key: u64,
    build: Build,
    closing: bool,
    /// When a timed one enters the tree, once the ones before it are gone.
    from: Option<Instant>,
    /// When it leaves the tree: set on close, or at show for timed ones.
    until: Option<Instant>,
}

/// One overlay shown through `Overlays`, passed to its builder.
#[derive(Clone, Copy)]
pub struct Shown {
    h: Handle<Overlays>,
    key: u64,
}

impl Shown {
    /// Animates it out, then removes it.
    pub fn close(self, cx: &mut EventCx) {
        let key = self.key;
        cx.update(self.h, move |o: &mut Overlays| {
            let now = task::now();
            let Some(e) = o.entries.iter_mut().find(|e| e.key == key && !e.closing) else {
                return;
            };
            e.closing = true;
            e.until = Some(now + MOTION);
            if e.from.is_some_and(|f| f > now) {
                (e.from, e.until) = (Some(now), Some(now));
            }
            requeue(&mut o.entries, now);
        });
    }
}

/// Moves timed entries still waiting at `now` up into time freed before them.
fn requeue(entries: &mut [Entry], now: Instant) {
    let mut end = now;
    for e in entries {
        let (Some(from), Some(until)) = (e.from, e.until) else {
            continue;
        };
        if from > now {
            (e.from, e.until) = (Some(end), Some(end + (until - from)));
        }
        end = end.max(e.until.unwrap_or(end));
    }
}

impl Handle<Overlays> {
    /// Shows the overlay `build` returns, rebuilt with the rest of the tree.
    /// Modal ones without `on_dismiss` close on a tap outside.
    pub fn show(self, cx: &mut EventCx, build: impl Fn(Shown) -> Overlay + 'static) {
        let h = self;
        cx.update(self, move |o: &mut Overlays| {
            let now = task::now();
            // Removed ones linger until the next show; they build nothing.
            o.entries.retain(|e| e.until.is_none_or(|u| u > now));
            let key = o.next;
            o.next += 1;
            let (from, until) = match build(Shown { h, key }).show {
                Show::For(d) => {
                    let last = o.entries.iter().filter(|e| e.from.is_some());
                    let from = last.filter_map(|e| e.until).fold(now, Instant::max);
                    (Some(from), Some(from + d + MOTION))
                }
                _ => (None, None),
            };
            o.entries.push(Entry {
                key,
                build: Rc::new(build),
                closing: false,
                from,
                until,
            });
        });
    }
}

impl Handle<Overlays> {
    /// Asks before an action: a dialog with `title`, `body`, Cancel and `ok`.
    /// `on_ok` runs as the dialog closes.
    pub fn confirm(
        self,
        cx: &mut EventCx,
        title: impl Into<String>,
        body: impl Into<String>,
        ok: impl Into<String>,
        on_ok: impl Fn(&mut EventCx) + 'static,
    ) {
        let (title, body, ok): (String, String, String) = (title.into(), body.into(), ok.into());
        let on_ok = Rc::new(on_ok);
        self.show(cx, move |me| {
            let on_ok = on_ok.clone();
            dialog(
                column([
                    text(title.clone()).size(20.0).into(),
                    text(body.clone()).size(14.0).color(Role::Muted).into(),
                    row([
                        button(text("Cancel").color(Role::Ink))
                            .color(Role::Surface)
                            .flat()
                            .on_click(move |cx| me.close(cx))
                            .into(),
                        button(text(ok.clone()))
                            .flat()
                            .on_click(move |cx| {
                                me.close(cx);
                                on_ok(cx);
                            })
                            .into(),
                    ])
                    .gap(8.0)
                    .justify(JustifyContent::FLEX_END)
                    .into(),
                ])
                .gap(12.0),
            )
        });
    }
}

/// Hosts `content` and the overlays its handlers show through the handle.
pub fn overlays<E: Into<Element>>(
    content: impl Fn(Handle<Overlays>) -> E + 'static,
) -> Stateful<Overlays> {
    stateful(move |s: &Overlays, h| {
        let shown = s.entries.iter().map(move |e| entry(e, h));
        column(std::iter::once(content(h).into()).chain(shown)).fill()
    })
}

/// An entry behind a timer that removes it at `until`, and a timed one
/// behind another that adds it at `from`. Always builders, so closing keeps
/// the overlay's state (its animation).
fn entry(e: &Entry, h: Handle<Overlays>) -> Element {
    let (build, closing, key, until) = (e.build.clone(), e.closing, e.key, e.until);
    let timer = move |_: Scope| async move {
        match until {
            Some(u) => sleep(u.saturating_duration_since(task::now())).await,
            None => std::future::pending().await,
        }
    };
    let view = move |snap: &Snapshot<Option<()>>| -> Element {
        if snap.done {
            return column([]).into();
        }
        let shown = Shown { h, key };
        let mut o = build(shown);
        o.closing = closing;
        if o.scrim.is_some() && o.on_dismiss.is_none() {
            o.on_dismiss = Some(Box::new(move |cx| shown.close(cx)));
        }
        o.into()
    };
    let shown = move || Element::from(future_builder((key, closing), timer, view.clone()));
    let Some(from) = e.from else {
        return shown().key(key);
    };
    let queue = move |_: Scope| sleep(from.saturating_duration_since(task::now()));
    let wait = move |snap: &Snapshot<Option<()>>| match snap.done {
        true => shown(),
        false => column([]).into(),
    };
    Element::from(future_builder((key, from), queue, wait)).key(key)
}

/// Runs `f` on Escape or the Android back button, unless an overlay takes it
/// first, and keeps the app open. Leave it out where Back should exit.
pub struct OnBack(Handler);

pub fn on_back(f: impl Fn(&mut EventCx) + 'static) -> OnBack {
    OnBack(Box::new(f))
}

impl From<OnBack> for Element {
    fn from(w: OnBack) -> Self {
        Element::new(w)
    }
}

impl Widget for OnBack {
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        if let Event::Back = ev {
            (self.0)(cx);
            cx.stop();
        }
    }
}

/// Shows `tip` above `child` while the pointer is over it.
pub fn tooltip(child: impl Into<Element>, tip: impl Into<String>) -> Container {
    let tip = container(text(tip).size(13.0).color(Role::OnInverse))
        .bg(Role::Inverse)
        .radius(6.0)
        .padding_xy(8.0, 4.0);
    let mut p = layer(Layer::Parent, Style::default(), beside(true), tip.into());
    p.show = Show::Hover;
    container(child).child(p)
}

/// Picks one of `labels`: shows the selected one and lists them all on tap.
pub struct Dropdown {
    labels: Vec<String>,
    selected: usize,
    on_change: Option<OnChange>,
}

pub fn dropdown<S: Into<String>>(labels: impl IntoIterator<Item = S>, selected: usize) -> Dropdown {
    Dropdown {
        labels: labels.into_iter().map(Into::into).collect(),
        selected,
        on_change: None,
    }
}

impl Dropdown {
    /// Called with the chosen option's index when it isn't already selected.
    pub fn on_change(mut self, f: impl Fn(&mut EventCx, usize) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl From<Dropdown> for Element {
    fn from(d: Dropdown) -> Self {
        let Dropdown {
            labels,
            selected,
            on_change,
        } = d;
        let on_change: OnChange = on_change.unwrap_or_else(|| Rc::new(|_, _| {}));
        stateful(move |&open: &bool, h| {
            let set =
                move |v: bool| move |cx: &mut EventCx| cx.update(h, move |o: &mut bool| *o = v);
            let field = button(
                row([
                    text(&labels[selected]).color(Role::Ink).into(),
                    text("▾").color(Role::Ink).into(),
                ])
                .gap(8.0),
            )
            .color(Role::Track)
            .on_click(set(!open));
            let menu = {
                let last = labels.len().saturating_sub(1);
                let items = labels.iter().enumerate().map(|(i, l)| {
                    let f = on_change.clone();
                    let bg = if i == selected {
                        Role::Selected
                    } else {
                        Role::Surface
                    };
                    button(text(l).color(Role::Ink))
                        .color(bg)
                        .flat()
                        .radius(6.0)
                        .justify(JustifyContent::FLEX_START)
                        .on_click(move |cx| {
                            cx.update(h, |o: &mut bool| *o = false);
                            if i != selected {
                                f(cx, i);
                            }
                        })
                        .into()
                });
                let mut options = scroll(column(items)).max_height(MENU_MAX_HEIGHT);
                if open {
                    // Linear in the index, which keeps the selected row in view.
                    options = options.start_at(selected as f32 / last.max(1) as f32);
                }
                let list = column([options.into()])
                    .bg(Role::Surface)
                    .radius(8.0)
                    .padding(4.0)
                    .shadow(SHADOW);
                // Kept while closed, so it can slide shut.
                let mut menu = popup(list).on_dismiss(set(false));
                menu.closing = !open;
                menu.slide = Some((0.0, -1.0));
                menu
            };
            column([field.into(), menu.into()]).align(AlignItems::FLEX_START)
        })
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_a_snackbar_pulls_the_queue_forward() {
        let now = task::now();
        let s = Duration::from_secs;
        let timed = |key, from, until| Entry {
            key,
            build: Rc::new(|_| snackbar("")),
            closing: false,
            from: Some(now + s(from)),
            until: Some(now + s(until)),
        };
        // The first was closed now and leaves at 1 s; the others were queued
        // behind its full 5 s.
        let mut entries = [timed(0, 0, 1), timed(1, 5, 10), timed(2, 10, 15)];
        requeue(&mut entries, now);
        let at = |e: &Entry| (e.from.unwrap() - now, e.until.unwrap() - now);
        assert_eq!(at(&entries[1]), (s(1), s(6)));
        assert_eq!(at(&entries[2]), (s(6), s(11)));
    }
}
