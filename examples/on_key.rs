use dfui::widgets::{Styled, column, on_key, stateful, text};
use dfui::{Element, Key};

fn view() -> Element {
    stateful(|&n: &i32, h| {
        column([
            on_key(move |cx, k| {
                let step = match k {
                    Key::Up => -1,
                    Key::Down => 1,
                    _ => return,
                };
                cx.update(h, move |n: &mut i32| *n += step);
                cx.stop();
            })
            .into(),
            text(format!("Row {n}. Press Up or Down.")).into(),
        ])
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui::run(view);
}
