use std::hash::{DefaultHasher, Hash, Hasher};

use taffy::{AvailableSpace, Size, Style};

use super::Styled;
use crate::render::Color;
use crate::text::{Font, Text};
use crate::widget::{Element, PaintCx, Widget};

pub struct Label {
    text: String,
    font: Font,
    size: f32,
    color: Color,
    style: Style,
}

pub fn text(s: impl Into<String>) -> Label {
    Label {
        text: s.into(),
        font: Font::DEFAULT,
        size: 16.0,
        color: Color::BLACK,
        style: Style::default(),
    }
}

impl Label {
    pub fn font(mut self, f: Font) -> Self {
        self.font = f;
        self
    }
    pub fn size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
}

impl<Msg: 'static> From<Label> for Element<Msg> {
    fn from(w: Label) -> Self {
        Element::new(w)
    }
}

impl Styled for Label {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<Msg> Widget<Msg> for Label {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn measure(
        &self,
        known: Size<Option<f32>>,
        available: Size<AvailableSpace>,
        text: &Text,
    ) -> Size<f32> {
        let max = known.width.or(match available.width {
            AvailableSpace::Definite(w) => Some(w),
            AvailableSpace::MinContent => Some(0.0),
            AvailableSpace::MaxContent => None,
        });
        let l = text.layout(&self.text, self.font, self.size, max);
        Size {
            width: known.width.unwrap_or(l.width),
            height: known.height.unwrap_or(l.height),
        }
    }
    fn measure_key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        (&self.text, self.font, self.size.to_bits()).hash(&mut h);
        h.finish()
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (r, s, c) = (cx.rect, self.size, self.color);
        cx.text(&self.text, self.font, s, c, Some(r.w + 1.0), (r.x, r.y));
    }
}
