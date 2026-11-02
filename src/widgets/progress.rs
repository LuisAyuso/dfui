use std::f32::consts::{PI, TAU};
use std::time::Duration;

use taffy::{AvailableSpace, Size, Style};
use web_time::Instant;

use super::Styled;
use crate::anim::{Animated, ease};
use crate::path::{Path, Point, Quad};
use crate::render::{Color, Rect};
use crate::text::Text;
use crate::widget::{Element, PaintCx, Widget};

const ACCENT: Color = Color::hex(0x3b6fe0);
const TRACK: Color = Color::hex(0xd5d8e0);
/// How long a determinate bar eases to a new value.
const STEP: Duration = Duration::from_millis(250);
/// One sweep of the wait bar.
const SWEEP: f32 = 1.6;
/// One grow-and-shrink of the spinner's arc.
const CYCLE: f32 = 1.4;

#[derive(Default)]
struct ProgressState {
    start: Option<Instant>,
    shown: Vec<Animated<f32>>,
}

/// A horizontal bar: one or more fractions filled over a track, or, with
/// none, a segment sweeping across while the length of a wait is unknown.
pub struct Progress {
    /// Fractions in 0..=1, each with its color.
    layers: Vec<(f32, Color)>,
    accent: Color,
    track: Color,
    style: Style,
}

/// A bar filled to `value` in 0..=1; changes ease in.
pub fn progress(value: f32) -> Progress {
    stacked_progress([(value, ACCENT)])
}

/// Several fractions in one bar, e.g. played over buffered, or tasks running
/// at different speeds. Smaller values are drawn on top, so each shows.
pub fn stacked_progress(layers: impl IntoIterator<Item = (f32, Color)>) -> Progress {
    Progress {
        layers: layers.into_iter().collect(),
        accent: ACCENT,
        track: TRACK,
        style: Style::default(),
    }
}

/// A bar for a wait of unknown length: a segment sweeps across, looping.
pub fn wait_bar() -> Progress {
    stacked_progress([])
}

impl Progress {
    /// Color of a single-value bar or the wait bar's segment.
    pub fn accent(mut self, c: Color) -> Self {
        self.accent = c;
        if let [(_, color)] = self.layers.as_mut_slice() {
            *color = c;
        }
        self
    }
    pub fn track(mut self, c: Color) -> Self {
        self.track = c;
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
        cx.fill(r, r.h / 2.0, self.track);
        let s = cx.state.get::<ProgressState>();
        if self.layers.is_empty() {
            let t = now.saturating_duration_since(*s.start.get_or_insert(now));
            let p = (t.as_secs_f32() / SWEEP).fract();
            // The head leads and the tail catches up, so the segment grows
            // out of the left edge and shrinks into the right one.
            let (tail, head) = (p * p * p, ease::out_cubic(p));
            cx.request_frame();
            return segment(cx, r, tail, head, self.accent);
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
            segment(cx, r, 0.0, v, color);
        }
    }
}

/// A spinning arc for a wait of unknown length: it grows and shrinks while
/// it turns.
pub struct Spinner {
    size: f32,
    color: Color,
    style: Style,
}

/// A 24 px spinner.
pub fn spinner() -> Spinner {
    Spinner {
        size: 24.0,
        color: ACCENT,
        style: Style::default(),
    }
}

impl Spinner {
    pub fn size(mut self, px: f32) -> Self {
        self.size = px;
        self
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
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

/// Start and end of the arc in turns, `t` seconds in. Each cycle the head
/// runs ahead and the tail catches up 3/4 turn later, on top of a slow spin.
fn arc_span(t: f32) -> (f32, f32) {
    let (k, c) = ((t / CYCLE).floor(), (t / CYCLE).fract());
    let grow = ease::in_out_cubic((c * 2.0).min(1.0));
    let shrink = ease::in_out_cubic((c * 2.0 - 1.0).max(0.0));
    let spin = t / 2.0;
    let a0 = (k + shrink) * 0.75 + spin;
    let a1 = (k + grow) * 0.75 + spin + 0.05;
    (a0, a1)
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

/// A ring segment from `a0` to `a1` with round caps, `w` thick.
fn stroke_arc(c: Point, r: f32, w: f32, a0: f32, a1: f32) -> Path {
    let (ro, ri, h) = (r + w / 2.0, r - w / 2.0, w / 2.0);
    let cap = |a: f32| [c[0] + r * a.cos(), c[1] + r * a.sin()];
    let edges = arc(c, ro, a0, a1)
        .chain(arc(cap(a1), h, a1, a1 + PI))
        .chain(arc(c, ri, a1, a0))
        .chain(arc(cap(a0), h, a0 + PI, a0 + TAU))
        .collect();
    Path {
        contours: vec![edges],
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
        let (a0, a1) = arc_span(t.as_secs_f32());
        let w = self.size / 9.0;
        let c = [r.x + r.w / 2.0, r.y + r.h / 2.0];
        // Turn 0 at 12 o'clock.
        let rad = |turns: f32| turns * TAU - PI / 2.0;
        let path = stroke_arc(c, (self.size - w) / 2.0, w, rad(a0), rad(a1));
        cx.path(&path, self.color);
        cx.request_frame();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_arc_is_continuous_across_cycles() {
        let (e, d) = (1e-3, 1e-4);
        let (b0, b1) = arc_span(CYCLE - d);
        let (n0, n1) = arc_span(CYCLE + d);
        assert!((b0 - n0).abs() < e && (b1 - n1).abs() < e);
        let (a0, a1) = arc_span(CYCLE * 0.5);
        assert!(a1 - a0 > 0.75, "fully grown at half cycle");
    }

    #[test]
    fn stroked_arc_closes() {
        let p = stroke_arc([0.0, 0.0], 10.0, 2.0, 0.0, 2.0);
        let q = &p.contours[0];
        for (a, b) in q.iter().zip(q.iter().cycle().skip(1)) {
            let gap = (a[2][0] - b[0][0]).hypot(a[2][1] - b[0][1]);
            assert!(gap < 1e-4, "gap {gap}");
        }
    }
}
