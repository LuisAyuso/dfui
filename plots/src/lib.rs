//! Experimental plots for dfui in the spirit of ggplot2: a [`plot`] of some
//! rows holds [`Layer`]s, each mapping rows to aesthetics (x, y, group,
//! ohlc), summarizing them with a [`Stat`] and drawing them as a [`Geom`].
//! Pass fresh rows on every rebuild and the plot follows them.

use std::ops::Range;
use std::rc::Rc;

use dfui::path::{Point, Stroke};
use dfui::taffy::{Dimension, Size, Style};
use dfui::widgets::Styled;
use dfui::{Color, Element, Font, Ink, PaintCx, Path, Rect, Widget};

const FONT: f32 = 12.0;
/// Fraction of the space between neighbouring x values a bar, box or candle takes.
const BAND: f32 = 0.8;
/// Segments per stroked path: the shader walks a band's curves per pixel.
const CHUNK: usize = 16;
const PALETTE: [u32; 6] = [0x3b6fe0, 0xe0703b, 0x2e9d5b, 0xb04fd0, 0xd64545, 0x2aa7b8];
const UP: Color = Color::hex(0x2e9d5b);
const DOWN: Color = Color::hex(0xd64545);

type Num<T> = Rc<dyn Fn(&T) -> f64>;
type Ohlc<T> = Rc<dyn Fn(&T) -> [f64; 4]>;
type Group<T> = Rc<dyn Fn(&T) -> u64>;

/// How a layer draws its summarized data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Geom {
    /// Connects each group's points in x order, straight or along a smooth
    /// curve through them.
    Line {
        smooth: bool,
        width: f32,
    },
    Point {
        radius: f32,
    },
    /// From zero to y; groups at one x stand side by side.
    Bar {
        radius: f32,
    },
    /// Quartiles, whiskers at 1.5 IQR and outliers; use with [`Stat::Box`].
    Box,
    /// Open/close body and high/low wick from the `ohlc` aesthetic.
    Candle,
}

/// How a layer summarizes rows that share a group and x before drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stat {
    /// Every row as it is.
    Identity,
    Mean,
    Sum,
    /// The number of rows; y is ignored.
    Count,
    Min,
    Max,
    /// Quartiles and whiskers, for [`Geom::Box`].
    Box,
}

/// One summarized value, in data units.
#[derive(Clone, Debug, PartialEq)]
struct Datum {
    group: u64,
    x: f64,
    y: f64,
    lo: f64,
    hi: f64,
    /// Box: first and third quartile. Candle: open and close.
    a: f64,
    b: f64,
    outliers: Vec<f64>,
}

impl Datum {
    fn new(group: u64, x: f64, y: f64) -> Datum {
        Datum {
            group,
            x,
            y,
            lo: y,
            hi: y,
            a: y,
            b: y,
            outliers: Vec::new(),
        }
    }
}

/// A geom with its aesthetics and stat; see [`line`], [`point`], [`bar`],
/// [`boxplot`] and [`candle`].
pub struct Layer<T> {
    geom: Geom,
    stat: Stat,
    x: Option<Num<T>>,
    y: Option<Num<T>>,
    ohlc: Option<Ohlc<T>>,
    group: Option<Group<T>>,
    color: Option<Ink>,
}

fn layer<T>(geom: Geom, stat: Stat) -> Layer<T> {
    Layer {
        geom,
        stat,
        x: None,
        y: None,
        ohlc: None,
        group: None,
        color: None,
    }
}

/// A line through each group's points, sharp at the samples.
pub fn line<T>() -> Layer<T> {
    layer(
        Geom::Line {
            smooth: false,
            width: 2.0,
        },
        Stat::Identity,
    )
}

pub fn point<T>() -> Layer<T> {
    layer(Geom::Point { radius: 3.0 }, Stat::Identity)
}

pub fn bar<T>() -> Layer<T> {
    layer(Geom::Bar { radius: 0.0 }, Stat::Identity)
}

/// Box and whiskers of the y values at each x.
pub fn boxplot<T>() -> Layer<T> {
    layer(Geom::Box, Stat::Box)
}

/// Candlesticks; set [`Layer::ohlc`].
pub fn candle<T>() -> Layer<T> {
    layer(Geom::Candle, Stat::Identity)
}

