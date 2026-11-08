//! Filled outlines drawn the same way as glyphs.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use crate::render::Rect;

pub type Point = [f32; 2];
/// Quadratic Bézier segment: start, control, end.
pub type Quad = [Point; 3];

/// Closed contours of quadratic segments in logical px (y down), filled by
/// nonzero winding like a glyph. Get one from `PaintCx::glyph_path` or
/// `Path::polygon`; draw it with `PaintCx::path`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub contours: Vec<Vec<Quad>>,
}

impl Path {
    /// A single contour of straight edges through `points`.
    pub fn polygon(points: &[Point]) -> Path {
        let edges = (0..points.len())
            .map(|i| line(points[i], points[(i + 1) % points.len()]))
            .collect();
        Path {
            contours: vec![edges],
        }
    }

    pub fn offset(mut self, dx: f32, dy: f32) -> Path {
        for q in self.contours.iter_mut().flatten() {
            for p in q {
                *p = [p[0] + dx, p[1] + dy];
            }
        }
        self
    }
}

/// A `Path` stored on the GPU once, like a glyph, and drawn into any rect
/// with `PaintCx::shape`: scaling it costs nothing. Use it for icons and
/// other shapes drawn every frame; cheap to clone. Equal shapes share their
/// storage, which is reused once a shape hasn't been drawn for a while.
#[derive(Clone)]
pub struct Shape(pub(crate) Rc<ShapeData>);

pub(crate) struct ShapeData {
    /// Hash of the path and view; equal shapes share their storage.
    pub key: u64,
    pub path: Path,
    pub view: Rect,
}

impl Shape {
    /// A shape written by the `svg2path` tool, usually `include_bytes!`d.
    /// Panics on anything else: these are assets bundled with the app.
    pub fn from_bytes(bytes: &[u8]) -> Shape {
        let body = bytes.strip_prefix(b"DFP1").expect("not a dfui path");
        let mut words = body.as_chunks::<4>().0.iter().copied();
        let mut next = || words.next().expect("truncated dfui path");
        let num = f32::from_le_bytes;
        let count = |w| u32::from_le_bytes(w) as usize;
        let view = Rect::new(num(next()), num(next()), num(next()), num(next()));
        let lengths: Vec<usize> = (0..count(next())).map(|_| count(next())).collect();
        let contours = lengths
            .iter()
            .map(|&n| {
                (0..n)
                    .map(|_| [(); 3].map(|_| [num(next()), num(next())]))
                    .collect()
            })
            .collect();
        Shape::new(Path { contours }, view)
    }

    /// `path` with `view` as the box that `PaintCx::shape` maps onto its
    /// target rect, like an SVG `viewBox`.
    pub fn new(path: Path, view: Rect) -> Shape {
        let mut h = DefaultHasher::new();
        let bits = [view.x, view.y, view.w, view.h];
        let quads = path.contours.iter().flatten().flatten().flatten();
        for v in bits.iter().chain(quads) {
            v.to_bits().hash(&mut h);
        }
        let key = h.finish();
        Shape(Rc::new(ShapeData { key, path, view }))
    }

    pub fn view(&self) -> Rect {
        self.0.view
    }

    pub fn path(&self) -> &Path {
        &self.0.path
    }
}

/// A straight segment as a quadratic.
pub(crate) fn line(a: Point, b: Point) -> Quad {
    [a, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], b]
}
