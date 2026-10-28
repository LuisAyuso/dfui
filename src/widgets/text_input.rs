use taffy::util::ResolveOrZero;
use taffy::{AvailableSpace, LengthPercentage, Size, Style};

use super::Styled;
use crate::render::{Color, Rect};
use crate::text::Text;
use crate::widget::{Element, Event, EventCx, Key, PaintCx, Widget};

type OnChange<Msg> = Box<dyn Fn(&mut EventCx<Msg>, String)>;

/// Caret position in chars.
#[derive(Default)]
struct Caret(usize);

/// Single-line controlled text field: shows `value`, reports edits via `on_change`.
// ponytail: no selection, clipboard or caret blink yet.
pub struct TextInput<Msg> {
    value: String,
    placeholder: String,
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
        let stops = text.caret_stops(&self.value, self.size);
        let shift = (stops[caret] + 2.0 - width).max(0.0);
        (stops, shift)
    }

    fn caret(&self, c: &mut Caret) -> usize {
        c.0 = c.0.min(self.value.chars().count());
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
    fn measure(
        &self,
        known: Size<Option<f32>>,
        _available: Size<AvailableSpace>,
        text: &Text,
    ) -> Size<f32> {
        Size {
            width: known.width.unwrap_or(160.0),
            height: known.height.unwrap_or(text.line_height(self.size)),
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
            cx.text(&self.placeholder, self.size, muted, None, (c.x, c.y));
        } else {
            cx.text(&self.value, self.size, self.color, None, (c.x - shift, c.y));
        }
        if cx.focused {
            let h = cx.text.line_height(self.size);
            let x = c.x + stops[caret] - shift;
            cx.fill(Rect::new(x, c.y, 1.5, h), 0.0, self.accent);
        }
        cx.clip = outer;
    }
    fn event(&self, ev: &Event, cx: &mut EventCx<Msg>) {
        let caret = self.caret(cx.state.get::<Caret>());
        let len = self.value.chars().count();
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
                let nearest = (0..stops.len())
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
            Event::Key(Key::Backspace) if caret > 0 => {
                let mut v = self.value.clone();
                v.replace_range(self.byte(caret - 1)..self.byte(caret), "");
                edit(cx, v, caret - 1);
            }
            Event::Key(Key::Delete) if caret < len => {
                let mut v = self.value.clone();
                v.replace_range(self.byte(caret)..self.byte(caret + 1), "");
                edit(cx, v, caret);
            }
            Event::Key(k @ (Key::Left | Key::Right | Key::Home | Key::End)) => {
                cx.state.get::<Caret>().0 = match k {
                    Key::Left => caret.saturating_sub(1),
                    Key::Right => (caret + 1).min(len),
                    Key::Home => 0,
                    _ => len,
                };
                cx.request_redraw();
            }
            _ => {}
        }
    }
}
