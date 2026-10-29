use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use harfrust::{Buffer, ShapeOptions, ShapePlan, ShapePlanKey, ShaperFont};
use read_fonts::model::Font as Face;
use read_fonts::types::GlyphId;

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

impl Hash for Font {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.0.as_ptr().hash(h);
    }
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
    /// Baseline plus the shaper's vertical offset, y down.
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
    /// Parsed fonts by first use, keyed by data address; index 0 is
    /// `Font::DEFAULT`.
    faces: RefCell<Vec<(*const u8, Face)>>,
    /// Shape plans are costly to build; one per font, script and direction.
    plans: RefCell<Vec<(FaceId, Rc<ShapePlan>)>>,
    buffer: RefCell<Option<Buffer>>,
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
            plans: RefCell::default(),
            buffer: RefCell::default(),
            glyphs: HashMap::new(),
            layouts: RefCell::default(),
            outlines: Outlines::default(),
        };
        t.face_id(Font::DEFAULT);
        t
    }
}

/// Line box of `f` scaled to `size`: (ascent, descent below baseline, gap).
fn line_box(f: &Face, size: f32) -> (f32, f32, f32) {
    // ponytail: a font without line metrics gets a zero-height line.
    let l = f.metrics().h_line().unwrap_or_default();
    let s = size / f.units_per_em() as f32;
    (
        l.ascender.to_f32() * s,
        -l.descender.to_f32() * s,
        l.line_gap.to_f32() * s,
    )
}

impl Text {
    fn face_id(&self, font: Font) -> FaceId {
        let ptr = font.0.as_ptr();
        let mut faces = self.faces.borrow_mut();
        if let Some(i) = faces.iter().position(|(p, _)| *p == ptr) {
            return i;
        }
        let face = Face::new(font.0, 0).expect("invalid font data");
        faces.push((ptr, face));
        faces.len() - 1
    }

    pub fn line_height(&self, font: Font, size: f32) -> f32 {
        let id = self.face_id(font);
        let (a, d, g) = line_box(&self.faces.borrow()[id].1, size);
        a + d + g
    }

    fn space(&self, face: FaceId, size: f32) -> f32 {
        let f = &self.faces.borrow()[face].1;
        let g = f.charmap().map_unicode(' ').unwrap_or_default();
        f.glyph_metrics().h_advance(g) * size / f.units_per_em() as f32
    }

    /// Shapes `word` (no spaces or newlines), appending its glyphs from x = 0
    /// and the (byte, x) where each cluster starts; returns its advance.
    /// Chars `face` lacks are shaped with the default font.
    fn word(
        &self,
        face: FaceId,
        word: &str,
        size: f32,
        glyphs: &mut Vec<PositionedGlyph>,
        clusters: &mut Vec<(usize, f32)>,
    ) -> f32 {
        let faces = self.faces.borrow();
        // ponytail: fallback picks a font per char, so a mark can split from its base.
        let pick = |c: char| {
            [face, 0]
                .into_iter()
                .find(|&f| faces[f].1.charmap().map_unicode(c).is_some())
                .unwrap_or(face)
        };
        let mut runs: Vec<(FaceId, usize, usize)> = Vec::new();
        for (i, c) in word.char_indices() {
            let f = pick(c);
            match runs.last_mut() {
                Some((rf, _, end)) if *rf == f => *end = i + c.len_utf8(),
                _ => runs.push((f, i, i + c.len_utf8())),
            }
        }
        let mut buf = self.buffer.borrow_mut();
        let mut x = 0.0;
        for (f, start, end) in runs {
            let font = &faces[f].1;
            let mut b = buf.take().unwrap_or_default();
            b.push_str(&word[start..end]);
            b.guess_segment_properties();
            let plan = self.plan(f, font, &b);
            harfrust::shape(
                &ShaperFont::new(font),
                &mut b,
                ShapeOptions::new().plan(Some(&plan)),
            )
            .expect("shape with a matching plan");
            let s = size / font.units_per_em() as f32;
            for (info, pos) in b.glyph_infos().iter().zip(b.glyph_positions()) {
                let byte = start + info.cluster as usize;
                if clusters.last().is_none_or(|&(c, _)| c != byte) {
                    clusters.push((byte, x));
                }
                glyphs.push(PositionedGlyph {
                    face: f,
                    id: GlyphId::new(info.glyph_id),
                    x: x + pos.x_offset as f32 * s,
                    y: -pos.y_offset as f32 * s,
                });
                x += pos.x_advance as f32 * s;
            }
            b.clear();
            *buf = Some(b);
        }
        x
    }