impl<T> Layer<T> {
    /// Position along x; the row's index without it.
    pub fn x(mut self, f: impl Fn(&T) -> f64 + 'static) -> Self {
        self.x = Some(Rc::new(f));
        self
    }

    pub fn y(mut self, f: impl Fn(&T) -> f64 + 'static) -> Self {
        self.y = Some(Rc::new(f));
        self
    }

    /// Open, high, low and close, for [`candle`].
    pub fn ohlc(mut self, f: impl Fn(&T) -> [f64; 4] + 'static) -> Self {
        self.ohlc = Some(Rc::new(f));
        self
    }

    /// Splits rows into series, each its own line and color.
    pub fn group(mut self, f: impl Fn(&T) -> u64 + 'static) -> Self {
        self.group = Some(Rc::new(f));
        self
    }

    pub fn stat(mut self, stat: Stat) -> Self {
        self.stat = stat;
        self
    }

    /// One color for every group, instead of one each from the palette.
    pub fn color(mut self, ink: impl Into<Ink>) -> Self {
        self.color = Some(ink.into());
        self
    }

    /// A line rounded through its samples instead of sharp at them.
    pub fn smooth(mut self) -> Self {
        if let Geom::Line { smooth, .. } = &mut self.geom {
            *smooth = true;
        }
        self
    }

    /// Line width, logical px.
    pub fn width(mut self, w: f32) -> Self {
        if let Geom::Line { width, .. } = &mut self.geom {
            *width = w;
        }
        self
    }

    /// Point radius, or a bar's corner radius, logical px.
    pub fn radius(mut self, r: f32) -> Self {
        match &mut self.geom {
            Geom::Point { radius } | Geom::Bar { radius } => *radius = r,
            _ => {}
        }
        self
    }

    /// The rows as data, sorted by group and x and summarized by the stat.
    fn compute(&self, rows: &[T]) -> Vec<Datum> {
        let mut d: Vec<Datum> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let x = self.x.as_ref().map_or(i as f64, |f| f(r));
                let g = self.group.as_ref().map_or(0, |f| f(r));
                match &self.ohlc {
                    Some(f) => {
                        let [o, h, l, c] = f(r);
                        Datum {
                            lo: l,
                            hi: h,
                            a: o,
                            b: c,
                            ..Datum::new(g, x, c)
                        }
                    }
                    None => Datum::new(g, x, self.y.as_ref().map_or(0.0, |f| f(r))),
                }
            })
            .filter(|d| d.x.is_finite() && d.y.is_finite())
            .collect();
        d.sort_by(|a, b| a.group.cmp(&b.group).then(a.x.total_cmp(&b.x)));
        if self.stat == Stat::Identity {
            return d;
        }
        d.chunk_by(|a, b| a.group == b.group && a.x == b.x)
            .map(|run| summarize(self.stat, run))
            .collect()
    }
}

fn summarize(stat: Stat, run: &[Datum]) -> Datum {
    let ys = run.iter().map(|d| d.y);
    let n = run.len() as f64;
    let y = match stat {
        Stat::Identity | Stat::Box => 0.0,
        Stat::Mean => ys.sum::<f64>() / n,
        Stat::Sum => ys.sum(),
        Stat::Count => n,
        Stat::Min => ys.fold(f64::INFINITY, f64::min),
        Stat::Max => ys.fold(f64::NEG_INFINITY, f64::max),
    };
    let d = Datum::new(run[0].group, run[0].x, y);
    if stat != Stat::Box {
        return d;
    }
    let mut v: Vec<f64> = run.iter().map(|d| d.y).collect();
    v.sort_by(f64::total_cmp);
    let (q1, q3) = (quantile(&v, 0.25), quantile(&v, 0.75));
    let fence = 1.5 * (q3 - q1);
    let inside = || {
        v.iter()
            .copied()
            .filter(|&y| y >= q1 - fence && y <= q3 + fence)
    };
    Datum {
        y: quantile(&v, 0.5),
        lo: inside().fold(f64::INFINITY, f64::min),
        hi: inside().fold(f64::NEG_INFINITY, f64::max),
        a: q1,
        b: q3,
        outliers: v
            .iter()
            .copied()
            .filter(|&y| y < q1 - fence || y > q3 + fence)
            .collect(),
        ..d
    }
}

