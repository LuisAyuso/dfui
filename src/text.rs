use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use ttf_parser::{Face, GlyphId};

use crate::outline::Outlines;
use crate::render::{Color, Rect, Scene};

/// TrueType/OpenType font data. Fonts are told apart by address, so keep each
/// one in a `static`; it is parsed on first use.
#[derive(Clone, Copy)]
pub struct Font(pub &'static [u8]);

impl Font {
    /// Bundled DejaVu Sans; also the fallback for chars a font lacks.
    pub const DEFAULT: Font = Font(include_bytes!("../assets/DejaVuSans.ttf"));
}

impl Default for Font {
    fn default() -> Self {
        Font::DEFAULT
    }
}

#[derive(Clone, Copy)]
struct Glyph {
    /// Em-space quad corners, y up.
    em: [f32; 4],
    /// Quad relative to pen position on the baseline, in em, y down.
    plane: Rect,
    /// Band header offset, horizontal and vertical band counts.
    bands: [f32; 3],
}

/// Index into `Text::faces`.
type FaceId = usize;

pub(crate) struct PositionedGlyph {
    face: FaceId,
    id: GlyphId,
    pub(crate) x: f32,
    /// Baseline.
    pub(crate) y: f32,
}

pub struct TextLayout {
    pub(crate) glyphs: Vec<PositionedGlyph>,
    pub width: f32,
    pub height: f32,
}

struct Cached {
    text: String,
    layout: Rc<TextLayout>,
    used: Cell<bool>,
}

/// Fonts plus lazily-built glyph outlines, rendered analytically on the GPU.
pub struct Text {
    /// Parsed fonts by first use; index 0 is `Font::DEFAULT`.
    // ponytail: leaks one parsed Face per distinct font; fine while apps use a handful.
    faces: RefCell<Vec<(*const u8, &'static Face<'static>)>>,
    glyphs: HashMap<(FaceId, GlyphId), Option<Glyph>>,
    /// Layouts by hash of (text, font, size, max width); `view` rebuilds the
    /// same strings every frame and taffy measures each label several times.
    layouts: RefCell<HashMap<u64, Cached>>,
    pub(crate) outlines: Outlines,
}

impl Default for Text {
    fn default() -> Self {
        let t = Text {
            faces: RefCell::default(),
            glyphs: HashMap::new(),
            layouts: RefCell::default(),
            outlines: Outlines::default(),
        };
        t.face_id(Font::DEFAULT);
        t
    }
}

impl Text {
    fn face_id(&self, font: Font) -> FaceId {
        let ptr = font.0.as_ptr();
        let mut faces = self.faces.borrow_mut();
        if let Some(i) = faces.iter().position(|(p, _)| *p == ptr) {
            return i;
        }
        let face = Face::parse(font.0, 0).expect("invalid font data");
        faces.push((ptr, Box::leak(Box::new(face))));
        faces.len() - 1
    }

    fn face(&self, id: FaceId) -> &'static Face<'static> {
        self.faces.borrow()[id].1
    }

    fn em(&self, face: FaceId) -> f32 {
        self.face(face).units_per_em() as f32
    }

    pub fn line_height(&self, font: Font, size: f32) -> f32 {
        let id = self.face_id(font);
        let f = self.face(id);
        (f.ascender() - f.descender() + f.line_gap()) as f32 / self.em(id) * size
    }

    /// `c` in `face`, else in the default font.
    fn resolve(&self, face: FaceId, c: char) -> Option<(FaceId, GlyphId)> {
        [face, 0]
            .into_iter()
            .find_map(|f| Some((f, self.face(f).glyph_index(c)?)))
    }

    fn advance(&self, face: FaceId, g: GlyphId, size: f32) -> f32 {
        self.face(face).glyph_hor_advance(g).unwrap_or(0) as f32 / self.em(face) * size
    }

    fn kern(&self, face: FaceId, l: GlyphId, r: GlyphId, size: f32) -> f32 {
        let Some(kern) = self.face(face).tables().kern else {
            return 0.0;
        };
        let k = kern
            .subtables
            .into_iter()
            .filter(|s| s.horizontal && !s.variable)
            .find_map(|s| s.glyphs_kerning(l, r))
            .unwrap_or(0);
        k as f32 / self.em(face) * size
    }

    /// Calls `f` with each char's glyph and kerned pen x from 0; returns the
    /// end x. Spaces and font switches break kerning.
    fn run(
        &self,
        face: FaceId,
        s: &str,
        size: f32,
        mut visit: impl FnMut(Option<(FaceId, GlyphId)>, f32),
    ) -> f32 {
        let (mut x, mut prev) = (0.0, None);
        for c in s.chars() {
            let g = self.resolve(face, c);
            if c != ' '
                && let (Some((pf, pg)), Some((f, g))) = (prev, g)
                && pf == f
            {
                x += self.kern(f, pg, g, size);
            }
            visit(g, x);
            x += g.map_or(0.0, |(f, g)| self.advance(f, g, size));
            prev = g.filter(|_| c != ' ');
        }
        x
    }

    /// Greedy word wrap at spaces and explicit newlines. No shaping or bidi.
    /// Cached until a frame passes without it being used.
    pub fn layout(&self, s: &str, font: Font, size: f32, max_width: Option<f32>) -> Rc<TextLayout> {
        let face = self.face_id(font);
        let mut h = DefaultHasher::new();
        (s, face, size.to_bits(), max_width.map(f32::to_bits)).hash(&mut h);
        let key = h.finish();
        if let Some(c) = self.layouts.borrow().get(&key)
            && c.text == s
        {
            c.used.set(true);
            return c.layout.clone();
        }
        let layout = Rc::new(self.shape(s, face, size, max_width));
        self.layouts.borrow_mut().insert(
            key,
            Cached {
                text: s.to_owned(),
                layout: layout.clone(),
                used: Cell::new(true),
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

    fn shape(&self, s: &str, face: FaceId, size: f32, max_width: Option<f32>) -> TextLayout {
        let f = self.face(face);
        let em = self.em(face);
        let lh = (f.ascender() - f.descender() + f.line_gap()) as f32 / em * size;
        let ascent = f.ascender() as f32 / em * size;
        let space = self.run(face, " ", size, |_, _| {});
        let max = max_width.unwrap_or(f32::INFINITY);
        let mut glyphs = Vec::new();
        let mut width: f32 = 0.0;
        let mut y = ascent;
        for line in s.split('\n') {
            let mut x = 0.0;
            for (i, word) in line.split(' ').enumerate() {
                let first = glyphs.len();
                let ww = self.run(face, word, size, |g, x| {
                    if let Some((face, id)) = g {
                        glyphs.push(PositionedGlyph {
                            face,
                            id,
                            x,
                            y: 0.0,
                        });
                    }
                });
                if x > 0.0 && x + space + ww > max {
                    width = width.max(x);
                    x = 0.0;
                    y += lh;
                } else if i > 0 {
                    x += space;
                }
                for g in &mut glyphs[first..] {
                    g.x += x;
                    g.y = y;
                }
                x += ww;
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
    pub(crate) fn caret_stops(&self, s: &str, font: Font, size: f32) -> Vec<f32> {
        let mut out = Vec::with_capacity(s.len() + 1);
        let end = self.run(self.face_id(font), s, size, |_, x| out.push(x));
        out.push(end);
        out
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw(
        &mut self,
        s: &str,
        font: Font,
        size: f32,
        max_width: Option<f32>,
        origin: (f32, f32),
        color: Color,
        clip: Rect,
        scene: &mut Scene,
    ) {
        for g in &self.layout(s, font, size, max_width).glyphs {
            let Some(gl) = self.glyph(g.face, g.id) else {
                continue;
            };
            let r = Rect::new(
                origin.0 + g.x + gl.plane.x * size,
                origin.1 + g.y + gl.plane.y * size,
                gl.plane.w * size,
                gl.plane.h * size,
            );
            scene.glyph(r, gl.em, color, clip, gl.bands);
        }
    }

    fn glyph(&mut self, face: FaceId, id: GlyphId) -> Option<Glyph> {
        if let Some(g) = self.glyphs.get(&(face, id)) {
            return *g;
        }
        let g = self.outlines.add(self.face(face), id).map(|g| {
            let [x0, y0, x1, y1] = g.bbox;
            Glyph {
                em: [x0, y1, x1, y0],
                plane: Rect::new(x0, -y1, x1 - x0, y1 - y0),
                bands: [g.header as f32, g.bands_h as f32, g.bands_v as f32],
            }
        });
        self.glyphs.insert((face, id), g);
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_width() {
        let t = Text::default();
        let one = t.layout("hello world", Font::DEFAULT, 20.0, None);
        assert_eq!(one.height, t.line_height(Font::DEFAULT, 20.0).ceil());
        let two = t.layout("hello world", Font::DEFAULT, 20.0, Some(one.width - 1.0));
        assert!(two.height > one.height);
        assert!(two.width < one.width);
    }

    #[test]
    fn caret_stops_match_layout() {
        let t = Text::default();
        let s = "Ta ké";
        let stops = t.caret_stops(s, Font::DEFAULT, 20.0);
        assert_eq!(stops.len(), s.chars().count() + 1);
        assert!(stops.windows(2).all(|w| w[0] <= w[1]));
        let l = t.layout(s, Font::DEFAULT, 20.0, None);
        assert_eq!(stops[3], l.glyphs[2].x);
        assert_eq!(stops[5].ceil(), l.width);
        let lead = t.caret_stops("  a", Font::DEFAULT, 20.0);
        assert_eq!(
            lead[2],
            t.layout("  a", Font::DEFAULT, 20.0, None).glyphs[0].x
        );
    }

    #[test]
    fn layout_cache_hits_and_evicts_unused() {
        let t = Text::default();
        let a = t.layout("cached", Font::DEFAULT, 16.0, None);
        assert!(Rc::ptr_eq(
            &a,
            &t.layout("cached", Font::DEFAULT, 16.0, None)
        ));
        assert!(!Rc::ptr_eq(
            &a,
            &t.layout("cached", Font::DEFAULT, 17.0, None)
        ));
        t.end_frame();
        assert!(Rc::ptr_eq(
            &a,
            &t.layout("cached", Font::DEFAULT, 16.0, None)
        ));
        t.end_frame();
        t.end_frame();
        assert!(!Rc::ptr_eq(
            &a,
            &t.layout("cached", Font::DEFAULT, 16.0, None)
        ));
    }

    #[test]
    fn missing_chars_fall_back_to_default_font() {
        static BOLD: Font = Font(include_bytes!("../assets/DejaVuSans-Bold.ttf"));
        let t = Text::default();
        // U+1D5A0 is in DejaVu Sans but not in its bold cut.
        let l = t.layout("a\u{1d5a0}", BOLD, 20.0, None);
        let faces: Vec<_> = l.glyphs.iter().map(|g| g.face).collect();
        assert_eq!(faces, [1, 0]);
        assert!(l.glyphs[1].x > 0.0);
        let again = t.layout("a", Font(BOLD.0), 20.0, None);
        assert_eq!(again.glyphs[0].face, 1);
    }
}
