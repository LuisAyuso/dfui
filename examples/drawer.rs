use dfui::widgets::{Overlays, Shown, Styled, button, column, drawer, overlays, text};
use dfui::{Element, Handle};

fn content(me: Shown) -> Element {
    column([
        text("Drawer").size(20.0).into(),
        button(text("Close"))
            .on_click(move |cx| me.close(cx))
            .into(),
    ])
    .gap(16.0)
    .into()
}

fn page(ov: Handle<Overlays>) -> Element {
    button(text("Open"))
        .on_click(move |cx| ov.show(cx, |me| drawer(content(me))))
        .into()
}

fn view() -> Element {
    column([overlays(page).into()]).fill().padding(16.0).into()
}

fn main() {
    dfui::run(view);
}
