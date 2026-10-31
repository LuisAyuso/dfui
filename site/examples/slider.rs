//! # Slider
//!
//! `slider(value, range)` picks a number in `range`. A tap jumps to the
//! pointer, a drag follows it (and keeps following off the track until
//! release), and `on_change` reports every new value.

use dfui::widgets::{Styled, column, slider, stateful, text};
use dfui::{Color, Element};

fn view() -> Element {
    stateful(|&volume: &f32, h| {
        column([
            text(format!("Volume {volume:.0}")).size(18.0).into(),
            slider(volume, 0.0..=100.0)
                .accent(Color::hex(0x3b6fe0))
                .on_change(move |cx, v| cx.update(h, move |s: &mut f32| *s = v))
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
