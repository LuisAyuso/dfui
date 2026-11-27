use std::borrow::Cow;
use std::cell::Cell;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::time::Duration;

use taffy::{AvailableSpace, Size, Style};

use super::Styled;
use crate::anim::{Animated, Lerp, Morph, ease};
use crate::render::Color;
use crate::text::{Font, Text};
use crate::theme::Ink;
use crate::widget::{BuildCx, Element, PaintCx, Widget};

/// How far toward bold an ancestor wants its labels, 0..=1, whether to dim
/// them, their default color, and whether a pressed one centers in its box;
/// a button sets the first two before its children paint.
#[derive(Clone, Default)]
pub(crate) struct Emphasis(pub Rc<Cell<f32>>, pub Rc<Cell<bool>>, pub Ink, pub bool);

#[derive(Default)]
struct LabelState {
    emphasis: Option<Emphasis>,
    /// Toward bold for [`Label::emphasized`].
    bold: Animated<f32>,
}

/// How long [`Label::emphasized`] takes to morph.
const MORPH: Duration = Duration::from_millis(160);

/// A run of text built by [`text`]; see there.
pub struct Label {
    text: String,
    font: Font,
    size: f32,
    color: Ink,
    max_lines: Option<usize>,
    emphasized: bool,
    style: Style,
}

/// A run of text, wrapped at spaces to the available width unless held to a
/// few lines by [`Label::max_lines`] (one for a table cell). In a `row` that
/// runs short of space, labels shrink and wrap like any flex item, down to
/// their longest word: to keep a short one like "5 min" whole beside a long
/// name, give it `.shrink(0.0)` and the name `.max_lines(1)`, which cuts it
/// with "…" and lets it give way.
#[doc = example!("text")]
pub fn text(s: impl Into<String>) -> Label {
    Label {
        text: s.into(),
        font: Font::DEFAULT,
        size: 16.0,
        color: Ink::Content,
        max_lines: None,
        emphasized: false,
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
    /// Morphs the glyphs to bold while `on`, e.g. for the focused item. Takes
    /// a one-line label in [`Font::DEFAULT`]; others switch font halfway.
    pub fn emphasized(mut self, on: bool) -> Self {
        self.emphasized = on;
        self
    }
    /// At most `n` lines; cut text ends in "…".
    pub fn max_lines(mut self, n: usize) -> Self {
        self.max_lines = Some(n.max(1));
        self
    }

    fn shown<'a>(&'a self, text: &Text, font: Font, max_width: Option<f32>) -> Cow<'a, str> {
        match (self.max_lines, max_width) {
            (Some(n), Some(w)) => text.truncate(&self.text, font, self.size, w, n),
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
        let l = text.layout(&self.shown(text, self.font, max), self.font, self.size, max);
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
        let (r, s, now) = (cx.rect, self.size, cx.now);
        let st = cx.state.get::<LabelState>();
        let (pressed, dim, content, centered) = st
            .emphasis
            .as_ref()
            .map_or((0.0, false, Ink::Content, false), |e| {
                (e.0.get(), e.1.get(), e.2, e.3)
            });
        let target = if self.emphasized { 1.0 } else { 0.0 };
        st.bold.to(target, now, MORPH, ease::out_cubic);
        let (own, moving) = (st.bold.get(now), st.bold.running(now));
        if moving {
            cx.request_frame();
        }
        let t = pressed.max(own);
        let ink = if self.color == Ink::Content {
            content
        } else {
            self.color
        };
        let c = cx.color(ink);
        let c = if dim { c.alpha(c.a * 0.5) } else { c };
        let regular = std::ptr::eq(self.font.0, Font::DEFAULT.0);
        let max = Some(r.w + 1.0);
        // A button centers its label; a list item's and an emphasized one keep
        // their start.
        let center = pressed > 0.0 && centered;
        if t > 0.0 && regular && !self.text.contains('\n') {
            let fits = match self.max_lines {
                None => true,
                Some(_) => cx.text.layout(&self.text, Font::BOLD, s, None).width <= r.w + 1.0,
            };
            if fits {
                return weight(cx, &self.text, s, c, t, (r.x, r.w), r.y, center);
            }
        }
        let font = if regular && t >= 0.5 {
            Font::BOLD
        } else {
            self.font
        };
        let shown = self.shown(cx.text, font, max);
        cx.text(&shown, font, s, c, max, (r.x, r.y));
    }
}

/// One line of `s` between regular (`t` 0) and bold (`t` 1) in `x..x + w`,
/// centered or from `x`, with its top at `y`. Between the two, each glyph's
/// outline morphs and the glyphs slide between the two spacings.
#[allow(clippy::too_many_arguments)]
pub(crate) fn weight(
    cx: &mut PaintCx,
    s: &str,
    size: f32,
    color: Color,
    t: f32,
    (x, w): (f32, f32),
    y: f32,
    center: bool,
) {
    let start = |lw: f32| if center { x + (w - lw) / 2.0 } else { x };
    if t <= 0.0 || t >= 1.0 {
        let font = if t >= 1.0 { Font::BOLD } else { Font::DEFAULT };
        let lw = cx.text.layout(s, font, size, None).width;
        cx.text(s, font, size, color, None, (start(lw), y));
        return;
    }
    let a = cx.text.caret_stops(s, Font::DEFAULT, size);
    let b = cx.text.caret_stops(s, Font::BOLD, size);
    let lw = a[a.len() - 1].lerp(b[b.len() - 1], t);
    let (x0, base) = (start(lw), y + cx.text.ascent(Font::DEFAULT, size));
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
