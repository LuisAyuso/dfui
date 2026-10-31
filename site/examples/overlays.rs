//! # Overlays
//!
//! Overlays float above the rest of the tree without taking part in its
//! layout, aren't clipped by scroll views, and get input first.
//!
//! `overlays(|ov| content)` hosts the ones opened from handlers:
//! `ov.show(cx, |me| dialog(..))` slides a `dialog`, `drawer`,
//! `bottom_sheet` or `snackbar` in, and `me.close(cx)` (or a tap outside a
//! modal one) slides it out and removes it once the animation ends. A
//! snackbar closes itself after four seconds.
//!
//! A `dropdown` and a `tooltip` own their popup, so they need no host.
//! `overlay(child)` (covers the window) and `popup(child)` (below its parent)
//! are the building blocks underneath.

use dfui::taffy::AlignItems;
use dfui::widgets::{
    Overlay, Overlays, Shown, Styled, bottom_sheet, button, column, dialog, drawer, dropdown,
    overlays, row, snackbar, stateful, text, tooltip,
};
use dfui::{Color, Element, Handle};

fn panel(title: &str, me: Shown) -> Element {
    column([
        text(title).size(20.0).into(),
        button(text("Close").color(Color::WHITE))
            .on_click(move |cx| me.close(cx))
            .into(),
    ])
    .gap(16.0)
    .into()
}

fn content(ov: Handle<Overlays>) -> Element {
    let open = move |label: &str, show: fn(Shown) -> Overlay| -> Element {
        button(text(label).color(Color::WHITE))
            .on_click(move |cx| ov.show(cx, show))
            .into()
    };
    stateful(move |&size: &usize, h| {
        column([
            row([
                open("Dialog", |me| dialog(panel("A dialog", me))),
                open("Drawer", |me| drawer(panel("A drawer", me))),
                open("Sheet", |me| bottom_sheet(panel("A bottom sheet", me))),
            ])
            .gap(8.0)
            .wrap()
            .into(),
            row([
                dropdown(["Small", "Medium", "Large"], size)
                    .on_change(move |cx, i| cx.update(h, move |s: &mut usize| *s = i))
                    .into(),
                tooltip(open("Snack", |_| snackbar("Saved")), "Shows a snackbar").into(),
            ])
            .gap(8.0)
            .align(AlignItems::CENTER)
            .into(),
        ])
        .gap(16.0)
    })
    .padding(16.0)
    .into()
}

fn view() -> Element {
    overlays(content).into()
}

fn main() {
    dfui_site::run(view);
}