/// Linearly interpolated quantile of sorted, non-empty `v`.
fn quantile(v: &[f64], q: f64) -> f64 {
    let p = q * (v.len() - 1) as f64;
    let (i, t) = (p.floor() as usize, p.fract());
    v[i] + (v[(i + 1).min(v.len() - 1)] - v[i]) * t
}

/// Round tick values inside `lo..hi`, about `n` of them, and their step.
fn ticks(lo: f64, hi: f64, n: usize) -> (Vec<f64>, f64) {
    let raw = (hi - lo) / n.max(1) as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|k| k * mag)
        .find(|&s| s >= raw)
        .unwrap_or(10.0 * mag);
    let first = (lo / step).ceil() as i64;
    let last = (hi / step).floor() as i64;
    ((first..=last).map(|i| i as f64 * step).collect(), step)
}

fn label(v: f64, step: f64) -> String {
    let decimals = (-step.log10().floor()).max(0.0) as usize;
    format!("{v:.decimals$}")
}

/// At most the first and last, lowest and highest point of each px column,
/// in their order, so a line over many samples keeps its envelope.
fn decimate(p: &[Point]) -> Vec<Point> {
    let mut out = Vec::new();
    for col in p.chunk_by(|a, b| a[0].floor() == b[0].floor()) {
        let by_y = |a: &&Point, b: &&Point| a[1].total_cmp(&b[1]);
        let lo = col.iter().min_by(by_y).unwrap();
        let hi = col.iter().max_by(by_y).unwrap();
        let mut keep = vec![col[0], *lo, *hi, col[col.len() - 1]];
        keep.sort_by(|a, b| {
            let at = |q: &Point| col.iter().position(|c| c == q);
            at(a).cmp(&at(b))
        });
        keep.dedup();
        out.extend(keep);
    }
    out
}

/// A monotone cubic (Fritsch–Carlson) through `p`, x increasing, sampled
/// into a polyline: it passes through every point and never overshoots them.
fn smooth(p: &[Point]) -> Vec<Point> {
    let n = p.len();
    if n < 3 {
        return p.to_vec();
    }
    let delta: Vec<f32> = p
        .windows(2)
        .map(|w| {
            let dx = w[1][0] - w[0][0];
            if dx > 0.0 {
                (w[1][1] - w[0][1]) / dx
            } else {
                0.0
            }
        })
        .collect();
    let mut m: Vec<f32> = (0..n)
        .map(|i| match i {
            0 => delta[0],
            _ if i == n - 1 => delta[n - 2],
            _ if delta[i - 1] * delta[i] <= 0.0 => 0.0,
            _ => (delta[i - 1] + delta[i]) / 2.0,
        })
        .collect();
    for i in 0..n - 1 {
        if delta[i] == 0.0 {
            (m[i], m[i + 1]) = (0.0, 0.0);
            continue;
        }
        let (a, b) = (m[i] / delta[i], m[i + 1] / delta[i]);
        let s = a * a + b * b;
        if s > 9.0 {
            let t = 3.0 / s.sqrt();
            (m[i], m[i + 1]) = (t * a * delta[i], t * b * delta[i]);
        }
    }
    let mut out = vec![p[0]];
    for i in 0..n - 1 {
        let ([x0, y0], [x1, y1]) = (p[i], p[i + 1]);
        let h = x1 - x0;
        let steps = (h / 3.0).clamp(1.0, 8.0) as usize;
        for k in 1..=steps {
            let t = k as f32 / steps as f32;
            let (t2, t3) = (t * t, t * t * t);
            let y = (2.0 * t3 - 3.0 * t2 + 1.0) * y0
                + (t3 - 2.0 * t2 + t) * h * m[i]
                + (-2.0 * t3 + 3.0 * t2) * y1
                + (t3 - t2) * h * m[i + 1];
            out.push([x0 + t * h, y]);
        }
    }
    out
}

/// A plot of `data`; add [`Layer`]s with [`Plot::layer`]. Fills its parent's
/// space by default.
pub fn plot<T: 'static>(data: &Rc<[T]>) -> Plot<T> {
    Plot {
        data: data.clone(),
        layers: Vec::new(),
        x_range: None,
        y_range: None,
        x_label: None,
        y_label: None,
        style: Style {
            size: Size {
                width: Dimension::percent(1.0),
                height: Dimension::percent(1.0),
            },
            ..Style::default()
        },
    }
}

