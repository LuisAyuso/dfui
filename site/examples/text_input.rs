use dfui::widgets::{Styled, column, stateful, text, text_input};
use dfui::{Color, Element};

fn view() -> Element {
    stateful(|name: &String, h| {
        let greeting = if name.is_empty() {
            "Type your name".to_string()
        } else {
            format!("Hello, {name}!")
        };
        column([
            text_input(name.clone())
                .placeholder("Your name")
                .accent(Color::hex(0x3b6fe0))
                .on_change(move |cx, v| cx.update(h, |n: &mut String| *n = v))
                .into(),
            text(greeting).size(18.0).into(),
        ])
        .gap(12.0)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui::run(view);
}
