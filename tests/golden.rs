//! Golden images of each widget, rendered on the GPU of the reference machine.
//! `DFUI_BLESS=1 cargo test --test golden` rewrites them; on a mismatch the
//! actual image and a diff land in `target/golden/`.

use std::path::PathBuf;
use std::time::Duration;

use dfui::widgets::{
    Overlay, Shown, Styled, bottom_sheet, button, column, dialog, drawer, dropdown, overlays,
    progress, radio, row, segmented, slider, snackbar, spacer, spinner, stacked_progress, tabs,
    text, text_input, toggle, tooltip, wait_bar,
};
use dfui::{App, Bench, Color, Element, Event};
use image::{Rgba, RgbaImage};

const SCALE: f32 = 2.0;
/// Per-channel difference a pixel may have before it counts as changed.
const TOLERANCE: u8 = 2;

struct View(fn() -> Element);

impl App for View {
    fn view(&self) -> Element {
        (self.0)()
    }
}

fn bench(view: fn() -> Element, size: (f32, f32)) -> Bench<View> {
    Bench::new(View(view), size)
}

fn padded(e: impl Into<Element>) -> Element {
    column([e.into()]).padding(8.0).into()
}

fn tap(b: &mut Bench<View>, x: f32, y: f32) {
    b.event(Event::PointerDown { x, y });
    b.event(Event::PointerUp { x, y });
}

/// Frames side by side, left to right.
fn strip(frames: &[RgbaImage]) -> RgbaImage {
    let (w, h) = frames[0].dimensions();
    let mut out = RgbaImage::new(w * frames.len() as u32, h);
    for (i, f) in frames.iter().enumerate() {
        image::imageops::replace(&mut out, f, (i as u32 * w) as i64, 0);
    }
    out
}

/// Shots at each of `at`, in ms from now.
fn film(b: &mut Bench<View>, at: &[u64]) -> Vec<RgbaImage> {
    let mut t = 0;
    at.iter()
        .map(|&ms| {
            b.advance(Duration::from_millis(ms - t));
            t = ms;
            b.shot(SCALE)
        })
        .collect()
}

fn check(name: &str, actual: &RgbaImage) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = root.join("tests/golden").join(format!("{name}.png"));
    if std::env::var_os("DFUI_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        actual.save(&path).unwrap();
        return;
    }
    let golden = image::open(&path)
        .unwrap_or_else(|e| panic!("{name}: {e}; run with DFUI_BLESS=1 to create it"))
        .into_rgba8();
    let out = root.join("target/golden");
    let fail = |why: String| {
        std::fs::create_dir_all(&out).unwrap();
        actual.save(out.join(format!("{name}.actual.png"))).unwrap();
        panic!("{name}: {why}; see target/golden/");
    };
    if golden.dimensions() != actual.dimensions() {
        fail(format!(
            "size {:?}, golden {:?}",
            actual.dimensions(),
            golden.dimensions()
        ));
    }
    let mut diff = RgbaImage::new(golden.width(), golden.height());
    let mut changed = 0;
    for ((g, a), d) in golden.pixels().zip(actual.pixels()).zip(diff.pixels_mut()) {
        let off = g.0.iter().zip(a.0).any(|(&g, a)| g.abs_diff(a) > TOLERANCE);
        *d = if off {
            changed += 1;
            Rgba([255, 0, 0, 255])
        } else {
            // The golden, faded, for context.
            Rgba(g.0.map(|c| 191 + c / 4))
        };
    }
    if changed > 0 {
        std::fs::create_dir_all(&out).unwrap();
        diff.save(out.join(format!("{name}.diff.png"))).unwrap();
        fail(format!("{changed} pixels differ"));
    }
}

#[test]
fn button_states() {
    let mut b = bench(|| padded(button(text("Button"))), (120.0, 56.0));
    let rest = b.shot(SCALE);
    b.event(Event::PointerMove { x: 20.0, y: 20.0 });
    b.settle();
    let hover = b.shot(SCALE);
    b.event(Event::PointerDown { x: 20.0, y: 20.0 });
    b.settle();
    let pressed = b.shot(SCALE);
    check("button", &strip(&[rest, hover, pressed]));
}

#[test]
fn toggle_off_on() {
    let v = || {
        row([toggle(false).into(), toggle(true).into()])
            .gap(8.0)
            .padding(8.0)
            .into()
    };
    check("toggle", &bench(v, (130.0, 50.0)).shot(SCALE));
}

#[test]
fn radios() {
    let v = || {
        column([
            radio(true, text("Selected")).into(),
            radio(false, text("Not selected")).into(),
        ])
        .padding(8.0)
        .into()
    };
    check("radio", &bench(v, (160.0, 90.0)).shot(SCALE));
}

#[test]
fn slider_values() {
    let v = || {
        column([
            slider(0.0, 0.0..=1.0).into(),
            slider(0.3, 0.0..=1.0).into(),
            slider(1.0, 0.0..=1.0).into(),
        ])
        .gap(8.0)
        .padding(8.0)
        .into()
    };
    check("slider", &bench(v, (200.0, 130.0)).shot(SCALE));
}

