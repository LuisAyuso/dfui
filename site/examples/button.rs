//! # Button
//!
//! `button` wraps any child in a tappable box. `on_click` runs when a press is
//! released inside it, and usually changes state through a `stateful` handle.
//!
//! `color` and `radius` restyle it; hover and press fade on their own.

use dfui::taffy::AlignItems;
use dfui::widgets::{Styled, button, row, stateful, text};
use dfui::{Color, Element};

fn view() -> Element {
    stateful(|count: &u32, h| {
        row([
            button(text("Click me").color(Color::WHITE))
                .on_click(move |cx| cx.update(h, |n: &mut u32| *n += 1))
                .into(),
            button(text("Reset").color(Color::hex(0x1d2030)))
                .color(Color::hex(0xe1e3ea))
                .radius(999.0)
                .on_click(move |cx| cx.update(h, |n: &mut u32| *n = 0))
                .into(),
            text(format!("{count} clicks")).size(18.0).into(),
        ])
        .gap(12.0)
        .align(AlignItems::CENTER)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
