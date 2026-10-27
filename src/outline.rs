//! Glyph outlines as monotonic quadratic Béziers plus per-glyph band lists; the
//! fragment shader evaluates glyph coverage directly from these curves.

use ttf_parser::{Face, GlyphId, OutlineBuilder};

type P = [f32; 2];
type Quad = [P; 3];

/// Where a glyph's data lives in the shared buffers. Coordinates are em, y up.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OutlineGlyph {
    pub header: u32,
    pub bands_h: u32,
    pub bands_v: u32,
    pub bbox: [f32; 4],
}

/// GPU-bound storage: each curve is two vec4s `(p0, p1), (p2, 0, 0)`; `bands` holds
/// per-glyph `[start, count]` headers (horizontal then vertical) and curve indices.
#[derive(Default)]
pub(crate) struct Outlines {
    pub curves: Vec<[f32; 4]>,
    pub bands: Vec<u32>,
    pub dirty: bool,
}

impl Outlines {
    pub fn add(&mut self, face: &Face, id: GlyphId) -> Option<OutlineGlyph> {
        let mut b = Builder {
            em: face.units_per_em() as f32,
            start: [0.0; 2],
            cur: [0.0; 2],
            quads: Vec::new(),
        };
        face.outline_glyph(id, &mut b)?;
        b.close();
        let mut quads = Vec::new();
        for q in b.quads {
            split_monotonic(q, &mut quads);
        }
        if quads.is_empty() {
            return None;
        }

        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for q in &quads {
            for p in q {
                for a in 0..2 {
                    lo[a] = lo[a].min(p[a]);
                    hi[a] = hi[a].max(p[a]);
                }
            }
        }

        let first = (self.curves.len() / 2) as u32;
        for q in &quads {
            self.curves.push([q[0][0], q[0][1], q[1][0], q[1][1]]);
            self.curves.push([q[2][0], q[2][1], 0.0, 0.0]);
        }

        let n = ((quads.len() as f32 / 2.0).ceil() as u32).clamp(1, 32);
        let header = self.bands.len() as u32;
        self.bands.resize(self.bands.len() + 4 * n as usize, 0);
        // Horizontal bands slice y and are scanned by +x rays; vertical bands the reverse.
        for (axis, base) in [(1usize, 0u32), (0, n)] {
            let size = (hi[axis] - lo[axis]) / n as f32;
            for i in 0..n {
                let (b0, b1) = (lo[axis] + i as f32 * size, lo[axis] + (i + 1) as f32 * size);
                let start = self.bands.len() as u32;
                let mut members: Vec<usize> = (0..quads.len())
                    .filter(|&k| {
                        let q = &quads[k];
                        q[0][axis].min(q[2][axis]) <= b1 && q[0][axis].max(q[2][axis]) >= b0
                    })
                    .collect();
                // Furthest reach along the ray first, so the shader can stop at the
                // first curve lying entirely behind the pixel.
                let ray = 1 - axis;
                members.sort_by(|&a, &b| {
                    let reach = |k: usize| quads[k][0][ray].max(quads[k][2][ray]);
                    reach(b).total_cmp(&reach(a))
                });
                self.bands.extend(members.iter().map(|&k| first + k as u32));
                let h = (header + 2 * (base + i)) as usize;
                self.bands[h] = start;
                self.bands[h + 1] = self.bands.len() as u32 - start;
            }
        }
        self.dirty = true;
        Some(OutlineGlyph {
            header,
            bands_h: n,
            bands_v: n,
            bbox: [lo[0], lo[1], hi[0], hi[1]],
        })
    }
}

struct Builder {
    em: f32,
    start: P,
    cur: P,
    quads: Vec<Quad>,
}

impl Builder {
    fn p(&self, x: f32, y: f32) -> P {
        [x / self.em, y / self.em]
    }
}

impl OutlineBuilder for Builder {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        self.start = self.p(x, y);
        self.cur = self.start;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.p(x, y);
        let mid = [(self.cur[0] + p[0]) / 2.0, (self.cur[1] + p[1]) / 2.0];
        self.quads.push([self.cur, mid, p]);
        self.cur = p;
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (c, p) = (self.p(x1, y1), self.p(x, y));
        self.quads.push([self.cur, c, p]);
        self.cur = p;
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        // ponytail: fixed 4-way split per cubic (CFF fonts); adaptive error bound if accuracy matters.
        let cubic = [self.cur, self.p(x1, y1), self.p(x2, y2), self.p(x, y)];
        let at = |t: f32| -> P {
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            [0, 1].map(|a| (0..4).map(|i| w[i] * cubic[i][a]).sum())
        };
        let d = |t: f32| -> P {
            let u = 1.0 - t;
            let w = [
                -3.0 * u * u,
                3.0 * u * u - 6.0 * u * t,
                6.0 * u * t - 3.0 * t * t,
                3.0 * t * t,
            ];
            [0, 1].map(|a| (0..4).map(|i| w[i] * cubic[i][a]).sum())
        };
        for i in 0..4 {
            let (t0, t1) = (i as f32 / 4.0, (i + 1) as f32 / 4.0);
            let (p0, p2) = (at(t0), at(t1));
            let (d0, d2) = (d(t0), d(t1));
            let h = (t1 - t0) / 2.0;
            // Average of the two tangent-implied control points.
            let c = [0, 1].map(|a| (p0[a] + d0[a] * h + p2[a] - d2[a] * h) / 2.0);
            self.quads.push([p0, c, p2]);
        }
        self.cur = cubic[3];
    }

    fn close(&mut self) {
        if self.cur != self.start {
            self.line_to(self.start[0] * self.em, self.start[1] * self.em);
        }
    }
}

