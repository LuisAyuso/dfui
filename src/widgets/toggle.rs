use std::time::Duration;

use taffy::{Dimension, Size, Style};

use super::Styled;
use super::button::Tap;
use crate::anim::{Animated, Lerp, ease};
use crate::render::{Color, Rect, Shadow};
use crate::widget::{Element, Event, EventCx, PaintCx, Widget};

type OnChange = Box<dyn Fn(&mut EventCx, bool)>;

const SLIDE: Duration = Duration::from_millis(160);
const OFF: Color = Color::hex(0xc9ccd6);

#[derive(Default)]
struct ToggleState {
    tap: Tap,
    /// 0 off, 1 on.
    pos: Animated<f32>,
}

/// Controlled on/off switch: shows `on`, reports taps via `on_change`.
pub struct Toggle {
    on: bool,
    accent: Color,
    on_change: Option<OnChange>,
    style: Style,
}

/// On/off switch; `on_change` gets the new value.
pub fn toggle(on: bool) -> Toggle {
    Toggle {
        on,
        accent: Color::hex(0x3b6fe0),
        on_change: None,
        style: Style {
            size: Size {
                width: Dimension::length(48.0),
                height: Dimension::length(28.0),
            },
            flex_shrink: 0.0,
            ..Style::default()
        },
    }
}

impl Toggle {
    /// Called with the flipped value.
    pub fn on_change(mut self, f: impl Fn(&mut EventCx, bool) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }
    pub fn accent(mut self, c: Color) -> Self {
        self.accent = c;
        self
    }
}

impl From<Toggle> for Element {
    fn from(w: Toggle) -> Self {
        Element::new(w)
    }
}

impl Styled for Toggle {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Toggle {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (now, r) = (cx.now, cx.rect);
        let s = cx.state.get::<ToggleState>();
        s.pos
            .to(if self.on { 1.0 } else { 0.0 }, now, SLIDE, ease::out_cubic);
        let t = s.pos.get(now);
        if s.pos.running(now) {
            cx.request_frame();
        }
        cx.fill(r, r.h / 2.0, OFF.lerp(self.accent, t));
        let d = r.h - 6.0;
        let thumb = Rect::new(r.x + 3.0 + t * (r.w - d - 6.0), r.y + 3.0, d, d);
        let sh = Shadow {
            color: Color::BLACK.alpha(0.2),
            offset: (0.0, 1.0),
            blur: 2.0,
        };
        cx.shadow(thumb, d / 2.0, sh);
        cx.fill(thumb, d / 2.0, Color::WHITE);
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let mut tap = std::mem::take(&mut cx.state.get::<ToggleState>().tap);
        let fire = tap.event(ev, cx);
        cx.state.get::<ToggleState>().tap = tap;
        if fire && let Some(f) = &self.on_change {
            f(cx, !self.on);
        }
    }
}
