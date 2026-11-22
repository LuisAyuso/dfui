//! Filled outlines drawn the same way as glyphs.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use crate::render::Rect;

/// `[x, y]` in logical px, y down.
pub type Point = [f32; 2];
/// Quadratic Bézier segment: start, control, end.
pub type Quad = [Point; 3];

/// Closed contours of quadratic segments in logical px (y down), filled by
/// nonzero winding like a glyph. Get one from `PaintCx::glyph_path` or
/// [`Path::polygon`]; draw it with `PaintCx::path`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    /// Each contour's segments in order, each starting where the last ended.
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

    /// The path moved by `dx`, `dy` logical px.
    pub fn offset(mut self, dx: f32, dy: f32) -> Path {
        for q in self.contours.iter_mut().flatten() {
            for p in q {
                *p = [p[0] + dx, p[1] + dy];
            }
        }
        self
    }
}

/// How [`stroke`] draws a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    /// Logical px across.
    pub width: f32,
    /// Half-disk caps at the first and last point; ends are square without.
    pub caps: [bool; 2],
    /// Fill the outside of each bend; without, bends show a notch.
    pub bevels: bool,
}

impl Stroke {
    /// Square ends, bevelled bends.
    pub fn new(width: f32) -> Stroke {
        Stroke {
            width,
            caps: [false; 2],
            bevels: true,
        }
    }

    /// Round ends, bevelled bends.
    pub fn round(width: f32) -> Stroke {
        Stroke {
            caps: [true; 2],
            ..Stroke::new(width)
        }
    }
}

/// Segments in a round cap's half circle.
pub const CAP_STEPS: usize = 8;

/// The line through `points` as polygons: a rectangle per segment, a
/// triangle on the outside of each bend and half-disk caps, all wound the
/// same way so overlaps fill as one. Polygons rather than a [`Path`] for
/// callers that cut them first (a map tile); [`Path::stroke`] draws them.
pub fn stroke(points: &[Point], s: Stroke) -> Vec<Vec<Point>> {
    let h = s.width / 2.0;
    let normal = |a: Point, b: Point| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = dx.hypot(dy);
        (len > 0.0).then(|| [-dy / len * h, dx / len * h])
    };
    let add = |p: Point, n: Point, k: f32| [p[0] + n[0] * k, p[1] + n[1] * k];
    let mut out = Vec::new();
    let n = points.len();
    if n < 2 {
        return out;
    }
    let ends = [(points[0], points[1]), (points[n - 1], points[n - 2])];
    for ((a, b), _) in ends.into_iter().zip(s.caps).filter(|(_, c)| *c) {
        let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
        let len = dx.hypot(dy);
        if len == 0.0 {
            continue;
        }
        let (ux, uy) = (dx / len * h, dy / len * h);
        let cap = (0..=CAP_STEPS).map(|i| {
            let t = std::f32::consts::PI * (i as f32 / CAP_STEPS as f32 - 0.5);
            let (sin, cos) = t.sin_cos();
            [a[0] + ux * cos - uy * sin, a[1] + uy * cos + ux * sin]
        });
        out.extend(clockwise(cap.collect()));
    }
    let mut last: Option<Point> = None;
    for w in points.windows(2) {
        let Some(m) = normal(w[0], w[1]) else {
            continue;
        };
        let quad = vec![
            add(w[0], m, 1.0),
            add(w[1], m, 1.0),
            add(w[1], m, -1.0),
            add(w[0], m, -1.0),
        ];
        out.extend(clockwise(quad));
        // Only the outside of a bend has a notch; the inside is covered.
        if let Some(l) = last.filter(|_| s.bevels) {
            let turn = l[0] * m[1] - l[1] * m[0];
            if turn != 0.0 {
                let k = -turn.signum();
                out.extend(clockwise(vec![w[0], add(w[0], l, k), add(w[0], m, k)]));
            }
        }
        last = Some(m);
    }
    out
}

