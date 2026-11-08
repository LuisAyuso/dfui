//! # Tooltip
//!
//! `tooltip(child, tip)` shows `tip` beside `child` while the pointer is over
//! it, or after a long press on a touch screen, and flips to stay in the window.

use dfui::Element;
use dfui::widgets::{Styled, button, text, tooltip};

fn view() -> Element {
    tooltip(button(text("Hover me")), "Saves the team")
        .padding(48.0)
        .into()
}

fn main() {
    dfui_site::run(view);
}
