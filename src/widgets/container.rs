use taffy::{FlexDirection, Style};

use super::Styled;
use crate::render::{Color, Shadow};
use crate::widget::{Element, PaintCx, Widget};

/// Flex box with optional rounded, bordered, shadowed background.
pub struct Container<Msg> {
    style: Style,
    children: Vec<Element<Msg>>,
    bg: Color,
    radius: f32,
    border: (f32, Color),
    shadow: Option<Shadow>,
}

pub fn row<Msg>(children: impl IntoIterator<Item = Element<Msg>>) -> Container<Msg> {
    Container {
        style: Style {
            flex_direction: FlexDirection::Row,
            ..Style::default()
        },
        children: children.into_iter().collect(),
        bg: Color::TRANSPARENT,
        radius: 0.0,
        border: (0.0, Color::TRANSPARENT),
        shadow: None,
    }
}

pub fn column<Msg>(children: impl IntoIterator<Item = Element<Msg>>) -> Container<Msg> {
    let mut c = row(children);
    c.style.flex_direction = FlexDirection::Column;
    c
}

pub fn container<Msg>(child: impl Into<Element<Msg>>) -> Container<Msg> {
    column([child.into()])
}

/// Flexible empty space; `spacer().grow(1.)` pushes siblings apart.
pub fn spacer<Msg>() -> Container<Msg> {
    row([])
}

impl<Msg> Container<Msg> {
    pub fn child(mut self, c: impl Into<Element<Msg>>) -> Self {
        self.children.push(c.into());
        self
    }
    pub fn bg(mut self, c: Color) -> Self {
        self.bg = c;
        self
    }
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    pub fn border(mut self, width: f32, c: Color) -> Self {
        self.border = (width, c);
        self
    }
    pub fn shadow(mut self, s: Shadow) -> Self {
        self.shadow = Some(s);
        self
    }
}

impl<Msg: 'static> From<Container<Msg>> for Element<Msg> {
    fn from(w: Container<Msg>) -> Self {
        Element::new(w)
    }
}

impl<Msg> Styled for Container<Msg> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<Msg> Widget<Msg> for Container<Msg> {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element<Msg>] {
        &self.children
    }
    fn paint(&self, cx: &mut PaintCx) {
        if let Some(s) = self.shadow {
            cx.shadow(cx.rect, self.radius, s);
        }
        if self.bg.a > 0.0 || self.border.0 > 0.0 {
            cx.fill_bordered(cx.rect, self.radius, self.bg, self.border.0, self.border.1);
        }
    }
}