/// `ring` wound clockwise on screen, or `None` if it encloses nothing.
fn clockwise(mut ring: Vec<Point>) -> Option<Vec<Point>> {
    let n = ring.len();
    let area: f32 = (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if n < 3 || area == 0.0 {
        return None;
    }
    if area < 0.0 {
        ring.reverse();
    }
    Some(ring)
}

impl Path {
    /// The line through `points`, ready for `PaintCx::path`; see [`stroke`].
    pub fn stroke(points: &[Point], s: Stroke) -> Path {
        Path {
            contours: stroke(points, s)
                .iter()
                .flat_map(|r| Path::polygon(r).contours)
                .collect(),
        }
    }
}

/// A [`Path`] stored on the GPU once, like a glyph, and drawn into any rect
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
    /// A shape written by the `svg2path` tool, usually `include_bytes!`d;
    /// `None` if `bytes` aren't one or are truncated.
    pub fn from_bytes(bytes: &[u8]) -> Option<Shape> {
        let body = bytes.strip_prefix(b"DFP1")?;
        let mut words = body
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&w| u32::from_le_bytes(w));
        // Counts are u32 words too; reading them as f32 bits keeps one closure.
        let mut f = || words.next().map(f32::from_bits);
        let view = Rect::new(f()?, f()?, f()?, f()?);
        let lengths: Vec<u32> = (0..f()?.to_bits())
            .map(|_| f().map(f32::to_bits))
            .collect::<Option<_>>()?;
        let mut contours = Vec::new();
        for n in lengths {
            let quads = (0..n)
                .map(|_| Some([[f()?, f()?], [f()?, f()?], [f()?, f()?]]))
                .collect::<Option<_>>()?;
            contours.push(quads);
        }
        Some(Shape::new(Path { contours }, view))
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

    /// The box mapped onto the target rect when drawn.
    pub fn view(&self) -> Rect {
        self.0.view
    }

    /// The outline, in the coordinates of `view`.
    pub fn path(&self) -> &Path {
        &self.0.path
    }
}

/// A straight segment as a quadratic.
pub(crate) fn line(a: Point, b: Point) -> Quad {
    [a, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], b]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(r: &[Point]) -> f32 {
        let n = r.len();
        (0..n)
            .map(|i| r[i][0] * r[(i + 1) % n][1] - r[(i + 1) % n][0] * r[i][1])
            .sum::<f32>()
            / 2.0
    }

    #[test]
    fn straight_line_is_one_rectangle() {
        let s = stroke(&[[0.0, 0.0], [10.0, 0.0]], Stroke::new(2.0));
        assert_eq!(s.len(), 1);
        assert_eq!(area(&s[0]), 20.0);
    }

    #[test]
    fn caps_and_bevels_add_pieces_wound_alike() {
        let bend = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        let square = stroke(&bend, Stroke::new(2.0));
        assert_eq!(square.len(), 3, "two segments and the bend's bevel");
        let round = stroke(&bend, Stroke::round(2.0));
        assert_eq!(round.len(), 5, "plus a cap at each end");
        assert!(round.iter().all(|r| area(r) > 0.0));
        let notched = Stroke {
            bevels: false,
            ..Stroke::new(2.0)
        };
        assert_eq!(stroke(&bend, notched).len(), 2);
    }

    #[test]
    fn repeated_points_and_single_points_draw_nothing_extra() {
        assert!(stroke(&[[1.0, 1.0]], Stroke::round(2.0)).is_empty());
        let s = stroke(&[[0.0, 0.0], [0.0, 0.0], [5.0, 0.0]], Stroke::new(2.0));
        assert_eq!(s.len(), 1);
        assert_eq!(
            Path::stroke(&[[0.0, 0.0], [5.0, 0.0]], Stroke::new(1.0))
                .contours
                .len(),
            1
        );
    }
}
