use taffy::{AlignItems, LengthPercentageAuto, Position};

use super::{Button, Container, Styled, button, container, text};
use crate::theme::Role;
use crate::widget::Element;

const RADIUS: f32 = 12.0;
const OVERHANG: f32 = 6.0;

/// A small rounded label; restyle with `.bg` and `.content`.
#[doc = example!("chip")]
pub fn chip(label: impl Into<String>) -> Container {
    container(text(label).size(13.0))
        .bg(Role::Track)
        .content(Role::Ink)
        .radius(RADIUS)
        .padding_xy(10.0, 4.0)
}

/// A chip to tap, filled while `selected`; handle it with `.on_click`.
pub fn choice_chip(label: impl Into<String>, selected: bool) -> Button {
    let (fill, ink) = match selected {
        true => (Role::Accent, Role::OnAccent),
        false => (Role::Track, Role::Ink),
    };
    button(text(label).size(13.0).color(ink))
        .color(fill)
        .radius(RADIUS)
        .flat()
        .padding_xy(10.0, 4.0)
}

/// `label` over the top right corner of `child`, such as a count; it takes
/// no layout space. In a column it stretches like any child, so the corner
/// is the column's.
pub fn badge(child: impl Into<Element>, label: impl Into<String>) -> Container {
    let mut mark = container(text(label).size(11.0))
        .bg(Role::Inverse)
        .content(Role::OnInverse)
        .radius(8.0)
        .padding_xy(5.0, 1.0)
        .min_width(16.0)
        .align(AlignItems::CENTER);
    let s = mark.style_mut();
    s.position = Position::Absolute;
    s.inset.top = LengthPercentageAuto::length(-OVERHANG);
    s.inset.right = LengthPercentageAuto::length(-OVERHANG);
    let mut host = container(child).child(mark);
    host.style_mut().position = Position::Relative;
    host
}
