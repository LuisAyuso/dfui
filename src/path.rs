//! Filled outlines drawn the same way as glyphs.

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

/// A straight segment as a quadratic.
pub(crate) fn line(a: Point, b: Point) -> Quad {
    [a, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], b]
}
