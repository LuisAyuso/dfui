//! Time-based transitions for values drawn in `paint`.

use std::time::{Duration, Instant};

use crate::render::Color;

/// Values `Animated` can blend between.
pub trait Lerp: Copy {
    fn lerp(self, to: Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(self, to: f32, t: f32) -> f32 {
        self + (to - self) * t
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
}
