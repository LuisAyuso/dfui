use std::time::Duration;

use taffy::{AlignItems, JustifyContent, LengthPercentage, Style};

use super::Styled;
use crate::anim::{Animated, Lerp, ease};
use crate::render::{Color, Shadow};
use crate::widget::{Element, Event, EventCx, PaintCx, Widget};

type Handler = Box<dyn Fn(&mut EventCx)>;

const FADE: Duration = Duration::from_millis(120);

/// Press and hover tracking shared by tappable widgets.
#[derive(Default)]
pub(crate) struct Tap {
    pub pressed: bool,
    pub hovered: bool,
}

impl Tap {
    /// Returns whether `ev` completes a tap: pressed and released inside.
    pub fn event(&mut self, ev: &Event, cx: &EventCx) -> bool {
        let hit = match *ev {
            Event::PointerDown { x, y }
            | Event::PointerMove { x, y }
            | Event::PointerUp { x, y } => cx.hit(x, y),
            _ => false,
        };
        match ev {
            Event::PointerDown { .. } if hit => self.pressed = true,
            Event::PointerMove { .. } => self.hovered = hit,
            Event::PointerUp { .. } => return std::mem::take(&mut self.pressed) && hit,
            Event::Cancel => *self = Tap::default(),
            Event::Leave => self.hovered = false,
            _ => {}
        }
        false
    }
}

#[derive(Default)]
struct ButtonState {
    tap: Tap,
    /// 0..=1 toward the hovered and pressed looks.
    hover: Animated<f32>,
    press: Animated<f32>,
}

pub struct Button {
    child: [Element; 1],
    on_click: Option<Handler>,
    style: Style,
    color: Color,
    radius: f32,
    flat: bool,
}

/// Tappable box around `child`; `on_click` runs on a release inside it.
pub fn button(child: impl Into<Element>) -> Button {
    Button {
        child: [child.into()],
        on_click: None,
        style: Style {
            padding: taffy::Rect {
                left: LengthPercentage::length(16.0),
                right: LengthPercentage::length(16.0),
                top: LengthPercentage::length(10.0),
                bottom: LengthPercentage::length(10.0),
            },
            align_items: AlignItems::CENTER,
            justify_content: JustifyContent::CENTER,
            ..Style::default()
        },
        color: Color::hex(0x3b6fe0),
        radius: 8.0,
        flat: false,
    }
}

impl Button {
    /// Runs on release inside the button, e.g. to `update` local state.
    pub fn on_click(mut self, f: impl Fn(&mut EventCx) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    /// No drop shadow, e.g. for items in a list.
    pub fn flat(mut self) -> Self {
        self.flat = true;
        self
    }
}

impl From<Button> for Element {
    fn from(w: Button) -> Self {
        Element::new(w)
    }
}

impl Styled for Button {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Button {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element] {
        &self.child
    }
    fn paint(&self, cx: &mut PaintCx) {
        let now = cx.now;
        let s = cx.state.get::<ButtonState>();
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        s.hover.to(flag(s.tap.hovered), now, FADE, ease::out_cubic);
        s.press.to(flag(s.tap.pressed), now, FADE, ease::out_cubic);
        let (hover, press) = (s.hover.get(now), s.press.get(now));
        if s.hover.running(now) || s.press.running(now) {
            cx.request_frame();
        }
        let k = 1.0.lerp(0.9, hover).lerp(0.75, press);
        let c = self.color;
        let fill = Color {
            r: c.r * k,
            g: c.g * k,
            b: c.b * k,
            a: c.a,
        };
        if press < 1.0 && !self.flat {
            let sh = Shadow {
                color: Color::BLACK.alpha(0.25 * (1.0 - press)),
                offset: (0.0, 2.0),
                blur: 4.0,
            };
            cx.shadow(cx.rect, self.radius, sh);
        }
        cx.fill(cx.rect, self.radius, fill);
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let mut tap = std::mem::take(&mut cx.state.get::<ButtonState>().tap);
        let before = (tap.pressed, tap.hovered);
        let fire = tap.event(ev, cx);
        if (tap.pressed, tap.hovered) != before {
            cx.request_redraw();
        }
        cx.state.get::<ButtonState>().tap = tap;
        if fire && let Some(f) = &self.on_click {
            f(cx);
        }
    }
}
