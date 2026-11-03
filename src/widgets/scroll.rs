use taffy::{Display, Overflow, Point, Style};
use web_time::Instant;

use super::Styled;
use crate::kinetic::Drag;
use crate::widget::{Element, Event, EventCx, PaintCx, Placement, State, Widget};

#[derive(Default)]
pub(crate) struct ScrollState {
    pub offset: f32,
    /// Largest offset, from the last event's layout.
    pub max: f32,
    pub drag: Drag,
}

/// Vertical scroll view; drag (touch, flings on release) or wheel. Children are clipped.
pub struct Scroll {
    child: [Element; 1],
    style: Style,
}

/// Vertical scroll view: wheel, drag and fling, clipping `child` to its box.
///
/// It's as tall as its parent gives it, but a flex parent grows to fit its
/// content unless told not to: give every flex box between the window and
/// the scroll `.fill()` (not just `.grow(1.0)`), or the scroll grows to the
/// whole list and never scrolls.
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
    }
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
    fn scroll(&self, state: &mut State, _: Placement, now: Instant) -> Option<(f32, f32)> {
        let s = state.get::<ScrollState>();
        if let Some(o) = s.drag.fling(now, s.max) {
            s.offset = o;
        }
        Some((0.0, s.offset))
    }
    fn paint(&self, cx: &mut PaintCx) {
        if cx.state.get::<ScrollState>().drag.flinging() {
            cx.request_placement();
        }
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        drag_scroll(ev, cx, |st| st.get::<ScrollState>());
    }
}

/// Wheel, drag and fling for a vertical scroller whose `ScrollState` `get` finds.
pub(crate) fn drag_scroll(
    ev: &Event,
    cx: &mut EventCx,
    get: impl FnOnce(&mut State) -> &mut ScrollState,
) {
    let max = (cx.content.height - cx.rect.h).max(0.0);
    let hit = |x, y| cx.rect.contains(x, y) && cx.clip.contains(x, y);
    let now = cx.now;
    let s = get(cx.state);
    s.max = max;
    let (mut capture, mut redraw, mut stop) = (false, false, false);
    match *ev {
        // ponytail: a press that catches a fling also reaches the widget under it.
        Event::PointerDown { x, y } if hit(x, y) => s.drag.down(y, s.offset, now),
        Event::PointerMove { y, .. } => {
            if let Some(o) = s.drag.moved(y, now) {
                s.offset = o.clamp(0.0, max);
                (capture, redraw) = (true, true);
            }
        }
        Event::PointerUp { y, .. } => redraw = s.drag.up(y, s.offset, now),
        Event::Cancel => s.drag.cancel(),
        Event::Wheel { x, y, dy } if hit(x, y) => {
            s.drag.stop();
            s.offset = (s.offset + dy).clamp(0.0, max);
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
