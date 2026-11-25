use dfui::widgets::{Styled, column, container, row, scroll, text};
use dfui::{Color, Element};

fn view() -> Element {
    // A shelf that scrolls sideways; its cards keep their width.
    let cards = (1..=12).map(|i| {
        container(text(format!("Card {i}")))
            .width(96.0)
            .height(64.0)
            .shrink(0.0)
            .padding(10.0)
            .radius(8.0)
            .bg(Color::hex(0xdde3f0))
            .into()
    });
    let shelf = scroll(row(cards).gap(8.0)).horizontal();
    let rows = (1..=50).map(|i| {
        let bg = if i % 2 == 0 { 0xe9ebf1 } else { 0xffffff };
        container(text(format!("Row {i}")))
            .padding_xy(12.0, 10.0)
            .bg(Color::hex(bg))
            .into()
    });
    column([shelf.into(), scroll(column(rows)).grow(1.0).into()])
        .gap(12.0)
        .padding(16.0)
        .into()
}

fn main() {
    dfui::run(view);
}
