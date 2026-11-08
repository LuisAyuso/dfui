//! # Watch
//!
//! `watch(&notifier, || view)` rebuilds its subtree each time the `Notifier`
//! is notified. Any thread may call `notify`, so it suits data that changes
//! outside the UI, such as a download's progress or a store. Here a button
//! stands in for the outside world.

use std::cell::Cell;
use std::rc::Rc;

use dfui::widgets::{Styled, button, column, text, watch};
use dfui::{Element, Notifier};

thread_local! {
    static STORE: (Notifier, Rc<Cell<u32>>) = Default::default();
}

fn view() -> Element {
    let (notifier, count) = STORE.with(|s| s.clone());
    let shown = count.clone();
    let n = notifier.clone();
    column([
        watch(&notifier, move || text(format!("Received {}", shown.get()))).into(),
        button(text("Send"))
            .on_click(move |_| {
                count.set(count.get() + 1);
                n.notify();
            })
            .into(),
    ])
    .gap(12.0)
    .padding(16.0)
    .into()
}

fn main() {
    dfui_site::run(view);
}
