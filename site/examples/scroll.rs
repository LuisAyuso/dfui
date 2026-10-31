//! # Scroll
//!
//! `scroll` clips its child to its own box and scrolls it vertically: wheel,
//! or drag with a fling that slows down after release. Give it a bounded
//! height (`grow` inside a column, or `height`) so there is something to
//! scroll.

use dfui::widgets::{Styled, column, container, scroll, text};
use dfui::{Color, Element};

fn view() -> Element {
    let rows = (1..=50).map(|i| {
        let bg = if i % 2 == 0 { 0xe9ebf1 } else { 0xffffff };
        container(text(format!("Row {i}")))
            .padding_xy(12.0, 10.0)
            .bg(Color::hex(bg))
            .into()
    });
    column([scroll(column(rows)).grow(1.0).into()])
        .padding(16.0)
        .into()
}

fn main() {
    dfui_site::run(view);
}
