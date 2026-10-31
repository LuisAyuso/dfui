//! # Text input
//!
//! `text_input` is a single-line field. It shows the `value` it is given and
//! reports each edit through `on_change`, so the text lives in your state.
//!
//! Tap to focus. Arrows, Home and End move the caret by whole graphemes; with
//! shift they select. Ctrl/Cmd+A, C, X and V select all, copy, cut and paste.

use dfui::widgets::{Styled, column, stateful, text, text_input};
use dfui::{Color, Element};

fn view() -> Element {
    stateful(|name: &String, h| {
        let greeting = if name.is_empty() {
            "Type your name".to_string()
        } else {
            format!("Hello, {name}!")
        };
        column([
            text_input(name.clone())
                .placeholder("Your name")
                .accent(Color::hex(0x3b6fe0))
                .on_change(move |cx, v| cx.update(h, |n: &mut String| *n = v))
                .into(),
            text(greeting).size(18.0).into(),
        ])
        .gap(12.0)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
