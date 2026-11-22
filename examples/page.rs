use dfui::Element;
use dfui::widgets::{Styled, button, drawer, heading, overlays, page, stateful, text};

fn view() -> Element {
    overlays(|ov| {
        stateful(move |open: &bool, h| -> Element {
            if *open {
                page(heading("Details"))
                    .back(move |cx| cx.update(h, |o: &mut bool| *o = false))
                    .child(text("Back, Escape or the Android back button return."))
                    .into()
            } else {
                page(heading("Home"))
                    .menu(move |cx| ov.show(cx, |_| drawer(text("Menu"))))
                    .child(
                        button(text("Open details"))
                            .on_click(move |cx| cx.update(h, |o: &mut bool| *o = true)),
                    )
                    .into()
            }
        })
        .fill()
    })
    .into()
}

fn main() {
    dfui::run(view);
}
