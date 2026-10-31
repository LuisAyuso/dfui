use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::time::Duration;

use taffy::util::ResolveOrZero;
use taffy::{AvailableSpace, LengthPercentage, Size, Style};
use unicode_segmentation::UnicodeSegmentation;
use web_time::Instant;

use super::Styled;
use crate::render::{Color, Rect};
use crate::text::{Font, Text};
use crate::widget::{Element, Event, EventCx, Key, PaintCx, Widget};

type OnChange = Box<dyn Fn(&mut EventCx, String)>;

/// Caret and selection anchor in chars, always on grapheme boundaries; the
/// selection is the range between them.
#[derive(Default)]
struct Caret {
    pos: usize,
    anchor: usize,
    dragging: bool,
    /// Start of the blink cycle: the next paint after the caret last moved.
    blink: Option<Instant>,
}

const BLINK: Duration = Duration::from_millis(500);

/// Single-line controlled text field: shows `value`, reports edits via `on_change`.
pub struct TextInput {
    value: String,
    placeholder: String,
    font: Font,
    size: f32,
    color: Color,
    accent: Color,
    on_change: Option<OnChange>,
    style: Style,
}

pub fn text_input(value: impl Into<String>) -> TextInput {
    TextInput {
        value: value.into(),
        placeholder: String::new(),
        font: Font::DEFAULT,
        size: 16.0,
        color: Color::BLACK,
        accent: Color::hex(0x3b6fe0),
        on_change: None,
        style: Style {
            padding: taffy::Rect {
                left: LengthPercentage::length(12.0),
                right: LengthPercentage::length(12.0),
                top: LengthPercentage::length(10.0),
                bottom: LengthPercentage::length(10.0),
            },
            ..Style::default()
        },
    }
}

impl TextInput {
    /// Called with the edited value; without it the field is read-only.
    pub fn on_change(mut self, f: impl Fn(&mut EventCx, String) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }
    pub fn placeholder(mut self, s: impl Into<String>) -> Self {
        self.placeholder = s.into();
        self
    }
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
    pub fn accent(mut self, c: Color) -> Self {
        self.accent = c;
        self
    }

    fn content(&self, r: Rect) -> Rect {
        let p = self.style.padding;
        let res = |l: LengthPercentage| l.resolve_or_zero(Some(r.w), |_, _| 0.0);
        let (l, t) = (res(p.left), res(p.top));
        Rect::new(
            r.x + l,
            r.y + t,
            r.w - l - res(p.right),
            r.h - t - res(p.bottom),
        )
    }

    /// Caret stops and the horizontal shift that keeps the caret in view.
    fn stops(&self, text: &Text, caret: usize, width: f32) -> (Vec<f32>, f32) {
        let stops = text.caret_stops(&self.value, self.font, self.size);
        let shift = (stops[caret] + 2.0 - width).max(0.0);
        (stops, shift)
    }

    /// Char indices where the caret may sit: grapheme cluster boundaries, so
    /// it never lands between a letter and its accent or inside an emoji.
    fn boundaries(&self) -> Vec<usize> {
        let mut n = 0;
        let ends = self.value.graphemes(true).map(|g| {
            n += g.chars().count();
            n
        });
        std::iter::once(0).chain(ends).collect()
    }

    /// The stored caret and anchor, snapped back to boundaries if the value changed.
    fn caret(&self, c: &mut Caret) -> (usize, usize) {
        let bounds = self.boundaries();
        let snap = |i: usize| bounds.iter().copied().rfind(|&b| b <= i).unwrap_or(0);
        (c.pos, c.anchor) = (snap(c.pos), snap(c.anchor));
        (c.pos, c.anchor)
    }

    /// Boundary nearest to `x`, with the field laid out for `caret`.
    fn index_at(&self, text: &Text, r: Rect, caret: usize, x: f32) -> usize {
        let c = self.content(r);
        let (stops, shift) = self.stops(text, caret, c.w);
        let at = x - c.x + shift;
        self.boundaries()
            .into_iter()
            .min_by(|&a, &b| (stops[a] - at).abs().total_cmp(&(stops[b] - at).abs()))
            .unwrap_or(0)
    }

    fn byte(&self, caret: usize) -> usize {
        self.value
            .char_indices()
            .nth(caret)
            .map_or(self.value.len(), |(b, _)| b)
    }
}

impl From<TextInput> for Element {
    fn from(w: TextInput) -> Self {
        Element::new(w)
    }
}

