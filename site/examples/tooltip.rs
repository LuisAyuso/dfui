use dfui::Element;
use dfui::widgets::{Styled, button, text, tooltip};

fn view() -> Element {
    tooltip(button(text("Hover me")), "Saves the team")
        .padding(48.0)
        .into()
}

fn main() {
    dfui::run(view);
}
