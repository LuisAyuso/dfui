use std::borrow::Cow;
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
    max_lines: Option<usize>,
    style: Style,
}

/// A run of text, wrapped at spaces to the available width.
pub fn text(s: impl Into<String>) -> Label {
    Label {
        text: s.into(),
        font: Font::DEFAULT,
        size: 16.0,
        color: Color::BLACK,
        max_lines: None,
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
    /// At most `n` lines; cut text ends in "…".
    pub fn max_lines(mut self, n: usize) -> Self {
        self.max_lines = Some(n.max(1));
        self
    }

    fn shown<'a>(&'a self, text: &Text, max_width: Option<f32>) -> Cow<'a, str> {
        match (self.max_lines, max_width) {
            (Some(n), Some(w)) => text.truncate(&self.text, self.font, self.size, w, n),
            _ => Cow::Borrowed(&self.text),
        }
    }
}

impl From<Label> for Element {
    fn from(w: Label) -> Self {
        Element::new(w)
    }
}

impl Styled for Label {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Label {
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
        let l = text.layout(&self.shown(text, max), self.font, self.size, max);
        Size {
            width: known.width.unwrap_or(l.width),
            height: known.height.unwrap_or(l.height),
        }
    }
    fn measure_key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        (&self.text, self.font, self.size.to_bits(), self.max_lines).hash(&mut h);
        h.finish()
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (r, s, c) = (cx.rect, self.size, self.color);
        let max = Some(r.w + 1.0);
        let shown = self.shown(cx.text, max);
        cx.text(&shown, self.font, s, c, max, (r.x, r.y));
    }
}
