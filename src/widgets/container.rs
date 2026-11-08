use std::any::Any;

use taffy::{FlexDirection, Style};

use super::Styled;
use super::label::Emphasis;
use crate::render::{Color, Shadow};
use crate::theme::Ink;
use crate::widget::{Element, PaintCx, Widget};

/// Flex box with optional rounded, bordered, shadowed background.
pub struct Container {
    style: Style,
    children: Vec<Element>,
    bg: Ink,
    radius: f32,
    border: (f32, Ink),
    shadow: Option<Shadow>,
    emphasis: Option<Emphasis>,
}

/// Lays `children` out left to right.
pub fn row(children: impl IntoIterator<Item = Element>) -> Container {
    Container {
        style: Style {
            flex_direction: FlexDirection::Row,
            ..Style::default()
        },
        children: children.into_iter().collect(),
        bg: Color::TRANSPARENT.into(),
        radius: 0.0,
        border: (0.0, Color::TRANSPARENT.into()),
        shadow: None,
        emphasis: None,
    }
}

/// Lays `children` out top to bottom.
pub fn column(children: impl IntoIterator<Item = Element>) -> Container {
    let mut c = row(children);
    c.style.flex_direction = FlexDirection::Column;
    c
}

/// A single child with optional background, radius, border and shadow.
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
    pub fn bg(mut self, c: impl Into<Ink>) -> Self {
        self.bg = c.into();
        self
    }
    /// Default color of the text and icons inside, to contrast with `bg`.
    pub fn content(mut self, c: impl Into<Ink>) -> Self {
        self.emphasis = Some(Emphasis(Default::default(), Default::default(), c.into()));
        self
    }
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    pub fn border(mut self, width: f32, c: impl Into<Ink>) -> Self {
        self.border = (width, c.into());
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
    fn provide(&self) -> Option<&dyn Any> {
        self.emphasis.as_ref().map(|e| e as &dyn Any)
    }
    fn paint(&self, cx: &mut PaintCx) {
        if let Some(s) = self.shadow {
            cx.shadow(cx.rect, self.radius, s);
        }
        let bg = cx.color(self.bg);
        if bg.a > 0.0 || self.border.0 > 0.0 {
            let border = cx.color(self.border.1);
            cx.fill_bordered(cx.rect, self.radius, bg, self.border.0, border);
        }
    }
}
