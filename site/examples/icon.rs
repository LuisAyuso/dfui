use dfui::taffy::AlignItems;
use dfui::widgets::{Styled, icon, icon_button, row, stateful, text};
use dfui::{Color, Element, Path, Rect, Shape};

/// A five-pointed star in a 24×24 box.
fn star() -> Shape {
    let points: Vec<_> = (0..10)
        .map(|i| {
            let r = if i % 2 == 0 { 11.0 } else { 4.5 };
            let a = std::f32::consts::PI * (i as f32 / 5.0 - 0.5);
            [12.0 + r * a.cos(), 12.5 + r * a.sin()]
        })
        .collect();
    Shape::new(Path::polygon(&points), Rect::new(0.0, 0.0, 24.0, 24.0))
}

// `view` runs on every rebuild, and each new `Shape` is stored for good.
thread_local! {
    static STAR: Shape = star();
}

fn view() -> Element {
    let star = STAR.with(Shape::clone);
    stateful(move |stars: &u32, h| {
        row([
            icon_button(&star)
                .size(32.0)
                .color(Color::hex(0xf5a524))
                .on_click(move |cx| cx.update(h, |n: &mut u32| *n += 1))
                .into(),
            text(format!("{stars} stars")).size(18.0).into(),
            icon(&star).size(12.0).color(Color::hex(0x6b7080)).into(),
            icon(&star).size(48.0).color(Color::hex(0x6b7080)).into(),
        ])
        .gap(12.0)
        .align(AlignItems::CENTER)
    })
    .padding(16.0)
    .into()
}

fn main() {
    dfui::run(view);
}
