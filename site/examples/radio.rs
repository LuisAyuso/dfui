//! # Radio
//!
//! `radio(selected, label)` is one option of a group. The group is just your
//! state: draw one radio per option and record the choice in `on_select`.
//! Tapping the label selects too.

use dfui::widgets::{Styled, column, radio, stateful, text};
use dfui::{Color, Element};

const SIZES: [&str; 3] = ["Small", "Medium", "Large"];

fn view() -> Element {
    stateful(|&picked: &usize, h| {
        column(SIZES.into_iter().enumerate().map(|(i, label)| {
            radio(picked == i, text(label))
                .accent(Color::hex(0x3b6fe0))
                .on_select(move |cx| cx.update(h, move |p: &mut usize| *p = i))
                .into()
        }))
        .gap(4.0)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
