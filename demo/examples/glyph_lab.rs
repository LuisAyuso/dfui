//! Compares ways to morph and hand-draw glyphs from approximated segments.
//! `cargo run -p dfui-demo --example glyph_lab`, or add
//! `--shot out-prefix AB CD` for one film-strip PNG per glyph pair.

// ponytail: a shelved experiment, linted loosely; clean these up when it moves into `anim::Morph`.
#![allow(
    clippy::needless_range_loop,
    clippy::type_complexity,
    clippy::filter_map_bool_then
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use std::rc::Rc;

use dfui::anim::Morph;
use dfui::path::{Point, Quad};
use dfui::taffy::{AlignItems, Dimension, Size, Style};
use dfui::widgets::{
    Styled, button, choice_chip, column, row, scroll, slider, stateful_with, text, text_input,
};
use dfui::{App, Bench, Color, Element, Font, Instant, PaintCx, Path, Widget};

type Run = Vec<Quad>;
/// A centreline sample: x, y, half width.
type Ink = [f32; 3];

const COLORS: [u32; 8] = [
    0xe5484d, 0x3b6fe0, 0x30a46c, 0xf5a524, 0x8e4ec6, 0x12a5b0, 0xd6409f, 0x7c5e3c,
];
const PAIRS: [&str; 12] = [
    "DO", "OC", "EF", "AH", "RB", "S8", "38", "ae", "nh", "il", "gy", "&B",
];
const CORNER: f32 = 30.0 * PI / 180.0;

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: Point, b: Point) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn dist(a: Point, b: Point) -> f32 {
    dot(sub(a, b), sub(a, b)).sqrt()
}
fn unit(a: Point) -> Point {
    let d = dot(a, a).sqrt().max(1e-6);
    [a[0] / d, a[1] / d]
}
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn signed(a: Point, b: Point) -> f32 {
    (a[0] * b[1] - a[1] * b[0]).atan2(dot(a, b))
}

fn at(q: &Quad, t: f32) -> Point {
    let w = [(1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t];
    [0, 1].map(|k| w[0] * q[0][k] + w[1] * q[1][k] + w[2] * q[2][k])
}
fn tangent(q: &Quad, t: f32) -> Point {
    let d = [0, 1].map(|k| 2.0 * (1.0 - t) * (q[1][k] - q[0][k]) + 2.0 * t * (q[2][k] - q[1][k]));
    unit(if dot(d, d) < 1e-12 {
        sub(q[2], q[0])
    } else {
        d
    })
}
fn piece(q: &Quad, t0: f32, t1: f32) -> Quad {
    let blossom = |a: f32, b: f32| {
        let w = [(1.0 - a) * (1.0 - b), (1.0 - a) * b + a * (1.0 - b), a * b];
        [0, 1].map(|k| w[0] * q[0][k] + w[1] * q[1][k] + w[2] * q[2][k])
    };
    [blossom(t0, t0), blossom(t0, t1), blossom(t1, t1)]
}
fn line(a: Point, b: Point) -> Quad {
    [a, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], b]
}

/// A run of quads addressed by normalised arc length.
struct Arc<'a> {
    quads: &'a [Quad],
    marks: Vec<[f32; 13]>,
}

impl<'a> Arc<'a> {
    fn new(quads: &'a [Quad]) -> Self {
        let mut total = 0.0;
        let mut marks: Vec<[f32; 13]> = quads
            .iter()
            .map(|q| {
                let mut m = [total; 13];
                for k in 1..13 {
                    let (a, b) = ((k - 1) as f32 / 12.0, k as f32 / 12.0);
                    total += dist(at(q, a), at(q, b));
                    m[k] = total;
                }
                m
            })
            .collect();
        for v in marks.iter_mut().flatten() {
            *v /= total.max(1e-6);
        }
        Arc { quads, marks }
    }

    fn index(&self, u: f32) -> usize {
        self.marks.partition_point(|m| m[0] <= u).saturating_sub(1)
    }

    fn param(&self, i: usize, u: f32) -> f32 {
        let m = &self.marks[i];
        let k = m.partition_point(|&x| x <= u).clamp(1, 12);
        let d = m[k] - m[k - 1];
        let f = if d > 0.0 { (u - m[k - 1]) / d } else { 0.0 };
        (k as f32 - 1.0 + f.clamp(0.0, 1.0)) / 12.0
    }

    fn point(&self, u: f32) -> (Point, Point) {
        let i = self.index(u);
        let t = self.param(i, u);
        (at(&self.quads[i], t), tangent(&self.quads[i], t))
    }

    fn slice(&self, u0: f32, u1: f32) -> Quad {
        let i = self.index((u0 + u1) / 2.0);
        piece(&self.quads[i], self.param(i, u0), self.param(i, u1))
    }

    fn breaks(&self) -> impl Iterator<Item = f32> + '_ {
        self.marks.iter().map(|m| m[0]).chain([1.0])
    }
}

fn run_len(r: &[Quad]) -> f32 {
    r.iter()
        .map(|q| {
            (0..8)
                .map(|k| dist(at(q, k as f32 / 8.0), at(q, (k + 1) as f32 / 8.0)))
                .sum::<f32>()
        })
        .sum()
}

/// Cuts a closed contour at corners, inflections and every ~100° of turning.
fn decompose(c: &[Quad]) -> Vec<Run> {
    let n = c.len();
    let joint = |i: usize| signed(tangent(&c[(i + n - 1) % n], 1.0), tangent(&c[i], 0.0));
    let turn = |i: usize| signed(tangent(&c[i], 0.0), tangent(&c[i], 1.0));
    let start = (0..n)
        .find(|&i| joint(i).abs() > CORNER)
        .unwrap_or_else(|| {
            (0..n)
                .min_by(|&a, &b| c[a][0][1].total_cmp(&c[b][0][1]))
                .unwrap()
        });
    let (mut runs, mut cur, mut acc) = (Vec::new(), Vec::new(), 0.0f32);
    for k in 0..n {
        let i = (start + k) % n;
        let (j, t) = (joint(i), turn(i));
        let inflects = acc.abs() > 0.35 && t * acc < 0.0 && t.abs() > 0.05;
        if !cur.is_empty() {
            if j.abs() > CORNER || (acc + j + t).abs() > 100f32.to_radians() || inflects {
                runs.push(std::mem::take(&mut cur));
                acc = 0.0;
            } else {
                acc += j;
            }
        }
        cur.push(c[i]);
        acc += t;
    }
    runs.push(cur);
    runs
}

/// One quad through a run's ends and arc midpoint, and its worst distance.
fn fit(r: &[Quad]) -> (Quad, f32) {
    let arc = Arc::new(r);
    let (a, b, m) = (r[0][0], r[r.len() - 1][2], arc.point(0.5).0);
    let f = [
        a,
        [
            2.0 * m[0] - (a[0] + b[0]) / 2.0,
            2.0 * m[1] - (a[1] + b[1]) / 2.0,
        ],
        b,
    ];
    let err = (0..=24)
        .map(|k| {
            let p = arc.point(k as f32 / 24.0).0;
            (0..=24)
                .map(|j| dist(p, at(&f, j as f32 / 24.0)))
                .fold(f32::MAX, f32::min)
        })
        .fold(0.0, f32::max);
    (f, err)
}

/// Both runs cut at the union of their breakpoints: exact at either end of
/// the morph, and every corner survives.
fn align(a: &[Quad], b: &[Quad]) -> (Run, Run) {
    let (x, y) = (Arc::new(a), Arc::new(b));
    let mut us: Vec<f32> = x.breaks().chain(y.breaks()).collect();
    us.sort_by(f32::total_cmp);
    us.dedup_by(|p, q| (*p - *q).abs() < 1e-4);
    us.windows(2)
        .map(|w| (x.slice(w[0], w[1]), y.slice(w[0], w[1])))
        .unzip()
}

fn run_cost(a: &[Run], b: &[Run]) -> f32 {
    let (a0, a1) = (a[0][0][0], a[a.len() - 1].last().unwrap()[2]);
    let (b0, b1) = (b[0][0][0], b[b.len() - 1].last().unwrap()[2]);
    let la: f32 = a.iter().map(|r| run_len(r)).sum();
    let lb: f32 = b.iter().map(|r| run_len(r)).sum();
    let bend = |r: &[Run]| -> f32 {
        r.iter()
            .flatten()
            .map(|q| signed(tangent(q, 0.0), tangent(q, 1.0)))
            .sum()
    };
    let d = |p: Point, q: Point| dot(sub(p, q), sub(p, q));
    let chord = dot(unit(sub(a1, a0)), unit(sub(b1, b0)));
    d(a0, b0)
        + d(a1, b1)
        + (la - lb).powi(2)
        + 0.04 * (1.0 - chord)
        + 0.01 * (bend(a) - bend(b)).abs()
        + 0.01 * (a.len() + b.len() - 2) as f32
}

/// The cyclic alignment of `a`'s runs with `b`'s: Sederberg-style dynamic
/// programming over segments, with merges. Index ranges into each side.
#[allow(clippy::type_complexity)]
fn match_runs(a: &[Run], b: &[Run]) -> (usize, Vec<(usize, usize, usize, usize)>) {
    let (n, m) = (a.len(), b.len());
    let reach = n.max(m) / n.min(m) + 2;
    let mut best = (f32::MAX, 0, Vec::new());
    for r in 0..m {
        let rb: Vec<Run> = (0..m).map(|k| b[(r + k) % m].clone()).collect();
        let mut dp = vec![vec![(f32::MAX, 0usize, 0usize); m + 1]; n + 1];
        dp[0][0].0 = 0.0;
        for i in 0..n {
            for j in 0..m {
                let base = dp[i][j].0;
                if base == f32::MAX {
                    continue;
                }
                for di in 1..=reach.min(n - i) {
                    for dj in 1..=reach.min(m - j) {
                        if di > 1 && dj > 1 {
                            continue;
                        }
                        let c = base + run_cost(&a[i..i + di], &rb[j..j + dj]);
                        if c < dp[i + di][j + dj].0 {
                            dp[i + di][j + dj] = (c, di, dj);
                        }
                    }
                }
            }
        }
        if dp[n][m].0 < best.0 {
            let (mut path, mut i, mut j) = (Vec::new(), n, m);
            while i > 0 {
                let (_, di, dj) = dp[i][j];
                path.push((i - di, i, j - dj, j));
                i -= di;
                j -= dj;
            }
            path.reverse();
            best = (dp[n][m].0, r, path);
        }
    }
    (best.1, best.2)
}