/// See [`plot`].
pub struct Plot<T> {
    data: Rc<[T]>,
    layers: Vec<Layer<T>>,
    x_range: Option<Range<f64>>,
    y_range: Option<Range<f64>>,
    x_label: Option<String>,
    y_label: Option<String>,
    style: Style,
}

impl<T> Plot<T> {
    /// Drawn over the layers added before it.
    pub fn layer(mut self, l: Layer<T>) -> Self {
        self.layers.push(l);
        self
    }

    /// Fixes the x axis instead of fitting the data.
    pub fn x_range(mut self, r: Range<f64>) -> Self {
        self.x_range = Some(r);
        self
    }

    pub fn y_range(mut self, r: Range<f64>) -> Self {
        self.y_range = Some(r);
        self
    }

    pub fn x_label(mut self, s: impl Into<String>) -> Self {
        self.x_label = Some(s.into());
        self
    }

    pub fn y_label(mut self, s: impl Into<String>) -> Self {
        self.y_label = Some(s.into());
        self
    }
}

impl<T: 'static> From<Plot<T>> for Element {
    fn from(p: Plot<T>) -> Self {
        Element::new(p)
    }
}

impl<T> Styled for Plot<T> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

/// Least distance between neighbouring x values, or 1 with only one.
fn spacing(d: &[Datum]) -> f64 {
    let mut xs: Vec<f64> = d.iter().map(|d| d.x).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    let s = xs
        .windows(2)
        .map(|w| w[1] - w[0])
        .fold(f64::INFINITY, f64::min);
    if s.is_finite() { s } else { 1.0 }
}

/// `r` widened by `k` of its span each side, or by 1 when it's a point.
fn pad(r: Range<f64>, k: f64) -> Range<f64> {
    let span = r.end - r.start;
    if span > 0.0 {
        r.start - span * k..r.end + span * k
    } else {
        r.start - 1.0..r.end + 1.0
    }
}

