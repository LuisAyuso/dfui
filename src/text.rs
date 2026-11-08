use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use harfrust::{Buffer, ShapeOptions, ShapePlan, ShapePlanKey, ShaperFont};
use read_fonts::model::Font as Face;
use read_fonts::types::GlyphId;
use unicode_segmentation::UnicodeSegmentation;

use crate::outline::{Block, Outlines};
use crate::path::{Path, Shape};
use crate::render::{Color, Rect, Scene};

/// TrueType/OpenType font data. Fonts are told apart by address, so keep each
/// one in a `static`; it is parsed on first use.
#[derive(Clone, Copy)]
pub struct Font(pub &'static [u8]);

// The bytes themselves are statics: only a reference to a static item has
// one address in every crate; a const's data is copied per crate using it.
static DEJAVU: [u8; include_bytes!("../assets/DejaVuSans.ttf").len()] =
    *include_bytes!("../assets/DejaVuSans.ttf");
static DEJAVU_BOLD: [u8; include_bytes!("../assets/DejaVuSans-Bold.ttf").len()] =
    *include_bytes!("../assets/DejaVuSans-Bold.ttf");

impl Font {
    /// Bundled DejaVu Sans; also the fallback for chars a font lacks.
    pub const DEFAULT: Font = Font(&DEJAVU);
    /// Bundled DejaVu Sans Bold.
    pub const BOLD: Font = Font(&DEJAVU_BOLD);
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

struct StoredShape {
    /// `None` for a shape with nothing to fill.
    stored: Option<(Glyph, Block)>,
    drawn: u64,
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

/// A word shaped once at size 1; positions scale with the font size.
struct ShapedWord {
    text: String,
    glyphs: Vec<PositionedGlyph>,
    clusters: Vec<(usize, f32)>,
    advance: f32,
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
    /// Shaped words by hash of (face, word).
    // ponytail: cleared whole at WORD_CACHE entries; LRU if apps cycle through more words than that.
    words: RefCell<HashMap<u64, Rc<ShapedWord>>>,
    glyphs: HashMap<(FaceId, GlyphId), Option<Glyph>>,
    /// `Shape`s by content, stored like glyphs in units of the view's larger side.
    shapes: HashMap<u64, StoredShape>,
    /// Counts painted frames, to age `shapes`.
    frame: u64,
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
            words: RefCell::default(),
            glyphs: HashMap::new(),
            shapes: HashMap::new(),
            frame: 0,
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

/// Painted frames a shape survives without being drawn.
const SHAPE_GRACE: u64 = 600;
const WORD_CACHE: usize = 4096;

impl Text {
    /// Frees the frame-scoped outlines; the runtime calls this before painting.
    pub(crate) fn begin_frame(&mut self) {
        self.outlines.begin_frame();
        self.frame += 1;
        let (frame, outlines) = (self.frame, &mut self.outlines);
        self.shapes.retain(|_, s| {
            let keep = frame - s.drawn < SHAPE_GRACE;
            if !keep && let Some((_, block)) = s.stored.take() {
                outlines.free(block);
            }
            keep
        });
    }

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

    /// Baseline below the top of a line.
    pub(crate) fn ascent(&self, font: Font, size: f32) -> f32 {
        let id = self.face_id(font);
        line_box(&self.faces.borrow()[id].1, size).0
    }

    pub fn line_height(&self, font: Font, size: f32) -> f32 {
        let id = self.face_id(font);
        let (a, d, g) = line_box(&self.faces.borrow()[id].1, size);
        a + d + g
    }

    /// `s` cut to fit `max_lines` lines of `max_width`, ending in "…" when
    /// cut. Words longer than a line are cut too.
    // ponytail: binary search shapes ~log2(graphemes) candidates per call; cache by (s, width) if it shows up in profiles.
    pub fn truncate<'a>(
        &self,
        s: &'a str,
        font: Font,
        size: f32,
        max_width: f32,
        max_lines: usize,
    ) -> Cow<'a, str> {
        // Layout heights are whole lines, rounded up.
        let max_height = self.line_height(font, size) * max_lines as f32 + 1.0;
        let fits = |t: &str| {
            let l = self.layout(t, font, size, Some(max_width));
            l.width <= max_width && l.height <= max_height
        };
        if fits(s) {
            return Cow::Borrowed(s);
        }
        let cut = |end: usize| format!("{}…", s[..end].trim_end());
        // Longest grapheme prefix that fits with the ellipsis.
        let ends: Vec<usize> = s.grapheme_indices(true).map(|(i, _)| i).collect();
        let n = ends.partition_point(|&e| fits(&cut(e)));
        Cow::Owned(cut(ends[n.saturating_sub(1)]))
    }

    fn space(&self, face: FaceId, size: f32) -> f32 {
        let f = &self.faces.borrow()[face].1;
        let g = f.charmap().map_unicode(' ').unwrap_or_default();
        f.glyph_metrics().h_advance(g) * size / f.units_per_em() as f32
    }

    /// Appends `word` (no spaces or newlines) at `size` from x = 0, and the
    /// (byte, x) where each cluster starts; returns its advance.
    fn word(
        &self,
        face: FaceId,
        word: &str,
        size: f32,
        glyphs: &mut Vec<PositionedGlyph>,
        clusters: &mut Vec<(usize, f32)>,
    ) -> f32 {
        let w = self.shaped(face, word);
        glyphs.extend(w.glyphs.iter().map(|g| PositionedGlyph {
            face: g.face,
            id: g.id,
            x: g.x * size,
            y: g.y * size,
        }));
        clusters.extend(w.clusters.iter().map(|&(b, x)| (b, x * size)));
        w.advance * size
    }

    fn shaped(&self, face: FaceId, word: &str) -> Rc<ShapedWord> {
        let mut h = DefaultHasher::new();
        (face, word).hash(&mut h);
        let key = h.finish();
        if let Some(w) = self.words.borrow().get(&key)
            && w.text == word
        {
            return w.clone();
        }
        let w = Rc::new(self.shape_word(face, word));
        let mut words = self.words.borrow_mut();
        if words.len() >= WORD_CACHE {
            words.clear();
        }
        words.insert(key, w.clone());
        w
    }

    /// Shapes `word` at size 1. Chars `face` lacks use the default font.
    fn shape_word(&self, face: FaceId, word: &str) -> ShapedWord {
        let (mut glyphs, mut clusters) = (Vec::new(), Vec::new());
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
            let s = 1.0 / font.units_per_em() as f32;
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
        ShapedWord {
            text: word.to_owned(),
            glyphs,
            clusters,
            advance: x,
        }
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

    /// Outline of `c` at `size` px (no shaping), pen at the origin on the
    /// baseline, y down. Falls back to the default font like `layout`.
    pub fn glyph_path(&self, font: Font, c: char, size: f32) -> Option<Path> {
        let face = self.face_id(font);
        let faces = self.faces.borrow();
        let (f, id) = [face, 0]
            .into_iter()
            .find_map(|f| Some((f, faces[f].1.charmap().map_unicode(c)?)))?;
        let contours = crate::outline::contours(&faces[f].1, id)?;
        Some(Path {
            contours: contours
                .into_iter()
                .map(|c| {
                    c.into_iter()
                        .map(|q| q.map(|[x, y]| [x * size, -y * size]))
                        .collect()
                })
                .collect(),
        })
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

    /// Draws `shape` with its view box mapped onto `r`.
    pub(crate) fn draw_shape(
        &mut self,
        shape: &Shape,
        r: Rect,
        color: Color,
        clip: Rect,
        scene: &mut Scene,
    ) {
        let d = &shape.0;
        let v = d.view;
        let span = v.w.max(v.h);
        if span <= 0.0 {
            return;
        }
        let entry = self.shapes.entry(d.key).or_insert_with(|| {
            // Unit box, y up, like glyph em space.
            let unit = d
                .path
                .contours
                .iter()
                .flatten()
                .map(|q| q.map(|[x, y]| [(x - v.x) / span, (v.y + v.h - y) / span]));
            // Room for the antialiased edge of shapes that touch the view box.
            let stored = self.outlines.store(unit.collect(), 0.02).map(|(g, block)| {
                let [x0, y0, x1, y1] = g.bbox;
                let glyph = Glyph {
                    em: [x0, y1, x1, y0],
                    plane: Rect::new(x0, y0, x1 - x0, y1 - y0),
                    bands: [g.header as f32, g.bands_h as f32, g.bands_v as f32],
                };
                (glyph, block)
            });
            StoredShape { stored, drawn: 0 }
        });
        entry.drawn = self.frame;
        let Some((g, _)) = &entry.stored else { return };
        // Unit box to `r`; `plane` holds the bbox, y up.
        let (sx, sy) = (r.w * span / v.w, r.h * span / v.h);
        let top = v.h / span - (g.plane.y + g.plane.h);
        let q = Rect::new(
            r.x + g.plane.x * sx,
            r.y + top * sy,
            g.plane.w * sx,
            g.plane.h * sy,
        );
        scene.glyph(q, g.em, color, clip, g.bands);
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
    fn truncate_ends_cut_text_with_an_ellipsis() {
        let t = Text::default();
        let (f, size) = (Font::DEFAULT, 16.0);
        let w = |s: &str| t.layout(s, f, size, None).width;
        let s = "The quick brown fox jumps over the lazy dog";
        assert_eq!(t.truncate(s, f, size, 1000.0, 1), s);
        let max = w("The quick brown");
        let one = t.truncate(s, f, size, max, 1);
        // The longest prefix that fits with its ellipsis.
        assert_eq!(one, "The quick bro…");
        assert!(w(&one) <= max && w("The quick brow…") > max);
        let two = t.truncate(s, f, size, w("The quick brown"), 2);
        assert!(
            two.starts_with("The quick brown fox") && two.ends_with('…'),
            "{two}"
        );
        // A single word wider than the line is cut inside it.
        assert_eq!(
            t.truncate("Supercalifragilistic", f, size, w("Super…"), 1),
            "Super…"
        );
        assert_eq!(t.truncate("line\nbreak", f, size, 1000.0, 1), "line…");
    }

    fn square(side: f32) -> Shape {
        let sq = Path::polygon(&[[0.0, 0.0], [side, 0.0], [side, side], [0.0, side]]);
        Shape::new(sq, Rect::new(0.0, 0.0, 24.0, 24.0))
    }

    fn draw(t: &mut Text, shape: &Shape) -> Scene {
        let mut scene = Scene::default();
        let all = Rect::new(0.0, 0.0, 100.0, 100.0);
        t.draw_shape(shape, all, Color::BLACK, all, &mut scene);
        scene
    }

    #[test]
    fn shaped_words_are_reused_and_scale_with_the_size() {
        let t = Text::default();
        let small = t.layout("alpha beta", Font::DEFAULT, 10.0, None);
        let words = t.words.borrow().len();
        assert_eq!(words, 2);
        let big = t.layout("beta alpha", Font::DEFAULT, 20.0, None);
        assert_eq!(t.words.borrow().len(), words, "no word shaped again");
        assert!((big.width - small.width * 2.0).abs() <= 2.0);
    }

    #[test]
    fn identical_shapes_share_their_storage() {
        let mut t = Text::default();
        draw(&mut t, &square(12.0));
        let stored = t.outlines.curves.len();
        for _ in 0..10 {
            t.begin_frame();
            draw(&mut t, &square(12.0));
        }
        assert_eq!(t.outlines.curves.len(), stored);
    }

    #[test]
    fn shapes_no_longer_drawn_give_their_storage_back() {
        let mut t = Text::default();
        let kept = square(1.0);
        let kept_at = draw(&mut t, &kept).instances[0].rect;
        for i in 0..SHAPE_GRACE * 3 {
            t.begin_frame();
            draw(&mut t, &kept);
            draw(&mut t, &square(2.0 + i as f32 / 100.0));
        }
        let peak = t.outlines.curves.len();
        for i in 0..SHAPE_GRACE * 3 {
            t.begin_frame();
            assert_eq!(draw(&mut t, &kept).instances[0].rect, kept_at);
            draw(&mut t, &square(1000.0 + i as f32 / 100.0));
        }
        assert_eq!(t.outlines.curves.len(), peak, "storage is reused");
    }

    #[test]
    fn shapes_are_stored_once_and_mapped_onto_the_rect() {
        let mut t = Text::default();
        // A 12×12 square in the middle of a 24×24 view.
        let sq = Path::polygon(&[[6.0, 6.0], [18.0, 6.0], [18.0, 18.0], [6.0, 18.0]]);
        let shape = Shape::new(sq, Rect::new(0.0, 0.0, 24.0, 24.0));
        let mut scene = Scene::default();
        let all = Rect::new(0.0, 0.0, 1000.0, 1000.0);
        t.draw_shape(
            &shape,
            Rect::new(100.0, 50.0, 48.0, 48.0),
            Color::BLACK,
            all,
            &mut scene,
        );
        let stored = t.outlines.curves.len();
        t.draw_shape(
            &shape,
            Rect::new(0.0, 0.0, 240.0, 240.0),
            Color::BLACK,
            all,
            &mut scene,
        );
        assert_eq!(
            t.outlines.curves.len(),
            stored,
            "drawn again from the store"
        );
        // Twice the view's size: the square lands at 12..36 plus the edge padding.
        let [x, y, w, h] = scene.instances[0].rect;
        let pad = 0.02 * 48.0;
        let near = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(near(x, 112.0 - pad) && near(y, 62.0 - pad), "{x} {y}");
        assert!(
            near(w, 24.0 + 2.0 * pad) && near(h, 24.0 + 2.0 * pad),
            "{w} {h}"
        );
    }

    #[test]
    fn missing_chars_fall_back_to_default_font() {
        const BOLD: Font = Font::BOLD;
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

    #[test]
    fn weights_morph_corner_to_corner() {
        let t = Text::default();
        let a = t.glyph_path(Font::DEFAULT, 'n', 100.0).unwrap();
        let b = t.glyph_path(Font::BOLD, 'n', 100.0).unwrap();
        let m = crate::anim::Morph::new(&a, &b);
        let (from, to) = (m.at(0.0), m.at(1.0));
        // Stem bottoms stay on the baseline, so they stay flat throughout.
        let on_base = |p: &[f32; 2]| p[1].abs() < 1e-3;
        for (p, q) in from.contours[0].iter().zip(&to.contours[0]) {
            assert_eq!(on_base(&p[0]), on_base(&q[0]), "{:?} -> {:?}", p[0], q[0]);
        }
        assert_eq!(
            from.contours[0].iter().filter(|q| on_base(&q[0])).count(),
            4
        );
    }

    #[test]
    fn glyphs_morph_contour_by_contour() {
        let t = Text::default();
        let o = t.glyph_path(Font::DEFAULT, 'O', 40.0).unwrap();
        let c = t.glyph_path(Font::DEFAULT, 'C', 40.0).unwrap();
        assert_eq!((o.contours.len(), c.contours.len()), (2, 1));
        let m = crate::anim::Morph::new(&o, &c);
        for k in 0..=4 {
            let p = m.at(k as f32 / 4.0);
            assert_eq!(p.contours.len(), 2);
            for c in &p.contours {
                for (a, b) in c.iter().zip(c.iter().cycle().skip(1)) {
                    assert!((a[2][0] - b[0][0]).abs() < 1e-3 && (a[2][1] - b[0][1]).abs() < 1e-3);
                }
            }
        }
        // O's outline becomes C's; its hole (listed first) has no partner and
        // shrinks to a point.
        assert_eq!(m.at(0.0).contours[0], o.contours[0]);
        let hole = &m.at(1.0).contours[0];
        assert!(hole.iter().flatten().all(|p| p == &hole[0][0]));
    }
}
