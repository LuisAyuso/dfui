use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::anim::Lerp;

/// An sRGB color with straight alpha, each channel in 0..=1. Blending
/// happens in sRGB, as in browsers.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Color {
    /// Red.
    pub r: f32,
    /// Green.
    pub g: f32,
    /// Blue.
    pub b: f32,
    /// Opacity: 0 transparent, 1 opaque.
    pub a: f32,
}

impl Color {
    /// Draws nothing.
    pub const TRANSPARENT: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    /// Opaque white.
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
    /// Opaque black.
    pub const BLACK: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };

    /// sRGB `0xRRGGBB`.
    pub const fn hex(rgb: u32) -> Color {
        Color {
            r: ((rgb >> 16) & 0xff) as f32 / 255.0,
            g: ((rgb >> 8) & 0xff) as f32 / 255.0,
            b: (rgb & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }

    /// The same color at opacity `a`, replacing the current one.
    pub const fn alpha(self, a: f32) -> Color {
        Color { a, ..self }
    }

    /// `self` at `t` 0, `to` at `t` 1, blended per channel, alpha included.
    pub fn mix(self, to: Color, t: f32) -> Color {
        self.lerp(to, t)
    }

    /// Moves `t` of the way to white, keeping alpha.
    pub fn lighten(self, t: f32) -> Color {
        self.mix(Color::WHITE.alpha(self.a), t)
    }

    /// Moves `t` of the way to black, keeping alpha.
    pub fn darken(self, t: f32) -> Color {
        self.mix(Color::BLACK.alpha(self.a), t)
    }

    pub(super) fn rgba(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
}

/// An axis-aligned box in logical px, y growing downward.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// A box with its top-left corner at `x`, `y`.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    /// Whether the point is inside: left and top edges count, right and bottom don't.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    /// The overlap of both boxes; zero-sized where they don't meet.
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = (self.x + self.w).min(o.x + o.w);
        let b = (self.y + self.h).min(o.y + o.h);
        Rect::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }

    /// The smallest box containing both.
    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        let r = (self.x + self.w).max(o.x + o.w);
        let b = (self.y + self.h).max(o.y + o.h);
        Rect::new(x, y, r - x, b - y)
    }
}

/// A rendered frame: sRGB RGBA8, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Four bytes per pixel, `width * height` pixels.
    pub rgba: Vec<u8>,
}

#[cfg(test)]
impl Pixels {
    pub(super) fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        self.rgba[i..i + 4].try_into().unwrap()
    }
}

/// Immutable RGBA8 pixels, cheap to clone and safe to build on any thread.
/// Decoding is up to the app; the GPU copy lives until the last clone drops.
#[derive(Clone)]
pub struct Bitmap(pub(super) Arc<BitmapData>);

pub(super) struct BitmapData {
    pub(super) id: u32,
    pub(super) width: u32,
    pub(super) height: u32,
    /// Premultiplied sRGB.
    pub(super) rgba: Box<[u8]>,
}

impl Bitmap {
    /// Straight-alpha sRGB RGBA8, row-major, as image decoders produce it.
    pub fn from_rgba8(width: u32, height: u32, mut rgba: Vec<u8>) -> Bitmap {
        static NEXT: AtomicU32 = AtomicU32::new(1);
        assert_eq!(
            rgba.len() as u64,
            width as u64 * height as u64 * 4,
            "rgba size"
        );
        for p in rgba.as_chunks_mut::<4>().0 {
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
        Bitmap(Arc::new(BitmapData {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            width,
            height,
            rgba: rgba.into(),
        }))
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.0.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.0.height
    }
}

/// A soft shadow: the shape's box moved by `offset`, its edge faded over `blur`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shadow {
    /// Color at the shadow's darkest, inside the box.
    pub color: Color,
    /// Shift from the shape, logical px; positive is right and down.
    pub offset: (f32, f32),
    /// Logical px over which the edge fades, on each side of it.
    pub blur: f32,
}

#[cfg(test)]
mod tests {
    use super::Bitmap;

    #[test]
    #[should_panic(expected = "rgba size")]
    fn bitmap_size_check_does_not_overflow() {
        Bitmap::from_rgba8(65536, 65536, Vec::new());
    }
}
