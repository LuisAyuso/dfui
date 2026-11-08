//! # Snackbar
//!
//! `snackbar(message)` is a short note at the bottom of the window for four
//! seconds. Shown through an `overlays` host, several wait their turn.

use dfui::widgets::{Overlays, Styled, button, column, overlays, snackbar, text};
use dfui::{Element, Handle};

fn page(ov: Handle<Overlays>) -> Element {
    button(text("Save"))
        .on_click(move |cx| ov.show(cx, |_| snackbar("Saved")))
        .into()
}

fn view() -> Element {
    column([overlays(page).into()]).fill().padding(16.0).into()
}

fn main() {
    dfui_site::run(view);
}