#[test]
fn segmented_and_tabs() {
    let v = || {
        column([
            segmented(["Day", "Week", "Month"], 1).into(),
            tabs(["One", "Two", "Three"], 0).into(),
        ])
        .gap(8.0)
        .padding(8.0)
        .into()
    };
    check("segmented", &bench(v, (240.0, 110.0)).shot(SCALE));
}

#[test]
fn text_inputs() {
    let v = || {
        column([text_input("").into(), text_input("Typed text").into()])
            .gap(8.0)
            .padding(8.0)
            .into()
    };
    check("text_input", &bench(v, (220.0, 110.0)).shot(SCALE));
}

#[test]
fn progress_bars() {
    let v = || {
        column([
            progress(0.4).into(),
            stacked_progress([(0.7, Color::hex(0x9db7f0)), (0.3, Color::hex(0x3b6fe0))]).into(),
            wait_bar().into(),
            spinner().into(),
        ])
        .gap(12.0)
        .padding(8.0)
        .into()
    };
    let mut b = bench(v, (180.0, 110.0));
    check("progress", &strip(&film(&mut b, &[0, 300, 700])));
}

#[test]
fn dropdown_opens() {
    let v = || padded(dropdown(["Apple", "Banana", "Cherry"], 1));
    let mut b = bench(v, (160.0, 196.0));
    let mut frames = vec![b.shot(SCALE)];
    tap(&mut b, 20.0, 20.0);
    frames.extend(film(&mut b, &[0, 60, 120]));
    b.settle();
    frames.push(b.shot(SCALE));
    check("dropdown", &strip(&frames));
}

#[test]
fn dropdown_flips_above_near_the_bottom() {
    let v = || {
        column([
            spacer().grow(1.0).into(),
            dropdown(["Apple", "Banana", "Cherry"], 1).into(),
        ])
        .padding(8.0)
        .into()
    };
    let mut b = bench(v, (160.0, 196.0));
    tap(&mut b, 20.0, 176.0);
    let mut frames = film(&mut b, &[0, 60, 120]);
    b.settle();
    frames.push(b.shot(SCALE));
    check("dropdown_flipped", &strip(&frames));
}

#[test]
fn tooltip_on_hover() {
    let v = || padded(tooltip(button(text("Hover")), "A tip"));
    let mut b = bench(v, (160.0, 100.0));
    b.event(Event::PointerMove { x: 20.0, y: 20.0 });
    b.settle();
    check("tooltip", &b.shot(SCALE));
}

/// A button at the top left that shows `open`'s overlay.
fn host(open: fn(Shown) -> Overlay) -> Element {
    overlays(move |ov| {
        column([button(text("Open"))
            .on_click(move |cx| ov.show(cx, open))
            .into()])
        .padding(8.0)
    })
    .into()
}

const PHONE: (f32, f32) = (320.0, 480.0);

/// Opens the host's overlay, then taps at `close`: frames while it slides
/// in, settled, while it slides out, and gone.
fn open_close(view: fn() -> Element, close: (f32, f32)) -> RgbaImage {
    let mut b = bench(view, PHONE);
    tap(&mut b, 20.0, 20.0);
    let mut frames = film(&mut b, &[0, 60, 120]);
    b.settle();
    frames.push(b.shot(SCALE));
    tap(&mut b, close.0, close.1);
    frames.extend(film(&mut b, &[60, 120]));
    b.settle();
    frames.push(b.shot(SCALE));
    strip(&frames)
}

fn confirm(me: Shown) -> Overlay {
    dialog(
        column([
            text("Delete file?").size(18.0).into(),
            text("This can't be undone.").size(14.0).into(),
            row([
                button(text("Cancel"))
                    .on_click(move |cx| me.close(cx))
                    .into(),
                button(text("Delete"))
                    .on_click(move |cx| me.close(cx))
                    .into(),
            ])
            .gap(8.0)
            .into(),
        ])
        .gap(12.0),
    )
}

#[test]
fn dialog_opens_and_closes() {
    // A tap on the scrim, outside the card.
    check("dialog", &open_close(|| host(confirm), (160.0, 470.0)));
}

#[test]
fn drawer_opens_and_closes() {
    fn menu(_: Shown) -> Overlay {
        drawer(column(["Inbox", "Sent", "Trash"].map(|l| text(l).into())).gap(16.0))
    }
    check("drawer", &open_close(|| host(menu), (310.0, 240.0)));
}

#[test]
fn bottom_sheet_opens_and_closes() {
    fn sheet(_: Shown) -> Overlay {
        bottom_sheet(column([
            text("Share").size(18.0).into(),
            text("Copy link").into(),
        ]))
    }
    check("bottom_sheet", &open_close(|| host(sheet), (160.0, 100.0)));
}

#[test]
fn snackbar_shows_then_leaves() {
    let mut b = bench(|| host(|_| snackbar("Saved")), PHONE);
    tap(&mut b, 20.0, 20.0);
    let mut frames = film(&mut b, &[0, 120, 500, 4100]);
    b.settle();
    frames.push(b.shot(SCALE));
    check("snackbar", &strip(&frames));
}
