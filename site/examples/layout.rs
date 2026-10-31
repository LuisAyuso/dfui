//! # Row, column, container
//!
//! Layout is flexbox. `row` and `column` place children along an axis,
//! `container` wraps a single child, and `spacer().grow(1.0)` takes up the
//! leftover room.
//!
//! The `Styled` modifiers set padding, gap, size, grow and shrink, alignment
//! and wrapping. Containers also draw a background, rounded corners, a border
//! and a soft shadow, all as distance fields.

use dfui::widgets::{Styled, column, container, row, spacer, text};
use dfui::{Color, Element, Shadow};

fn chip(label: String, color: u32) -> Element {
    container(text(label).color(Color::WHITE))
        .bg(Color::hex(color))
        .radius(10.0)
        .padding_xy(12.0, 8.0)
        .into()
}

fn view() -> Element {
    column([
        row([
            chip("left".into(), 0x3b6fe0),
            spacer().grow(1.0).into(),
            chip("right".into(), 0x8a5ce2),
        ])
        .into(),
        row((1..=9).map(|i| chip(format!("wrap {i}"), 0x4cae6e)))
            .wrap()
            .gap(8.0)
            .into(),
        container(text("Border and shadow"))
            .bg(Color::WHITE)
            .radius(14.0)
            .border(1.0, Color::hex(0xdfe1e8))
            .shadow(Shadow {
                color: Color::BLACK.alpha(0.15),
                offset: (0.0, 4.0),
                blur: 12.0,
            })
            .padding(20.0)
            .into(),
    ])
    .gap(16.0)
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
