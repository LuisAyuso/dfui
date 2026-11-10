//! Drag-to-scroll with fling, for widgets that scroll themselves.

use std::time::Duration;

use web_time::Instant;

/// Movement before a press becomes a drag, px.
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

/// One axis of drag and fling. Feed it pointer events with the widget's
/// scroll offset; it returns new offsets, which the widget clamps and stores.
#[derive(Default)]
pub struct Drag {
    /// Pointer and offset at the press.
    start: Option<(f32, f32)>,
    dragging: bool,
    /// Recent (time, pointer position) while pressed.
    samples: Vec<(Instant, f32)>,
    fling: Option<Fling>,
}

#[derive(Clone, Copy)]
struct Fling {
    start: Instant,
    from: f32,
    /// Offset speed at release, px/s.
    v: f32,
}

impl Drag {
    /// A press at `pos` with the content at `offset`; stops any fling.
    pub fn down(&mut self, pos: f32, offset: f32, now: Instant) {
        self.fling = None;
        self.start = Some((pos, offset));
        self.dragging = false;
        self.samples = vec![(now, pos)];
    }

    /// The offset following the pointer once it has moved past the drag
    /// slop, `None` before (still a tap) or without a press. Capture the
    /// pointer while it returns `Some`.
    pub fn moved(&mut self, pos: f32, now: Instant) -> Option<f32> {
        let (start, from) = self.start?;
        self.samples
            .retain(|(t, _)| now.saturating_duration_since(*t) <= VELOCITY_WINDOW);
        self.samples.push((now, pos));
        self.dragging |= (pos - start).abs() > DRAG_SLOP;
        // Content moves with the finger, so the offset moves against it.
        self.dragging.then_some(from - (pos - start))
    }

    /// Release at `pos`; flings from `offset` if the pointer was moving fast.
    /// Returns whether this was a drag rather than a tap.
    pub fn up(&mut self, pos: f32, offset: f32, now: Instant) -> bool {
        let dragged = std::mem::take(&mut self.dragging);
        if dragged {
            self.samples.push((now, pos));
            let v = -velocity(&self.samples, now);
            if v.abs() > MIN_FLING {
                self.fling = Some(Fling {
                    start: now,
                    from: offset,
                    v,
                });
            }
        }
        self.start = None;
        self.samples.clear();
        dragged
    }

    /// Drops the press, and with it the drag, without a fling.
    pub fn cancel(&mut self) {
        self.start = None;
        self.dragging = false;
        self.samples.clear();
    }

    /// Stops a fling, e.g. when the wheel or code moves the content.
    pub fn stop(&mut self) {
        self.fling = None;
    }

    /// Moves the press's start offset and a fling's origin by `d`, for when
    /// the content moves under the drag (an item above it collapses).
    pub fn shift(&mut self, d: f32) {
        if let Some((_, from)) = &mut self.start {
            *from += d;
        }
        if let Some(f) = &mut self.fling {
            f.from += d;
        }
    }

    /// A press is down (a tap or a drag in progress).
    pub fn pressed(&self) -> bool {
        self.start.is_some()
    }

    /// A fling started on release and hasn't ended or been stopped.
    pub fn flinging(&self) -> bool {
        self.fling.is_some()
    }

    /// Offset at `now` of a running fling, clamped to `0..=max`; the fling
    /// ends at an edge or once it's slow. Keep painting while it's `Some`.
    pub fn fling(&mut self, now: Instant, max: f32) -> Option<f32> {
        let f = self.fling?;
        let t = now.saturating_duration_since(f.start).as_secs_f32();
        let decay = (-t / FRICTION_TAU).exp();
        let free = f.from + f.v * FRICTION_TAU * (1.0 - decay);
        let offset = free.clamp(0.0, max.max(0.0));
        if offset != free || (f.v * decay).abs() <= STOP_SPEED {
            self.fling = None;
        }
        Some(offset)
    }
}

/// Px/s of the pointer over the samples within [`VELOCITY_WINDOW`] of `now`.
fn velocity(samples: &[(Instant, f32)], now: Instant) -> f32 {
    let mut recent = samples
        .iter()
        .filter(|(t, _)| now.saturating_duration_since(*t) <= VELOCITY_WINDOW);
    let (Some((t0, p0)), Some((t1, p1))) = (recent.next(), recent.next_back()) else {
        return 0.0;
    };
    let dt = t1.saturating_duration_since(*t0).as_secs_f32();
    if dt > 0.0 { (p1 - p0) / dt } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slop_then_follow_then_fling_to_the_edge() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut d = Drag::default();
        d.down(100.0, 40.0, at(0));
        assert_eq!(d.moved(104.0, at(10)), None, "within slop: still a tap");
        assert_eq!(d.moved(90.0, at(20)), Some(50.0));
        d.shift(-10.0);
        assert_eq!(d.moved(90.0, at(30)), Some(40.0));
        d.shift(10.0);
        // 60 px up in 40 ms: a fast fling towards larger offsets.
        assert_eq!(d.moved(40.0, at(60)), Some(100.0));
        assert!(d.up(40.0, 100.0, at(60)));
        let mid = d.fling(at(100), 1000.0).unwrap();
        assert!(mid > 100.0, "{mid}");
        assert_eq!(d.fling(at(2000), 300.0), Some(300.0), "stops at the edge");
        assert_eq!(d.fling(at(2016), 300.0), None);
    }

    #[test]
    fn tap_and_paused_drag_dont_fling() {
        let t0 = Instant::now();
        let at = |ms| t0 + Duration::from_millis(ms);
        let mut d = Drag::default();
        d.down(0.0, 0.0, at(0));
        assert!(d.pressed());
        assert!(!d.up(0.0, 0.0, at(50)), "a tap");
        assert!(!d.pressed());
        d.down(0.0, 0.0, at(100));
        d.moved(-80.0, at(120));
        // Held still for longer than the velocity window before lifting.
        assert!(d.up(-80.0, 80.0, at(400)));
        assert!(!d.flinging());
    }
}
