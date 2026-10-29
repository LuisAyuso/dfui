use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use ttf_parser::Face;

use crate::outline::Outlines;
use crate::render::{Color, Rect, Scene};

static DEFAULT_FONT: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

#[derive(Clone, Copy)]
struct Glyph {
    /// Em-space quad corners, y up.
    em: [f32; 4],
    /// Quad relative to pen position on the baseline, in em, y down.
    plane: Rect,
    /// Band header offset, horizontal and vertical band counts.
    bands: [f32; 3],
}

pub struct PositionedGlyph {
    pub c: char,
    pub x: f32,
    /// Baseline.
    pub y: f32,
}

pub struct TextLayout {
    pub glyphs: Vec<PositionedGlyph>,
    pub width: f32,
    pub height: f32,
}

struct Cached {
    text: String,
    layout: Rc<TextLayout>,
    used: Cell<bool>,
}

/// Font plus lazily-built glyph outlines, rendered analytically on the GPU.
pub struct Text {
    face: Face<'static>,
    glyphs: HashMap<char, Option<Glyph>>,
    /// Layouts by hash of (text, size, max width); `view` rebuilds the same
    /// strings every frame and taffy measures each label several times.
    layouts: RefCell<HashMap<u64, Cached>>,
    pub(crate) outlines: Outlines,
}

impl Default for Text {
    fn default() -> Self {
        Text {
            face: Face::parse(DEFAULT_FONT, 0).expect("bundled font"),
            glyphs: HashMap::new(),
            layouts: RefCell::default(),
            outlines: Outlines::default(),
        }
    }
}

impl Text {
    fn em(&self) -> f32 {
        self.face.units_per_em() as f32
    }

    pub fn line_height(&self, size: f32) -> f32 {
        (self.face.ascender() - self.face.descender() + self.face.line_gap()) as f32 / self.em()
            * size
    }

    fn advance(&self, c: char, size: f32) -> f32 {
        self.face
            .glyph_index(c)
            .and_then(|g| self.face.glyph_hor_advance(g))
            .unwrap_or(0) as f32
            / self.em()
            * size
    }

    fn kern(&self, a: char, b: char, size: f32) -> f32 {
        let (Some(l), Some(r)) = (self.face.glyph_index(a), self.face.glyph_index(b)) else {
            return 0.0;
        };
        let Some(kern) = self.face.tables().kern else {
            return 0.0;
        };
        let k = kern
            .subtables
            .into_iter()
            .filter(|s| s.horizontal && !s.variable)
            .find_map(|s| s.glyphs_kerning(l, r))
            .unwrap_or(0);
        k as f32 / self.em() * size
    }

    fn word_width(&self, w: &str, size: f32) -> f32 {
        let mut x = 0.0;
        let mut prev = None;
        for c in w.chars() {
            if let Some(p) = prev {
                x += self.kern(p, c, size);
            }
            x += self.advance(c, size);
            prev = Some(c);
        }
        x
    }

    /// Greedy word wrap at spaces and explicit newlines. No shaping or bidi.
    /// Cached until a frame passes without it being used.
    pub fn layout(&self, s: &str, size: f32, max_width: Option<f32>) -> Rc<TextLayout> {
        let mut h = DefaultHasher::new();
        (s, size.to_bits(), max_width.map(f32::to_bits)).hash(&mut h);
        let key = h.finish();
        let mut cache = self.layouts.borrow_mut();
        if let Some(c) = cache.get(&key)
            && c.text == s
        {
            c.used.set(true);
            return c.layout.clone();
        }
        let layout = Rc::new(self.shape(s, size, max_width));
        let used = Cell::new(true);
        let text = s.to_owned();
        cache.insert(
            key,
            Cached {
                text,
                layout: layout.clone(),
                used,
            },
        );
        layout
    }

    /// Drops cached layouts not used since the last call; the runtime calls
    /// this once per frame.
    pub fn end_frame(&self) {
        self.layouts
            .borrow_mut()
            .retain(|_, c| c.used.replace(false));
    }

