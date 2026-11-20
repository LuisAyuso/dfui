use dfui::Element;
use dfui::widgets::{Styled, column, dropdown, stateful, text};

const SIZES: [&str; 4] = ["Small", "Medium", "Large", "Huge"];

fn view() -> Element {
    stateful(|&i: &usize, h| {
        column([
            dropdown(SIZES, i)
                .on_change(move |cx, v| cx.update(h, move |s: &mut usize| *s = v))
                .into(),
            text(format!("Picked {}", SIZES[i])).into(),
        ])
        .gap(12.0)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui::run(view);
}
