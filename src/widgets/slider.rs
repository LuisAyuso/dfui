use std::ops::RangeInclusive;
use std::time::Duration;

use taffy::{AvailableSpace, Size, Style};

use super::Styled;
use crate::anim::{Animated, ease};
use crate::render::{Color, Rect, Shadow};
use crate::text::Text;
use crate::theme::{Ink, Role};
use crate::widget::{Element, Event, EventCx, PaintCx, Widget};

type OnChange = Box<dyn Fn(&mut EventCx, f32)>;

const THUMB: f32 = 22.0;
/// Thumb scale while dragged.
const GRAB: f32 = 1.2;
const GROW: Duration = Duration::from_millis(120);

#[derive(Default)]
struct SliderState {
    dragging: bool,
    scale: Animated<f32>,
}

/// Controlled horizontal slider over `range`: shows `value`, reports drags
/// via `on_change`. Tapping the track jumps there.
pub struct Slider {
    value: f32,
    range: RangeInclusive<f32>,
    accent: Ink,
    on_change: Option<OnChange>,
    style: Style,
}

/// Thumb on a track for a value in `range`; tap to jump, drag to follow.
pub fn slider(value: f32, range: RangeInclusive<f32>) -> Slider {
    Slider {
        value,
        range,
        accent: Role::Accent.into(),
        on_change: None,
        style: Style::default(),
    }
}

impl Slider {
    /// Called with each new value while dragging, within `range`.
    pub fn on_change(mut self, f: impl Fn(&mut EventCx, f32) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }
    /// Color of the track left of the thumb; `Role::Accent` by default.
    pub fn accent(mut self, c: impl Into<Ink>) -> Self {
        self.accent = c.into();
        self
    }

    /// The thumb centre's travel, inset so the thumb stays inside `r`.
    fn track(r: Rect) -> (f32, f32) {
        (r.x + THUMB / 2.0, (r.w - THUMB).max(0.0))
    }

    fn fraction(&self) -> f32 {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        if hi > lo {
            ((self.value - lo) / (hi - lo)).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

impl From<Slider> for Element {
    fn from(w: Slider) -> Self {
        Element::new(w)
    }
}

impl Styled for Slider {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Slider {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn measure(&self, known: Size<Option<f32>>, _: Size<AvailableSpace>, _: &Text) -> Size<f32> {
        Size {
            width: known.width.unwrap_or(160.0),
            height: known.height.unwrap_or(THUMB + 10.0),
        }
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (now, r) = (cx.now, cx.rect);
        let s = cx.state.get::<SliderState>();
        let target = if s.dragging { GRAB } else { 1.0 };
        s.scale.to(target, now, GROW, ease::out_cubic);
        let d = THUMB * s.scale.get(now);
        if s.scale.running(now) {
            cx.request_frame();
        }
        let (x0, len) = Self::track(r);
        let cy = r.y + r.h / 2.0;
        let x = x0 + self.fraction() * len;
        cx.fill(Rect::new(x0, cy - 2.0, len, 4.0), 2.0, cx.theme.track);
        cx.fill(
            Rect::new(x0, cy - 2.0, x - x0, 4.0),
            2.0,
            cx.color(self.accent),
        );
        let thumb = Rect::new(x - d / 2.0, cy - d / 2.0, d, d);
        let sh = Shadow {
            color: Color::BLACK.alpha(0.25),
            offset: (0.0, 1.0),
            blur: 3.0,
        };
        cx.shadow(thumb, d / 2.0, sh);
        cx.fill_bordered(thumb, d / 2.0, cx.theme.surface, 1.0, cx.theme.track);
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let dragging = cx.state.get::<SliderState>().dragging;
        let x = match *ev {
            Event::PointerDown { x, y } if cx.hit(x, y) => {
                cx.state.get::<SliderState>().dragging = true;
                cx.capture();
                cx.request_redraw();
                x
            }
            Event::PointerMove { x, .. } if dragging => x,
            Event::PointerUp { .. } | Event::Cancel => {
                if dragging {
                    cx.state.get::<SliderState>().dragging = false;
                    cx.request_redraw();
                }
                return;
            }
            _ => return,
        };
        let (x0, len) = Self::track(cx.rect);
        let t = if len > 0.0 {
            ((x - x0) / len).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let v = lo + t * (hi - lo);
        if v != self.value
            && let Some(f) = &self.on_change
        {
            f(cx, v);
        }
    }
}
