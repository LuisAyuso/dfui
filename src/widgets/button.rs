use taffy::{AlignItems, JustifyContent, LengthPercentage, Style};

use super::Styled;
use crate::render::{Color, Shadow};
use crate::widget::{Element, Event, EventCx, PaintCx, Widget};

type Handler<Msg> = Box<dyn Fn(&mut EventCx<Msg>)>;

#[derive(Default)]
struct ButtonState {
    pressed: bool,
    hovered: bool,
}

pub struct Button<Msg> {
    child: [Element<Msg>; 1],
    on_click: Option<Handler<Msg>>,
    style: Style,
    color: Color,
    radius: f32,
}

pub fn button<Msg>(child: impl Into<Element<Msg>>) -> Button<Msg> {
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
    }
}

impl<Msg> Button<Msg> {
    /// Runs on release inside the button; can `emit` messages or `update` local state.
    pub fn on_click(mut self, f: impl Fn(&mut EventCx<Msg>) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
    pub fn on_press(self, msg: Msg) -> Self
    where
        Msg: Clone + 'static,
    {
        self.on_click(move |cx| cx.emit(msg.clone()))
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
}

impl<Msg: 'static> From<Button<Msg>> for Element<Msg> {
    fn from(w: Button<Msg>) -> Self {
        Element::new(w)
    }
}

impl<Msg> Styled for Button<Msg> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<Msg> Widget<Msg> for Button<Msg> {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element<Msg>] {
        &self.child
    }
    fn paint(&self, cx: &mut PaintCx) {
        let s = cx.state.get::<ButtonState>();
        let c = self.color;
        let k = if s.pressed {
            0.75
        } else if s.hovered {
            0.9
        } else {
            1.0
        };
        let fill = Color {
            r: c.r * k,
            g: c.g * k,
            b: c.b * k,
            a: c.a,
        };
        if !s.pressed {
            let sh = Shadow {
                color: Color::BLACK.alpha(0.25),
                offset: (0.0, 2.0),
                blur: 4.0,
            };
            cx.shadow(cx.rect, self.radius, sh);
        }
        cx.fill(cx.rect, self.radius, fill);
    }
    fn event(&self, ev: &Event, cx: &mut EventCx<Msg>) {
        let hit = match *ev {
            Event::PointerDown { x, y }
            | Event::PointerMove { x, y }
            | Event::PointerUp { x, y } => cx.hit(x, y),
            _ => false,
        };
        let before = (
            cx.state.get::<ButtonState>().pressed,
            cx.state.get::<ButtonState>().hovered,
        );
        let s = cx.state.get::<ButtonState>();
        match ev {
            Event::PointerDown { .. } if hit => s.pressed = true,
            Event::PointerMove { .. } => s.hovered = hit,
            Event::PointerUp { .. } => {
                let fire = s.pressed && hit;
                s.pressed = false;
                if fire && let Some(f) = &self.on_click {
                    f(cx);
                }
            }
            Event::Cancel => (s.pressed, s.hovered) = (false, false),
            Event::Leave => s.hovered = false,
            _ => {}
        }
        let s = cx.state.get::<ButtonState>();
        if (s.pressed, s.hovered) != before {
            cx.request_redraw();
        }
    }
}
