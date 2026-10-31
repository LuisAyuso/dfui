//! # Tabs
//!
//! `tabs(labels, selected)` is a tab bar with a sliding underline. It only
//! draws the bar: show the selected page yourself, as below.

use dfui::widgets::{Styled, column, stateful, tabs, text};
use dfui::{Color, Element};

const PAGES: [(&str, &str); 3] = [
    ("Inbox", "3 new messages"),
    ("Sent", "Nothing waiting to send"),
    ("Archive", "128 messages"),
];

fn view() -> Element {
    stateful(|&tab: &usize, h| {
        column([
            tabs(PAGES.map(|(label, _)| label), tab)
                .accent(Color::hex(0x3b6fe0))
                .on_change(move |cx, i| cx.update(h, move |t: &mut usize| *t = i))
                .into(),
            text(PAGES[tab].1).size(18.0).into(),
        ])
        .gap(16.0)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
