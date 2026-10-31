//! # Text
//!
//! `text` draws a run of text straight from glyph outlines, so it stays sharp
//! at any size and zoom. It wraps at spaces to the width it is given.
//!
//! Set `size` in logical pixels, `color`, and `font` with any TrueType or
//! OpenType bytes; characters a font lacks fall back to the bundled DejaVu Sans.

use dfui::widgets::{Styled, column, text};
use dfui::{Color, Element};

fn view() -> Element {
    column([
        text("Small print").size(12.0).color(Color::hex(0x6b7080)).into(),
        text("Body text").size(16.0).into(),
        text("Headline").size(36.0).color(Color::hex(0x3b6fe0)).into(),
        text("Long paragraphs wrap at spaces to the available width. Resize the frame to watch them reflow.")
            .size(16.0)
            .into(),
    ])
    .gap(12.0)
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
