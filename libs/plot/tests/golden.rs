//! Golden image of a plot, rendered on the GPU of the reference machine.
//! `DFUI_BLESS=1 cargo test -p dfui-plot --test golden` rewrites it; on a
//! mismatch the actual image lands in `target/golden/`.

use std::path::PathBuf;
use std::rc::Rc;

use dfui::widgets::{Styled, column};
use dfui::{Color, Element, Role};
use dfui_plot::{line, plot};
use image::RgbaImage;

const TOLERANCE: u8 = 2;

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
    let changed = golden.dimensions() != actual.dimensions()
        || golden
            .pixels()
            .zip(actual.pixels())
            .any(|(g, a)| g.0.iter().zip(a.0).any(|(&g, a)| g.abs_diff(a) > TOLERANCE));
    if changed {
        let out = root.join("../target/golden");
        std::fs::create_dir_all(&out).unwrap();
        actual.save(out.join(format!("{name}.actual.png"))).unwrap();
        panic!("{name} differs; see target/golden/");
    }
}

fn view() -> Element {
    let rows: Rc<[(u64, f64, f64)]> = (0..40)
        .flat_map(|i| {
            let x = i as f64 / 4.0;
            [(0, x, x.sin() * 3.0), (1, x, x * 0.4 - 1.0)]
        })
        .collect();
    let p = plot(&rows)
        .layer(
            line()
                .x(|r: &(u64, f64, f64)| r.1)
                .y(|r| r.2)
                .filter(|r| r.0 == 0)
                .name("sine"),
        )
        .layer(
            line()
                .x(|r: &(u64, f64, f64)| r.1)
                .y(|r| r.2)
                .filter(|r| r.0 == 1)
                .name("trend"),
        )
        .vline("today", 6.5, Color::hex(0xd64545))
        .hline("", 0.0, Role::Muted)
        .x_label("x");
    column([p.into()]).padding(8.0).fill().into()
}

#[test]
fn legend_and_rules() {
    let p = dfui::snapshot(view, (720, 440), 2.0);
    check(
        "legend_and_rules",
        &RgbaImage::from_raw(p.width, p.height, p.rgba).unwrap(),
    );
}
