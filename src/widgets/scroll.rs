use std::time::Duration;

use taffy::{Display, Overflow, Point, Style};
use web_time::Instant;

use super::Styled;
use crate::widget::{Element, Event, EventCx, PaintCx, State, Widget};

const DRAG_SLOP: f32 = 6.0;
/// Fling speed decays as e^(-t/τ) seconds, so a fling travels v·τ in total.
const FRICTION_TAU: f32 = 0.35;
/// Release speeds below this (px/s) don't fling.
const MIN_FLING: f32 = 50.0;
/// A fling stops once it slows below this (px/s).
const STOP_SPEED: f32 = 10.0;
/// Only pointer samples this recent count toward the release velocity, so a
/// drag that pauses before lifting doesn't fling.
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);

#[derive(Default)]
struct ScrollState {
    offset: f32,
    drag: Option<(f32, f32)>,
    dragging: bool,
    /// Recent (time, pointer y) while dragging.
    samples: Vec<(Instant, f32)>,
    fling: Option<Fling>,
}

#[derive(Clone, Copy)]
struct Fling {
    start: Instant,
    from: f32,
    /// Offset speed at release, px/s.
    v: f32,
    max: f32,
}

impl Fling {
    /// Offset at `now`, and whether the fling is still moving.
    fn at(&self, now: Instant) -> (f32, bool) {
        let t = now.saturating_duration_since(self.start).as_secs_f32();
        let decay = (-t / FRICTION_TAU).exp();
        let free = self.from + self.v * FRICTION_TAU * (1.0 - decay);
        let offset = free.clamp(0.0, self.max);
        (
            offset,
            offset == free && (self.v * decay).abs() > STOP_SPEED,
        )
    }
}

/// Px/s of the pointer over the samples within `VELOCITY_WINDOW` of `now`.
fn velocity(samples: &[(Instant, f32)], now: Instant) -> f32 {
    let recent: Vec<_> = samples
        .iter()
        .filter(|(t, _)| now.saturating_duration_since(*t) <= VELOCITY_WINDOW)
        .collect();
    let (Some((t0, y0)), Some((t1, y1))) = (recent.first(), recent.last()) else {
        return 0.0;
    };
    let dt = t1.saturating_duration_since(*t0).as_secs_f32();
    if dt > 0.0 { (y1 - y0) / dt } else { 0.0 }
}

/// Vertical scroll view; drag (touch, flings on release) or wheel. Children are clipped.
pub struct Scroll {
    child: [Element; 1],
    style: Style,
}

pub fn scroll(child: impl Into<Element>) -> Scroll {
    Scroll {
        child: [child.into()],
        style: Style {
            display: Display::Block,
            overflow: Point {
                x: Overflow::Hidden,
                y: Overflow::Scroll,
            },
            scrollbar_width: 0.0,
            ..Style::default()
        },
    }
}

impl From<Scroll> for Element {
    fn from(w: Scroll) -> Self {
        Element::new(w)
    }
}

impl Styled for Scroll {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Scroll {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element] {
        &self.child
    }
    fn scroll(&self, state: &mut State, now: Instant) -> Option<(f32, f32)> {
        let s = state.get::<ScrollState>();
        if let Some(f) = s.fling {
            let (offset, moving) = f.at(now);
            s.offset = offset;
            if !moving {
                s.fling = None;
            }
        }
        Some((0.0, s.offset))
    }
    fn paint(&self, cx: &mut PaintCx) {
        if cx.state.get::<ScrollState>().fling.is_some() {
            cx.request_placement();
        }
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let max = (cx.content.height - cx.rect.h).max(0.0);
        let hit = |x, y| cx.hit(x, y);
        let now = cx.now;
        let (mut capture, mut redraw, mut stop) = (false, false, false);
        match *ev {
            Event::PointerDown { x, y } if hit(x, y) => {
                let s = cx.state.get::<ScrollState>();
                // ponytail: a press that catches a fling also reaches the widget under it.
                s.fling = None;
                s.drag = Some((y, s.offset));
                s.samples = vec![(now, y)];
            }
            Event::PointerMove { y, .. } => {
                let s = cx.state.get::<ScrollState>();
                if let Some((start, from)) = s.drag {
                    s.samples
                        .retain(|(t, _)| now.saturating_duration_since(*t) <= VELOCITY_WINDOW);
                    s.samples.push((now, y));
                    if !s.dragging && (y - start).abs() > DRAG_SLOP {
                        s.dragging = true;
                        capture = true;
                    }
                    if s.dragging {
                        s.offset = (from - (y - start)).clamp(0.0, max);
                        redraw = true;
                    }
                }
            }
            Event::PointerUp { y, .. } => {
                let s = cx.state.get::<ScrollState>();
                if s.dragging {
                    s.samples.push((now, y));
                    // Content moves opposite to the finger.
                    let v = -velocity(&s.samples, now);
                    if v.abs() > MIN_FLING {
                        s.fling = Some(Fling {
                            start: now,
                            from: s.offset,
                            v,
                            max,
                        });
                        redraw = true;
                    }
                }
                s.drag = None;
                s.dragging = false;
                s.samples.clear();
            }
            Event::Cancel => {
                let s = cx.state.get::<ScrollState>();
                s.drag = None;
                s.dragging = false;
                s.samples.clear();
            }
            Event::Wheel { x, y, dy } if hit(x, y) => {
                let s = cx.state.get::<ScrollState>();
                s.fling = None;
                s.offset = (s.offset + dy).clamp(0.0, max);
                redraw = true;
                stop = true;
            }
            _ => {}
        }
        if capture {
            cx.capture();
        }
        if redraw {
            cx.request_redraw();
        }
        if stop {
            cx.stop();
        }
    }
}
