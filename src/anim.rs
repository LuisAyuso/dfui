//! Time-based transitions for values drawn in `paint`.

use std::time::{Duration, Instant};

use crate::path::{Path, Point, Quad};
use crate::render::Color;

/// Values `Animated` can blend between.
pub trait Lerp: Copy {
    fn lerp(self, to: Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(self, to: f32, t: f32) -> f32 {
        // Exact at both ends, so a settled transition lands on its target.
        self * (1.0 - t) + to * t
    }
}

impl Lerp for Color {
    fn lerp(self, to: Color, t: f32) -> Color {
        Color {
            r: self.r.lerp(to.r, t),
            g: self.g.lerp(to.g, t),
            b: self.b.lerp(to.b, t),
            a: self.a.lerp(to.a, t),
        }
    }
}

/// Easing curves: map linear progress in 0..=1 to eased progress.
pub mod ease {
    pub fn linear(t: f32) -> f32 {
        t
    }

    /// Fast start, gentle stop; the usual choice for UI feedback.
    pub fn out_cubic(t: f32) -> f32 {
        1.0 - (1.0 - t).powi(3)
    }

    pub fn in_out_cubic(t: f32) -> f32 {
        if t < 0.5 {
            4.0 * t * t * t
        } else {
            1.0 - (2.0 - 2.0 * t).powi(3) / 2.0
        }
    }
}

/// A value easing toward a target. Keep it in widget state, call `to` with
/// the target each paint, read `get`, and `request_frame` while `running`.
#[derive(Clone, Copy)]
pub struct Animated<T> {
    from: T,
    to: T,
    start: Instant,
    duration: Duration,
    ease: fn(f32) -> f32,
    /// No target yet; the first one is taken without animating.
    fresh: bool,
}

impl<T: Lerp + Default> Default for Animated<T> {
    fn default() -> Self {
        Animated {
            from: T::default(),
            to: T::default(),
            start: Instant::now(),
            duration: Duration::ZERO,
            ease: ease::linear,
            fresh: true,
        }
    }
}

impl<T: Lerp + PartialEq> Animated<T> {
    /// Eases from the current value to `to` over `duration`, starting at
    /// `now`. A new target mid-flight starts from where the value is.
    pub fn to(&mut self, to: T, now: Instant, duration: Duration, ease: fn(f32) -> f32) {
        if self.fresh {
            (self.from, self.to, self.fresh) = (to, to, false);
        } else if to != self.to {
            *self = Animated {
                from: self.get(now),
                to,
                start: now,
                duration,
                ease,
                fresh: false,
            };
        }
    }

    pub fn get(&self, now: Instant) -> T {
        self.from.lerp(self.to, (self.ease)(self.progress(now)))
    }

    pub fn running(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }

    fn progress(&self, now: Instant) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let t = now.saturating_duration_since(self.start).as_secs_f32();
        (t / self.duration.as_secs_f32()).min(1.0)
    }
}

/// Morph between two `Path`s, e.g. two glyphs. Contours pair up in order;
/// one without a partner shrinks to its centre. Within a pair, the contour
/// with fewer segments has them cut (which keeps its shape) into as many
/// pieces as needed to match the other, the longest pieces first, and the
/// pairing starts at the offset with the least travel. Each segment's points
/// then travel to its partner's. Both paths must be wound the same way.
pub struct Morph {
    from: Path,
    to: Path,
}

impl Morph {
    pub fn new(from: &Path, to: &Path) -> Self {
        let n = from.contours.len().max(to.contours.len());
        let (mut a, mut b) = (Path::default(), Path::default());
        for i in 0..n {
            let (x, y) = match (from.contours.get(i), to.contours.get(i)) {
                (Some(x), Some(y)) if x.len() <= y.len() => (split(x, y), y.clone()),
                (Some(x), Some(y)) => (x.clone(), split(y, x)),
                (Some(x), None) => (x.clone(), collapse(x)),
                (None, Some(y)) => (collapse(y), y.clone()),
                (None, None) => unreachable!(),
            };
            a.contours.push(x);
            b.contours.push(y);
        }
        Morph { from: a, to: b }
    }

    /// The outline at progress `t` (0 = `from`, 1 = `to`).
    pub fn at(&self, t: f32) -> Path {
        let contours = self.from.contours.iter().zip(&self.to.contours);
        Path {
            contours: contours
                .map(|(a, b)| a.iter().zip(b).map(|(p, q)| lerp_quad(p, q, t)).collect())
                .collect(),
        }
    }
}

fn lerp_quad(a: &Quad, b: &Quad, t: f32) -> Quad {
    [0, 1, 2].map(|i| [a[i][0].lerp(b[i][0], t), a[i][1].lerp(b[i][1], t)])
}

/// Every point of `c` at its centre, so a contour can grow from or shrink to nothing.
fn collapse(c: &[Quad]) -> Vec<Quad> {
    let n = (3 * c.len()) as f32;
    let sum = c
        .iter()
        .flatten()
        .fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
    let mid = [sum[0] / n, sum[1] / n];
    vec![[mid; 3]; c.len()]
}

/// The piece of `q` between `t0` and `t1`, via the quadratic's blossom.
fn piece(q: &Quad, t0: f32, t1: f32) -> Quad {
    let blossom = |a: f32, b: f32| {
        let w = [(1.0 - a) * (1.0 - b), (1.0 - a) * b + a * (1.0 - b), a * b];
        [0, 1].map(|k| w[0] * q[0][k] + w[1] * q[1][k] + w[2] * q[2][k])
    };
    [blossom(t0, t0), blossom(t0, t1), blossom(t1, t1)]
}

