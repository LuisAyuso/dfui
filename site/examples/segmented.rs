//! # Segmented
//!
//! `segmented(labels, selected)` is a set of mutually exclusive options in one
//! pill. The highlight slides to the selected option, and `on_change` gets its
//! index.

use dfui::Element;
use dfui::widgets::{Styled, column, segmented, stateful, text};

const RANGES: [&str; 3] = ["Day", "Week", "Month"];

fn view() -> Element {
    stateful(|&selected: &usize, h| {
        column([
            segmented(RANGES, selected)
                .on_change(move |cx, i| cx.update(h, move |s: &mut usize| *s = i))
                .into(),
            text(format!("Showing one {}", RANGES[selected].to_lowercase()))
                .size(18.0)
                .into(),
        ])
        .gap(12.0)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
