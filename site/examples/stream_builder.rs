//! # Stream builder
//!
//! `stream_builder(deps, start, build)` polls the stream that `start` returns
//! and collects its items into the snapshot's `data`, which can be any
//! `Default + Extend<Item>` type such as `Vec` or `String`. `done` turns true
//! when the stream ends. Items that arrive together cost one rebuild.
//!
//! The stream here is made with `futures_util::stream::unfold` and
//! `dfui::sleep`, so it runs in the browser without threads.

use std::time::Duration;

use dfui::widgets::{
    Snapshot, Styled, button, column, container, row, scroll, stateful, stream_builder, text,
};
use dfui::{Color, Element, sleep};
use futures_util::{Stream, stream};

fn ticks() -> impl Stream<Item = u32> {
    stream::unfold(1, |i| async move {
        if i > 20 {
            return None;
        }
        sleep(Duration::from_millis(250)).await;
        Some((i, i + 1))
    })
}

fn view() -> Element {
    stateful(|&run: &u32, h| {
        let feed = stream_builder(
            run,
            |_| ticks(),
            |snap: &Snapshot<Vec<u32>>| {
                let status = if snap.done { "Done" } else { "Receiving" };
                let items = snap.data.iter().rev().map(|i| {
                    container(text(format!("Item {i}")))
                        .padding_xy(12.0, 8.0)
                        .bg(Color::WHITE)
                        .radius(8.0)
                        .into()
                });
                column([
                    text(format!("{status}: {} items", snap.data.len()))
                        .color(Color::hex(0x6b7080))
                        .into(),
                    scroll(column(items).gap(6.0)).grow(1.0).into(),
                ])
                .gap(8.0)
                .grow(1.0)
            },
        );
        column([
            row([button(text("Restart").color(Color::WHITE))
                .on_click(move |cx| cx.update(h, |r: &mut u32| *r += 1))
                .into()])
            .into(),
            feed.grow(1.0).into(),
        ])
        .gap(12.0)
        .grow(1.0)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
