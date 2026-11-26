use std::ops::RangeInclusive;
use std::rc::Rc;

use taffy::{FlexDirection, Style};

use super::{Styled, stateful, text_input};
use crate::render::Color;
use crate::widget::{Element, EventCx, Keyboard, OnChange};

/// Text color of a draft that isn't a number in range.
const INVALID: Color = Color::hex(0xd64545);

/// A field for a number in `range`; see [`number_input`].
pub struct NumberInput {
    value: f64,
    range: RangeInclusive<f64>,
    on_change: Option<Rc<OnChange<f64>>>,
    style: Style,
}

/// Text field for a number. What's typed stays as typed ("3.", "-", "2" on
/// the way to "2026") and shows red while it isn't a number in range;
/// `on_change` gets each value that is. Enter clamps the draft into range.
#[doc = example!("number_input")]
pub fn number_input(value: f64) -> NumberInput {
    NumberInput {
        value,
        range: f64::NEG_INFINITY..=f64::INFINITY,
        on_change: None,
        style: Style {
            flex_direction: FlexDirection::Column,
            ..Style::default()
        },
    }
}

impl NumberInput {
    /// Values `on_change` may report; any finite number by default.
    pub fn range(mut self, r: RangeInclusive<f64>) -> Self {
        self.range = r;
        self
    }
    /// Called with each valid value typed, and the clamped one on Enter.
    pub fn on_change(mut self, f: impl Fn(&mut EventCx, f64) + 'static) -> Self {
        self.on_change = Some(Rc::new(Box::new(f)));
        self
    }
}

fn parse(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

impl Styled for NumberInput {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl From<NumberInput> for Element {
    fn from(n: NumberInput) -> Self {
        let NumberInput {
            value,
            range,
            on_change,
            style,
        } = n;
        let range = Rc::new(range);
        // The draft and the value it was typed over; another value (set by
        // the app, or this field's value adjusted by it) replaces the draft.
        let mut s = stateful(move |draft: &Option<(f64, String)>, h| {
            let shown = match draft {
                Some((v, s)) if *v == value => s.clone(),
                _ => value.to_string(),
            };
            let valid = parse(&shown).is_some_and(|v| range.contains(&v));
            let (r, f) = (range.clone(), on_change.clone());
            let (r2, f2, typed) = (range.clone(), on_change.clone(), shown.clone());
            let mut field = text_input(shown)
                .keyboard(Keyboard::Number)
                .on_change(move |cx, s| {
                    let v = parse(&s).filter(|v| r.contains(v));
                    if let (Some(v), Some(f)) = (v, &f) {
                        f(cx, v);
                    }
                    let over = v.unwrap_or(value);
                    cx.update(h, move |d: &mut Option<(f64, String)>| *d = Some((over, s)));
                })
                .on_submit(move |cx: &mut EventCx| {
                    let v = parse(&typed).unwrap_or(value).clamp(*r2.start(), *r2.end());
                    if let Some(f) = &f2 {
                        f(cx, v);
                    }
                    cx.update(h, |d: &mut Option<(f64, String)>| *d = None);
                });
            if !valid {
                field = field.color(INVALID);
            }
            field
        });
        *s.style_mut() = style;
        s.into()
    }
}
