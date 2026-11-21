//! The keyboard focus ring: one ring the runtime draws above everything, gliding from the
//! widget that had focus to the one that has it. Its look and motion are the constants below.

use std::time::Duration;

use web_time::Instant;

use crate::anim::ease;
use crate::render::{Color, Rect, Scene};

/// Stroke width, logical px.
const WIDTH: f32 = 2.5;
/// Space between the widget's edge and the ring.
const GAP: f32 = 3.0;
/// Time to glide to the next widget.
const GLIDE: Duration = Duration::from_millis(180);
const EASE: fn(f32) -> f32 = ease::out_cubic;
/// Opacity at rest; the ring fades in from 0 when focus first appears.
const ALPHA: f32 = 1.0;

/// A box and its corner radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Outline {
    pub rect: Rect,
    pub radius: f32,
}

impl Outline {
    fn lerp(a: Outline, b: Outline, t: f32) -> Outline {
        let l = |a: f32, b: f32| a + (b - a) * t;
        Outline {
            rect: Rect::new(
                l(a.rect.x, b.rect.x),
                l(a.rect.y, b.rect.y),
                l(a.rect.w, b.rect.w),
                l(a.rect.h, b.rect.h),
            ),
            radius: l(a.radius, b.radius),
        }
    }

    /// The radius, at most half the shorter side, so a pill's 999 glides like its real curve.
    fn round(self) -> Outline {
        let max = self.rect.w.min(self.rect.h) / 2.0;
        Outline {
            radius: self.radius.clamp(0.0, max.max(0.0)),
            ..self
        }
    }
}

#[derive(Default)]
pub(crate) struct Ring {
    id: Option<u64>,
    /// Where the glide to `id` started, `None` when the ring fades in.
    from: Option<Outline>,
    start: Option<Instant>,
    /// What the last frame drew.
    shown: Option<Outline>,
}

impl Ring {
    /// Draws the ring around `to`, the focused widget `id`'s outline, or nothing for `None`.
    /// Returns whether it's still moving.
    pub fn paint(
        &mut self,
        id: Option<u64>,
        to: Option<Outline>,
        color: Color,
        clip: Rect,
        now: Instant,
        scene: &mut Scene,
    ) -> bool {
        if id != self.id {
            self.id = id;
            self.from = self.shown;
            self.start = Some(now);
        }
        let Some(to) = to.map(Outline::round) else {
            self.shown = None;
            return false;
        };
        let t = self
            .start
            .map_or(1.0, |s| {
                now.saturating_duration_since(s).as_secs_f32() / GLIDE.as_secs_f32()
            })
            .min(1.0);
        let e = EASE(t);
        let (at, alpha) = match self.from {
            Some(from) => (Outline::lerp(from, to, e), ALPHA),
            None => (to, ALPHA * e),
        };
        self.shown = Some(at);
        let out = GAP + WIDTH;
        let r = at.rect;
        scene.shape(
            Rect::new(r.x - out, r.y - out, r.w + 2.0 * out, r.h + 2.0 * out),
            at.radius + out,
            Color::TRANSPARENT,
            WIDTH,
            color.alpha(alpha),
            clip,
        );
        t < 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outline(x: f32) -> Outline {
        Outline {
            rect: Rect::new(x, 0.0, 10.0, 10.0),
            radius: 2.0,
        }
    }

    #[test]
    fn glides_from_the_last_drawn_outline_and_settles() {
        let mut ring = Ring::default();
        let mut scene = Scene::default();
        let all = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        let t0 = Instant::now();
        // First focus fades in where it is.
        ring.paint(
            Some(1),
            Some(outline(0.0)),
            Color::BLACK,
            all,
            t0,
            &mut scene,
        );
        assert_eq!(ring.shown, Some(outline(0.0)));
        let done = t0 + GLIDE;
        assert!(!ring.paint(
            Some(1),
            Some(outline(0.0)),
            Color::BLACK,
            all,
            done,
            &mut scene
        ));
        // Next focus starts at the old outline and ends at the new one.
        assert!(ring.paint(
            Some(2),
            Some(outline(100.0)),
            Color::BLACK,
            all,
            done,
            &mut scene
        ));
        assert_eq!(ring.shown, Some(outline(0.0)));
        let mid = done + GLIDE / 2;
        ring.paint(
            Some(2),
            Some(outline(100.0)),
            Color::BLACK,
            all,
            mid,
            &mut scene,
        );
        let x = ring.shown.unwrap().rect.x;
        assert!(
            x > 50.0 && x < 100.0,
            "out_cubic is past halfway at half time: {x}"
        );
        let end = done + GLIDE;
        assert!(!ring.paint(
            Some(2),
            Some(outline(100.0)),
            Color::BLACK,
            all,
            end,
            &mut scene
        ));
        assert_eq!(ring.shown, Some(outline(100.0)));
    }
}