impl<T: 'static> Widget for Plot<T> {
    fn style(&self) -> Style {
        self.style.clone()
    }

    // ponytail: recomputes stats and scales every paint, fine to tens of
    // thousands of rows; cache by data pointer in state if that shows up.
    fn paint(&self, cx: &mut PaintCx) {
        let layers: Vec<Vec<Datum>> = self.layers.iter().map(|l| l.compute(&self.data)).collect();
        let banded = |l: &Layer<T>| matches!(l.geom, Geom::Bar { .. } | Geom::Box | Geom::Candle);
        let (mut x, mut y) = (
            [f64::INFINITY, f64::NEG_INFINITY],
            [f64::INFINITY, f64::NEG_INFINITY],
        );
        for (l, d) in self.layers.iter().zip(&layers) {
            let half = if banded(l) { spacing(d) / 2.0 } else { 0.0 };
            for d in d {
                x = [x[0].min(d.x - half), x[1].max(d.x + half)];
                let out = d.outliers.iter().copied();
                for v in out.chain([d.lo, d.hi]) {
                    y = [y[0].min(v), y[1].max(v)];
                }
            }
            if matches!(l.geom, Geom::Bar { .. }) {
                y = [y[0].min(0.0), y[1].max(0.0)];
            }
        }
        if !x[0].is_finite() {
            (x, y) = ([0.0, 1.0], [0.0, 1.0]);
        }
        let xr = self.x_range.clone().unwrap_or(x[0]..x[1]);
        let xr = if xr.end > xr.start { xr } else { pad(xr, 0.0) };
        let yr = self
            .y_range
            .clone()
            .unwrap_or_else(|| pad(y[0]..y[1], 0.05));

        let r = cx.rect;
        let (xt, xs) = ticks(xr.start, xr.end, (r.w / 80.0).max(2.0) as usize);
        let (yt, ys) = ticks(yr.start, yr.end, (r.h / 50.0).max(2.0) as usize);
        let line_h = cx.measure("0", Font::DEFAULT, FONT, None).height;
        let y_labels: Vec<String> = yt.iter().map(|&v| label(v, ys)).collect();
        let label_w = y_labels
            .iter()
            .map(|s| cx.measure(s, Font::DEFAULT, FONT, None).width)
            .fold(0.0, f32::max);
        let top = if self.y_label.is_some() {
            line_h + 4.0
        } else {
            8.0
        };
        let bottom = line_h
            + 6.0
            + if self.x_label.is_some() {
                line_h + 2.0
            } else {
                0.0
            };
        let left = label_w + 8.0;
        let area = Rect::new(
            r.x + left,
            r.y + top,
            (r.w - left - 12.0).max(1.0),
            (r.h - top - bottom).max(1.0),
        );
        let px = |v: f64| area.x + ((v - xr.start) / (xr.end - xr.start)) as f32 * area.w;
        let py = |v: f64| area.y + area.h - ((v - yr.start) / (yr.end - yr.start)) as f32 * area.h;

        let (ink, muted, divider) = (cx.theme.ink, cx.theme.muted, cx.theme.divider);
        for (v, s) in yt.iter().zip(&y_labels) {
            let p = py(*v);
            cx.fill(Rect::new(area.x, p - 0.5, area.w, 1.0), 0.0, divider);
            let w = cx.measure(s, Font::DEFAULT, FONT, None).width;
            cx.text(
                s,
                Font::DEFAULT,
                FONT,
                muted,
                None,
                (area.x - 6.0 - w, p - line_h / 2.0),
            );
        }
        for v in &xt {
            let (p, s) = (px(*v), label(*v, xs));
            cx.fill(Rect::new(p - 0.5, area.y, 1.0, area.h), 0.0, divider);
            let w = cx.measure(&s, Font::DEFAULT, FONT, None).width;
            cx.text(
                &s,
                Font::DEFAULT,
                FONT,
                muted,
                None,
                (p - w / 2.0, area.y + area.h + 4.0),
            );
        }
        if let Some(s) = &self.x_label {
            let w = cx.measure(s, Font::DEFAULT, FONT, None).width;
            let at = (area.x + (area.w - w) / 2.0, r.y + r.h - line_h);
            cx.text(s, Font::DEFAULT, FONT, ink, None, at);
        }
        if let Some(s) = &self.y_label {
            cx.text(s, Font::DEFAULT, FONT, ink, None, (r.x, r.y));
        }

        cx.with_clip(area, |cx| {
            for (l, d) in self.layers.iter().zip(&layers) {
                let groups: Vec<u64> = {
                    let mut g: Vec<u64> = d.iter().map(|d| d.group).collect();
                    g.dedup();
                    g
                };
                let color = |cx: &PaintCx, g: u64| match l.color {
                    Some(ink) => cx.color(ink),
                    None => {
                        let gi = groups.iter().position(|&x| x == g).unwrap_or(0);
                        Color::hex(PALETTE[gi % PALETTE.len()])
                    }
                };
                let band = (spacing(d) as f32 * area.w / (xr.end - xr.start) as f32) * BAND;
                match l.geom {
                    Geom::Line {
                        smooth: round,
                        width,
                    } => {
                        for run in d.chunk_by(|a, b| a.group == b.group) {
                            let c = color(cx, run[0].group);
                            let pts: Vec<Point> = run.iter().map(|d| [px(d.x), py(d.y)]).collect();
                            let pts = decimate(&pts);
                            let pts = if round { smooth(&pts) } else { pts };
                            stroke(cx, &pts, width, c);
                        }
                    }
                    Geom::Point { radius } => {
                        for d in d {
                            let c = color(cx, d.group);
                            cx.fill(
                                Rect::new(
                                    px(d.x) - radius,
                                    py(d.y) - radius,
                                    2.0 * radius,
                                    2.0 * radius,
                                ),
                                radius,
                                c,
                            );
                        }
                    }
                    Geom::Bar { radius } => {
                        let w = band / groups.len() as f32;
                        for d in d {
                            let gi = groups.iter().position(|&g| g == d.group).unwrap_or(0);
                            let x0 = px(d.x) - band / 2.0 + gi as f32 * w;
                            let (a, b) = (py(0.0), py(d.y));
                            let rect = Rect::new(x0, a.min(b), w, (a - b).abs());
                            cx.fill(rect, radius.min(w / 2.0), color(cx, d.group));
                        }
                    }
                    Geom::Box => {
                        for d in d {
                            let c = color(cx, d.group);
                            let x = px(d.x);
                            cx.fill(
                                Rect::new(x - 0.75, py(d.hi), 1.5, py(d.lo) - py(d.hi)),
                                0.0,
                                c,
                            );
                            let body = Rect::new(x - band / 2.0, py(d.b), band, py(d.a) - py(d.b));
                            cx.fill_bordered(body, 2.0, c.mix(cx.theme.background, 0.75), 1.5, c);
                            cx.fill(Rect::new(x - band / 2.0, py(d.y) - 1.0, band, 2.0), 0.0, c);
                            for &o in &d.outliers {
                                cx.fill(Rect::new(x - 2.5, py(o) - 2.5, 5.0, 5.0), 2.5, c);
                            }
                        }
                    }
                    Geom::Candle => {
                        for d in d {
                            let c = match l.color {
                                Some(ink) => cx.color(ink),
                                None if d.b >= d.a => UP,
                                None => DOWN,
                            };
                            let x = px(d.x);
                            cx.fill(
                                Rect::new(x - 0.5, py(d.hi), 1.0, py(d.lo) - py(d.hi)),
                                0.0,
                                c,
                            );
                            let (o, k) = (py(d.a), py(d.b));
                            let body =
                                Rect::new(x - band / 2.0, o.min(k), band, (o - k).abs().max(1.0));
                            cx.fill(body, 0.0, c);
                        }
                    }
                }
            }
        });
    }
}

