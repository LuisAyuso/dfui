//! # On back
//!
//! `on_back(f)` runs `f` on Escape or the Android back button and keeps the
//! app open; an open overlay takes the press first. Add it to screens that
//! have somewhere to go back to, and leave it out where Back should exit.

use dfui::Element;
use dfui::widgets::{Styled, button, column, on_back, stateful, text};

fn view() -> Element {
    stateful(|&inner: &bool, h| -> Element {
        if inner {
            column([
                on_back(move |cx| cx.update(h, |s: &mut bool| *s = false)).into(),
                text("Details. Press Escape to go back.").into(),
            ])
            .into()
        } else {
            button(text("Open details"))
                .on_click(move |cx| cx.update(h, |s: &mut bool| *s = true))
                .into()
        }
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