/// Same idea with no segments: dense samples, position plus tangent.
fn point_dp(a: &[Quad], b: &[Quad]) -> (Run, Run) {
    const N: usize = 96;
    let sample = |c: &[Quad]| -> Vec<(Point, Point)> {
        let arc = Arc::new(c);
        (0..N).map(|k| arc.point(k as f32 / N as f32)).collect()
    };
    let (x, y) = (sample(a), sample(b));
    let cost = |i: usize, j: usize| {
        let d = sub(x[i % N].0, y[j % N].0);
        dot(d, d) + 0.02 * (1.0 - dot(x[i % N].1, y[j % N].1))
    };
    let mut best = (f32::MAX, Vec::new());
    for r in (0..N).step_by(2) {
        let mut dp = vec![[(f32::MAX, 0u8); N + 1]; N + 1];
        dp[0][0].0 = cost(0, r);
        for i in 0..=N {
            for j in 0..=N {
                if i == 0 && j == 0 {
                    continue;
                }
                let c = cost(i, j + r);
                let from = [
                    (i > 0 && j > 0).then(|| dp[i - 1][j - 1].0),
                    (i > 0).then(|| dp[i - 1][j].0),
                    (j > 0).then(|| dp[i][j - 1].0),
                ];
                let (k, v) = from
                    .iter()
                    .enumerate()
                    .filter_map(|(k, v)| v.map(|v| (k, v)))
                    .min_by(|p, q| p.1.total_cmp(&q.1))
                    .unwrap();
                dp[i][j] = (v + c, k as u8);
            }
        }
        if dp[N][N].0 < best.0 {
            let (mut path, mut i, mut j) = (vec![(N, N + r)], N, N);
            while i > 0 || j > 0 {
                match dp[i][j].1 {
                    0 => (i, j) = (i - 1, j - 1),
                    1 => i -= 1,
                    _ => j -= 1,
                }
                path.push((i, j + r));
            }
            path.reverse();
            best = (dp[N][N].0, path);
        }
    }
    best.1
        .windows(2)
        .map(|w| {
            (
                line(x[w[0].0 % N].0, x[w[1].0 % N].0),
                line(y[w[0].1 % N].0, y[w[1].1 % N].0),
            )
        })
        .unzip()
}

struct Sample {
    u: f32,
    p: Point,
    into: Point,
    out: Point,
    joint: f32,
}

/// Uniform samples plus one exactly on every corner.
fn corner_samples(c: &[Quad], n: usize) -> Vec<Sample> {
    let arc = Arc::new(c);
    let m = c.len();
    let joints: Vec<(f32, usize, f32)> = (0..m)
        .map(|i| {
            let j = signed(tangent(&c[(i + m - 1) % m], 1.0), tangent(&c[i], 0.0));
            (arc.marks[i][0], i, j)
        })
        .filter(|x| x.2.abs() > CORNER)
        .collect();
    let mut us: Vec<f32> = (0..n)
        .map(|k| k as f32 / n as f32)
        .filter(|u| joints.iter().all(|x| (u - x.0).abs() > 0.4 / n as f32))
        .chain(joints.iter().map(|x| x.0))
        .collect();
    us.sort_by(f32::total_cmp);
    us.into_iter()
        .map(|u| {
            let (p, t) = arc.point(u);
            match joints.iter().find(|x| x.0 == u) {
                Some(&(_, i, joint)) => Sample {
                    u,
                    p,
                    into: tangent(&c[(i + m - 1) % m], 1.0),
                    out: t,
                    joint,
                },
                None => Sample {
                    u,
                    p,
                    into: t,
                    out: t,
                    joint: 0.0,
                },
            }
        })
        .collect()
}

/// Point DP finds the correspondence, corners attract corners, and the exact
/// curves are cut where either side has a breakpoint.
fn hybrid(a: &[Quad], b: &[Quad]) -> (Run, Run) {
    const N: usize = 96;
    const CORNER_WEIGHT: f32 = 0.05;
    let (x, y) = (corner_samples(a, N), corner_samples(b, N));
    let (n, m) = (x.len(), y.len());
    let cost = |i: usize, j: usize| {
        let (p, q) = (&x[i % n], &y[j % m]);
        let d = sub(p.p, q.p);
        dot(d, d)
            + 0.01 * (2.0 - dot(p.into, q.into) - dot(p.out, q.out))
            + CORNER_WEIGHT * (p.joint - q.joint).abs()
    };
    let mut best = (f32::MAX, Vec::new());
    for r in 0..m {
        let mut dp = vec![(f32::MAX, 0u8); (n + 1) * (m + 1)];
        let at = |i: usize, j: usize| i * (m + 1) + j;
        dp[0].0 = cost(0, r);
        for i in 0..=n {
            for j in 0..=m {
                if i == 0 && j == 0 {
                    continue;
                }
                let from = [
                    (i > 0 && j > 0).then(|| dp[at(i - 1, j - 1)].0),
                    (i > 0).then(|| dp[at(i - 1, j)].0),
                    (j > 0).then(|| dp[at(i, j - 1)].0),
                ];
                let (k, v) = from
                    .iter()
                    .enumerate()
                    .filter_map(|(k, v)| v.map(|v| (k, v)))
                    .min_by(|p, q| p.1.total_cmp(&q.1))
                    .unwrap();
                dp[at(i, j)] = (v + cost(i, j + r), k as u8);
            }
        }
        if dp[at(n, m)].0 < best.0 {
            let (mut path, mut i, mut j) = (vec![(n, m + r)], n, m);
            while i > 0 || j > 0 {
                match dp[at(i, j)].1 {
                    0 => (i, j) = (i - 1, j - 1),
                    1 => i -= 1,
                    _ => j -= 1,
                }
                path.push((i, j + r));
            }
            path.reverse();
            best = (dp[at(n, m)].0, path);
        }
    }
    // Arc length along each side; `b`'s is unrolled past 1 where it wraps.
    let ua = |i: usize| if i == n { 1.0 } else { x[i].u };
    let ub = |j: usize| (j / m) as f32 + y[j % m].u;
    let line: Vec<(f32, f32)> = best.1.iter().map(|&(i, j)| (ua(i), ub(j))).collect();
    let (ax, by) = (Arc::new(a), Arc::new(b));
    let (first, last) = (line[0], line[line.len() - 1]);
    let map = |v: f32, key: fn(&(f32, f32)) -> f32, val: fn(&(f32, f32)) -> f32| {
        let k = line
            .partition_point(|p| key(p) < v)
            .clamp(1, line.len() - 1);
        let (p, q) = (&line[k - 1], &line[k]);
        let d = key(q) - key(p);
        val(p) + (val(q) - val(p)) * if d > 0.0 { (v - key(p)) / d } else { 1.0 }
    };
    let mut knots = vec![first, last];
    knots.extend(best.1.iter().filter_map(|&(i, j)| {
        (x[i % n].joint != 0.0 && y[j % m].joint != 0.0).then(|| (ua(i), ub(j)))
    }));
    for u in ax.breaks().filter(|u| *u > 0.0 && *u < 1.0) {
        knots.push((u, map(u, |p| p.0, |p| p.1)));
    }
    for u in by.breaks().flat_map(|u| [u, u + 1.0]) {
        if u > first.1 && u < last.1 {
            knots.push((map(u, |p| p.1, |p| p.0), u));
        }
    }
    knots.sort_by(|p, q| (p.0 + p.1).total_cmp(&(q.0 + q.1)));
    knots.dedup_by(|p, q| (p.0 - q.0).abs() < 1e-5 && (p.1 - q.1).abs() < 1e-5);
    knots
        .windows(2)
        .map(|w| {
            let off = ((w[0].1 + w[1].1) / 2.0).floor();
            let (u0, u1) = (
                (w[0].1 - off).clamp(0.0, 1.0),
                (w[1].1 - off).clamp(0.0, 1.0),
            );
            (ax.slice(w[0].0, w[1].0), by.slice(u0, u1.max(u0)))
        })
        .unzip()
}

/// Contours paired as `Morph` does it: same winding, nearest centre.
fn pair_contours<'a>(from: &'a Path, to: &'a Path) -> Vec<(Option<&'a Run>, Option<&'a Run>)> {
    let centre = |c: &Run| {
        let n = c.len() as f32;
        let s = c
            .iter()
            .fold([0.0, 0.0], |s, q| [s[0] + q[0][0], s[1] + q[0][1]]);
        [s[0] / n, s[1] / n]
    };
    let wind = |c: &Run| {
        c.iter()
            .map(|q| q[0][0] * q[2][1] - q[2][0] * q[0][1])
            .sum::<f32>()
            > 0.0
    };
    let mut free: Vec<&Run> = to.contours.iter().collect();
    let mut out = Vec::new();
    for x in &from.contours {
        let best = (free.iter().enumerate())
            .filter(|(_, y)| wind(y) == wind(x))
            .min_by(|a, b| dist(centre(x), centre(a.1)).total_cmp(&dist(centre(x), centre(b.1))))
            .map(|(k, _)| k);
        out.push((Some(x), best.map(|k| free.remove(k))));
    }
    out.extend(free.into_iter().map(|y| (None, Some(y))));
    out
}

fn collapsed(c: &Run) -> Run {
    let n = c.len() as f32;
    let s = c
        .iter()
        .fold([0.0, 0.0], |s, q| [s[0] + q[0][0], s[1] + q[0][1]]);
    vec![[[s[0] / n, s[1] / n]; 3]; c.len()]
}

// ---- Skeleton ----

#[derive(Clone)]
struct Stroke {
    pts: Vec<Ink>,
    closed: bool,
    /// A short dead end: only ever the start or end of a drawing.
    stub: bool,
    /// Graph nodes at each end; equal ids meet.
    ends: [usize; 2],
}

struct Raw {
    pts: Vec<Ink>,
    stub: bool,
    ends: [Option<usize>; 2],
}

