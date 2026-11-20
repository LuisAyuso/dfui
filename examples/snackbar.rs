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
    dfui::run(view);
}
