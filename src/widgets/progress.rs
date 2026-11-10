use std::f32::consts::{PI, TAU};
use std::time::Duration;

use taffy::{AvailableSpace, Size, Style};
use web_time::Instant;

use super::Styled;
use crate::anim::{Animated, ease};
use crate::path::{Path, Point, Quad};
use crate::render::{Color, Rect};
use crate::text::Text;
use crate::theme::{Ink, Role};
use crate::widget::{Element, PaintCx, Widget};

/// How long a determinate bar eases to a new value.
const STEP: Duration = Duration::from_millis(250);
/// One sweep of the wait bar.
const SWEEP: f32 = 1.6;
/// One draw-and-retract of the spinner's spiral.
const CYCLE: f32 = 2.0;
/// Turns of the spinner's spiral, outer end to centre.
const TURNS: f32 = 2.0;

#[derive(Default)]
struct ProgressState {
    start: Option<Instant>,
    shown: Vec<Animated<f32>>,
}

/// A horizontal bar: one or more fractions filled over a track, or, with
/// none, a segment sweeping across while the length of a wait is unknown.
pub struct Progress {
    /// Fractions in 0..=1, each with its color.
    layers: Vec<(f32, Ink)>,
    accent: Ink,
    track: Ink,
    style: Style,
}

/// A bar filled to `value` in 0..=1; changes ease in.
#[doc = example!("progress")]
pub fn progress(value: f32) -> Progress {
    let mut p = wait_bar();
    p.layers.push((value, Role::Accent.into()));
    p
}

/// Several fractions in one bar, e.g. played over buffered, or tasks running
/// at different speeds. Smaller values are drawn on top, so each shows.
pub fn stacked_progress(layers: impl IntoIterator<Item = (f32, Color)>) -> Progress {
    Progress {
        layers: layers.into_iter().map(|(v, c)| (v, c.into())).collect(),
        accent: Role::Accent.into(),
        track: Role::Track.into(),
        style: Style::default(),
    }
}

/// A bar for a wait of unknown length: a segment sweeps across, looping.
pub fn wait_bar() -> Progress {
    stacked_progress([])
}

impl Progress {
    /// Color of a single-value bar or the wait bar's segment.
    pub fn accent(mut self, c: impl Into<Ink>) -> Self {
        let c = c.into();
        self.accent = c;
        if let [(_, color)] = self.layers.as_mut_slice() {
            *color = c;
        }
        self
    }
    /// Color of the unfilled bar; [`Role::Track`] by default.
    pub fn track(mut self, c: impl Into<Ink>) -> Self {
        self.track = c.into();
        self
    }
}

impl From<Progress> for Element {
    fn from(w: Progress) -> Self {
        Element::new(w)
    }
}

impl Styled for Progress {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

/// Fills `x0..x1` of the bar `r`, rounded like the track.
fn segment(cx: &mut PaintCx, r: Rect, x0: f32, x1: f32, color: Color) {
    let w = (x1 - x0) * r.w;
    if w > 0.0 {
        let seg = Rect::new(r.x + x0 * r.w, r.y, w, r.h);
        cx.fill(seg, (w.min(r.h) / 2.0).max(0.0), color);
    }
}

impl Widget for Progress {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn measure(&self, known: Size<Option<f32>>, _: Size<AvailableSpace>, _: &Text) -> Size<f32> {
        Size {
            width: known.width.unwrap_or(160.0),
            height: known.height.unwrap_or(4.0),
        }
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (now, r) = (cx.now, cx.rect);
        cx.fill(r, r.h / 2.0, cx.color(self.track));
        let s = cx.state.get::<ProgressState>();
        if self.layers.is_empty() {
            let t = now.saturating_duration_since(*s.start.get_or_insert(now));
            let p = (t.as_secs_f32() / SWEEP).fract();
            // The head leads and the tail catches up, so the segment grows
            // out of the left edge and shrinks into the right one.
            let (tail, head) = (p * p * p, ease::out_cubic(p));
            cx.request_frame();
            let accent = cx.color(self.accent);
            return segment(cx, r, tail, head, accent);
        }
        s.shown.resize_with(self.layers.len(), Animated::default);
        let mut shown = Vec::with_capacity(self.layers.len());
        for (a, &(v, color)) in s.shown.iter_mut().zip(&self.layers) {
            a.to(v.clamp(0.0, 1.0), now, STEP, ease::out_cubic);
            shown.push((a.get(now), color));
        }
        if s.shown.iter().any(|a| a.running(now)) {
            cx.request_frame();
        }
        shown.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (v, color) in shown {
            let c = cx.color(color);
            segment(cx, r, 0.0, v, c);
        }
    }
}

/// A spiral for a wait of unknown length, thin outside and thick at the
/// centre: it draws in from the outer end, then retracts the same way.
pub struct Spinner {
    size: f32,
    color: Ink,
    style: Style,
}

/// A 24 px spinner.
pub fn spinner() -> Spinner {
    Spinner {
        size: 24.0,
        color: Role::Accent.into(),
        style: Style::default(),
    }
}

impl Spinner {
    /// Width and height, logical px; 24 by default.
    pub fn size(mut self, px: f32) -> Self {
        self.size = px;
        self
    }
    /// [`Role::Accent`] by default.
    pub fn color(mut self, c: impl Into<Ink>) -> Self {
        self.color = c.into();
        self
    }
}

impl From<Spinner> for Element {
    fn from(w: Spinner) -> Self {
        Element::new(w)
    }
}

impl Styled for Spinner {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

/// Tail and head along the spiral (0 outer end, 1 centre), `t` seconds in.
fn spiral_span(t: f32) -> (f32, f32) {
    let c = (t / CYCLE).fract();
    let head = ease::in_out_cubic((c * 2.0).min(1.0));
    let tail = ease::in_out_cubic((c * 2.0 - 1.0).max(0.0));
    (tail, head)
}

/// Circular arc around `c` from angle `a0` to `a1` (radians, either way) as
/// quadratics of at most 45° each.
fn arc(c: Point, r: f32, a0: f32, a1: f32) -> impl Iterator<Item = Quad> {
    let n = ((a1 - a0).abs() / (PI / 4.0)).ceil().max(1.0) as usize;
    let step = (a1 - a0) / n as f32;
    let at = move |a: f32, d: f32| [c[0] + d * a.cos(), c[1] + d * a.sin()];
    (0..n).map(move |i| {
        let a = a0 + step * i as f32;
        let ctrl = r / (step / 2.0).cos();
        [at(a, r), at(a + step / 2.0, ctrl), at(a + step, r)]
    })
}

/// Archimedean spiral of [`TURNS`] turns fitting a circle of radius `r`,
/// starting at 12 o'clock and winding clockwise to the centre.
struct Spiral {
    c: Point,
    r: f32,
}

impl Spiral {
    /// Gap between turns; the inner end sits half a gap from the centre.
    fn gap(&self) -> f32 {
        self.r / (TURNS + 0.5)
    }