/// `pts` stroked in pieces of [`CHUNK`] segments that share their ends.
// ponytail: transient paths each frame; a static line could be a cached Shape.
fn stroke(cx: &mut PaintCx, pts: &[Point], width: f32, c: Color) {
    let mut i = 0;
    while i + 1 < pts.len() {
        let end = (i + CHUNK).min(pts.len() - 1);
        cx.path(&Path::stroke(&pts[i..=end], Stroke::round(width)), c);
        i = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_round() {
        assert_eq!(
            ticks(0.0, 10.0, 5),
            (vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0], 2.0)
        );
        let (t, s) = ticks(0.13, 0.91, 4);
        assert_eq!(s, 0.2);
        assert_eq!(t.len(), 4);
        assert_eq!(label(0.4, s), "0.4");
    }

    #[test]
    fn stats_summarize_by_group_and_x() {
        let rows = [(1.0, 2.0, 0), (1.0, 4.0, 0), (2.0, 5.0, 0), (1.0, 10.0, 1)];
        let ys = |s| {
            let l = bar::<(f64, f64, u64)>()
                .x(|r| r.0)
                .y(|r| r.1)
                .group(|r| r.2)
                .stat(s);
            l.compute(&rows).iter().map(|d| d.y).collect::<Vec<_>>()
        };
        assert_eq!(ys(Stat::Mean), [3.0, 5.0, 10.0]);
        assert_eq!(ys(Stat::Sum), [6.0, 5.0, 10.0]);
        assert_eq!(ys(Stat::Count), [2.0, 1.0, 1.0]);
        assert_eq!(ys(Stat::Identity).len(), 4);
    }

    #[test]
    fn box_stat_has_quartiles_and_outliers() {
        let rows: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0, 5.0, 100.0];
        let d = boxplot::<f64>().x(|_| 0.0).y(|v| *v).compute(&rows);
        assert_eq!(d.len(), 1);
        assert_eq!((d[0].a, d[0].y, d[0].b), (2.25, 3.5, 4.75));
        assert_eq!((d[0].lo, d[0].hi), (1.0, 5.0));
        assert_eq!(d[0].outliers, [100.0]);
    }

    #[test]
    fn smooth_passes_through_samples_without_overshoot() {
        let p = [
            [0.0, 0.0],
            [30.0, 10.0],
            [60.0, 10.0],
            [90.0, 0.0],
            [120.0, 5.0],
        ];
        let s = smooth(&p);
        for q in p {
            assert!(
                s.iter()
                    .any(|r| (r[0] - q[0]).abs() < 1e-3 && (r[1] - q[1]).abs() < 1e-3)
            );
        }
        assert!(s.iter().all(|r| (0.0..=10.0).contains(&r[1])));
        assert!(s.len() > p.len());
    }

    #[test]
    fn decimate_keeps_envelope() {
        let p: Vec<Point> = (0..1000)
            .map(|i| [i as f32 / 100.0, (i % 7) as f32])
            .collect();
        let d = decimate(&p);
        assert!(d.len() <= 4 * 10);
        assert!(d.iter().any(|q| q[1] == 6.0) && d.iter().any(|q| q[1] == 0.0));
    }
}
