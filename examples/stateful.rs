use dfui::taffy::AlignItems;
use dfui::widgets::{Styled, button, column, row, stateful, text};
use dfui::{Color, Element};

fn view() -> Element {
    column((1..=3).map(card)).gap(12.0).padding(16.0).into()
}

fn card(n: u32) -> Element {
    stateful(move |&open: &bool, h| {
        let mut body = vec![text(format!("Card {n}")).size(18.0).into()];
        if open {
            body.push(
                text("This text is shown from the card's own state.")
                    .color(Color::hex(0x6b7080))
                    .into(),
            );
        }
        row([
            column(body).gap(4.0).grow(1.0).shrink(1.0).into(),
            button(text(if open { "Less" } else { "More" }).color(Color::WHITE))
                .on_click(move |cx| cx.update(h, |o: &mut bool| *o = !*o))
                .into(),
        ])
        .gap(12.0)
        .padding(14.0)
        .bg(Color::WHITE)
        .radius(12.0)
        .align(AlignItems::CENTER)
    })
    .into()
}

fn main() {
    dfui::run(view);
}
