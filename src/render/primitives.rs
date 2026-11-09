use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::anim::Lerp;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    pub const WHITE: Color = Color {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };
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

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = (self.x + self.w).min(o.x + o.w);
        let b = (self.y + self.h).min(o.y + o.h);
        Rect::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }

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
    pub width: u32,
    pub height: u32,
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

    pub fn width(&self) -> u32 {
        self.0.width
    }

    pub fn height(&self) -> u32 {
        self.0.height
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Shadow {
    pub color: Color,
    pub offset: (f32, f32),
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
