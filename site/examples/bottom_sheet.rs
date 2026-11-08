//! # Bottom sheet
//!
//! `bottom_sheet(child)` is a modal panel rising from the bottom edge, full
//! width. Show it through an `overlays` host, like a `dialog`.

use dfui::widgets::{Overlays, Shown, Styled, bottom_sheet, button, column, overlays, text};
use dfui::{Element, Handle};

fn content(me: Shown) -> Element {
    column([
        text("Bottom sheet").size(20.0).into(),
        button(text("Close"))
            .on_click(move |cx| me.close(cx))
            .into(),
    ])
    .gap(16.0)
    .into()
}

fn page(ov: Handle<Overlays>) -> Element {
    button(text("Open"))
        .on_click(move |cx| ov.show(cx, |me| bottom_sheet(content(me))))
        .into()
}

fn view() -> Element {
    column([overlays(page).into()]).fill().padding(16.0).into()
}

fn main() {
    dfui_site::run(view);
}