fn lerp(a: P, b: P, t: f32) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// Splits at x/y extrema so each piece crosses any axis-aligned line at most once,
/// which lets the shader count crossings from endpoints alone.
fn split_monotonic(q: Quad, out: &mut Vec<Quad>) {
    let mut ts: Vec<f32> = (0..2)
        .filter_map(|a| {
            let d = q[0][a] - 2.0 * q[1][a] + q[2][a];
            let t = (q[0][a] - q[1][a]) / d;
            (d.abs() > 1e-9 && t > 1e-4 && t < 1.0 - 1e-4).then_some(t)
        })
        .collect();
    ts.sort_by(f32::total_cmp);
    let mut rest = q;
    let mut done = 0.0;
    for t in ts {
        let t = (t - done) / (1.0 - done);
        let (a, b) = (lerp(rest[0], rest[1], t), lerp(rest[1], rest[2], t));
        let m = lerp(a, b, t);
        push_clamped([rest[0], a, m], out);
        rest = [m, b, rest[2]];
        done += (1.0 - done) * t;
    }
    push_clamped(rest, out);
}

fn push_clamped(mut q: Quad, out: &mut Vec<Quad>) {
    if q[0] == q[2] {
        return;
    }
    // Float error after splitting can leave the control point just outside the
    // endpoint range, breaking monotonicity.
    for a in 0..2 {
        q[1][a] = q[1][a].clamp(q[0][a].min(q[2][a]), q[0][a].max(q[2][a]));
    }
    out.push(q);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CPU mirror of the shader's horizontal-ray winding count.
    fn winding(o: &Outlines, g: &OutlineGlyph, p: P) -> i32 {
        let band = (((p[1] - g.bbox[1]) / (g.bbox[3] - g.bbox[1]) * g.bands_h as f32) as u32)
            .min(g.bands_h - 1);
        let h = (g.header + 2 * band) as usize;
        let (start, count) = (o.bands[h] as usize, o.bands[h + 1] as usize);
        let mut w = 0;
        for &ci in &o.bands[start..start + count] {
            let (a, b) = (o.curves[2 * ci as usize], o.curves[2 * ci as usize + 1]);
            let (p0, p1, p2) = ([a[0], a[1]], [a[2], a[3]], [b[0], b[1]]);
            if (p0[1] <= p[1]) == (p2[1] <= p[1]) {
                continue;
            }
            let (qa, qb, qc) = (
                p0[1] - 2.0 * p1[1] + p2[1],
                2.0 * (p1[1] - p0[1]),
                p0[1] - p[1],
            );
            let t = if qa.abs() < 1e-7 {
                -qc / qb
            } else {
                let d = (qb * qb - 4.0 * qa * qc).max(0.0).sqrt();
                let t1 = (-qb + d) / (2.0 * qa);
                if (-1e-4..=1.0 + 1e-4).contains(&t1) {
                    t1
                } else {
                    (-qb - d) / (2.0 * qa)
                }
            };
            let x = lerp(lerp(p0, p1, t), lerp(p1, p2, t), t)[0];
            if x > p[0] {
                w += if p2[1] > p0[1] { 1 } else { -1 };
            }
        }
        w
    }

    #[test]
    fn o_has_a_hole() {
        let face = Face::parse(include_bytes!("../assets/DejaVuSans.ttf"), 0).unwrap();
        let mut o = Outlines::default();
        let g = o.add(&face, face.glyph_index('O').unwrap()).unwrap();
        let [x0, y0, x1, y1] = g.bbox;
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        assert_eq!(winding(&o, &g, [cx, cy]), 0, "center of O is the hole");
        assert_ne!(winding(&o, &g, [x0 + 0.02, cy]), 0, "left stroke is inside");
        assert_eq!(
            winding(&o, &g, [x0 - 0.1, cy]),
            0,
            "left of glyph is outside"
        );
        assert_ne!(winding(&o, &g, [cx, y1 - 0.02]), 0, "top stroke is inside");
    }
}
