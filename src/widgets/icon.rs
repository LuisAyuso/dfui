use std::time::Duration;

use taffy::{AvailableSpace, Size, Style};

use super::Styled;
use super::button::{Handler, Tap};
use super::label::Emphasis;
use crate::anim::{Animated, ease};
use crate::path::Shape;
use crate::render::Rect;
use crate::text::Text;
use crate::theme::Ink;
use crate::widget::{BuildCx, Element, Event, EventCx, PaintCx, Widget};

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
    emphasis: Option<Emphasis>,
}

/// A `Shape` drawn `size` px square in one color; a button with `on_click`.
pub struct Icon {
    shape: Shape,
    size: f32,
    color: Ink,
    on_click: Option<Handler>,
    enabled: bool,
    style: Style,
}

/// `shape` as a 24 px icon.
pub fn icon(shape: &Shape) -> Icon {
    Icon {
        shape: shape.clone(),
        size: 24.0,
        color: Ink::Content,
        on_click: None,
        enabled: true,
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
    pub fn color(mut self, c: impl Into<Ink>) -> Self {
        self.color = c.into();
        self
    }
    pub fn on_click(mut self, f: impl Fn(&mut EventCx) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
    /// When false the icon is dimmed and ignores input; it keeps its size.
    pub fn enabled(mut self, on: bool) -> Self {
        self.enabled = on;
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
    fn expand(&self, cx: &mut BuildCx) {
        cx.state.get::<IconState>().emphasis = cx.scope.get::<Emphasis>().cloned();
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (now, r) = (cx.now, cx.rect);
        let s = cx.state.get::<IconState>();
        if s.tap.pressed && self.enabled {
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
        let content = cx
            .state
            .get::<IconState>()
            .emphasis
            .as_ref()
            .map_or(Ink::Content, |e| e.2);
        let c = cx.color(if self.color == Ink::Content {
            content
        } else {
            self.color
        });
        cx.shape(
            &self.shape,
            box_,
            if self.enabled { c } else { c.alpha(c.a * 0.4) },
        );
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        if !self.enabled {
            cx.state.get::<IconState>().tap = Tap::default();
            return;
        }
        let Some(f) = &self.on_click else { return };
        let (fire, changed) = Tap::feed(ev, cx, |s: &mut IconState| &mut s.tap);
        if changed {
            cx.request_redraw();
        }
        if fire {
            f(cx);
        }
    }
}