    /// Half the line width at `u`: a hairline outside, most of a gap inside.
    fn half(&self, u: f32) -> f32 {
        self.gap() * (0.06 + 0.24 * u)
    }

    /// Point and unit normal at `u` (0 outer end, 1 inner end).
    fn at(&self, u: f32) -> (Point, Point) {
        let (inner, d) = (self.gap() / 2.0, TURNS * TAU);
        let rho = self.r - (self.r - inner) * u;
        let a = u * d - PI / 2.0;
        let (sin, cos) = a.sin_cos();
        let t = [
            -(self.r - inner) * cos - rho * d * sin,
            -(self.r - inner) * sin + rho * d * cos,
        ];
        let len = t[0].hypot(t[1]);
        let p = [self.c[0] + rho * cos, self.c[1] + rho * sin];
        (p, [-t[1] / len, t[0] / len])
    }

    /// Edge at `side` (±1) half widths off the line from `u0` to `u1`, as
    /// quadratics through each piece's midpoint.
    fn edge(&self, side: f32, u0: f32, u1: f32) -> impl Iterator<Item = Quad> + '_ {
        let e = move |u: f32| {
            let (p, n) = self.at(u);
            let h = side * self.half(u);
            [p[0] + n[0] * h, p[1] + n[1] * h]
        };
        let n = ((u1 - u0).abs() * TURNS * 16.0).ceil().max(1.0) as usize;
        let step = (u1 - u0) / n as f32;
        (0..n).map(move |i| {
            let u = u0 + step * i as f32;
            let (a, m, b) = (e(u), e(u + step / 2.0), e(u + step));
            let ctrl = [
                2.0 * m[0] - (a[0] + b[0]) / 2.0,
                2.0 * m[1] - (a[1] + b[1]) / 2.0,
            ];
            [a, ctrl, b]
        })
    }

    /// The line from `u0` to `u1` with round caps.
    fn stroke(&self, u0: f32, u1: f32) -> Path {
        let cap = |u: f32| {
            let (p, n) = self.at(u);
            (p, n[1].atan2(n[0]), self.half(u))
        };
        let ((p0, a0, h0), (p1, a1, h1)) = (cap(u0), cap(u1));
        // Each cap turns from one edge to the other through the line's end.
        let edges = self
            .edge(1.0, u0, u1)
            .chain(arc(p1, h1, a1, a1 - PI))
            .chain(self.edge(-1.0, u1, u0))
            .chain(arc(p0, h0, a0 + PI, a0))
            .collect();
        Path {
            contours: vec![edges],
        }
    }
}

impl Widget for Spinner {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn measure(&self, _: Size<Option<f32>>, _: Size<AvailableSpace>, _: &Text) -> Size<f32> {
        Size::length(self.size)
    }
    fn measure_key(&self) -> u64 {
        self.size.to_bits() as u64
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (now, r) = (cx.now, cx.rect);
        let s = cx.state.get::<ProgressState>();
        let t = now.saturating_duration_since(*s.start.get_or_insert(now));
        let (u0, u1) = spiral_span(t.as_secs_f32());
        let spiral = Spiral {
            c: [r.x + r.w / 2.0, r.y + r.h / 2.0],
            r: self.size / 2.0 - 0.5,
        };
        if u1 - u0 > 1e-3 {
            cx.path(&spiral.stroke(u0, u1), cx.color(self.color));
        }
        cx.request_frame();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spiral_draws_in_then_retracts() {
        assert_eq!(spiral_span(0.0), (0.0, 0.0), "starts empty");
        assert_eq!(spiral_span(CYCLE * 0.5), (0.0, 1.0), "drawn to the centre");
        let (t0, t1) = spiral_span(CYCLE * 0.75);
        assert!(t0 > 0.0 && t1 == 1.0, "retracts from the outer end");
        let (e0, e1) = spiral_span(CYCLE - 1e-4);
        assert!(e1 - e0 < 1e-3, "gone before the next cycle");
    }

    #[test]
    fn spiral_stroke_closes() {
        let s = Spiral {
            c: [0.0, 0.0],
            r: 20.0,
        };
        let q = &s.stroke(0.1, 0.9).contours[0];
        for (a, b) in q.iter().zip(q.iter().cycle().skip(1)) {
            let gap = (a[2][0] - b[0][0]).hypot(a[2][1] - b[0][1]);
            assert!(gap < 1e-3, "gap {gap}");
        }
        assert!(q.len() < 200, "{} curves", q.len());
    }
}