    fn shape(&self, s: &str, size: f32, max_width: Option<f32>) -> TextLayout {
        let lh = self.line_height(size);
        let ascent = self.face.ascender() as f32 / self.em() * size;
        let space = self.advance(' ', size);
        let max = max_width.unwrap_or(f32::INFINITY);
        let mut glyphs = Vec::new();
        let mut width: f32 = 0.0;
        let mut y = ascent;
        for line in s.split('\n') {
            let mut x = 0.0;
            for (i, word) in line.split(' ').enumerate() {
                let ww = self.word_width(word, size);
                if x > 0.0 && x + space + ww > max {
                    width = width.max(x);
                    x = 0.0;
                    y += lh;
                } else if i > 0 {
                    x += space;
                }
                let mut prev = None;
                for c in word.chars() {
                    if let Some(p) = prev {
                        x += self.kern(p, c, size);
                    }
                    glyphs.push(PositionedGlyph { c, x, y });
                    x += self.advance(c, size);
                    prev = Some(c);
                }
            }
            width = width.max(x);
            y += lh;
        }
        TextLayout {
            glyphs,
            width: width.ceil(),
            height: (y - ascent).ceil(),
        }
    }

    /// x of every char boundary on one unwrapped line; `chars + 1` entries,
    /// matching `layout`'s spacing.
    pub(crate) fn caret_stops(&self, s: &str, size: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(s.len() + 1);
        let (mut x, mut prev) = (0.0, None);
        for c in s.chars() {
            if c != ' '
                && let Some(p) = prev
            {
                x += self.kern(p, c, size);
            }
            out.push(x);
            x += self.advance(c, size);
            prev = (c != ' ').then_some(c);
        }
        out.push(x);
        out
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        s: &str,
        size: f32,
        max_width: Option<f32>,
        origin: (f32, f32),
        color: Color,
        clip: Rect,
        scene: &mut Scene,
    ) {
        for g in &self.layout(s, size, max_width).glyphs {
            let Some(gl) = self.glyph(g.c) else { continue };
            let r = Rect::new(
                origin.0 + g.x + gl.plane.x * size,
                origin.1 + g.y + gl.plane.y * size,
                gl.plane.w * size,
                gl.plane.h * size,
            );
            scene.glyph(r, gl.em, color, clip, gl.bands);
        }
    }

    fn glyph(&mut self, c: char) -> Option<Glyph> {
        if let Some(g) = self.glyphs.get(&c) {
            return *g;
        }
        let g = self.outline(c);
        self.glyphs.insert(c, g);
        g
    }

    fn outline(&mut self, c: char) -> Option<Glyph> {
        let g = self.outlines.add(&self.face, self.face.glyph_index(c)?)?;
        let [x0, y0, x1, y1] = g.bbox;
        Some(Glyph {
            em: [x0, y1, x1, y0],
            plane: Rect::new(x0, -y1, x1 - x0, y1 - y0),
            bands: [g.header as f32, g.bands_h as f32, g.bands_v as f32],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_width() {
        let t = Text::default();
        let one = t.layout("hello world", 20.0, None);
        assert_eq!(one.height, t.line_height(20.0).ceil());
        let two = t.layout("hello world", 20.0, Some(one.width - 1.0));
        assert!(two.height > one.height);
        assert!(two.width < one.width);
    }

    #[test]
    fn caret_stops_match_layout() {
        let t = Text::default();
        let s = "Ta ké";
        let stops = t.caret_stops(s, 20.0);
        assert_eq!(stops.len(), s.chars().count() + 1);
        assert!(stops.windows(2).all(|w| w[0] <= w[1]));
        let l = t.layout(s, 20.0, None);
        assert_eq!(stops[3], l.glyphs[2].x);
        assert_eq!(stops[5].ceil(), l.width);
        let lead = t.caret_stops("  a", 20.0);
        assert_eq!(lead[2], t.layout("  a", 20.0, None).glyphs[0].x);
    }

    #[test]
    fn layout_cache_hits_and_evicts_unused() {
        let t = Text::default();
        let a = t.layout("cached", 16.0, None);
        assert!(Rc::ptr_eq(&a, &t.layout("cached", 16.0, None)));
        assert!(!Rc::ptr_eq(&a, &t.layout("cached", 17.0, None)));
        t.end_frame();
        assert!(Rc::ptr_eq(&a, &t.layout("cached", 16.0, None)));
        t.end_frame();
        t.end_frame();
        assert!(!Rc::ptr_eq(&a, &t.layout("cached", 16.0, None)));
    }
}
