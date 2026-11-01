use std::time::Duration;

use taffy::{AvailableSpace, Size, Style};

use super::Styled;
use super::button::Tap;
use crate::anim::{Animated, ease};
use crate::path::Shape;
use crate::render::{Color, Rect};
use crate::text::Text;
use crate::widget::{Element, Event, EventCx, PaintCx, Widget};

type Handler = Box<dyn Fn(&mut EventCx)>;

/// Icon scale while pressed.
const PRESSED: f32 = 0.8;
const PRESS: Duration = Duration::from_millis(90);
/// Long enough for the overshoot to read as a spring.
const RELEASE: Duration = Duration::from_millis(420);

/// Ease-out that passes the target by 25% of the move halfway through and
/// settles back: from `PRESSED`, the icon peaks ~5% above its size.
fn spring(t: f32) -> f32 {
    let u = t - 1.0;
    1.0 + 4.0 * u * u * u + 3.0 * u * u
}

#[derive(Default)]
struct IconState {
    tap: Tap,
    scale: Animated<f32>,
}

/// A `Shape` drawn `size` px square in one color; a button with `on_click`.
pub struct Icon {
    shape: Shape,
    size: f32,
    color: Color,
    on_click: Option<Handler>,
    style: Style,
}

/// `shape` as a 24 px icon.
pub fn icon(shape: &Shape) -> Icon {
    Icon {
        shape: shape.clone(),
        size: 24.0,
        color: Color::BLACK,
        on_click: None,
        style: Style::default(),
    }
}

/// A borderless button showing `shape`: the icon shrinks while pressed and
/// springs back past its size on release. Padded to a comfortable touch target.
pub fn icon_button(shape: &Shape) -> Icon {
    icon(shape).padding(10.0)
}

impl Icon {
    pub fn size(mut self, px: f32) -> Self {
        self.size = px;
        self
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&mut EventCx) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl From<Icon> for Element {
    fn from(w: Icon) -> Self {
        Element::new(w)
    }
}

impl Styled for Icon {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Icon {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn measure(&self, _: Size<Option<f32>>, _: Size<AvailableSpace>, _: &Text) -> Size<f32> {
        Size::length(self.size)
    }
    fn measure_key(&self) -> u64 {
        self.size.to_bits() as u64
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (now, r) = (cx.now, cx.rect);
        let s = cx.state.get::<IconState>();
        if s.tap.pressed {
            s.scale.to(PRESSED, now, PRESS, ease::out_cubic);
        } else {
            s.scale.to(1.0, now, RELEASE, spring);
        }
        let k = s.scale.get(now);
        if s.scale.running(now) {
            cx.request_frame();
        }
        // Centered in the padded box, scaled about the center.
        let side = self.size * k;
        let (cx0, cy0) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let box_ = Rect::new(cx0 - side / 2.0, cy0 - side / 2.0, side, side);
        cx.shape(&self.shape, box_, self.color);
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let Some(f) = &self.on_click else { return };
        let mut tap = std::mem::take(&mut cx.state.get::<IconState>().tap);
        let before = tap.pressed;
        let fire = tap.event(ev, cx);
        if tap.pressed != before {
            cx.request_redraw();
        }
        cx.state.get::<IconState>().tap = tap;
        if fire {
            f(cx);
        }
    }
}
