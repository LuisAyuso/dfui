//! Line icons for common actions, drawn in a 24×24 box with 2 px strokes
//! and round ends, in the style of Lucide. [`widgets::page`](crate::widgets::page)
//! uses them; apps can too, with `icon` and `icon_button`.

use crate::path::{Path, Point, Shape, Stroke, stroke};
use crate::render::Rect;

fn lines(parts: &[&[Point]]) -> Shape {
    let rings = parts.iter().flat_map(|l| stroke(l, Stroke::round(2.0)));
    let contours = rings.flat_map(|r| Path::polygon(&r).contours).collect();
    Shape::new(Path { contours }, Rect::new(0.0, 0.0, 24.0, 24.0))
}

// Shapes are `Rc`s, so one per thread; built once, cloned per use.
thread_local! {
    static MENU: Shape = lines(&[
        &[[4.0, 6.0], [20.0, 6.0]],
        &[[4.0, 12.0], [20.0, 12.0]],
        &[[4.0, 18.0], [20.0, 18.0]],
    ]);
    static BACK: Shape = lines(&[
        &[[19.0, 12.0], [5.0, 12.0]],
        &[[12.0, 19.0], [5.0, 12.0], [12.0, 5.0]],
    ]);
    static CLOSE: Shape = lines(&[&[[18.0, 6.0], [6.0, 18.0]], &[[6.0, 6.0], [18.0, 18.0]]]);
}

/// Three bars, for a button that opens a menu or drawer.
pub fn menu() -> Shape {
    MENU.with(Shape::clone)
}

/// An arrow pointing left, for going back.
pub fn back() -> Shape {
    BACK.with(Shape::clone)
}

/// A cross, for closing.
pub fn close() -> Shape {
    CLOSE.with(Shape::clone)
}
