//! # Chip
//!
//! `chip(label)` is a small rounded label, and `choice_chip(label, selected)`
//! one you can tap. `badge(child, label)` puts a label, usually a count, over
//! the top right corner of another widget.

use dfui::taffy::AlignItems;
use dfui::widgets::{Styled, badge, button, chip, choice_chip, column, row, stateful, text};
use dfui::{Element, Role};

fn view() -> Element {
    stateful(|&picked: &usize, h| {
        let choice = move |i: usize, label: &str| -> Element {
            choice_chip(label, picked == i)
                .on_click(move |cx| cx.update(h, move |p: &mut usize| *p = i))
                .into()
        };
        column([
            row([
                chip("Block").into(),
                chip("Injured")
                    .bg(Role::Inverse)
                    .content(Role::OnInverse)
                    .into(),
            ])
            .gap(8.0)
            .into(),
            row([
                choice(0, "All"),
                choice(1, "Linemen"),
                choice(2, "Throwers"),
            ])
            .gap(8.0)
            .into(),
            row([badge(button(text("Inbox")), "3").into()])
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
