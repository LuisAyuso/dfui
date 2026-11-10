use std::borrow::Cow;
use std::cell::Cell;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use taffy::{AvailableSpace, Size, Style};

use super::Styled;
use crate::anim::{Lerp, Morph};
use crate::render::Color;
use crate::text::{Font, Text};
use crate::theme::Ink;
use crate::widget::{BuildCx, Element, PaintCx, Widget};

/// How far toward bold an ancestor wants its labels, 0..=1, whether to dim
/// them, and their default color; a button sets the first two before its
/// children paint.
#[derive(Clone, Default)]
pub(crate) struct Emphasis(pub Rc<Cell<f32>>, pub Rc<Cell<bool>>, pub Ink);

#[derive(Default)]
struct LabelState {
    emphasis: Option<Emphasis>,
}

/// A run of text built by [`text`]; see there.
pub struct Label {
    text: String,
    font: Font,
    size: f32,
    color: Ink,
    max_lines: Option<usize>,
    style: Style,
}

/// A run of text, wrapped at spaces to the available width.
pub fn text(s: impl Into<String>) -> Label {
    Label {
        text: s.into(),
        font: Font::DEFAULT,
        size: 16.0,
        color: Ink::Content,
        max_lines: None,
        style: Style::default(),
    }
}

impl Label {
    /// [`Font::DEFAULT`] unless set.
    pub fn font(mut self, f: Font) -> Self {
        self.font = f;
        self
    }
    /// Font size, logical px; 16 by default.
    pub fn size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }
    /// Text color; by default the content color of the enclosing container or button.
    pub fn color(mut self, c: impl Into<Ink>) -> Self {
        self.color = c.into();
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
    fn expand(&self, cx: &mut BuildCx) {
        cx.state.get::<LabelState>().emphasis = cx.scope.get::<Emphasis>().cloned();
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (r, s) = (cx.rect, self.size);
        let st = cx.state.get::<LabelState>();
        let (t, dim, content) = st
            .emphasis
            .as_ref()
            .map_or((0.0, false, Ink::Content), |e| (e.0.get(), e.1.get(), e.2));
        let ink = if self.color == Ink::Content {
            content
        } else {
            self.color
        };
        let c = cx.color(ink);
        let c = if dim { c.alpha(c.a * 0.5) } else { c };
        let regular = std::ptr::eq(self.font.0, Font::DEFAULT.0);
        if t > 0.0 && regular && self.max_lines.is_none() && !self.text.contains('\n') {
            return weight(cx, &self.text, s, c, t, (r.x, r.w), r.y);
        }
        let max = Some(r.w + 1.0);
        let shown = self.shown(cx.text, max);
        cx.text(&shown, self.font, s, c, max, (r.x, r.y));
    }
}

/// One line of `s` between regular (`t` 0) and bold (`t` 1), centered in
/// `x..x + w` with its top at `y`. Between the two, each glyph's outline
/// morphs and the glyphs slide between the two spacings.
pub(crate) fn weight(
    cx: &mut PaintCx,
    s: &str,
    size: f32,
    color: Color,
    t: f32,
    (x, w): (f32, f32),
    y: f32,
) {
    if t <= 0.0 || t >= 1.0 {
        let font = if t >= 1.0 { Font::BOLD } else { Font::DEFAULT };
        let lw = cx.text.layout(s, font, size, None).width;
        cx.text(s, font, size, color, None, (x + (w - lw) / 2.0, y));
        return;
    }
    let a = cx.text.caret_stops(s, Font::DEFAULT, size);
    let b = cx.text.caret_stops(s, Font::BOLD, size);
    let lw = a[a.len() - 1].lerp(b[b.len() - 1], t);
    let (x0, base) = (x + (w - lw) / 2.0, y + cx.text.ascent(Font::DEFAULT, size));
    for (i, ch) in s.chars().enumerate() {
        let regular = cx.glyph_path(Font::DEFAULT, ch, size);
        let bold = cx.glyph_path(Font::BOLD, ch, size);
        if let (Some(p), Some(q)) = (regular, bold) {
            let glyph = Morph::new(&p, &q)
                .at(t)
                .offset(x0 + a[i].lerp(b[i], t), base);
            cx.path(&glyph, color);
        }
    }
}
