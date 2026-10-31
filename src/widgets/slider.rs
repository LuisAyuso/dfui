use std::ops::RangeInclusive;

use taffy::{AvailableSpace, Size, Style};

use super::Styled;
use crate::render::{Color, Rect, Shadow};
use crate::text::Text;
use crate::widget::{Element, Event, EventCx, PaintCx, Widget};

type OnChange<Msg> = Box<dyn Fn(&mut EventCx<Msg>, f32)>;

const THUMB: f32 = 22.0;
const TRACK: Color = Color::hex(0xd5d8e0);

/// Controlled horizontal slider over `range`: shows `value`, reports drags
/// via `on_change`. Tapping the track jumps there.
pub struct Slider<Msg> {
    value: f32,
    range: RangeInclusive<f32>,
    accent: Color,
    on_change: Option<OnChange<Msg>>,
    style: Style,
}

pub fn slider<Msg>(value: f32, range: RangeInclusive<f32>) -> Slider<Msg> {
    Slider {
        value,
        range,
        accent: Color::hex(0x3b6fe0),
        on_change: None,
        style: Style::default(),
    }
}

impl<Msg> Slider<Msg> {
    /// Called with each new value while dragging, within `range`.
    pub fn on_change(mut self, f: impl Fn(&mut EventCx<Msg>, f32) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }
    pub fn accent(mut self, c: Color) -> Self {
        self.accent = c;
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

impl<Msg: 'static> From<Slider<Msg>> for Element<Msg> {
    fn from(w: Slider<Msg>) -> Self {
        Element::new(w)
    }
}

impl<Msg> Styled for Slider<Msg> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<Msg> Widget<Msg> for Slider<Msg> {
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
        let r = cx.rect;
        let (x0, len) = Self::track(r);
        let cy = r.y + r.h / 2.0;
        let x = x0 + self.fraction() * len;
        cx.fill(Rect::new(x0, cy - 2.0, len, 4.0), 2.0, TRACK);
        cx.fill(Rect::new(x0, cy - 2.0, x - x0, 4.0), 2.0, self.accent);
        let thumb = Rect::new(x - THUMB / 2.0, cy - THUMB / 2.0, THUMB, THUMB);
        let sh = Shadow {
            color: Color::BLACK.alpha(0.25),
            offset: (0.0, 1.0),
            blur: 3.0,
        };
        cx.shadow(thumb, THUMB / 2.0, sh);
        cx.fill_bordered(thumb, THUMB / 2.0, Color::WHITE, 1.0, TRACK);
    }
    fn event(&self, ev: &Event, cx: &mut EventCx<Msg>) {
        let dragging = *cx.state.get::<bool>();
        let x = match *ev {
            Event::PointerDown { x, y } if cx.hit(x, y) => {
                *cx.state.get::<bool>() = true;
                cx.capture();
                x
            }
            Event::PointerMove { x, .. } if dragging => x,
            Event::PointerUp { .. } | Event::Cancel => {
                *cx.state.get::<bool>() = false;
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