/// `few` cut into `many.len()` segments, rotated to pair index-for-index
/// with `many`.
fn split(few: &[Quad], many: &[Quad]) -> Vec<Quad> {
    let dist = |a: Point, b: Point| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    // Rough arc length: between the chord and the control polygon.
    let len = |q: &Quad| (dist(q[0], q[1]) + dist(q[1], q[2]) + dist(q[0], q[2])) / 2.0;
    let mut cuts = vec![1usize; few.len()];
    for _ in few.len()..many.len() {
        let longest = (0..few.len())
            .max_by(|&a, &b| {
                (len(&few[a]) / cuts[a] as f32).total_cmp(&(len(&few[b]) / cuts[b] as f32))
            })
            .unwrap();
        cuts[longest] += 1;
    }
    let pieces: Vec<Quad> = few
        .iter()
        .zip(&cuts)
        .flat_map(|(q, &k)| {
            (0..k).map(move |i| piece(q, i as f32 / k as f32, (i + 1) as f32 / k as f32))
        })
        .collect();
    let m = many.len();
    let travel = |r: usize| -> f32 {
        let d = |a: &Quad, b: &Quad| (0..3).map(|i| dist(a[i], b[i]).powi(2)).sum::<f32>();
        (0..m).map(|i| d(&pieces[i], &many[(i + r) % m])).sum()
    };
    let r = (0..m)
        .min_by(|&a, &b| travel(a).total_cmp(&travel(b)))
        .unwrap();
    let mut out = pieces.clone();
    for (i, p) in pieces.into_iter().enumerate() {
        out[(i + r) % m] = p;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eases_retargets_and_settles() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let d = Duration::from_millis(100);
        let mut a = Animated::<f32>::default();
        a.to(1.0, t0, d, ease::linear);
        assert_eq!(
            (a.get(t0), a.running(t0)),
            (1.0, false),
            "first target snaps"
        );

        a.to(0.0, t0, d, ease::linear);
        assert_eq!(a.get(ms(50)), 0.5);
        assert!(a.running(ms(50)));
        // Heading back from halfway continues from 0.5 instead of jumping.
        a.to(1.0, ms(50), d, ease::linear);
        assert_eq!(a.get(ms(50)), 0.5);
        assert_eq!(a.get(ms(100)), 0.75);
        assert_eq!((a.get(ms(150)), a.running(ms(150))), (1.0, false));
        // Same target again doesn't restart.
        a.to(1.0, ms(200), d, ease::linear);
        assert!(!a.running(ms(200)));
    }

    #[test]
    fn curves_span_zero_to_one() {
        for f in [ease::linear, ease::out_cubic, ease::in_out_cubic] {
            assert_eq!((f(0.0), f(1.0)), (0.0, 1.0));
            assert!(f(0.5) > 0.0 && f(0.5) < 1.0);
        }
    }

    fn rect() -> Path {
        Path::polygon(&[[-1.0, -0.6], [1.0, -0.6], [1.0, 0.6], [-1.0, 0.6]])
    }

    /// Pointy-top hexagon: its top and bottom tips sit where the rect's
    /// long edges get cut.
    fn hexagon() -> Path {
        let a = |k: usize| std::f32::consts::FRAC_PI_3 * k as f32 - std::f32::consts::FRAC_PI_2;
        Path::polygon(&(0..6).map(|k| [a(k).cos(), a(k).sin()]).collect::<Vec<_>>())
    }

    #[test]
    fn rect_to_hexagon_cuts_the_long_edges() {
        for (m, rect_end) in [
            (Morph::new(&rect(), &hexagon()), 0.0),
            (Morph::new(&hexagon(), &rect()), 1.0),
        ] {
            assert_eq!(m.at(1.0 - rect_end), hexagon());
            let r = m.at(rect_end).contours.remove(0);
            assert_eq!(r.len(), 6);
            // Still one closed contour: cutting a segment keeps the shape.
            for (a, b) in r.iter().zip(r.iter().cycle().skip(1)) {
                assert_eq!(a[2], b[0]);
            }
            // The only new points are the midpoints of the long edges.
            let mut ends: Vec<_> = r.iter().map(|q| q[0]).collect();
            ends.retain(|p| p[0].abs() != 1.0);
            assert_eq!(ends.len(), 2, "{r:?}");
            assert!(ends.contains(&[0.0, 0.6]) && ends.contains(&[0.0, -0.6]));
        }
    }

    #[test]
    fn cuts_as_many_pieces_as_needed() {
        let square = Path::polygon(&[[0.0, 0.0], [3.0, 0.0], [3.0, 3.0], [0.0, 3.0]]);
        let a = |k: usize| std::f32::consts::TAU / 12.0 * k as f32;
        let dodecagon = Path::polygon(
            &(0..12)
                .map(|k| [a(k).cos(), a(k).sin()])
                .collect::<Vec<_>>(),
        );
        let start = Morph::new(&square, &dodecagon).at(0.0).contours.remove(0);
        assert_eq!(start.len(), 12);
        // Three equal pieces per edge: every piece is 1 long.
        for q in &start {
            let d = ((q[2][0] - q[0][0]).powi(2) + (q[2][1] - q[0][1]).powi(2)).sqrt();
            assert!((d - 1.0).abs() < 1e-5, "{q:?}");
        }
    }

    #[test]
    fn unpaired_contours_shrink_to_their_centre() {
        let ring = Path {
            contours: vec![rect().contours[0].clone(), hexagon().contours[0].clone()],
        };
        let m = Morph::new(&ring, &rect());
        let end = m.at(1.0);
        assert_eq!(end.contours[0], rect().contours[0]);
        let hole = &end.contours[1];
        assert!(hole.iter().flatten().all(|p| p == &hole[0][0]));
    }
}
