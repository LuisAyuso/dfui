use taffy::{FlexDirection, Style};

use super::Styled;
use crate::render::{Color, Shadow};
use crate::widget::{Element, PaintCx, Widget};

/// Flex box with optional rounded, bordered, shadowed background.
pub struct Container {
    style: Style,
    children: Vec<Element>,
    bg: Color,
    radius: f32,
    border: (f32, Color),
    shadow: Option<Shadow>,
}

pub fn row(children: impl IntoIterator<Item = Element>) -> Container {
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

pub fn column(children: impl IntoIterator<Item = Element>) -> Container {
    let mut c = row(children);
    c.style.flex_direction = FlexDirection::Column;
    c
}

pub fn container(child: impl Into<Element>) -> Container {
    column([child.into()])
}

/// Flexible empty space; `spacer().grow(1.)` pushes siblings apart.
pub fn spacer() -> Container {
    row([])
}

impl Container {
    pub fn child(mut self, c: impl Into<Element>) -> Self {
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

impl From<Container> for Element {
    fn from(w: Container) -> Self {
        Element::new(w)
    }
}

impl Styled for Container {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Container {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element] {
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
