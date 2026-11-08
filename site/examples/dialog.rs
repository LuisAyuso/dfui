//! # Dialog
//!
//! `dialog(child)` is a modal card centered over the dimmed window. Show it
//! through an `overlays` host with `ov.show(cx, |me| ..)`; `me.close(cx)`, a
//! tap outside or Back closes it. `ov.confirm` is the common yes-or-no case.

use dfui::widgets::{Overlays, Shown, Styled, button, column, dialog, overlays, text};
use dfui::{Element, Handle};

fn content(me: Shown) -> Element {
    column([
        text("Dialog").size(20.0).into(),
        button(text("Close"))
            .on_click(move |cx| me.close(cx))
            .into(),
    ])
    .gap(16.0)
    .into()
}

fn page(ov: Handle<Overlays>) -> Element {
    button(text("Open"))
        .on_click(move |cx| ov.show(cx, |me| dialog(content(me))))
        .into()
}

fn view() -> Element {
    column([overlays(page).into()]).fill().padding(16.0).into()
}

fn main() {
    dfui_site::run(view);
}
