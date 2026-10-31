use std::rc::Rc;
use std::time::Duration;

use taffy::{
    AlignItems, Dimension, FlexDirection, JustifyContent, LengthPercentageAuto, Position, Style,
};
use web_time::Instant;

use super::{
    Container, Snapshot, Stateful, Styled, button, column, container, future_builder, row,
    stateful, text,
};
use crate::anim::{Animated, ease};
use crate::render::{Color, Rect, Shadow};
use crate::task::sleep;
use crate::widget::{Element, Event, EventCx, Handle, Layer, PaintCx, Scope, State, Widget};

type Handler = Box<dyn Fn(&mut EventCx)>;
type OnChange = Rc<dyn Fn(&mut EventCx, usize)>;
type Build = Rc<dyn Fn(Shown) -> Overlay>;

/// Slide and scrim fade, in and out.
const MOTION: Duration = Duration::from_millis(250);

const SCRIM: Color = Color::BLACK.alpha(0.4);
const INK: Color = Color::hex(0x1d2030);
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
    /// Where the content slides in from, in overlay sizes; `None` pops.
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
    /// Runs on a press outside the content, which is consumed.
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
    fn scroll(&self, state: &mut State, rect: Rect, now: Instant) -> Option<(f32, f32)> {
        let s = state.get::<OverlayState>();
        let first = s.since.is_none();
        let since = *s.since.get_or_insert(now);
        s.shown = !self.closing
            && match self.show {
                Show::Always => true,
                Show::Hover => s.hovered,
                Show::For(d) => now < since + d,
            };
        // Hidden content is scrolled out of the overlay's clip.
        let Some((dx, dy)) = self.slide else {
            return (!s.shown).then_some((0.0, 1e6));
        };
        if first {
            s.t.to(0.0, now, MOTION, ease::out_cubic);
        }
        let target = if s.shown { 1.0 } else { 0.0 };
        s.t.to(target, now, MOTION, ease::out_cubic);
        let k = 1.0 - s.t.get(now);
        (k > 0.0).then_some((-dx * k * rect.w, -dy * k * rect.h))
    }
    fn paint(&self, cx: &mut PaintCx) {
        let now = cx.now;
        let s = cx.state.get::<OverlayState>();
        let (shown, since, moving) = (s.shown, s.since, s.t.running(now));
        let t = match self.slide {
            Some(_) => s.t.get(now),
            None if shown => 1.0,
            None => 0.0,
        };
        let timer = match (self.show, since) {
            (Show::For(d), Some(since)) if shown => Some(since + d),
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
        let at = pointer(ev);
        if let Show::Hover = self.show {
            let hit = at.map(|(x, y)| cx.hit(x, y));
            let s = cx.state.get::<OverlayState>();
            let hovered = match ev {
                Event::Leave | Event::Cancel => false,
                _ => hit.unwrap_or(s.hovered),
            };
            if hovered != s.hovered {
                s.hovered = hovered;
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
        .bg(Color::WHITE)
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
    let mut o = overlay(card(child))
        .modal(SCRIM)
        .justify(JustifyContent::FLEX_END);
    o.slide = Some((0.0, 1.0));
    o
}

/// Short message at the bottom of the window for four seconds after it
/// enters the tree. Shown through `Overlays` it is also removed after;
/// in a plain tree give it a new `key` to show it again.
pub fn snackbar(message: impl Into<String>) -> Overlay {
    let bar = container(text(message).size(14.0).color(Color::WHITE))
        .bg(Color::hex(0x323232))
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
            if let Some(e) = o.entries.iter_mut().find(|e| e.key == key && !e.closing) {
                e.closing = true;
                e.until = Some(Instant::now() + MOTION);
            }
        });
    }
}

impl Handle<Overlays> {
    /// Shows the overlay `build` returns, rebuilt with the rest of the tree.
    /// Modal ones without `on_dismiss` close on a tap outside.
    pub fn show(self, cx: &mut EventCx, build: impl Fn(Shown) -> Overlay + 'static) {
        let h = self;
        cx.update(self, move |o: &mut Overlays| {
            let now = Instant::now();
            // Removed ones linger until the next show; they build nothing.
            o.entries.retain(|e| e.until.is_none_or(|u| u > now));
            let key = o.next;
            o.next += 1;
            let until = match build(Shown { h, key }).show {
                Show::For(d) => Some(now + d + MOTION),
                _ => None,
            };
            o.entries.push(Entry {
                key,
                build: Rc::new(build),
                closing: false,
                until,
            });
        });
    }
}

/// Hosts `content` and the overlays its handlers show through the handle.
pub fn overlays<E: Into<Element>>(
    content: impl Fn(Handle<Overlays>) -> E + 'static,
) -> Stateful<Overlays> {
    stateful(move |s: &Overlays, h| {
        let shown = s.entries.iter().map(move |e| entry(e, h));
        column(std::iter::once(content(h).into()).chain(shown)).grow(1.0)
    })
}

/// An entry behind a timer that removes it at `until`. Always a builder, so
/// closing keeps the overlay's state (its animation).
fn entry(e: &Entry, h: Handle<Overlays>) -> Element {
    let (build, closing, key) = (e.build.clone(), e.closing, e.key);
    let wait = e.until.map(|u| u.saturating_duration_since(Instant::now()));
    let timer = move |_: Scope| async move {
        match wait {
            Some(d) => sleep(d).await,
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
    Element::from(future_builder((key, closing), timer, view)).key(key)
}

/// Shows `tip` above `child` while the pointer is over it.
pub fn tooltip(child: impl Into<Element>, tip: impl Into<String>) -> Container {
    let tip = container(text(tip).size(13.0).color(Color::WHITE))
        .bg(Color::hex(0x404040))
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
                    text(&labels[selected]).color(INK).into(),
                    text("▾").color(INK).into(),
                ])
                .gap(8.0),
            )
            .color(Color::hex(0xe8e9ef))
            .on_click(set(!open));
            let menu = open.then(|| {
                let items = labels.iter().enumerate().map(|(i, l)| {
                    let f = on_change.clone();
                    let bg = if i == selected {
                        Color::hex(0xe8eefc)
                    } else {
                        Color::WHITE
                    };
                    button(text(l).color(INK))
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
                let list = column(items)
                    .bg(Color::WHITE)
                    .radius(8.0)
                    .padding(4.0)
                    .shadow(SHADOW);
                popup(list).on_dismiss(set(false)).into()
            });
            column(std::iter::once(field.into()).chain(menu)).align(AlignItems::FLEX_START)
        })
        .into()
    }
}
