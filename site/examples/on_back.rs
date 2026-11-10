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
    dfui::run(view);
}