    fn plan(&self, face: FaceId, font: &Face, b: &Buffer) -> Rc<ShapePlan> {
        let key = ShapePlanKey::new(font, b.script(), b.direction());
        let mut plans = self.plans.borrow_mut();
        if let Some((_, p)) = plans.iter().find(|(f, p)| *f == face && key.matches(p)) {
            return p.clone();
        }
        let p = Rc::new(ShapePlan::new(font, b.direction(), b.script(), None, &[]));
        plans.push((face, p.clone()));
        p
    }

    /// Greedy word wrap at spaces and explicit newlines; each word is shaped
    /// on its own. No bidi. Cached until a frame passes without it being used.
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
        let (ascent, descent, gap) = line_box(&self.faces.borrow()[face].1, size);
        let lh = ascent + descent + gap;
        let space = self.space(face, size);
        let max = max_width.unwrap_or(f32::INFINITY);
        let (mut glyphs, mut clusters) = (Vec::new(), Vec::new());
        let mut width: f32 = 0.0;
        let mut y = ascent;
        for line in s.split('\n') {
            let mut x = 0.0;
            for (i, word) in line.split(' ').enumerate() {
                let first = glyphs.len();
                clusters.clear();
                let ww = self.word(face, word, size, &mut glyphs, &mut clusters);
                if x > 0.0 && x + space + ww > max {
                    width = width.max(x);
                    x = 0.0;
                    y += lh;
                } else if i > 0 {
                    x += space;
                }
                for g in &mut glyphs[first..] {
                    g.x += x;
                    g.y += y;
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
    /// matching `layout`'s spacing. A cluster's width is split evenly among
    /// its chars, so carets land inside ligatures.
    pub(crate) fn caret_stops(&self, s: &str, font: Font, size: f32) -> Vec<f32> {
        let face = self.face_id(font);
        let space = self.space(face, size);
        let (mut glyphs, mut clusters) = (Vec::new(), Vec::new());
        let mut out = Vec::with_capacity(s.len() + 1);
        let mut x = 0.0;
        for (i, word) in s.split(' ').enumerate() {
            if i > 0 {
                out.push(x);
                x += space;
            }
            clusters.clear();
            let w = self.word(face, word, size, &mut glyphs, &mut clusters);
            clusters.sort_by_key(|c| c.0);
            for (k, &(b, cx)) in clusters.iter().enumerate() {
                let (end, ex) = clusters.get(k + 1).copied().unwrap_or((word.len(), w));
                let n = word[b..end].chars().count();
                out.extend((0..n).map(|j| x + cx + (ex - cx) * j as f32 / n as f32));
            }
            x += w;
        }
        out.push(x);
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
        let g = self
            .outlines
            .add(&self.faces.get_mut()[face].1, id)
            .map(|g| {
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

    #[test]
    fn shapes_contextual_forms() {
        let t = Text::default();
        let ids = |s: &str| -> Vec<GlyphId> {
            let l = t.layout(s, Font::DEFAULT, 20.0, None);
            l.glyphs.iter().map(|g| g.id).collect()
        };
        // Arabic beh joins: isolated, then initial + final forms.
        let alone = ids("\u{628}");
        let pair = ids("\u{628}\u{628}");
        assert_eq!(pair.len(), 2);
        assert!(pair.iter().all(|g| *g != alone[0]), "{alone:?} {pair:?}");
        assert_ne!(pair[0], pair[1]);
    }

    #[test]
    fn carets_split_ligatures() {
        let t = Text::default();
        assert_eq!(t.layout("fi", Font::DEFAULT, 20.0, None).glyphs.len(), 1);
        let s = t.caret_stops("fi", Font::DEFAULT, 20.0);
        assert_eq!(s.len(), 3);
        assert!((s[1] - s[2] / 2.0).abs() < 1e-3, "{s:?}");
    }
}
