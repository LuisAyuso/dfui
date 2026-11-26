use dfui::Element;
use dfui::widgets::{Styled, column, number_input, stateful_with, text};

fn view() -> Element {
    stateful_with(
        || 2026.0,
        |&year: &f64, h| {
            column([
                number_input(year)
                    .range(1990.0..=2100.0)
                    .on_change(move |cx, v| cx.update(h, move |y: &mut f64| *y = v))
                    .into(),
                text(format!("Starts in {year}")).size(18.0).into(),
            ])
            .gap(12.0)
        },
    )
    .padding(16.0)
    .into()
}

fn main() {
    dfui::run(view);
}
