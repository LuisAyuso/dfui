//! # Toggle
//!
//! `toggle(on)` is an on/off switch. It shows the value it is given and
//! reports taps through `on_change` with the new value; the thumb slides
//! between the two states.

use dfui::taffy::AlignItems;
use dfui::widgets::{Styled, row, stateful, text, toggle};
use dfui::{Color, Element};

fn view() -> Element {
    stateful(|&on: &bool, h| {
        row([
            toggle(on)
                .accent(Color::hex(0x4cae6e))
                .on_change(move |cx, v| cx.update(h, move |s: &mut bool| *s = v))
                .into(),
            text(if on { "Wi-Fi on" } else { "Wi-Fi off" })
                .size(18.0)
                .into(),
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
