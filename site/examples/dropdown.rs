//! # Dropdown
//!
//! `dropdown(labels, selected)` shows the selected label and lists them all on
//! a tap. The app owns the selection: `on_change` gets the new index. Long
//! lists scroll and open at the selected option, and the menu flips above the
//! field near the bottom of the window.

use dfui::Element;
use dfui::widgets::{Styled, column, dropdown, stateful, text};

const SIZES: [&str; 4] = ["Small", "Medium", "Large", "Huge"];

fn view() -> Element {
    stateful(|&i: &usize, h| {
        column([
            dropdown(SIZES, i)
                .on_change(move |cx, v| cx.update(h, move |s: &mut usize| *s = v))
                .into(),
            text(format!("Picked {}", SIZES[i])).into(),
        ])
        .gap(12.0)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
