use std::time::Duration;

use taffy::{Display, Overflow, Point, Style};
use web_time::Instant;

use super::Styled;
use crate::anim::{Animated, ease};
use crate::kinetic::Drag;
use crate::widget::{Element, Event, EventCx, PaintCx, Placement, State, Widget};

#[derive(Default)]
pub(crate) struct ScrollState {
    pub offset: f32,
    /// Largest offset, from the last layout.
    pub max: f32,
    pub drag: Drag,
    /// The `start_at` already applied.
    start: Option<f32>,
    /// The `reveal` span already applied, and the glide toward it.
    revealed: Option<(f32, f32)>,
    glide: Option<Animated<f32>>,
}

/// How long `reveal` takes to bring its span into view.
const GLIDE: Duration = Duration::from_millis(150);

/// Scroll view; drag (touch, flings on release) or wheel. Children are clipped.
pub struct Scroll {
    child: [Element; 1],
    style: Style,
    stick: bool,
    start: Option<f32>,
    horizontal: bool,
    reveal: Option<(f32, f32)>,
}

/// Vertical scroll view: wheel, drag and fling, clipping `child` to its box.
/// [`horizontal`](Scroll::horizontal) makes it scroll sideways instead.
///
/// It's as tall as its parent gives it, but a flex parent grows to fit its
/// content unless told not to: give every flex box between the window and
/// the scroll `.fill()` (not just `.grow(1.0)`), or the scroll grows to the
/// whole list and never scrolls.
#[doc = example!("scroll")]
pub fn scroll(child: impl Into<Element>) -> Scroll {
    Scroll {
        child: [child.into()],
        style: Style {
            display: Display::Block,
            overflow: Point {
                x: Overflow::Hidden,
                y: Overflow::Scroll,
            },
            scrollbar_width: 0.0,
            ..Style::default()
        },
        stick: false,
        start: None,
        horizontal: false,
        reveal: None,
    }
}

impl Scroll {
    /// Follows the end as content grows, e.g. for a log. Scrolling away stops
    /// that until the user scrolls back to the end.
    pub fn stick_to_end(mut self) -> Self {
        self.stick = true;
        self
    }
    /// Jumps to this fraction of the range, 0 top to 1 bottom, whenever the
    /// value changes; the user scrolls freely after that.
    pub fn start_at(mut self, fraction: f32) -> Self {
        self.start = Some(fraction);
        self
    }
    /// Scrolls sideways instead: a row wider than its box, like a shelf of
    /// posters. Drags and sideways wheel or touchpad motion (or shift+wheel)
    /// move it; a plain vertical wheel passes through to the page around it.
    /// Give the row's items `.shrink(0.0)`, or the row squeezes them into the box.
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        // As a flex row the child keeps its items' width; a block would stretch
        // it to the box, hiding how far the items reach.
        self.style.display = Display::Flex;
        self.style.overflow = Point {
            x: Overflow::Scroll,
            y: Overflow::Hidden,
        };
        self
    }
    /// Glides the least distance that shows `span` (start and end along the
    /// scroll axis, in content px) whenever it changes, e.g. the item with
    /// keyboard focus; the user scrolls freely after that.
    pub fn reveal(mut self, span: (f32, f32)) -> Self {
        self.reveal = Some(span);
        self
    }
}

/// The offset nearest `from` that shows `span` in a `view` long box, its start
/// if it doesn't fit, within `0..=max`.
fn reveal_offset(from: f32, span: (f32, f32), view: f32, max: f32) -> f32 {
    from.max(span.1 - view).min(span.0).clamp(0.0, max)
}

impl From<Scroll> for Element {
    fn from(w: Scroll) -> Self {
        Element::new(w)
    }
}

impl Styled for Scroll {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Scroll {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element] {
        &self.child
    }
    fn scroll(&self, state: &mut State, at: Placement, now: Instant) -> Option<(f32, f32)> {
        let s = state.get::<ScrollState>();
        if let Some(o) = s.drag.fling(now, s.max) {
            s.offset = o;
        }
        let (view, length) = match self.horizontal {
            true => (at.clip.w, at.content.width),
            false => (at.clip.h, at.content.height),
        };
        let max = (length - view).max(0.0);
        if self.reveal != s.revealed {
            if let Some(span) = self.reveal {
                let mut glide = Animated::default();
                glide.to(s.offset, now, GLIDE, ease::out_cubic);
                glide.to(
                    reveal_offset(s.offset, span, view, max),
                    now,
                    GLIDE,
                    ease::out_cubic,
                );
                s.glide = Some(glide);
            }
            s.revealed = self.reveal;
        }
        if let Some(g) = &s.glide {
            s.offset = g.get(now);
            if !g.running(now) {
                s.glide = None;
            }
        }
        if self.stick && s.offset >= s.max - 1.0 {
            s.offset = max;
        }
        if self.start != s.start {
            if let Some(f) = self.start {
                s.offset = max * f;
            }
            s.start = self.start;
        }
        s.max = max;
        Some(match self.horizontal {
            true => (s.offset, 0.0),
            false => (0.0, s.offset),
        })
    }
    fn paint(&self, cx: &mut PaintCx) {
        let s = cx.state.get::<ScrollState>();
        if s.drag.flinging() || s.glide.is_some() {
            cx.request_placement();
        }
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        drag_scroll(ev, cx, self.horizontal, |st| st.get::<ScrollState>());
    }
}

/// Wheel, drag and fling for a scroller whose [`ScrollState`] `get` finds.
pub(crate) fn drag_scroll(
    ev: &Event,
    cx: &mut EventCx,
    horizontal: bool,
    get: impl FnOnce(&mut State) -> &mut ScrollState,
) {
    let max = match horizontal {
        true => (cx.content.width - cx.rect.w).max(0.0),
        false => (cx.content.height - cx.rect.h).max(0.0),
    };
    let along = |x: f32, y: f32| if horizontal { x } else { y };
    let hit = |x, y| cx.rect.contains(x, y) && cx.clip.contains(x, y);
    let now = cx.now;
    let s = get(cx.state);
    s.max = max;
    let (mut capture, mut redraw, mut stop) = (false, false, false);
    match *ev {
        Event::PointerDown { x, y } if hit(x, y) => {
            // Catching a fling isn't a tap on what's under the finger.
            capture = s.drag.flinging();
            s.glide = None;
            s.drag.down(along(x, y), s.offset, now);
        }
        Event::PointerMove { x, y } => {
            if let Some(o) = s.drag.moved(along(x, y), now) {
                s.offset = o.clamp(0.0, max);
                (capture, redraw) = (true, true);
            }
        }
        Event::PointerUp { x, y } => redraw = s.drag.up(along(x, y), s.offset, now),
        Event::Cancel => s.drag.cancel(),
        // Only motion along this axis; the rest is for scrollers around it.
        Event::Wheel { x, y, dx, dy } if hit(x, y) && along(dx, dy) != 0.0 => {
            s.drag.stop();
            s.glide = None;
            s.offset = (s.offset + along(dx, dy)).clamp(0.0, max);
            (redraw, stop) = (true, true);
        }
        _ => {}
    }
    if capture {
        cx.capture();
    }
    if redraw {
        cx.request_redraw();
    }
    if stop {
        cx.stop();
    }
}
