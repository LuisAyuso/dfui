use std::hash::{DefaultHasher, Hash, Hasher};

use taffy::util::ResolveOrZero;
use taffy::{AvailableSpace, LengthPercentage, Size, Style};
use unicode_segmentation::UnicodeSegmentation;

use super::Styled;
use crate::render::{Color, Rect};
use crate::text::{Font, Text};
use crate::widget::{Element, Event, EventCx, Key, PaintCx, Widget};

type OnChange<Msg> = Box<dyn Fn(&mut EventCx<Msg>, String)>;

/// Caret position in chars, always on a grapheme boundary.
#[derive(Default)]
struct Caret(usize);

/// Single-line controlled text field: shows `value`, reports edits via `on_change`.
// ponytail: no selection, clipboard or caret blink yet.
pub struct TextInput<Msg> {
    value: String,
    placeholder: String,
    font: Font,
    size: f32,
    color: Color,
    accent: Color,
    on_change: Option<OnChange<Msg>>,
    style: Style,
}

pub fn text_input<Msg>(value: impl Into<String>) -> TextInput<Msg> {
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

impl<Msg> TextInput<Msg> {
    /// Called with the edited value; without it the field is read-only.
    pub fn on_change(mut self, f: impl Fn(&mut EventCx<Msg>, String) + 'static) -> Self {
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

    /// The stored caret, snapped back to a boundary if the value changed.
    fn caret(&self, c: &mut Caret) -> usize {
        c.0 = self
            .boundaries()
            .into_iter()
            .rfind(|&b| b <= c.0)
            .unwrap_or(0);
        c.0
    }

    fn byte(&self, caret: usize) -> usize {
        self.value
            .char_indices()
            .nth(caret)
            .map_or(self.value.len(), |(b, _)| b)
    }
}

impl<Msg: 'static> From<TextInput<Msg>> for Element<Msg> {
    fn from(w: TextInput<Msg>) -> Self {
        Element::new(w)
    }
}

impl<Msg> Styled for TextInput<Msg> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<Msg> Widget<Msg> for TextInput<Msg> {
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
        let caret = self.caret(cx.state.get::<Caret>());
        let (stops, shift) = self.stops(cx.text, caret, c.w);
        let outer = cx.clip;
        cx.clip = outer.intersect(&c);
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
            let h = cx.text.line_height(self.font, self.size);
            let x = c.x + stops[caret] - shift;
            cx.fill(Rect::new(x, c.y, 1.5, h), 0.0, self.accent);
        }
        cx.clip = outer;
    }
    fn event(&self, ev: &Event, cx: &mut EventCx<Msg>) {
        let caret = self.caret(cx.state.get::<Caret>());
        let bounds = self.boundaries();
        let prev = bounds.iter().rfind(|&&b| b < caret).copied();
        let next = bounds.iter().find(|&&b| b > caret).copied();
        let edit = |cx: &mut EventCx<Msg>, value: String, caret: usize| {
            cx.state.get::<Caret>().0 = caret;
            if let Some(f) = &self.on_change {
                f(cx, value);
            }
            cx.request_redraw();
        };
        match ev {
            Event::PointerDown { x, y } if cx.hit(*x, *y) => {
                let c = self.content(cx.rect);
                let (stops, shift) = self.stops(cx.text, caret, c.w);
                let at = x - c.x + shift;
                let nearest = bounds
                    .iter()
                    .copied()
                    .min_by(|&a, &b| (stops[a] - at).abs().total_cmp(&(stops[b] - at).abs()))
                    .unwrap_or(0);
                cx.state.get::<Caret>().0 = nearest;
                cx.focus();
                cx.request_redraw();
            }
            Event::Text(t) => {
                let mut v = self.value.clone();
                v.insert_str(self.byte(caret), t);
                edit(cx, v, caret + t.chars().count());
            }
            Event::Key(Key::Backspace) if let Some(p) = prev => {
                let mut v = self.value.clone();
                v.replace_range(self.byte(p)..self.byte(caret), "");
                edit(cx, v, p);
            }
            Event::Key(Key::Delete) if let Some(n) = next => {
                let mut v = self.value.clone();
                v.replace_range(self.byte(caret)..self.byte(n), "");
                edit(cx, v, caret);
            }
            Event::Key(k @ (Key::Left | Key::Right | Key::Home | Key::End)) => {
                cx.state.get::<Caret>().0 = match k {
                    Key::Left => prev.unwrap_or(caret),
                    Key::Right => next.unwrap_or(caret),
                    Key::Home => 0,
                    _ => *bounds.last().unwrap(),
                };
                cx.request_redraw();
            }
            _ => {}
        }
    }
}
