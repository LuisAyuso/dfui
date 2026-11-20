use dfui::taffy::AlignItems;
use dfui::widgets::{Styled, image, row};
use dfui::{Bitmap, Color, Element};

/// A small gradient, standing in for a decoded file.
fn gradient() -> Bitmap {
    let size = 64u32;
    let rgba = (0..size * size)
        .flat_map(|i| [(i % size * 4) as u8, (i / size * 4) as u8, 200, 255])
        .collect();
    Bitmap::from_rgba8(size, size, rgba)
}

thread_local! {
    static PICTURE: Bitmap = gradient();
}

fn view() -> Element {
    let pic = PICTURE.with(Bitmap::clone);
    row([
        image(pic.clone()).width(64.0).into(),
        image(pic.clone()).width(96.0).radius(16.0).into(),
        image(pic)
            .width(96.0)
            .radius(48.0)
            .tint(Color::WHITE.alpha(0.5))
            .into(),
    ])
    .gap(16.0)
    .padding(16.0)
    .wrap()
    .align(AlignItems::CENTER)
    .into()
}

fn main() {
    dfui::run(view);
}
