//! # Progress
//!
//! `progress(value)` fills a bar to a fraction in 0..=1, easing to each new
//! value. `stacked_progress` shows several fractions in one bar (here
//! played over buffered), the smaller on top. For waits of unknown length,
//! `wait_bar()` sweeps a segment across and `spinner()` turns an arc.

use dfui::taffy::AlignItems;
use dfui::widgets::{
    Styled, column, progress, row, slider, spinner, stacked_progress, stateful, text, wait_bar,
};
use dfui::{Color, Element};

fn view() -> Element {
    stateful(|&v: &f32, h| {
        let buffered = (v * 1.6).min(1.0);
        column([
            text(format!("{:.0}%", v * 100.0)).size(18.0).into(),
            slider(v, 0.0..=1.0)
                .on_change(move |cx, v| cx.update(h, move |s: &mut f32| *s = v))
                .fill_width()
                .into(),
            progress(v).fill_width().into(),
            stacked_progress([(v, Color::hex(0x3b6fe0)), (buffered, Color::hex(0xa9bdf0))])
                .fill_width()
                .into(),
            wait_bar().fill_width().into(),
            row([spinner().into(), spinner().size(40.0).into()])
                .gap(16.0)
                .align(AlignItems::CENTER)
                .into(),
        ])
        .gap(16.0)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