impl Styled for TextInput {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for TextInput {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn measure_key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        (self.font, self.size.to_bits()).hash(&mut h);
        h.finish()
    }
    fn measure(
        &self,
        known: Size<Option<f32>>,
        _available: Size<AvailableSpace>,
        text: &Text,
    ) -> Size<f32> {
        Size {
            width: known.width.unwrap_or(160.0),
            height: known
                .height
                .unwrap_or(text.line_height(self.font, self.size)),
        }
    }
    fn paint(&self, cx: &mut PaintCx) {
        let r = cx.rect;
        let (bw, bc) = if cx.focused {
            (2.0, self.accent)
        } else {
            (1.0, Color::hex(0xc9ccd6))
        };
        cx.fill_bordered(r, 8.0, Color::WHITE, bw, bc);

        let c = self.content(r);
        let (caret, anchor) = self.caret(cx.state.get::<Caret>());
        let (stops, shift) = self.stops(cx.text, caret, c.w);
        let h = cx.text.line_height(self.font, self.size);
        let outer = cx.clip;
        cx.clip = outer.intersect(&c);
        if cx.focused && caret != anchor {
            let (a, b) = (stops[caret.min(anchor)], stops[caret.max(anchor)]);
            let sel = Rect::new(c.x + a - shift, c.y, b - a, h);
            cx.fill(sel, 0.0, self.accent.alpha(0.25));
        }
        if self.value.is_empty() {
            let muted = self.color.alpha(0.4);
            cx.text(
                &self.placeholder,
                self.font,
                self.size,
                muted,
                None,
                (c.x, c.y),
            );
        } else {
            cx.text(
                &self.value,
                self.font,
                self.size,
                self.color,
                None,
                (c.x - shift, c.y),
            );
        }
        if cx.focused {
            let now = cx.now;
            let start = *cx.state.get::<Caret>().blink.get_or_insert(now);
            let phase = now.saturating_duration_since(start).as_millis() / BLINK.as_millis();
            if phase.is_multiple_of(2) {
                let x = c.x + stops[caret] - shift;
                cx.fill(Rect::new(x, c.y, 1.5, h), 0.0, self.accent);
            }
            cx.request_frame_at(start + BLINK * (phase as u32 + 1));
        }
        cx.clip = outer;
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let (caret, anchor) = self.caret(cx.state.get::<Caret>());
        let sel = caret.min(anchor)..caret.max(anchor);
        let bounds = self.boundaries();
        let prev = bounds.iter().rfind(|&&b| b < caret).copied();
        let next = bounds.iter().find(|&&b| b > caret).copied();
        let select = |cx: &mut EventCx, pos: usize, anchor: usize| {
            let c = cx.state.get::<Caret>();
            (c.pos, c.anchor) = (pos, anchor);
            // Restart the blink, solid first, so the caret shows while typing.
            c.blink = None;
            cx.request_redraw();
        };
        // Replaces `range` (chars) with `with` and puts the caret after it.
        let replace = |cx: &mut EventCx, range: Range<usize>, with: &str| {
            let mut v = self.value.clone();
            v.replace_range(self.byte(range.start)..self.byte(range.end), with);
            let at = range.start + with.chars().count();
            select(cx, at, at);
            if let Some(f) = &self.on_change {
                f(cx, v);
            }
        };
        match ev {
            Event::PointerDown { x, y } if cx.hit(*x, *y) => {
                let at = self.index_at(cx.text, cx.rect, caret, *x);
                let keep = if cx.shift { anchor } else { at };
                select(cx, at, keep);
                cx.state.get::<Caret>().dragging = true;
                cx.focus();
            }
            Event::PointerMove { x, .. } if cx.state.get::<Caret>().dragging => {
                let at = self.index_at(cx.text, cx.rect, caret, *x);
                if at != caret {
                    select(cx, at, anchor);
                }
            }
            Event::PointerUp { .. } | Event::Cancel => cx.state.get::<Caret>().dragging = false,
            Event::Text(t) => replace(cx, sel, t),
            Event::Key(Key::Backspace | Key::Delete) if !sel.is_empty() => replace(cx, sel, ""),
            Event::Key(Key::Backspace) if let Some(p) = prev => replace(cx, p..caret, ""),
            Event::Key(Key::Delete) if let Some(n) = next => replace(cx, caret..n, ""),
            Event::Key(k @ (Key::Left | Key::Right | Key::Home | Key::End)) => {
                let collapse = !cx.shift && !sel.is_empty();
                let to = match k {
                    Key::Left if collapse => sel.start,
                    Key::Right if collapse => sel.end,
                    Key::Left => prev.unwrap_or(caret),
                    Key::Right => next.unwrap_or(caret),
                    Key::Home => 0,
                    _ => *bounds.last().unwrap(),
                };
                select(cx, to, if cx.shift { anchor } else { to });
            }
            Event::Key(Key::SelectAll) => select(cx, *bounds.last().unwrap(), 0),
            Event::Key(k @ (Key::Copy | Key::Cut)) if !sel.is_empty() => {
                cx.copy(&self.value[self.byte(sel.start)..self.byte(sel.end)]);
                if *k == Key::Cut {
                    replace(cx, sel, "");
                }
            }
            _ => {}
        }
    }
}
