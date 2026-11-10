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
    dfui::run(view);
}