const H: f32 = 1.0 / 180.0;
const AROUND: [(i32, i32); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

struct Grid {
    w: i32,
    h: i32,
    on: Vec<bool>,
}

impl Grid {
    fn get(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.w && y < self.h && self.on[(y * self.w + x) as usize]
    }
    fn ring(&self, x: i32, y: i32) -> [bool; 8] {
        AROUND.map(|(dx, dy)| self.get(x + dx, y + dy))
    }
    fn crossings(&self, x: i32, y: i32) -> usize {
        let r = self.ring(x, y);
        (0..8).filter(|&i| !r[i] && r[(i + 1) % 8]).count()
    }
}

fn thin(g: &mut Grid) {
    loop {
        let mut changed = false;
        for pass in 0..2 {
            let mut off = Vec::new();
            for y in 0..g.h {
                for x in 0..g.w {
                    if !g.get(x, y) {
                        continue;
                    }
                    let r = g.ring(x, y);
                    let b = r.iter().filter(|v| **v).count();
                    let (p2, p4, p6, p8) = (r[0], r[2], r[4], r[6]);
                    let keep = match pass {
                        0 => (p2 && p4 && p6) || (p4 && p6 && p8),
                        _ => (p2 && p4 && p8) || (p2 && p6 && p8),
                    };
                    if (2..=6).contains(&b) && g.crossings(x, y) == 1 && !keep {
                        off.push((y * g.w + x) as usize);
                    }
                }
            }
            changed |= !off.is_empty();
            for i in off {
                g.on[i] = false;
            }
        }
        if !changed {
            return;
        }
    }
}

fn seg_dist(p: Point, a: Point, b: Point) -> f32 {
    let (d, q) = (sub(b, a), sub(p, a));
    let t = (dot(q, d) / dot(d, d).max(1e-9)).clamp(0.0, 1.0);
    dist(p, [a[0] + d[0] * t, a[1] + d[1] * t])
}

fn smooth(pts: &mut [Ink], closed: bool) {
    let n = pts.len() as i32;
    for _ in 0..4 {
        let old = pts.to_vec();
        let lo = if closed { 0 } else { 1 };
        let hi = if closed { n } else { n - 1 };
        for i in lo..hi {
            let mut s = [0.0; 3];
            for k in -2i32..=2 {
                let j = if closed {
                    (i + k).rem_euclid(n)
                } else {
                    (i + k).clamp(0, n - 1)
                };
                for c in 0..3 {
                    s[c] += old[j as usize][c] / 5.0;
                }
            }
            pts[i as usize] = s;
        }
    }
}

fn resample(pts: &[Ink], n: usize) -> Vec<Ink> {
    let d = |a: &Ink, b: &Ink| dist([a[0], a[1]], [b[0], b[1]]);
    let total: f32 = pts.windows(2).map(|w| d(&w[0], &w[1])).sum();
    let (mut out, mut k, mut walked) = (Vec::with_capacity(n), 0, 0.0);
    for i in 0..n {
        let want = total * i as f32 / (n - 1).max(1) as f32;
        while k + 2 < pts.len() && walked + d(&pts[k], &pts[k + 1]) < want {
            walked += d(&pts[k], &pts[k + 1]);
            k += 1;
        }
        let seg = d(&pts[k], &pts[k + 1]).max(1e-6);
        let t = ((want - walked) / seg).clamp(0.0, 1.0);
        out.push([0, 1, 2].map(|c| mix(pts[k][c], pts[k + 1][c], t)));
    }
    out
}

fn poly_len(p: &[Ink]) -> f32 {
    p.windows(2)
        .map(|w| dist([w[0][0], w[0][1]], [w[1][0], w[1][1]]))
        .sum()
}

/// Medial axis of the filled outline as pen strokes. Rasterises, thins,
/// prunes spurs and joins pieces that run straight through a junction.
/// ponytail: raster thinning at 180 cells/em costs ms per glyph; precompute it or go analytic if it ships.
fn skeleton(path: &Path, through: bool) -> Vec<Stroke> {
    let flat: Vec<(Point, Point)> = path
        .contours
        .iter()
        .flatten()
        .flat_map(|q| (0..6).map(move |k| (at(q, k as f32 / 6.0), at(q, (k + 1) as f32 / 6.0))))
        .collect();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for (p, _) in &flat {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let origin = [lo[0] - 3.0 * H, lo[1] - 3.0 * H];
    let w = ((hi[0] - origin[0]) / H) as i32 + 4;
    let h = ((hi[1] - origin[1]) / H) as i32 + 4;
    let world = |x: i32, y: i32| {
        [
            origin[0] + (x as f32 + 0.5) * H,
            origin[1] + (y as f32 + 0.5) * H,
        ]
    };
    let mut filled = Grid {
        w,
        h,
        on: vec![false; (w * h) as usize],
    };
    for y in 0..h {
        let yc = world(0, y)[1];
        let mut hits: Vec<(f32, i32)> = flat
            .iter()
            .filter(|(a, b)| (a[1] <= yc) != (b[1] <= yc))
            .map(|(a, b)| {
                let x = a[0] + (yc - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                (x, if b[1] > a[1] { 1 } else { -1 })
            })
            .collect();
        hits.sort_by(|p, q| p.0.total_cmp(&q.0));
        let mut wind = 0;
        for pair in hits.windows(2) {
            wind += pair[0].1;
            if wind != 0 {
                let x0 = ((pair[0].0 - origin[0]) / H - 0.5).ceil().max(0.0) as i32;
                let x1 = ((pair[1].0 - origin[0]) / H - 0.5).floor() as i32;
                for x in x0..=x1.min(w - 1) {
                    filled.on[(y * w + x) as usize] = true;
                }
            }
        }
    }
    let mut g = Grid {
        w,
        h,
        on: filled.on.clone(),
    };
    thin(&mut g);

    let radius = |x: i32, y: i32| {
        let p = world(x, y);
        flat.iter()
            .map(|(a, b)| seg_dist(p, *a, *b))
            .fold(f32::MAX, f32::min)
    };
    let ink = |(x, y): (i32, i32)| {
        let p = world(x, y);
        [p[0], p[1], radius(x, y)]
    };

    // Junction clusters and free ends become graph nodes.
    let mut node: HashMap<(i32, i32), usize> = HashMap::new();
    let mut count = 0;
    for y in 0..h {
        for x in 0..w {
            if !g.get(x, y) || node.contains_key(&(x, y)) {
                continue;
            }
            match g.crossings(x, y) {
                1 => {
                    node.insert((x, y), count);
                    count += 1;
                }
                c if c >= 3 => {
                    let mut stack = vec![(x, y)];
                    while let Some((px, py)) = stack.pop() {
                        if node.contains_key(&(px, py)) {
                            continue;
                        }
                        node.insert((px, py), count);
                        for (dx, dy) in AROUND {
                            let q = (px + dx, py + dy);
                            if g.get(q.0, q.1) && g.crossings(q.0, q.1) >= 3 {
                                stack.push(q);
                            }
                        }
                    }
                    count += 1;
                }
                _ => {}
            }
        }
    }
    let mut centre = vec![([0.0; 3], 0.0); count];
    for (&p, &id) in &node {
        let v = ink(p);
        for c in 0..3 {
            centre[id].0[c] += v[c];
        }
        centre[id].1 += 1.0;
    }
    let centre: Vec<Ink> = centre.iter().map(|(s, n)| s.map(|v| v / n)).collect();

    let mut seen: HashMap<(i32, i32), ()> = HashMap::new();
    let mut raws: Vec<Raw> = Vec::new();
    let mut starts: Vec<_> = node.keys().copied().collect();
    starts.sort();
    let order = [0usize, 2, 4, 6, 1, 3, 5, 7];
    for s in starts {
        let id = node[&s];
        for (dx, dy) in order.map(|i| AROUND[i]) {
            let q = (s.0 + dx, s.1 + dy);
            if !g.get(q.0, q.1) || node.contains_key(&q) || seen.contains_key(&q) {
                continue;
            }
            seen.insert(q, ());
            let (mut pts, mut cur, mut end) = (vec![centre[id], ink(q)], q, None);
            'walk: loop {
                for (dx, dy) in order.map(|i| AROUND[i]) {
                    let n = (cur.0 + dx, cur.1 + dy);
                    if let Some(&e) = node.get(&n)
                        && (e != id || pts.len() > 6)
                    {
                        end = Some(e);
                        pts.push(centre[e]);
                        break 'walk;
                    }
                }
                let next = order
                    .map(|i| AROUND[i])
                    .into_iter()
                    .map(|(dx, dy)| (cur.0 + dx, cur.1 + dy))
                    .find(|n| g.get(n.0, n.1) && !node.contains_key(n) && !seen.contains_key(n));
                let Some(n) = next else { break };
                seen.insert(n, ());
                pts.push(ink(n));
                cur = n;
            }
            raws.push(Raw {
                pts,
                stub: false,
                ends: [Some(id), end],
            });
        }
    }
    // Rings with no junction at all, like O.
    for y in 0..h {
        for x in 0..w {
            if !g.get(x, y) || node.contains_key(&(x, y)) || seen.contains_key(&(x, y)) {
                continue;
            }
            seen.insert((x, y), ());
            let (mut pts, mut cur) = (vec![ink((x, y))], (x, y));
            while let Some(n) = order
                .map(|i| AROUND[i])
                .into_iter()
                .map(|(dx, dy)| (cur.0 + dx, cur.1 + dy))
                .find(|n| g.get(n.0, n.1) && !seen.contains_key(n))
            {
                seen.insert(n, ());
                pts.push(ink(n));
                cur = n;
            }
            if pts.len() > 12 {
                let first = pts[0];
                pts.push(first);
                raws.push(Raw {
                    pts,
                    stub: false,
                    ends: [Some(count), Some(count)],
                });
                count += 1;
            }
        }
    }

    let degree = |raws: &[Raw], id: usize| {
        raws.iter()
            .flat_map(|r| r.ends)
            .filter(|e| *e == Some(id))
            .count()
    };
    // Thinning forks into the corners of a terminal: those come in pairs, or
    // bisect a bend. A lone short branch running straight on from another
    // edge is real ink, like the stem above the arch of an n.
    loop {
        let mut groups: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
        for (i, r) in raws.iter().enumerate().filter(|(_, r)| !r.stub) {
            for e in 0..2 {
                let dead = r.ends[e].is_none_or(|id| degree(&raws, id) == 1);
                let Some(root) = r.ends[1 - e].filter(|&id| degree(&raws, id) >= 3) else {
                    continue;
                };
                let (tip, base) = match e {
                    0 => (r.pts[0], r.pts[r.pts.len() - 1]),
                    _ => (r.pts[r.pts.len() - 1], r.pts[0]),
                };
                if dead && poly_len(&r.pts) < 2.0 * base[2].max(tip[2]) {
                    groups.entry(root).or_default().push((i, 1 - e));
                }
            }
        }
        let outward = |r: &Raw, e: usize| {
            let k = 8.min(r.pts.len() - 1);
            let (a, b) = match e {
                0 => (r.pts[0], r.pts[k]),
                _ => (r.pts[r.pts.len() - 1], r.pts[r.pts.len() - 1 - k]),
            };
            unit([b[0] - a[0], b[1] - a[1]])
        };
        let (mut gone, mut keep) = (Vec::new(), Vec::new());
        for (root, spurs) in &groups {
            if let [(i, e)] = spurs[..] {
                let mut dots: Vec<f32> = (0..raws.len())
                    .filter(|&j| j != i)
                    .flat_map(|j| (0..2).map(move |f| (j, f)))
                    .filter(|&(j, f)| raws[j].ends[f] == Some(*root))
                    .map(|(j, f)| dot(outward(&raws[i], e), outward(&raws[j], f)))
                    .collect();
                dots.sort_by(f32::total_cmp);
                // A third edge opposing the spur makes it a corner, as in Z.
                let straight =
                    dots.first().is_some_and(|d| *d < -0.7) && dots[1..].iter().all(|d| *d > -0.55);
                if straight {
                    keep.push(i);
                    continue;
                }
            }
            gone.extend(spurs.iter().map(|s| s.0));
        }
        if gone.is_empty() && keep.is_empty() {
            break;
        }
        for i in keep {
            raws[i].stub = true;
        }
        gone.sort();
        gone.dedup();
        for i in gone.into_iter().rev() {
            raws.remove(i);
        }
    }

    // Join pieces that continue each other through a node.
    loop {
        let mut joined = false;
        for id in 0..count {
            let inc: Vec<(usize, usize)> = (0..raws.len())
                .flat_map(|i| (0..2).map(move |e| (i, e)))
                .filter(|&(i, e)| raws[i].ends[e] == Some(id))
                .collect();
            let out = |&(i, e): &(usize, usize)| {
                let p = &raws[i].pts;
                let k = 8.min(p.len() - 1);
                let (a, b) = if e == 0 {
                    (p[0], p[k])
                } else {
                    (p[p.len() - 1], p[p.len() - 1 - k])
                };
                unit([b[0] - a[0], b[1] - a[1]])
            };
            let mut pick = None;
            let mut best = -0.8;
            for x in 0..inc.len() {
                for y in x + 1..inc.len() {
                    if inc[x].0 == inc[y].0 {
                        continue;
                    }
                    let d = dot(out(&inc[x]), out(&inc[y]));
                    if inc.len() == 2 || (through && d < best) {
                        best = d;
                        pick = Some((inc[x], inc[y]));
                    }
                }
            }
            if let Some(((i, e), (j, f))) = pick {
                let (i, j, e, f) = if i > j { (j, i, f, e) } else { (i, j, e, f) };
                let b = raws.remove(j);
                let a = &mut raws[i];
                if e == 0 {
                    a.pts.reverse();
                    a.ends.swap(0, 1);
                }
                let mut tail = b.pts;
                let mut tail_ends = b.ends;
                if f == 1 {
                    tail.reverse();
                    tail_ends.swap(0, 1);
                }
                a.pts.extend(tail.into_iter().skip(1));
                a.ends[1] = tail_ends[1];
                joined = true;
                break;
            }
            if inc.len() == 2 && inc[0].0 == inc[1].0 {
                raws[inc[0].0].ends = [None, None];
                joined = true;
                break;
            }
        }
        if !joined {
            break;
        }
    }

    let inside = |p: Point| {
        let (x, y) = (
            ((p[0] - origin[0]) / H) as i32,
            ((p[1] - origin[1]) / H) as i32,
        );
        filled.get(x, y)
    };
    let mut strokes = Vec::new();
    for (i, r) in raws.iter().enumerate() {
        if r.pts.len() < 4 {
            continue;
        }
        let closed = r.ends == [None, None];
        let mut pts = r.pts.clone();
        smooth(&mut pts, closed);
        let n = ((poly_len(&pts) / 0.012) as usize).max(4);
        let mut pts = resample(&pts, n);
        if !closed {
            for e in 0..2 {
                let free = r.ends[e].is_none_or(|id| degree(&raws, id) == 1);
                if !free {
                    continue;
                }
                if e == 0 {
                    pts.reverse();
                }
                // The radius collapses into a terminal; hold it, then run
                // the centreline out to the outline for a flat end.
                let last = pts.len() - 1;
                let held = pts[last.saturating_sub(5)][2];
                let keep = last.saturating_sub((0.8 * held / 0.012) as usize);
                let r_keep = pts[keep][2];
                for p in &mut pts[keep..] {
                    p[2] = r_keep;
                }
                let (a, b) = (pts[last.saturating_sub(4)], pts[last]);
                let dir = unit([b[0] - a[0], b[1] - a[1]]);
                let mut tip = [b[0], b[1]];
                for _ in 0..200 {
                    let q = [tip[0] + dir[0] * H, tip[1] + dir[1] * H];
                    if !inside(q) || dist(q, [b[0], b[1]]) > 2.5 * r_keep {
                        break;
                    }
                    tip = q;
                }
                pts.push([tip[0], tip[1], r_keep]);
                if e == 0 {
                    pts.reverse();
                }
            }
        }
        let fresh = count + 2 * i;
        let ends = match closed {
            true => [fresh, fresh],
            false => [r.ends[0].unwrap_or(fresh), r.ends[1].unwrap_or(fresh + 1)],
        };
        strokes.push(Stroke {
            pts,
            closed,
            stub: r.stub,
            ends,
        });
    }
    strokes
}

// ---- Route planning ----

/// How a hand trades the options, in em of ink. A retrace costs its length, so
/// `LIFT / RETRACE` is the longest line worth going over twice.
const LIFT: f32 = 0.45;
const LIFT_DIST: f32 = 0.25;
const RETRACE: f32 = 0.9;
/// A stub at a three-way junction means the hand came back along a neighbour.
const NEXT_TO_STUB: f32 = 0.4;
const TURN: f32 = 0.08;
const START: f32 = 0.6;
/// Extra distance a top-left start counts for against a bottom-left one.
const TOP_LEFT_BIAS: f32 = 0.12;
const BACKWARD: f32 = 0.04;
/// Starting a curve from its lower end: hands draw 2, 3, S and C from the top.
const CURVE_FROM_BELOW: f32 = 0.5;
/// Stubs on the right end a drawing and stubs on the left start one.
const STUB_WRONG_END: f32 = 0.5;
/// A first stroke rarely begins where lines already meet.
const JUNCTION_START: f32 = 0.5;
/// Leaving a junction any other way while an un-inked line runs straight on.
const LEAVE_STRAIGHT: f32 = 0.35;
/// Cosine of the angle under which two lines count as one direction.
const PARALLEL: f32 = 0.87;
const MAX_EDGES: usize = 14;
const CORNER_TURN: f32 = 50.0 * PI / 180.0;

struct Route {
    /// One polyline per pen-down, retraces included.
    drawings: Vec<Vec<Ink>>,
    retraced: Vec<Vec<Ink>>,
}

fn xy(p: Ink) -> Point {
    [p[0], p[1]]
}

fn turn_at(p: &[Ink], i: usize, w: usize) -> f32 {
    signed(sub(xy(p[i]), xy(p[i - w])), sub(xy(p[i + w]), xy(p[i])))
}

/// Cuts edges at sharp bends so a pen can start in a corner (E, L). Rings
/// open at their sharpest corner, or else at their top-left.
fn split_corners(mut edges: Vec<Stroke>) -> Vec<Stroke> {
    const W: usize = 5;
    let mut next = edges.iter().flat_map(|e| e.ends).max().unwrap_or(0) + 1;
    for e in edges.iter_mut().filter(|e| e.closed) {
        e.pts.pop();
        let n = e.pts.len();
        let bend = |p: &[Ink], i: usize| {
            signed(
                sub(xy(p[i]), xy(p[(i + n - W) % n])),
                sub(xy(p[(i + W) % n]), xy(p[i])),
            )
            .abs()
        };
        let sharp = (0..n).max_by(|&a, &b| bend(&e.pts, a).total_cmp(&bend(&e.pts, b)));
        let corner = sharp.filter(|&i| bend(&e.pts, i) > CORNER_TURN);
        let start = corner.unwrap_or_else(|| {
            let key = |p: Ink| p[0] + p[1];
            (0..n)
                .min_by(|&a, &b| key(e.pts[a]).total_cmp(&key(e.pts[b])))
                .unwrap()
        });
        e.pts.rotate_left(start);
        let first = e.pts[0];
        e.pts.push(first);
        e.closed = corner.is_none();
    }
    let mut cuts: Vec<(f32, usize, usize)> = Vec::new();
    for (k, e) in edges.iter().enumerate().filter(|(_, e)| !e.closed) {
        let p = &e.pts;
        for i in W..p.len().saturating_sub(W) {
            let t = turn_at(p, i, W).abs();
            let peak = (i.saturating_sub(W).max(W)..(i + W).min(p.len() - W))
                .all(|j| turn_at(p, j, W).abs() <= t);
            if t > CORNER_TURN && peak {
                cuts.push((t, k, i));
            }
        }
    }
    cuts.sort_by(|a, b| b.0.total_cmp(&a.0));
    cuts.truncate(MAX_EDGES.saturating_sub(edges.len()));
    cuts.sort_by_key(|c| std::cmp::Reverse((c.1, c.2)));
    for (_, k, i) in cuts {
        let tail = edges[k].pts.split_off(i);
        let joint = tail[0];
        edges[k].pts.push(joint);
        let end = edges[k].ends[1];
        edges[k].ends[1] = next;
        edges.push(Stroke {
            pts: tail,
            closed: false,
            stub: false,
            ends: [next, end],
        });
        next += 1;
    }
    edges
}

/// Cheapest way to ink every edge: Dijkstra over (edges inked, last edge and
/// direction), so lifts, retraces and turns compete on one scale.
fn plan(edges: Vec<Stroke>) -> Route {
    let e = edges.len();
    if e == 0 || e > MAX_EDGES {
        return Route {
            drawings: pen_order(edges).into_iter().map(|s| s.pts).collect(),
            retraced: Vec::new(),
        };
    }
    let oriented = |i: usize, d: usize| -> Vec<Ink> {
        let mut p = edges[i].pts.clone();
        if d == 1 {
            p.reverse();
        }
        p
    };
    struct Way {
        from: Point,
        to: Point,
        enter: Point,
        leave: Point,
        start: usize,
        end: usize,
        len: f32,
        draw: f32,
        from_below: bool,
        tip_start: bool,
        tip_end: bool,
    }
    let meeting = |id: usize| {
        edges
            .iter()
            .flat_map(|s| s.ends)
            .filter(|n| *n == id)
            .count()
    };
    let ways: Vec<Way> = (0..2 * e)
        .map(|w| {
            let (i, d) = (w / 2, w % 2);
            let p = oriented(i, d);
            let (n, k) = (p.len(), 6.min(p.len() - 1));
            let (from, to) = (xy(p[0]), xy(p[n - 1]));
            let len = poly_len(&p);
            let d_all = sub(to, from);
            let backward = d_all[0] < 0.0 && d_all[0].abs() > d_all[1].abs();
            // The largest bend so far, so the opposite bends of an S don't cancel.
            let (mut sweep, mut bend) = (0.0f32, 0.0f32);
            for w in p.windows(3) {
                sweep += signed(sub(xy(w[1]), xy(w[0])), sub(xy(w[2]), xy(w[1])));
                bend = bend.max(sweep.abs());
            }
            Way {
                from,
                to,
                enter: unit(sub(xy(p[k]), from)),
                leave: unit(sub(to, xy(p[n - 1 - k]))),
                start: edges[i].ends[d],
                end: edges[i].ends[1 - d],
                len,
                draw: if backward { BACKWARD * len } else { 0.0 },
                // Rings have no lower end: hands run them counterclockwise.
                from_below: (bend > 1.5 && from[1] > to[1] + 0.1)
                    || (edges[i].closed && sweep > 0.0),
                tip_start: edges[i].stub && meeting(edges[i].ends[d]) == 1,
                tip_end: edges[i].stub && meeting(edges[i].ends[1 - d]) == 1,
            }
        })
        .collect();
    let retrace: Vec<f32> = edges
        .iter()
        .map(|x| {
            let by_stub = edges.iter().any(|y| {
                y.stub
                    && !x.stub
                    && x.ends
                        .iter()
                        .any(|n| y.ends.contains(n) && meeting(*n) == 3)
            });
            RETRACE * if by_stub { NEXT_TO_STUB } else { 1.0 }
        })
        .collect();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in edges.iter().flat_map(|s| &s.pts) {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let (bottom_left, top_left) = ([lo[0], hi[1]], lo);
    let centre = (lo[0] + hi[0]) / 2.0;
    let wrong_start = |w: &Way| {
        let right_stub = w.tip_start && w.from[0] > centre;
        if right_stub { STUB_WRONG_END } else { 0.0 }
    };
    let start_cost = |w: &Way| {
        let near = dist(w.from, bottom_left).min(dist(w.from, top_left) + TOP_LEFT_BIAS);
        START * near
            + wrong_start(w)
            + if w.from_below { CURVE_FROM_BELOW } else { 0.0 }
            + if meeting(w.start) >= 3 {
                JUNCTION_START
            } else {
                0.0
            }
    };
    let end_cost = |w: &Way| {
        if w.tip_end && w.to[0] <= centre {
            STUB_WRONG_END
        } else {
            0.0
        }
    };
    let turn = |a: Point, b: Point| TURN * (signed(a, b).abs() / PI).powi(2);

    let full = (1usize << e) - 1;
    let index = |mask: usize, w: usize| mask * 2 * e + w;
    let mut best = vec![f32::MAX; (full + 1) * 2 * e];
    let mut parent = vec![(u32::MAX, false); best.len()];
    let mut heap = std::collections::BinaryHeap::new();
    let push = |heap: &mut std::collections::BinaryHeap<_>,
                best: &mut Vec<f32>,
                parent: &mut Vec<(u32, bool)>,
                state: usize,
                cost: f32,
                from: u32,
                lift: bool| {
        if cost < best[state] {
            best[state] = cost;
            parent[state] = (from, lift);
            heap.push(std::cmp::Reverse(((cost * 1e5) as u64, state)));
        }
    };
    for (w, way) in ways.iter().enumerate() {
        let cost = start_cost(way) + way.draw;
        push(
            &mut heap,
            &mut best,
            &mut parent,
            index(1 << (w / 2), w),
            cost,
            u32::MAX,
            true,
        );
    }
    let mut goal: Option<(f32, usize)> = None;
    while let Some(std::cmp::Reverse((c, state))) = heap.pop() {
        let (mask, w) = (state / (2 * e), state % (2 * e));
        if (c as f32) / 1e5 > best[state] + 1e-3 {
            continue;
        }
        if goal.is_some_and(|g| best[state] >= g.0) {
            break;
        }
        if mask == full {
            let total = best[state] + end_cost(&ways[w]);
            if goal.is_none_or(|g| total < g.0) {
                goal = Some((total, state));
            }
            continue;
        }
        let way = &ways[w];
        let along = |v: &Way| dot(way.leave, v.enter) > PARALLEL;
        let straight_on = ways
            .iter()
            .enumerate()
            .any(|(v, n)| n.start == way.end && mask & (1 << (v / 2)) == 0 && along(n));
        let swerve = |v: &Way| {
            if straight_on && !along(v) {
                LEAVE_STRAIGHT
            } else {
                0.0
            }
        };
        for (v, next) in ways.iter().enumerate() {
            let bit = 1usize << (v / 2);
            let inked = mask & bit != 0;
            if next.start == way.end && !(inked && edges[v / 2].stub) {
                let step = swerve(next)
                    + turn(way.leave, next.enter)
                    + if inked {
                        retrace[v / 2] * next.len
                    } else {
                        next.draw
                    };
                let cost = best[state] + step;
                push(
                    &mut heap,
                    &mut best,
                    &mut parent,
                    index(mask | bit, v),
                    cost,
                    state as u32,
                    false,
                );
            }
            if !inked {
                let cost = best[state]
                    + LIFT
                    + LIFT_DIST * dist(way.to, next.from)
                    + next.draw
                    + wrong_start(next)
                    + swerve(next);
                push(
                    &mut heap,
                    &mut best,
                    &mut parent,
                    index(mask | bit, v),
                    cost,
                    state as u32,
                    true,
                );
            }
        }
    }
    let mut chain = Vec::new();
    let mut at_state = goal.expect("every edge is reachable by a lift").1;
    loop {
        let (from, lift) = parent[at_state];
        chain.push((at_state, lift, from));
        if from == u32::MAX {
            break;
        }
        at_state = from as usize;
    }
    chain.reverse();
    let (mut drawings, mut retraced) = (Vec::<Vec<Ink>>::new(), Vec::new());
    for &(state, lift, from) in &chain {
        let (mask, w) = (state / (2 * e), state % (2 * e));
        let pts = oriented(w / 2, w % 2);
        if from != u32::MAX && (from as usize) / (2 * e) == mask {
            retraced.push(pts.clone());
        }
        if lift {
            drawings.push(pts);
        } else {
            drawings.last_mut().unwrap().extend(pts.into_iter().skip(1));
        }
    }
    Route { drawings, retraced }
}

fn ribbon(out: &mut Path, pts: &[Ink]) {
    if pts.len() < 2 {
        return;
    }
    let n = pts.len();
    let normal = |i: usize| {
        let (a, b) = (pts[i.saturating_sub(1)], pts[(i + 1).min(n - 1)]);
        let d = unit([b[0] - a[0], b[1] - a[1]]);
        [-d[1], d[0]]
    };
    let side = |i: usize, s: f32| {
        let m = normal(i);
        [
            pts[i][0] + m[0] * pts[i][2] * s,
            pts[i][1] + m[1] * pts[i][2] * s,
        ]
    };
    for i in 0..n - 1 {
        let mut c = [
            side(i, 1.0),
            side(i + 1, 1.0),
            side(i + 1, -1.0),
            side(i, -1.0),
        ];
        let area: f32 = (0..4)
            .map(|k| c[k][0] * c[(k + 1) % 4][1] - c[(k + 1) % 4][0] * c[k][1])
            .sum();
        if area < 0.0 {
            c.reverse();
        }
        out.contours.extend(Path::polygon(&c).contours);
    }
}

fn disc(out: &mut Path, p: Ink) {
    if p[2] > 0.0 {
        let pts: Vec<Point> = (0..10)
            .map(|k| {
                let a = TAU * k as f32 / 10.0;
                [p[0] + p[2] * a.cos(), p[1] + p[2] * a.sin()]
            })
            .collect();
        out.contours.extend(Path::polygon(&pts).contours);
    }
}

/// The first `len` of `pts`, the tip cut mid-sample.
fn prefix(pts: &[Ink], len: f32) -> Vec<Ink> {
    let mut out = vec![pts[0]];
    let mut left = len;
    for w in pts.windows(2) {
        let d = dist([w[0][0], w[0][1]], [w[1][0], w[1][1]]).max(1e-6);
        if d >= left {
            let t = left / d;
            out.push([0, 1, 2].map(|c| mix(w[0][c], w[1][c], t)));
            return out;
        }
        left -= d;
        out.push(w[1]);
    }
    out
}

/// Pen order: start top-left, then whichever stroke end is closest.
fn pen_order(mut left: Vec<Stroke>) -> Vec<Stroke> {
    for s in left.iter_mut().filter(|s| s.closed) {
        let top = (0..s.pts.len())
            .min_by(|&a, &b| s.pts[a][1].total_cmp(&s.pts[b][1]))
            .unwrap();
        s.pts.pop();
        let top = top.min(s.pts.len() - 1);
        s.pts.rotate_left(top);
        let first = s.pts[0];
        s.pts.push(first);
    }
    let mut out = Vec::new();
    let mut pen: Option<Point> = None;
    while !left.is_empty() {
        let score = |p: Ink| match pen {
            None => p[1] + 0.4 * p[0],
            Some(q) => dist([p[0], p[1]], q) + 0.3 * (p[1] + 0.4 * p[0]),
        };
        let (i, flip) = (0..left.len())
            .flat_map(|i| [(i, false), (i, true)])
            .min_by(|a, b| {
                let s = |(i, f): (usize, bool)| {
                    let p = &left[i].pts;
                    score(if f { p[p.len() - 1] } else { p[0] })
                };
                s(*a).total_cmp(&s(*b))
            })
            .unwrap();
        let mut s = left.remove(i);
        if flip {
            s.pts.reverse();
        }
        let e = s.pts[s.pts.len() - 1];
        pen = Some([e[0], e[1]]);
        out.push(s);
    }
    out
}

fn stroke_pairs(a: &[Stroke], b: &[Stroke]) -> Vec<(Vec<Ink>, Vec<Ink>)> {
    const N: usize = 48;
    let sa: Vec<Vec<Ink>> = a.iter().map(|s| resample(&s.pts, N)).collect();
    let sb: Vec<Vec<Ink>> = b.iter().map(|s| resample(&s.pts, N)).collect();
    let mean = |p: &[Ink], q: &[Ink]| -> f32 {
        p.iter()
            .zip(q)
            .map(|(u, v)| dist([u[0], u[1]], [v[0], v[1]]))
            .sum::<f32>()
            / N as f32
    };
    let variants = |p: &[Ink], closed: bool| -> Vec<Vec<Ink>> {
        let turns = if closed { 16 } else { 1 };
        (0..turns)
            .flat_map(|t| {
                let mut r = p.to_vec();
                if closed {
                    r.pop();
                    let by = t * r.len() / 16;
                    r.rotate_left(by);
                    let f = r[0];
                    r.push(f);
                }
                let mut f = r.clone();
                f.reverse();
                [r, f]
            })
            .collect()
    };
    let mut cands = Vec::new();
    for (i, p) in sa.iter().enumerate() {
        for (j, q) in sb.iter().enumerate() {
            for v in variants(p, a[i].closed) {
                cands.push((mean(&v, q), i, j, v));
            }
        }
    }
    cands.sort_by(|x, y| x.0.total_cmp(&y.0));
    let (mut used_a, mut used_b, mut out) =
        (vec![false; a.len()], vec![false; b.len()], Vec::new());
    for (_, i, j, v) in cands {
        if !used_a[i] && !used_b[j] {
            used_a[i] = true;
            used_b[j] = true;
            out.push((v, sb[j].clone()));
        }
    }
    let fold = |p: &[Ink], onto: &[Vec<Ink>]| -> Vec<Ink> {
        p.iter()
            .map(|u| {
                *onto
                    .iter()
                    .flatten()
                    .min_by(|x, y| {
                        let d = |v: &Ink| dist([u[0], u[1]], [v[0], v[1]]);
                        d(x).total_cmp(&d(y))
                    })
                    .unwrap_or(u)
            })
            .collect()
    };
    for (i, p) in sa.iter().enumerate() {
        if !used_a[i] {
            out.push((p.clone(), fold(p, &sb)));
        }
    }
    for (j, q) in sb.iter().enumerate() {
        if !used_b[j] {
            out.push((fold(q, &sa), q.clone()));
        }
    }
    out
}

// ---- Analysis, cached per glyph and pair ----

struct Glyph {
    path: Path,
    strokes: Vec<Stroke>,
    route: Route,
    worst_fit: f32,
    fits: usize,
}

struct Pairing {
    a: Rc<Glyph>,
    b: Rc<Glyph>,
    current: Morph,
    segs: (Path, Path),
    points: (Path, Path),
    hybrid: (Path, Path),
    /// Runs of each glyph, coloured by which match they belong to.
    runs_a: Vec<(Run, usize)>,
    runs_b: Vec<(Run, usize)>,
    inks: Vec<(Vec<Ink>, Vec<Ink>)>,
}

thread_local! {
    static GLYPHS: RefCell<HashMap<char, Option<Rc<Glyph>>>> = RefCell::default();
    static PAIRINGS: RefCell<HashMap<(char, char), Option<Rc<Pairing>>>> = RefCell::default();
    static CLOCK: Instant = Instant::now();
}

fn glyph(cx: &PaintCx, c: char) -> Option<Rc<Glyph>> {
    if let Some(g) = GLYPHS.with_borrow(|m| m.get(&c).cloned()) {
        return g;
    }
    let g = cx
        .glyph_path(Font::DEFAULT, c, 1.0)
        .filter(|p| !p.contours.is_empty())
        .map(|mut path| {
            // Centred on x so a pair doesn't slide sideways.
            let xs = path.contours.iter().flatten().flatten().map(|p| p[0]);
            let (lo, hi) = xs.fold((f32::MAX, f32::MIN), |(l, h), x| (l.min(x), h.max(x)));
            path = path.offset(-(lo + hi) / 2.0, 0.0);
            for c in &mut path.contours {
                c.retain(|q| dist(q[0], q[2]) > 1e-5 || dist(q[0], q[1]) > 1e-5);
            }
            let runs: Vec<Run> = path.contours.iter().flat_map(|c| decompose(c)).collect();
            let worst_fit = runs.iter().map(|r| fit(r).1).fold(0.0, f32::max);
            let strokes = pen_order(skeleton(&path, true));
            let route = plan(split_corners(skeleton(&path, false)));
            Rc::new(Glyph {
                path,
                strokes,
                route,
                worst_fit,
                fits: runs.len(),
            })
        });
    GLYPHS.with_borrow_mut(|m| m.insert(c, g.clone()));
    g
}

fn pairing(cx: &PaintCx, a: char, b: char) -> Option<Rc<Pairing>> {
    if let Some(p) = PAIRINGS.with_borrow(|m| m.get(&(a, b)).cloned()) {
        return p;
    }
    let p = glyph(cx, a).zip(glyph(cx, b)).map(|(a, b)| {
        let (mut sa, mut sb, mut pa, mut pb) = (
            Path::default(),
            Path::default(),
            Path::default(),
            Path::default(),
        );
        let (mut runs_a, mut runs_b, mut color) = (Vec::new(), Vec::new(), 0);
        for pair in pair_contours(&a.path, &b.path) {
            let (x, y, u, v) = match pair {
                (Some(x), Some(y)) => {
                    let (rx, ry) = (decompose(x), decompose(y));
                    let (r, m) = match_runs(&rx, &ry);
                    let ry: Vec<Run> = (0..ry.len())
                        .map(|k| ry[(r + k) % ry.len()].clone())
                        .collect();
                    let (mut p, mut q) = (Vec::new(), Vec::new());
                    for (i0, i1, j0, j1) in m {
                        let u: Run = rx[i0..i1].concat();
                        let v: Run = ry[j0..j1].concat();
                        let (s, t) = align(&u, &v);
                        p.extend(s);
                        q.extend(t);
                        runs_a.push((u, color));
                        runs_b.push((v, color));
                        color += 1;
                    }
                    let (u, v) = point_dp(x, y);
                    (p, q, u, v)
                }
                (Some(x), None) => {
                    runs_a.push((x.clone(), color));
                    color += 1;
                    (x.clone(), collapsed(x), x.clone(), collapsed(x))
                }
                (None, Some(y)) => {
                    runs_b.push((y.clone(), color));
                    color += 1;
                    (collapsed(y), y.clone(), collapsed(y), y.clone())
                }
                (None, None) => unreachable!(),
            };
            sa.contours.push(x);
            sb.contours.push(y);
            pa.contours.push(u);
            pb.contours.push(v);
        }
        let (mut ha, mut hb) = (Path::default(), Path::default());
        for pair in pair_contours(&a.path, &b.path) {
            let (u, v) = match pair {
                (Some(x), Some(y)) => hybrid(x, y),
                (Some(x), None) => (x.clone(), collapsed(x)),
                (None, Some(y)) => (collapsed(y), y.clone()),
                (None, None) => unreachable!(),
            };
            ha.contours.push(u);
            hb.contours.push(v);
        }
        Rc::new(Pairing {
            hybrid: (ha, hb),
            current: Morph::new(&a.path, &b.path),
            segs: (sa, sb),
            points: (pa, pb),
            runs_a,
            runs_b,
            inks: stroke_pairs(&a.strokes, &b.strokes),
            a,
            b,
        })
    });
    PAIRINGS.with_borrow_mut(|m| m.insert((a, b), p.clone()));
    p
}

fn lerped(a: &Path, b: &Path, t: f32) -> Path {
    Path {
        contours: a
            .contours
            .iter()
            .zip(&b.contours)
            .map(|(x, y)| {
                x.iter()
                    .zip(y)
                    .map(|(p, q)| {
                        [0, 1, 2].map(|i| [mix(p[i][0], q[i][0], t), mix(p[i][1], q[i][1], t)])
                    })
                    .collect()
            })
            .collect(),
    }
}

// ---- Panels ----

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    SegmentsA,
    SegmentsB,
    SkeletonA,
    Route,
    Planned,
    Current,
    SegmentDp,
    PointDp,
    Hybrid,
    StrokeMorph,
    Outline,
    Nearest,
    Pen,
}

const MORPHS: [(Kind, &str); 5] = [
    (Kind::Current, "Current Morph"),
    (Kind::SegmentDp, "Segment DP"),
    (Kind::PointDp, "Point DP"),
    (Kind::Hybrid, "Hybrid"),
    (Kind::StrokeMorph, "Skeleton strokes"),
];
const DRAWS: [(Kind, &str); 5] = [
    (Kind::Route, "Route plan"),
    (Kind::Planned, "Planned pen"),
    (Kind::Outline, "Walk outline"),
    (Kind::Nearest, "Segments, nearest"),
    (Kind::Pen, "Skeleton pen"),
];

struct Panel {
    kind: Kind,
    a: char,
    b: char,
    size: f32,
    /// Fixed progress, or `None` to loop on the clock.
    t: Option<f32>,
}

fn run_ink(r: &Run, w: f32) -> Vec<Ink> {
    let mut v: Vec<Ink> = vec![[r[0][0][0], r[0][0][1], w]];
    for q in r {
        v.extend((1..=8).map(|k| {
            let p = at(q, k as f32 / 8.0);
            [p[0], p[1], w]
        }));
    }
    v
}

/// Discs along the inked part of `drawings`, as a mask over the real glyph.
/// Barely wider than the stroke, more at terminals and corners to reach their
/// tips; a wider disc elsewhere would uncover branches early. The radius is capped so a swollen junction can't uncover a neighbouring stroke.
fn reveal(drawings: &[Vec<Ink>], p: f32) -> Path {
    let mut radii: Vec<f32> = drawings.iter().flatten().map(|s| s[2]).collect();
    radii.sort_by(f32::total_cmp);
    let cap = radii[radii.len() / 2] * 1.15;
    let total: f32 = drawings.iter().map(|d| poly_len(d)).sum();
    let mut left = total * p;
    let mut out = Path::default();
    for d in drawings {
        if left <= 0.0 {
            break;
        }
        let l = poly_len(d);
        let shown = prefix(d, left.min(l));
        let last = shown.len() - 1;
        let done = left >= l;
        // ponytail: ~1000 mask curves per glyph; cache the finished strokes if it ships.
        for i in (0..last).step_by(2).chain([last]) {
            let [x, y, r] = shown[i];
            let corner = i >= 5 && i + 5 <= last && turn_at(&shown, i, 5).abs() > CORNER_TURN;
            let end = i == 0 || (done && i == last);
            let grow = if corner || end { 1.5 } else { 1.1 };
            disc(&mut out, [x, y, r.min(cap) * grow + 0.004]);
        }
        left -= l;
    }
    out
}

fn write_on(out: &mut Path, polys: &[Vec<Ink>], p: f32, round: bool) {
    let total: f32 = polys.iter().map(|s| poly_len(s)).sum();
    let mut left = total * p;
    for s in polys {
        let l = poly_len(s);
        let shown = prefix(s, left.min(l));
        ribbon(out, &shown);
        if round && left > 0.0 && left < l {
            disc(out, shown[shown.len() - 1]);
        }
        left -= l;
        if left <= 0.0 {
            break;
        }
    }
}

impl Widget for Panel {
    fn style(&self) -> Style {
        Style {
            size: Size {
                width: Dimension::length(self.size),
                height: Dimension::length(self.size),
            },
            flex_shrink: 0.0,
            ..Style::default()
        }
    }

    fn paint(&self, cx: &mut PaintCx) {
        let r = cx.rect;
        cx.fill(r, 6.0, Color::hex(0xffffff));
        let Some(m) = pairing(cx, self.a, self.b) else {
            return;
        };
        let t = self.t.unwrap_or_else(|| {
            cx.request_frame();
            let s = CLOCK.with(|c| c.elapsed().as_secs_f32());
            // Two seconds each way, with a short hold at both ends.
            let f = (s % 5.0) / 5.0;
            let tri = if f < 0.5 { f * 2.0 } else { 2.0 - f * 2.0 };
            let v = ((tri - 0.1) / 0.8).clamp(0.0, 1.0);
            v * v * (3.0 - 2.0 * v)
        });
        let s = r.h * 0.72;
        let (ox, oy) = (r.x + r.w / 2.0, r.y + r.h * 0.84);
        let ink = Color::hex(0x1d2030);
        let place = |p: &Path| {
            let mut p = p.clone();
            for q in p.contours.iter_mut().flatten().flatten() {
                *q = [q[0] * s + ox, q[1] * s + oy];
            }
            p
        };
        let w = 0.012;
        match self.kind {
            Kind::SegmentsA | Kind::SegmentsB => {
                let (g, runs) = match self.kind {
                    Kind::SegmentsA => (&m.a, &m.runs_a),
                    _ => (&m.b, &m.runs_b),
                };
                cx.path(&place(&g.path), ink.alpha(0.12));
                for (run, c) in runs {
                    let mut p = Path::default();
                    ribbon(&mut p, &run_ink(run, w));
                    disc(&mut p, [run[0][0][0], run[0][0][1], w * 2.0]);
                    cx.path(&place(&p), Color::hex(COLORS[c % COLORS.len()]));
                }
            }
            Kind::SkeletonA => {
                cx.path(&place(&m.a.path), ink.alpha(0.12));
                for (i, st) in m.a.strokes.iter().enumerate() {
                    let mut p = Path::default();
                    ribbon(&mut p, &st.pts);
                    cx.path(&place(&p), Color::hex(COLORS[i % COLORS.len()]).alpha(0.75));
                }
            }
            Kind::Current => cx.path(&place(&m.current.at(t)), ink),
            Kind::SegmentDp => cx.path(&place(&lerped(&m.segs.0, &m.segs.1, t)), ink),
            Kind::PointDp => cx.path(&place(&lerped(&m.points.0, &m.points.1, t)), ink),
            Kind::Hybrid => cx.path(&place(&lerped(&m.hybrid.0, &m.hybrid.1, t)), ink),
            Kind::StrokeMorph => {
                // Ribbons only approximate the outlines, so hand over at the ends.
                let fade = (t / 0.12).min((1.0 - t) / 0.12).clamp(0.0, 1.0);
                if fade < 1.0 {
                    let g = if t < 0.5 { &m.a } else { &m.b };
                    cx.path(&place(&g.path), ink.alpha(1.0 - fade));
                }
                if fade > 0.0 {
                    let mut p = Path::default();
                    for (x, y) in &m.inks {
                        let v: Vec<Ink> = x
                            .iter()
                            .zip(y)
                            .map(|(u, v)| [0, 1, 2].map(|c| mix(u[c], v[c], t)))
                            .collect();
                        ribbon(&mut p, &v);
                    }
                    cx.path(&place(&p), ink.alpha(fade));
                }
            }
            Kind::Outline | Kind::Nearest => {
                let mut runs: Vec<Vec<Ink>> =
                    m.a.path
                        .contours
                        .iter()
                        .flat_map(|c| decompose(c))
                        .map(|r| run_ink(&r, w))
                        .collect();
                if self.kind == Kind::Nearest {
                    let strokes = runs
                        .drain(..)
                        .map(|pts| Stroke {
                            pts,
                            closed: false,
                            stub: false,
                            ends: [0, 0],
                        })
                        .collect();
                    runs = pen_order(strokes).into_iter().map(|s| s.pts).collect();
                }
                let mut p = Path::default();
                write_on(&mut p, &runs, (t / 0.8).min(1.0), false);
                cx.path(&place(&p), ink);
                if t > 0.8 {
                    cx.path(&place(&m.a.path), ink.alpha((t - 0.8) / 0.2));
                }
            }
            Kind::Route => {
                cx.path(&place(&m.a.path), ink.alpha(0.12));
                let thin = |poly: &[Ink], r: f32| -> Vec<Ink> {
                    poly.iter().map(|p| [p[0], p[1], r]).collect()
                };
                let mut over = Path::default();
                for poly in &m.a.route.retraced {
                    ribbon(&mut over, &thin(poly, w * 2.6));
                }
                cx.path(&place(&over), ink.alpha(0.55));
                for (i, d) in m.a.route.drawings.iter().enumerate() {
                    let color = Color::hex(COLORS[i % COLORS.len()]);
                    let mut p = Path::default();
                    ribbon(&mut p, &thin(d, w));
                    cx.path(&place(&p), color);
                    let mut dot = Path::default();
                    disc(&mut dot, [d[0][0], d[0][1], w * 3.5]);
                    cx.path(&place(&dot), ink);
                    let mut dot = Path::default();
                    disc(&mut dot, [d[0][0], d[0][1], w * 2.2]);
                    cx.path(&place(&dot), color);
                }
            }
            Kind::Planned => {
                let p = (t / 0.88).min(1.0);
                if p >= 1.0 {
                    cx.path(&place(&m.a.path), ink);
                } else {
                    let mask = reveal(&m.a.route.drawings, p);
                    cx.path_masked(&place(&m.a.path), &place(&mask), ink);
                }
            }
            Kind::Pen => {
                let polys: Vec<Vec<Ink>> = m.a.strokes.iter().map(|s| s.pts.clone()).collect();
                let mut p = Path::default();
                write_on(&mut p, &polys, (t / 0.88).min(1.0), true);
                let fade = ((t - 0.88) / 0.12).clamp(0.0, 1.0);
                cx.path(&place(&p), ink.alpha(1.0 - fade));
                if fade > 0.0 {
                    cx.path(&place(&m.a.path), ink.alpha(fade));
                }
            }
        }
    }
}

impl From<Panel> for Element {
    fn from(w: Panel) -> Self {
        Element::new(w)
    }
}

fn caption(s: impl Into<String>, width: f32) -> Element {
    text(s)
        .size(12.0)
        .color(Color::hex(0x6b7080))
        .width(width)
        .into()
}

fn labelled(label: &str, panel: Panel) -> Element {
    let w = panel.size;
    column([caption(label, w), panel.into()]).gap(4.0).into()
}

// ---- Live app ----

struct Lab;

struct S {
    a: char,
    b: char,
    t: f32,
    playing: bool,
}

impl App for Lab {
    fn view(&self) -> Element {
        scroll(stateful_with_lab().into()).grow(1.0).into()
    }
}

fn stateful_with_lab() -> impl Into<Element> {
    stateful_with(
        || S {
            a: 'D',
            b: 'O',
            t: 0.5,
            playing: true,
        },
        |s: &S, h| {
            let (a, b, t, size) = (
                s.a,
                s.b,
                s.playing.then_some(()).map_or(Some(s.t), |_| None),
                150.0,
            );
            let panel = |kind| Panel {
                kind,
                a,
                b,
                size,
                t,
            };
            let group = |items: &[(Kind, &str)]| -> Element {
                row(items
                    .iter()
                    .map(|&(k, l)| labelled(l, panel(k)))
                    .collect::<Vec<_>>())
                .gap(10.0)
                .into()
            };
            let chips: Vec<Element> = PAIRS
                .iter()
                .map(|p| {
                    let mut c = p.chars();
                    let (x, y) = (c.next().unwrap(), c.next().unwrap());
                    choice_chip(*p, (x, y) == (a, b))
                        .on_click(move |cx| {
                            cx.update(h, move |s: &mut S| (s.a, s.b) = (x, y));
                        })
                        .into()
                })
                .collect();
            column([
                row(chips).gap(6.0).into(),
                row([
                    text_input(format!("{a}{b}"))
                        .placeholder("two characters")
                        .on_change(move |cx, v: String| {
                            let mut c = v.chars();
                            if let (Some(x), Some(y)) = (c.next(), c.next()) {
                                cx.update(h, move |s: &mut S| (s.a, s.b) = (x, y));
                            }
                        })
                        .width(120.0)
                        .into(),
                    button(text(if s.playing { "Pause" } else { "Play" }))
                        .on_click(move |cx| cx.update(h, |s: &mut S| s.playing = !s.playing))
                        .into(),
                    slider(s.t, 0.0..=1.0)
                        .on_change(move |cx, v| {
                            cx.update(h, move |s: &mut S| {
                                s.t = v;
                                s.playing = false;
                            })
                        })
                        .grow(1.0)
                        .into(),
                ])
                .align(AlignItems::CENTER)
                .gap(10.0)
                .into(),
                group(&[
                    (Kind::SegmentsA, "Segments A (colour = match)"),
                    (Kind::SegmentsB, "Segments B"),
                    (Kind::SkeletonA, "Skeleton A"),
                ]),
                group(&MORPHS),
                group(&DRAWS),
            ])
            .gap(12.0)
            .padding(14.0)
        },
    )
}

// ---- Film strips ----

struct Strip(char, char);

const STEPS: usize = 7;

impl App for Strip {
    fn view(&self) -> Element {
        let (a, b, size) = (self.0, self.1, 96.0);
        let strip = |kind, label: &str| -> Element {
            let mut cells = vec![caption(label, 96.0)];
            cells.extend((0..STEPS).map(|i| {
                let t = Some(i as f32 / (STEPS - 1) as f32);
                Panel {
                    kind,
                    a,
                    b,
                    size,
                    t,
                }
                .into()
            }));
            row(cells).align(AlignItems::CENTER).gap(4.0).into()
        };
        let mut rows = vec![
            row([
                caption("Segments, skeleton", 96.0),
                Panel {
                    kind: Kind::SegmentsA,
                    a,
                    b,
                    size,
                    t: Some(0.0),
                }
                .into(),
                Panel {
                    kind: Kind::SegmentsB,
                    a,
                    b,
                    size,
                    t: Some(0.0),
                }
                .into(),
                Panel {
                    kind: Kind::SkeletonA,
                    a,
                    b,
                    size,
                    t: Some(0.0),
                }
                .into(),
                Panel {
                    kind: Kind::SkeletonA,
                    a: b,
                    b: a,
                    size,
                    t: Some(0.0),
                }
                .into(),
            ])
            .gap(4.0)
            .into(),
        ];
        rows.extend(MORPHS.iter().chain(&DRAWS).map(|&(k, l)| strip(k, l)));
        column(rows).gap(4.0).padding(8.0).into()
    }
    fn background(&self) -> Color {
        Color::hex(0xe8e9ef)
    }
}

/// Self-crossings of a path's flattened contours, the visible tears in a morph.
fn crossings(p: &Path) -> usize {
    let edges: Vec<(usize, Point, Point)> = p
        .contours
        .iter()
        .enumerate()
        .flat_map(|(c, quads)| {
            quads.iter().flat_map(move |q| {
                (0..4).map(move |k| (c, at(q, k as f32 / 4.0), at(q, (k + 1) as f32 / 4.0)))
            })
        })
        .filter(|(_, a, b)| dist(*a, *b) > 1e-5)
        .collect();
    let side = |a: Point, b: Point, p: Point| {
        let (d, q) = (sub(b, a), sub(p, a));
        d[0] * q[1] - d[1] * q[0]
    };
    let mut n = 0;
    for (i, e) in edges.iter().enumerate() {
        for f in edges.iter().skip(i + 2) {
            let touch = dist(e.1, f.2) < 1e-4 || dist(e.2, f.1) < 1e-4;
            if !touch
                && side(e.1, e.2, f.1) * side(e.1, e.2, f.2) < 0.0
                && side(f.1, f.2, e.1) * side(f.1, f.2, e.2) < 0.0
            {
                n += 1;
            }
        }
    }
    n
}

fn score(pairs: &[(char, char)]) {
    let mut totals = [(0usize, 0usize); 4];
    let (mut hybrid_torn, mut frames_by) = ([0usize; 2], [0usize; 2]);
    let (mut build, mut most) = (std::time::Duration::ZERO, 0);
    for &(a, b) in pairs {
        drop(Bench::new(Strip(a, b), (900.0, 1200.0)));
        let Some(m) = PAIRINGS.with_borrow(|p| p[&(a, b)].clone()) else {
            continue;
        };
        let start = Instant::now();
        for (x, y) in m.a.path.contours.iter().zip(&m.b.path.contours) {
            std::hint::black_box(hybrid(x, y));
        }
        build += start.elapsed();
        most = most.max(m.hybrid.0.contours.iter().map(Vec::len).sum::<usize>());
        for t in [0.2, 0.35, 0.5, 0.65, 0.8] {
            let paths = [
                m.current.at(t),
                lerped(&m.segs.0, &m.segs.1, t),
                lerped(&m.points.0, &m.points.1, t),
                lerped(&m.hybrid.0, &m.hybrid.1, t),
            ];
            let same = m.a.path.contours.len() == m.b.path.contours.len();
            hybrid_torn[usize::from(same)] += usize::from(crossings(&paths[3]) > 0);
            frames_by[usize::from(same)] += 1;
            for (s, p) in totals.iter_mut().zip(&paths) {
                let c = crossings(p);
                s.0 += c;
                s.1 += usize::from(c > 0);
            }
        }
    }
    let frames = pairs.len() * 5;
    println!(
        "hybrid torn: same contour count {}/{}, different {}/{}",
        hybrid_torn[1], frames_by[1], hybrid_torn[0], frames_by[0]
    );
    println!(
        "hybrid build {:.1} ms/pair, at most {most} curves",
        build.as_secs_f32() * 1000.0 / pairs.len() as f32
    );
    println!("{} pairs, {frames} frames", pairs.len());
    for (name, (c, f)) in ["current", "segment DP", "point DP", "hybrid"]
        .iter()
        .zip(totals)
    {
        println!("{name:12} {c:6} crossings, {f:4} torn frames");
    }
}

// ---- Alphabet sheet ----

struct Sheet(Vec<char>);

impl App for Sheet {
    fn view(&self) -> Element {
        let size = 70.0;
        let lines: Vec<Element> = self
            .0
            .chunks(4)
            .map(|line| {
                let cells: Vec<Element> = line
                    .iter()
                    .map(|&a| {
                        let cell = |kind, t| Panel {
                            kind,
                            a,
                            b: a,
                            size,
                            t: Some(t),
                        };
                        row([
                            cell(Kind::Route, 0.0).into(),
                            cell(Kind::Planned, 0.25).into(),
                            cell(Kind::Planned, 0.5).into(),
                            cell(Kind::Planned, 0.75).into(),
                        ])
                        .gap(2.0)
                        .into()
                    })
                    .collect();
                row(cells).gap(10.0).into()
            })
            .collect();
        column(lines).gap(4.0).padding(6.0).into()
    }
    fn background(&self) -> Color {
        Color::hex(0xe8e9ef)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--draw") {
        let chars: Vec<char> = args[2].chars().collect();
        let lines = chars.len().div_ceil(4) as f32;
        let mut bench = Bench::new(Sheet(chars.clone()), (1190.0, lines * 74.0 + 12.0));
        let p = bench.shot(1.5);
        image::save_buffer(
            &args[1],
            &p.rgba,
            p.width,
            p.height,
            image::ColorType::Rgba8,
        )
        .unwrap();
        for c in chars {
            if let Some(g) = GLYPHS.with_borrow(|m| m[&c].clone()) {
                print!("{c}:{}/{} ", g.route.drawings.len(), g.route.retraced.len());
            }
        }
        println!();
        return;
    }
    if args.first().is_some_and(|a| a == "--score") {
        let set: Vec<char> = ('A'..='Z').chain('a'..='z').chain('0'..='9').collect();
        let pairs: Vec<(char, char)> = (0..set.len())
            .flat_map(|i| {
                [
                    (set[i], set[(i + 1) % set.len()]),
                    (set[i], set[(i + 7) % set.len()]),
                ]
            })
            .collect();
        score(&pairs);
        return;
    }
    if args.first().is_some_and(|a| a == "--ui") {
        let p = Bench::new(Lab, (820.0, 640.0)).shot(1.5);
        image::save_buffer(
            &args[1],
            &p.rgba,
            p.width,
            p.height,
            image::ColorType::Rgba8,
        )
        .unwrap();
        return;
    }
    if args.first().is_some_and(|a| a == "--shot") {
        let prefix = &args[1];
        for pair in &args[2..] {
            let mut c = pair.chars();
            let (a, b) = (c.next().unwrap(), c.next().unwrap());
            let width = 8.0 * 100.0 + 12.0;
            let height = 11.0 * 100.0 + 16.0;
            let mut bench = Bench::new(Strip(a, b), (width, height));
            let p = bench.shot(1.5);
            let file = format!("{prefix}-{:x}-{:x}.png", a as u32, b as u32);
            image::save_buffer(&file, &p.rgba, p.width, p.height, image::ColorType::Rgba8).unwrap();
            let g = GLYPHS.with_borrow(|m| m[&a].clone()).unwrap();
            println!(
                "{pair}: {} segs, worst fit {:.4} em, {} strokes -> {file}",
                g.fits,
                g.worst_fit,
                g.strokes.len()
            );
        }
        return;
    }
    dfui::run(Lab);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arm(dir: Point, len: f32, id: usize) -> Stroke {
        let pts = (0..=20)
            .map(|k| {
                let d = len * k as f32 / 20.0;
                [dir[0] * d, dir[1] * d, 0.05]
            })
            .collect();
        Stroke {
            pts,
            closed: false,
            stub: false,
            ends: [0, id],
        }
    }

    fn star(len: f32) -> Route {
        plan(vec![
            arm([-1.0, 0.0], len, 1),
            arm([1.0, 0.0], len, 2),
            arm([0.0, 1.0], len, 3),
        ])
    }

    #[test]
    fn short_lines_are_retraced_and_long_ones_lifted() {
        let short = star(0.1);
        assert_eq!((short.drawings.len(), short.retraced.len()), (1, 1));
        let long = star(0.8);
        assert_eq!((long.drawings.len(), long.retraced.len()), (2, 0));
    }

    #[test]
    fn a_stub_on_the_right_finishes_the_drawing() {
        let mut stub = arm([0.0, -1.0], 0.08, 1);
        stub.stub = true;
        let route = plan(vec![
            arm([-1.0, 0.0], 0.5, 2),
            stub,
            arm([0.0, 1.0], 0.3, 3),
        ]);
        let last = route.drawings.last().unwrap();
        let tip = last[last.len() - 1];
        assert!(tip[1] < -0.07, "ended at {tip:?}");
    }

    #[test]
    fn the_pen_runs_straight_through_a_junction() {
        let route = plan(vec![
            arm([0.0, 1.0], 0.5, 1),
            arm([0.0, -1.0], 0.5, 2),
            arm([1.0, 0.0], 0.5, 3),
        ]);
        let first = &route.drawings[0];
        let far = first.iter().position(|p| p[1] < -0.45).unwrap();
        assert!(first[..far].iter().all(|p| p[0] < 0.05), "{first:?}");
    }

    #[test]
    fn the_first_stroke_starts_on_a_free_end() {
        let route = plan(vec![
            arm([0.0, 1.0], 0.1, 1),
            arm([1.0, 0.0], 0.6, 2),
            arm([0.0, -1.0], 0.6, 3),
        ]);
        let p = route.drawings[0][0];
        assert!(dist([p[0], p[1]], [0.0, 0.0]) > 0.05, "started at {p:?}");
    }

    #[test]
    fn a_stub_ends_the_drawing_and_its_neighbour_is_retraced() {
        let mut stub = arm([0.0, -1.0], 0.08, 1);
        stub.stub = true;
        let route = plan(vec![stub, arm([0.0, 1.0], 0.3, 2), arm([1.0, 0.0], 0.5, 3)]);
        assert_eq!((route.drawings.len(), route.retraced.len()), (1, 1));
        assert!(poly_len(&route.retraced[0]) > 0.25);
    }
}
