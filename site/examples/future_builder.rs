//! # Future builder
//!
//! `future_builder(deps, start, build)` runs the future that `start` returns
//! and rebuilds when it resolves; until then the snapshot's `data` is `None`.
//! Changing `deps` drops the running future and starts a new one, and
//! removing the widget cancels it.
//!
//! `dfui::sleep` stands in for a network request; any std `Future` works,
//! from any runtime.

use std::time::Duration;

use dfui::widgets::{Snapshot, Styled, button, column, future_builder, row, stateful, text};
use dfui::{Color, Element, sleep};

fn view() -> Element {
    stateful(|&reloads: &u32, h| {
        let result = future_builder(
            reloads,
            move |_| async move {
                sleep(Duration::from_secs(1)).await;
                format!("Loaded, request #{}", reloads + 1)
            },
            |snap: &Snapshot<Option<String>>| match &snap.data {
                Some(s) => text(s).size(20.0),
                None => text("Loading…").size(20.0).color(Color::hex(0x6b7080)),
            },
        );
        column([
            row([button(text("Reload").color(Color::WHITE))
                .on_click(move |cx| cx.update(h, |n: &mut u32| *n += 1))
                .into()])
            .into(),
            result.into(),
        ])
        .gap(12.0)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
