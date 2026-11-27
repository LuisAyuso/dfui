//! Vector maps for dfui: the app projects its geometry (e.g.
//! from OpenStreetMap) with a [`MapBuilder`], which packs it into tiled
//! [`Shape`]s at a few levels of detail, and [`map`] pans and zooms them.
//! Shapes are stored on the GPU as they come into view, so moving the map
//! mostly moves the rects they're drawn into.

use std::cell::{Cell, OnceCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use dfui::path::{CAP_STEPS, Point, Quad, Stroke, stroke};
use dfui::taffy::{Dimension, Size, Style};
use dfui::widgets::Styled;
use dfui::{
    Color, Element, Event, EventCx, Font, Instant, PaintCx, Path, Rect, Shape, Text, Widget,
};

/// Most segments in one shape: the shader walks a band's segments per pixel.
const MAX_QUADS: usize = 128;
/// Cells aren't split below this side, metres.
const MIN_CELL: f32 = 16.0;
/// How far past its cell a tile's geometry is cut, metres, so the cut edge
/// stays outside the cell's clip and never antialiases into a seam. Exact
/// while it is half a px or more: from 0.25 px per metre.
const MARGIN: f32 = 2.0;
const EARTH_RADIUS: f64 = 6_371_000.0;
/// Font size, logical px, at which a label starts to show.
const MIN_LABEL_PX: f32 = 7.0;
/// Least screen distance between labels with the same text, logical px.
const LABEL_SPACING_PX: f32 = 300.0;
/// Sharpest turn between neighbouring glyphs of a label, radians.
const MAX_LABEL_TURN: f32 = 0.35;
/// Pointer travel before a press becomes a pan, logical px.
const SLOP: f32 = 6.0;
/// Scales, logical px per metre, where a finer level of detail takes over;
/// from the last one up the map draws at full detail.
const LEVELS: [f32; 6] = [1.0 / 32.0, 1.0 / 16.0, 1.0 / 8.0, 0.25, 0.5, 1.0];
const FULL: usize = LEVELS.len();
/// How far a simplified line may stray, px at the largest scale of its level.
const TOLERANCE_PX: f32 = 0.25;
/// Features smaller than this, px, are left out of a level.
const MIN_FEATURE_PX: f32 = 0.75;
/// Narrower lines get no caps or bevels in a level, px.
const JOIN_PX: f32 = 1.5;
/// Largest tile side, px at the largest scale of its level: a tile costs
/// every pixel it covers, so a sparse layer (a single bus line) mustn't
/// spread few segments over a large cell.
const MAX_CELL_PX: f32 = 256.0;
/// Scale [`MAX_CELL_PX`] is measured at for full detail, px per metre.
const FULL_CELL_SCALE: f32 = 4.0;
/// How long the view takes to move to a new focus or fit.
const GLIDE: Duration = Duration::from_millis(450);
/// Time a frame spends storing new tiles before leaving the rest for the
/// next frames, coarser levels standing in.
const BUDGET: Duration = Duration::from_millis(4);

/// Collects areas and lines in metres around an origin and packs them into
/// [`MapData`]. Layers draw in the order they were added.
pub struct MapBuilder {
    origin: (f64, f64),
    cos: f64,
    layers: Vec<LayerData>,
    /// One tile each, drawn over every layer.
    labels: Vec<Tile>,
    /// Centres of the labels placed so far, by text.
    placed: HashMap<String, Vec<Point>>,
}

struct LayerData {
    color: Color,
    min_scale: f32,
    /// Closed rings, outlines wound positive and holes negative, so they
    /// fill by nonzero winding however they're cut into tiles.
    rings: Vec<Vec<Point>>,
    lines: Vec<Run>,
}

/// A line by its middle, stroked when its tile is drawn.
#[derive(Clone, Debug, PartialEq)]
struct Run {
    points: Vec<Point>,
    width: f32,
    /// Round caps at the first and last point; none where a tile cut it.
    caps: [bool; 2],
    /// Fill the outside of each bend.
    bevels: bool,
}

impl Run {
    /// The line as outlines, see [`stroke`].
    fn stroke(&self) -> Vec<Vec<Point>> {
        let style = Stroke {
            width: self.width,
            caps: self.caps,
            bevels: self.bevels,
        };
        stroke(&self.points, style)
    }
}

/// One of a [`MapBuilder`]'s layers.
#[derive(Clone, Copy, Debug)]
pub struct Layer(usize);

impl MapBuilder {
    /// A map centred on `lat`, `lon` in degrees.
    pub fn new(lat: f64, lon: f64) -> Self {
        MapBuilder {
            origin: (lat, lon),
            cos: lat.to_radians().cos(),
            layers: Vec::new(),
            labels: Vec::new(),
            placed: HashMap::new(),
        }
    }

    /// `lat`, `lon` in metres east and south of the origin; an
    /// equirectangular projection, fine for a city.
    pub fn point(&self, lat: f64, lon: f64) -> Point {
        let x = (lon - self.origin.1).to_radians() * self.cos * EARTH_RADIUS;
        let y = (self.origin.0 - lat).to_radians() * EARTH_RADIUS;
        [x as f32, y as f32]
    }

    /// A layer drawn in `color`, hidden while zoomed out below `min_scale`
    /// logical px per metre (0 always shows it).
    pub fn layer(&mut self, color: Color, min_scale: f32) -> Layer {
        self.layers.push(LayerData {
            color,
            min_scale,
            rings: Vec::new(),
            lines: Vec::new(),
        });
        Layer(self.layers.len() - 1)
    }

    /// A filled area, e.g. an OSM multipolygon: `outers` filled, `holes` cut
    /// out of them. Rings may wind either way.
    pub fn area(&mut self, layer: Layer, outers: &[Vec<Point>], holes: &[Vec<Point>]) {
        let rings = &mut self.layers[layer.0].rings;
        let all = outers.iter().map(|r| (r, true));
        rings.extend(
            all.chain(holes.iter().map(|r| (r, false)))
                .filter_map(|(r, outer)| orient(r.clone(), outer)),
        );
    }

    /// A line `width` metres wide through `points`, with round caps and
    /// bevelled joins.
    pub fn line(&mut self, layer: Layer, points: &[Point], width: f32) {
        let mut points = points.to_vec();
        points.dedup();
        if points.len() >= 2 {
            self.layers[layer.0].lines.push(Run {
                points,
                width,
                caps: [true, true],
                bevels: true,
            });
        }
    }

    /// `s` at `size` metres along the polyline, e.g. a street's name inside
    /// it: centred, reading left to right, each glyph turned to the line
    /// under its middle. Shows once `size` reaches 7 logical px on screen and
    /// the same text placed earlier is 300 px away; false,
    /// adding nothing, when the text is longer than the line or the line
    /// bends too sharply under it.
    pub fn label(
        &mut self,
        layer: Layer,
        points: &[Point],
        text: &Text,
        font: Font,
        s: &str,
        size: f32,
    ) -> bool {
        let mut pts = points.to_vec();
        pts.dedup();
        if pts.len() < 2 {
            return false;
        }
        if pts[pts.len() - 1][0] < pts[0][0] {
            pts.reverse();
        }
        let mut along = vec![0.0];
        for w in pts.windows(2) {
            along.push(along[along.len() - 1] + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]));
        }
        let length = along[along.len() - 1];
        let layout = text.layout(s, font, size, None);
        if layout.width + size > length {
            return false;
        }
        let start = (length - layout.width) / 2.0;
        // Point and unit direction at distance `d` along the line.
        let at = |d: f32| {
            let i = along.partition_point(|&a| a <= d).clamp(1, pts.len() - 1);
            let (a, b) = (pts[i - 1], pts[i]);
            let len = along[i] - along[i - 1];
            let t = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
            let k = d - along[i - 1];
            ([a[0] + t[0] * k, a[1] + t[1] * k], t)
        };
        let mid_y = layout.height / 2.0;
        let mut contours = Vec::new();
        let mut last: Option<f32> = None;
        for g in text.glyph_paths(s, font, size) {
            let xs = g.contours.iter().flatten().flatten().map(|p| p[0]);
            let (lo, hi) = xs.fold((f32::MAX, f32::MIN), |(l, h), x| (l.min(x), h.max(x)));
            let mid_x = (lo + hi) / 2.0;
            let (o, t) = at(start + mid_x);
            let angle = t[1].atan2(t[0]);
            if let Some(a) = last {
                let turn = (angle - a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU);
                if (turn - std::f32::consts::PI).abs() > MAX_LABEL_TURN {
                    return false;
                }
            }
            last = Some(angle);
            // Glyph x runs along the line and y (down) to its right.
            let place = |[x, y]: Point| {
                let (u, v) = (x - mid_x, y - mid_y);
                [o[0] + u * t[0] - v * t[1], o[1] + u * t[1] + v * t[0]]
            };
            contours.extend(
                g.contours
                    .into_iter()
                    .map(|c| c.into_iter().map(|q| q.map(place)).collect::<Vec<_>>()),
            );
        }
        let glyphs = Path { contours };
        let Some(view) = bounds(glyphs.contours.iter().flatten().flatten()) else {
            return false;
        };
        let (c, _) = at(length / 2.0);
        let near = self.placed.entry(s.to_owned()).or_default();
        let d = near.iter().map(|p| (p[0] - c[0]).hypot(p[1] - c[1]));
        let spaced = LABEL_SPACING_PX / d.fold(f32::MAX, f32::min);
        near.push(c);
        let l = &self.layers[layer.0];
        let pad = size / 4.0;
        let cell = Rect::new(
            view.x - pad,
            view.y - pad,
            view.w + 2.0 * pad,
            view.h + 2.0 * pad,
        );
        let min_scale = l.min_scale.max(MIN_LABEL_PX / size).max(spaced);
        let geo = Geometry {
            glyphs,
            ..Geometry::default()
        };
        self.labels.push(Tile::new(
            cell,
            l.color,
            min_scale,
            (layer.0 as u32, LABELS),
            geo,
        ));
        true
    }

    /// Cuts every layer at each level of detail into quadtree cells of at
    /// most 128 segments.
    pub fn build(self) -> MapData {
        let (mut tiles, mut table) = (Vec::new(), Vec::new());
        for (i, l) in self.layers.iter().enumerate() {
            let mut serve = [FULL as u8; FULL + 1];
            let mut finer = usize::MAX;
            for level in (level_of(l.min_scale)..=FULL).rev() {
                let (rings, runs) = detail(l, level);
                // Not worth a level unless it halves what the finer one draws.
                let c = cost(&rings, &runs);
                if c.saturating_mul(2) > finer {
                    serve[level] = serve[level + 1];
                    continue;
                }
                (serve[level], finer) = (level as u8, c);
                let lines = runs.iter().flat_map(|r| {
                    let h = r.width / 2.0;
                    r.points
                        .iter()
                        .flat_map(move |p| [[p[0] - h, p[1] - h], [p[0] + h, p[1] + h]])
                });
                let all: Vec<Point> = rings.iter().flatten().copied().chain(lines).collect();
                if let Some(b) = bounds(all.iter()) {
                    let from = tiles.len();
                    split(l, (i as u32, level as u32), b, rings, runs, &mut tiles);
                    tiles[from..].sort_by(|a, b| a.cell.x.total_cmp(&b.cell.x));
                }
            }
            table.push((l.color, serve));
        }
        let full = tiles.iter().filter(|t| t.level == FULL as u32);
        let bounds = full.map(|t| t.cell).reduce(|a, b| a.union(&b));
        let mut labels = self.labels;
        labels.sort_by(|a, b| a.cell.x.total_cmp(&b.cell.x));
        tiles.extend(labels);
        MapData::new(tiles, table, bounds.unwrap_or_default(), &[])
    }
}

/// Which level draws at `scale`.
fn level_of(scale: f32) -> usize {
    LEVELS.iter().filter(|&&s| s <= scale).count()
}

/// A layer's rings and lines as drawn at `level`: simplified, small features
/// left out, thin lines without caps or bevels.
fn detail(l: &LayerData, level: usize) -> (Vec<Vec<Point>>, Vec<Run>) {
    let Some(&px) = LEVELS.get(level) else {
        return (l.rings.clone(), l.lines.clone());
    };
    let tol = TOLERANCE_PX / px;
    let big = |pts: &[Point], pad: f32| {
        bounds(pts.iter()).is_some_and(|b| (b.w.max(b.h) + pad) * px >= MIN_FEATURE_PX)
    };
    let rings = l.rings.iter().filter(|r| big(r, 0.0)).filter_map(|r| {
        let mut closed = r.clone();
        closed.push(r[0]);
        let mut s = simplify(&closed, tol);
        s.pop();
        let outer = signed_area(r) > 0.0;
        (s.len() >= 3 && (signed_area(&s) > 0.0) == outer && signed_area(&s) != 0.0).then_some(s)
    });
    let runs = l.lines.iter().filter(|r| big(&r.points, r.width)).map(|r| {
        let fine = r.width * px >= JOIN_PX;
        Run {
            points: simplify(&r.points, tol),
            width: r.width,
            caps: r.caps.map(|c| c && fine),
            bevels: r.bevels && fine,
        }
    });
    (rings.collect(), runs.collect())
}

/// `pts` with the points that move it by no more than `tol` left out
/// (Douglas–Peucker); the ends stay.
fn simplify(pts: &[Point], tol: f32) -> Vec<Point> {
    let n = pts.len();
    if n < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; n];
    (keep[0], keep[n - 1]) = (true, true);
    let mut todo = vec![(0, n - 1)];
    while let Some((a, b)) = todo.pop() {
        let (mut far, mut most) = (0, tol);
        for (i, &p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = distance(p, pts[a], pts[b]);
            if d > most {
                (far, most) = (i, d);
            }
        }
        if far != 0 {
            keep[far] = true;
            todo.extend([(a, far), (far, b)]);
        }
    }
    pts.iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(p, _)| *p)
        .collect()
}

/// From `p` to the segment `a`–`b`.
fn distance(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = match len2 > 0.0 {
        true => (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0),
        false => 0.0,
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

/// `ring` wound positive for an outline, negative for a hole; `None` when
/// it encloses nothing.
fn orient(mut ring: Vec<Point>, outer: bool) -> Option<Vec<Point>> {
    let area = signed_area(&ring);
    if ring.len() < 3 || area == 0.0 {
        return None;
    }
    if (area > 0.0) != outer {
        ring.reverse();
    }
    Some(ring)
}

/// Twice the ring's area, positive when it winds clockwise on screen (y down).
fn signed_area(ring: &[Point]) -> f32 {
    let n = ring.len();
    (0..n)
        .map(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum()
}

fn bounds<'a>(points: impl Iterator<Item = &'a Point>) -> Option<Rect> {
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in points {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    (lo[0] <= hi[0]).then(|| Rect::new(lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]))
}

fn grow(r: Rect, by: f32) -> Rect {
    Rect::new(r.x - by, r.y - by, r.w + 2.0 * by, r.h + 2.0 * by)
}

/// Segments the shape of these rings and runs will have, roughly.
fn cost(rings: &[Vec<Point>], runs: &[Run]) -> usize {
    let lines = runs.iter().map(|r| {
        let n = r.points.len() - 1;
        let caps = r.caps.iter().filter(|c| **c).count();
        4 * n + if r.bevels { 3 * (n - 1) } else { 0 } + caps * (CAP_STEPS + 1)
    });
    rings.iter().map(Vec::len).sum::<usize>() + lines.sum::<usize>()
}

/// One tile for `rings` and `runs` (already cut to `cell` plus [`MARGIN`]),
/// or four for its quarters while it holds too many segments.
fn split(
    l: &LayerData,
    at: (u32, u32),
    cell: Rect,
    rings: Vec<Vec<Point>>,
    runs: Vec<Run>,
    tiles: &mut Vec<Tile>,
) {
    if rings.is_empty() && runs.is_empty() {
        return;
    }
    let px = LEVELS
        .get(at.1 as usize)
        .copied()
        .unwrap_or(FULL_CELL_SCALE);
    let side = cell.w.max(cell.h);
    if (cost(&rings, &runs) <= MAX_QUADS && side * px <= MAX_CELL_PX) || side < MIN_CELL {
        let geo = Geometry {
            rings,
            runs,
            glyphs: Path::default(),
        };
        tiles.push(Tile::new(cell, l.color, l.min_scale, at, geo));
        return;
    }
    let (w, h) = (cell.w / 2.0, cell.h / 2.0);
    for (i, j) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let c = Rect::new(cell.x + i * w, cell.y + j * h, w, h);
        let m = grow(c, MARGIN);
        let cut = rings.iter().filter_map(|r| clip(r, m)).collect();
        // Past the margin by half the width, a segment's outline can't reach it.
        let pieces = runs.iter().flat_map(|r| cut_run(r, grow(m, r.width / 2.0)));
        split(l, at, c, cut, pieces.collect(), tiles);
    }
}

/// `ring` cut to `r` (Sutherland–Hodgman): for a concave ring the pieces stay
/// joined along `r`'s edges, by edges that cancel out when filled.
fn clip(ring: &[Point], r: Rect) -> Option<Vec<Point>> {
    let b = bounds(ring.iter())?;
    let inside = b.x >= r.x && b.y >= r.y && b.x + b.w <= r.x + r.w && b.y + b.h <= r.y + r.h;
    if inside {
        return Some(ring.to_vec());
    }
    let hit = r.intersect(&b);
    if hit.w <= 0.0 || hit.h <= 0.0 {
        return None;
    }
    let edges = [
        (0, r.x, 1.0),
        (0, r.x + r.w, -1.0),
        (1, r.y, 1.0),
        (1, r.y + r.h, -1.0),
    ];
    let mut out = ring.to_vec();
    for (axis, at, side) in edges {
        let keep = |p: Point| (p[axis] - at) * side >= 0.0;
        let input = std::mem::take(&mut out);
        for (k, &a) in input.iter().enumerate() {
            let b = input[(k + 1) % input.len()];
            if keep(a) {
                out.push(a);
            }
            if keep(a) != keep(b) {
                let t = (at - a[axis]) / (b[axis] - a[axis]);
                out.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
            }
        }
        if out.is_empty() {
            return None;
        }
    }
    (out.len() >= 3 && signed_area(&out) != 0.0).then_some(out)
}

/// The pieces of `run` inside `r`; ends made by the cut get no cap.
fn cut_run(run: &Run, r: Rect) -> Vec<Run> {
    let Some(b) = bounds(run.points.iter()) else {
        return Vec::new();
    };
    if b.x >= r.x && b.y >= r.y && b.x + b.w <= r.x + r.w && b.y + b.h <= r.y + r.h {
        return vec![run.clone()];
    }
    let (mut out, mut cur, mut cap) = (Vec::new(), Vec::new(), false);
    let mut flush = |cur: &mut Vec<Point>, caps: [bool; 2]| {
        let mut points = std::mem::take(cur);
        points.dedup();
        if points.len() >= 2 {
            out.push(Run {
                points,
                caps,
                ..*run
            });
        }
    };
    let last = run.points.len() - 2;
    for (i, w) in run.points.windows(2).enumerate() {
        let Some((a, z, t0, t1)) = clip_segment(w[0], w[1], r) else {
            flush(&mut cur, [cap, false]);
            continue;
        };
        if cur.is_empty() {
            cur.push(a);
            cap = run.caps[0] && i == 0 && t0 == 0.0;
        }
        cur.push(z);
        if t1 < 1.0 {
            flush(&mut cur, [cap, false]);
        } else if i == last {
            flush(&mut cur, [cap, run.caps[1]]);
        }
    }
    out
}

/// The part of `a`–`b` inside `r` and where it starts and ends along it
/// (Liang–Barsky).
fn clip_segment(a: Point, b: Point, r: Rect) -> Option<(Point, Point, f32, f32)> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [
        (-d[0], a[0] - r.x),
        (d[0], r.x + r.w - a[0]),
        (-d[1], a[1] - r.y),
        (d[1], r.y + r.h - a[1]),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else if p < 0.0 {
            t0 = t0.max(q / p);
        } else {
            t1 = t1.min(q / p);
        }
    }
    let at = |t: f32| [a[0] + t * d[0], a[1] + t * d[1]];
    (t0 <= t1).then(|| (at(t0), at(t1), t0, t1))
}

/// What a tile draws: areas cut to its cell, lines by their middle and,
/// for a label, glyph outlines.
#[derive(Default, Debug, PartialEq)]
struct Geometry {
    rings: Vec<Vec<Point>>,
    runs: Vec<Run>,
    glyphs: Path,
}

impl Geometry {
    /// Everything as one path, lines stroked and cut to `margin`.
    fn path(&self, margin: Rect) -> Path {
        let mut stroked = Vec::new();
        for r in &self.runs {
            stroked.extend(r.stroke());
        }
        let cut = stroked.iter().filter_map(|r| clip(r, margin));
        let rings = self.rings.iter().cloned().chain(cut);
        let mut contours: Vec<Vec<Quad>> = rings.flat_map(|r| Path::polygon(&r).contours).collect();
        contours.extend(self.glyphs.contours.iter().cloned());
        Path { contours }
    }
}

/// The level of label tiles, drawn over every layer at every scale.
const LABELS: u32 = u32::MAX;

struct Tile {
    /// What the tile covers and is clipped to when drawn, metres.
    cell: Rect,
    color: Color,
    min_scale: f32,
    layer: u32,
    /// Index into [`LEVELS`], [`FULL`], or [`LABELS`].
    level: u32,
    /// Set for a built map; a map read from bytes decodes it from `at`.
    geo: Option<Geometry>,
    at: Range<usize>,
    /// The shape and its view box, made on first use.
    shape: OnceCell<Option<(Shape, Rect)>>,
    /// Drawn at least once, so its shape is on the GPU.
    stored: Cell<bool>,
}

impl Tile {
    fn new(
        cell: Rect,
        color: Color,
        min_scale: f32,
        (layer, level): (u32, u32),
        geo: Geometry,
    ) -> Tile {
        Tile {
            cell,
            color,
            min_scale,
            layer,
            level,
            geo: Some(geo),
            at: 0..0,
            shape: OnceCell::new(),
            stored: Cell::new(false),
        }
    }
}

/// Tiles of one layer at one level, or the labels, sorted by their cell's
/// left edge.
#[derive(Clone, Debug, Default, PartialEq)]
struct Group {
    tiles: Range<usize>,
    /// The widest cell, to find the tiles that may reach into a view.
    widest: f32,
}

/// Geometry ready to draw, from [`MapBuilder::build`] or
/// [`MapData::from_bytes`]. Make it once and share it, e.g. in the `App`.
pub struct MapData {
    /// By layer then level, labels last.
    tiles: Vec<Tile>,
    /// Each layer's tiles at each level.
    layers: Vec<[Group; FULL + 1]>,
    /// Each layer's color and the level that draws in place of each.
    table: Vec<(Color, [u8; FULL + 1])>,
    labels: Group,
    bounds: Rect,
    /// What [`MapData::from_bytes`] read; the tiles' geometry lies in it.
    bytes: &'static [u8],
}

/// Coordinates are stored on this grid, metres: fine enough at the deepest zoom.
const GRID: f32 = 0.001;

/// Zigzag varints of grid coordinates, as deltas from the previous point.
struct Writer {
    out: Vec<u8>,
    origin: Point,
    at: [i64; 2],
}

impl Writer {
    fn put(&mut self, v: i64) {
        let mut u = ((v << 1) ^ (v >> 63)) as u64;
        while u >= 0x80 {
            self.out.push(u as u8 | 0x80);
            u >>= 7;
        }
        self.out.push(u as u8);
    }

    fn grid(&self, p: Point) -> [i64; 2] {
        let g = |v: f32, o: f32| ((v - o) / GRID).round() as i64;
        [g(p[0], self.origin[0]), g(p[1], self.origin[1])]
    }

    fn point(&mut self, p: Point) {
        let q = self.grid(p);
        self.put(q[0] - self.at[0]);
        self.put(q[1] - self.at[1]);
        self.at = q;
    }

    fn points(&mut self, pts: &[Point]) {
        self.put(pts.len() as i64);
        for &p in pts {
            self.point(p);
        }
    }
}

struct Reader<'a> {
    bytes: std::slice::Iter<'a, u8>,
    origin: Point,
    at: [i64; 2],
}

impl Reader<'_> {
    fn get(&mut self) -> Option<i64> {
        let (mut u, mut shift) = (0u64, 0);
        loop {
            let b = *self.bytes.next()?;
            u |= u64::from(b & 0x7f) << shift.min(63);
            if b < 0x80 {
                return Some((u >> 1) as i64 ^ -((u & 1) as i64));
            }
            shift += 7;
        }
    }

    fn count(&mut self) -> Option<usize> {
        usize::try_from(self.get()?).ok()
    }

    fn pos(&self, q: [i64; 2]) -> Point {
        [
            self.origin[0] + q[0] as f32 * GRID,
            self.origin[1] + q[1] as f32 * GRID,
        ]
    }

    fn point(&mut self) -> Option<Point> {
        self.at = [self.at[0] + self.get()?, self.at[1] + self.get()?];
        Some(self.pos(self.at))
    }

    fn points(&mut self) -> Option<Vec<Point>> {
        (0..self.count()?).map(|_| self.point()).collect()
    }
}

/// A tile's geometry, compactly: its rings as counted point lists; its
/// runs each with a width in grid steps, a flags varint (caps at either
/// end, bevels) and the points; then the glyph contours, each a count of
/// quads, its start point and each quad's end point and, for a curve, its
/// control point. Points are grid steps from the cell's corner, stored as
/// zigzag varint deltas; a straight glyph edge, whose control point is its
/// middle, stores none and flags that in the low bit of its first delta.
fn encode(geo: &Geometry, origin: Point) -> Vec<u8> {
    let mut w = Writer {
        out: Vec::new(),
        origin,
        at: [0, 0],
    };
    w.put(geo.rings.len() as i64);
    for r in &geo.rings {
        w.points(r);
    }
    w.put(geo.runs.len() as i64);
    for r in &geo.runs {
        w.put((r.width / GRID).round() as i64);
        let flags = [r.caps[0], r.caps[1], r.bevels];
        w.put(flags.iter().rev().fold(0, |f, &b| f << 1 | i64::from(b)));
        w.points(&r.points);
    }
    w.put(geo.glyphs.contours.len() as i64);
    for c in &geo.glyphs.contours {
        w.put(c.len() as i64);
        w.point(c.first().map_or(origin, |s| s[0]));
        for &[a, ctl, b] in c {
            let curve = ctl != [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
            let (e, k) = (w.grid(b), w.grid(ctl));
            let at = w.at;
            w.put((e[0] - at[0]) * 2 + i64::from(curve));
            w.put(e[1] - at[1]);
            if curve {
                w.put(k[0] - at[0]);
                w.put(k[1] - at[1]);
            }
            w.at = e;
        }
    }
    w.out
}

/// The geometry [`encode`] wrote; `None` if `bytes` are truncated.
fn decode(bytes: &[u8], origin: Point) -> Option<Geometry> {
    let mut r = Reader {
        bytes: bytes.iter(),
        origin,
        at: [0, 0],
    };
    let rings = (0..r.count()?).map(|_| r.points()).collect::<Option<_>>()?;
    let mut runs = Vec::new();
    for _ in 0..r.count()? {
        let width = r.get()? as f32 * GRID;
        let f = r.get()?;
        runs.push(Run {
            width,
            caps: [f & 1 != 0, f & 2 != 0],
            bevels: f & 4 != 0,
            points: r.points()?,
        });
    }
    let mut contours = Vec::new();
    for _ in 0..r.count()? {
        let n = r.count()?;
        r.point()?;
        let mut quads = Vec::new();
        for _ in 0..n {
            let (dx, dy) = (r.get()?, r.get()?);
            let (a, curve) = (r.pos(r.at), dx & 1 == 1);
            let e = [r.at[0] + (dx - i64::from(curve)) / 2, r.at[1] + dy];
            let ctl = match curve {
                true => {
                    let k = [r.at[0] + r.get()?, r.at[1] + r.get()?];
                    r.pos(k)
                }
                false => [(a[0] + r.pos(e)[0]) / 2.0, (a[1] + r.pos(e)[1]) / 2.0],
            };
            quads.push([a, ctl, r.pos(e)]);
            r.at = e;
        }
        contours.push(quads);
    }
    Some(Geometry {
        rings,
        runs,
        glyphs: Path { contours },
    })
}

/// Words per tile in the index: cell, min_scale, layer and level, and the
/// length of its geometry.
const INDEX_WORDS: usize = 7;

impl MapData {
    fn new(
        tiles: Vec<Tile>,
        table: Vec<(Color, [u8; FULL + 1])>,
        bounds: Rect,
        bytes: &'static [u8],
    ) -> MapData {
        let mut layers: Vec<[Group; FULL + 1]> = vec![Default::default(); table.len()];
        let mut labels = Group::default();
        let span = |g: &mut Group, i: usize| {
            let r = &mut g.tiles;
            *r = if r.start == r.end {
                i..i + 1
            } else {
                r.start.min(i)..r.end.max(i + 1)
            };
            g.widest = g.widest.max(tiles[i].cell.w);
        };
        for (i, t) in tiles.iter().enumerate() {
            if t.level == LABELS {
                span(&mut labels, i);
                continue;
            }
            if let Some(l) = layers.get_mut(t.layer as usize) {
                span(&mut l[(t.level as usize).min(FULL)], i);
            }
        }
        for (l, (_, serve)) in layers.iter_mut().zip(&table) {
            *l = serve.map(|k| l[(k as usize).min(FULL)].clone());
        }
        MapData {
            tiles,
            layers,
            table,
            labels,
            bounds,
            bytes,
        }
    }

    /// The map as [`MapData::from_bytes`] reads it, little-endian: `DFM2`,
    /// the bounds as four f32; a u32 count of layers and for each its color
    /// as four f32 and a u32 of the level drawn in place of each, four bits
    /// a level; a u32 count of tiles and for each its cell as four f32,
    /// min_scale, a u32 of its layer and (high byte, 255 for labels) level
    /// and a u32 length of its geometry; then each tile's geometry as
    /// the crate's `encode` writes it.
    pub fn to_bytes(&self) -> Vec<u8> {
        let geos: Vec<Vec<u8>> = self
            .tiles
            .iter()
            .map(|t| match &t.geo {
                Some(g) => encode(g, [t.cell.x, t.cell.y]),
                None => self.bytes[t.at.clone()].to_vec(),
            })
            .collect();
        let b = self.bounds;
        let bits = f32::from_bits;
        let mut words = vec![b.x, b.y, b.w, b.h, bits(self.table.len() as u32)];
        for (k, serve) in &self.table {
            let packed = serve.iter().rev().fold(0, |p, &l| p << 4 | u32::from(l));
            words.extend([k.r, k.g, k.b, k.a, bits(packed)]);
        }
        words.push(bits(self.tiles.len() as u32));
        for (t, geo) in self.tiles.iter().zip(&geos) {
            let c = t.cell;
            let at = t.layer | t.level.min(255) << 24;
            words.extend([
                c.x,
                c.y,
                c.w,
                c.h,
                t.min_scale,
                bits(at),
                bits(geo.len() as u32),
            ]);
        }
        let mut out = b"DFM2".to_vec();
        out.extend(words.iter().flat_map(|w| w.to_le_bytes()));
        out.extend(geos.concat());
        out
    }

    /// A map written by [`MapData::to_bytes`], usually `include_bytes!`d.
    /// Only the index is read here; each tile is decoded the first time
    /// it's drawn. `None` if `bytes` aren't one or are truncated.
    pub fn from_bytes(bytes: &'static [u8]) -> Option<MapData> {
        let body = bytes.strip_prefix(b"DFM2")?;
        let word = |i: usize| {
            let w = body.get(i * 4..i * 4 + 4)?;
            Some(f32::from_le_bytes(w.try_into().ok()?))
        };
        let rect = |i: usize| {
            Some(Rect::new(
                word(i)?,
                word(i + 1)?,
                word(i + 2)?,
                word(i + 3)?,
            ))
        };
        let bits = |i: usize| Some(word(i)?.to_bits());
        let (bounds, layers) = (rect(0)?, bits(4)? as usize);
        let mut table = Vec::new();
        for i in (0..layers).map(|i| 5 + i * 5) {
            let (k, packed) = (rect(i)?, bits(i + 4)?);
            let color = Color {
                r: k.x,
                g: k.y,
                b: k.w,
                a: k.h,
            };
            let serve: [u8; FULL + 1] = std::array::from_fn(|l| (packed >> (4 * l) & 15) as u8);
            table.push((color, serve));
        }
        let first = 5 + layers.checked_mul(5)? + 1;
        let n = bits(first - 1)? as usize;
        let mut at = 4 + (first + n.checked_mul(INDEX_WORDS)?) * 4;
        let mut tiles = Vec::with_capacity(n.min(bytes.len()));
        for i in (0..n).map(|i| first + i * INDEX_WORDS) {
            let (packed, len) = (bits(i + 5)?, bits(i + 6)? as usize);
            let layer = packed & 0xff_ffff;
            let level = match packed >> 24 {
                255 => LABELS,
                l => l,
            };
            tiles.push(Tile {
                cell: rect(i)?,
                color: table.get(layer as usize)?.0,
                min_scale: word(i + 4)?,
                layer,
                level,
                geo: None,
                at: at..at + len,
                shape: OnceCell::new(),
                stored: Cell::new(false),
            });
            at += len;
        }
        (at <= bytes.len()).then(|| MapData::new(tiles, table, bounds, bytes))
    }

    /// The tile's shape and view box, made on first use; `None` if it has
    /// nothing to draw or its bytes are corrupt.
    fn shape<'a>(&self, t: &'a Tile) -> Option<&'a (Shape, Rect)> {
        t.shape
            .get_or_init(|| {
                let decoded;
                let geo = match &t.geo {
                    Some(g) => g,
                    None => {
                        decoded = decode(&self.bytes[t.at.clone()], [t.cell.x, t.cell.y])?;
                        &decoded
                    }
                };
                let path = geo.path(grow(t.cell, MARGIN));
                let view = bounds(path.contours.iter().flatten().flatten())?;
                Some((Shape::new(path, view), view))
            })
            .as_ref()
    }

    /// The group's tiles whose cells may overlap `view`.
    fn near(&self, g: &Group, view: Rect) -> &[Tile] {
        let tiles = &self.tiles[g.tiles.clone()];
        let lo = tiles.partition_point(|t| t.cell.x + g.widest < view.x);
        let hi = tiles.partition_point(|t| t.cell.x <= view.x + view.w);
        &tiles[lo..hi.max(lo)]
    }

    /// The tiles across every level, for gauging how heavy a map is.
    pub fn tiles(&self) -> usize {
        self.tiles.len()
    }

    /// The segments across all tiles, each 32 bytes on the GPU once drawn.
    pub fn segments(&self) -> usize {
        let quads = self
            .tiles
            .iter()
            .filter_map(|t| self.shape(t))
            .map(|(s, _)| s.path().contours.iter().map(Vec::len).sum::<usize>());
        quads.sum()
    }
}

#[derive(Default)]
struct View {
    /// Metres at the widget's centre.
    center: Point,
    /// Logical px per metre; 0 until fitted to the widget.
    scale: f32,
    /// Where the press began and the last pointer position.
    press: Option<(Point, Point)>,
    panning: bool,
    /// The last [`Map::focus`] applied.
    focused: Option<(Point, f32)>,
    /// The last [`Map::fit`] applied.
    fitted: Option<Rect>,
    /// A move to a new focus or fit under way.
    glide: Option<Glide>,
}

/// From a centre and scale to another, starting at a time.
#[derive(Clone, Copy)]
struct Glide {
    from: (Point, f32),
    to: (Point, f32),
    start: Instant,
}

impl View {
    /// Applies a new start, focus or fit: the start at once, the others
    /// gliding there from `now`.
    fn fit(&mut self, map: &Map, r: Rect, now: Instant) {
        let b = map.data.bounds;
        let first = self.scale <= 0.0;
        if first && b.w > 0.0 {
            (self.center, self.scale) = map.start.unwrap_or((
                [b.x + b.w / 2.0, b.y + b.h / 2.0],
                (r.w / b.w).min(r.h / b.h),
            ));
        }
        let mut to = None;
        if map.focus != self.focused {
            if let Some((center, scale)) = map.focus {
                to = Some((center, self.scale.max(scale)));
            }
            self.focused = map.focus;
        }
        if map.fit.is_some() && map.fit != self.fitted {
            if let Some(b) = map.fit.filter(|b| b.w > 0.0 && b.h > 0.0) {
                to = Some((
                    [b.x + b.w / 2.0, b.y + b.h / 2.0],
                    (r.w / b.w).min(r.h / b.h),
                ));
            }
            self.fitted = map.fit;
        }
        match to {
            Some(to) if first => (self.center, self.scale) = to,
            Some(to) => {
                let from = (self.center, self.scale);
                self.glide = Some(Glide {
                    from,
                    to,
                    start: now,
                });
            }
            None => {}
        }
    }

    /// Moves along the glide to `now`; true while it hasn't arrived.
    fn step(&mut self, now: Instant) -> bool {
        let Some(g) = self.glide else { return false };
        let t =
            (now.saturating_duration_since(g.start).as_secs_f32() / GLIDE.as_secs_f32()).min(1.0);
        // Ease in and out (cubic).
        let e = if t < 0.5 {
            4.0 * t * t * t
        } else {
            1.0 - (2.0 - 2.0 * t).powi(3) / 2.0
        };
        let ((c0, s0), (c1, s1)) = (g.from, g.to);
        // ponytail: centre lerps while scale moves geometrically, so long hops
        // curve slightly; van Wijk–Nuij's zoom path if that ever shows.
        self.scale = s0 * (s1 / s0).powf(e);
        self.center = [c0[0] + (c1[0] - c0[0]) * e, c0[1] + (c1[1] - c0[1]) * e];
        if t >= 1.0 {
            self.glide = None;
        }
        self.glide.is_some()
    }

    /// Zooms by `k` keeping the metre under `(x, y)` in place.
    fn zoom(&mut self, data: &MapData, r: Rect, x: f32, y: f32, k: f32) {
        self.glide = None;
        let fit = (r.w / data.bounds.w).min(r.h / data.bounds.h);
        let to = (self.scale * k).clamp(fit / 4.0, 200.0);
        let (dx, dy) = (x - r.x - r.w / 2.0, y - r.y - r.h / 2.0);
        let d = 1.0 / self.scale - 1.0 / to;
        self.center = [self.center[0] + dx * d, self.center[1] + dy * d];
        self.scale = to;
    }
}
/// A pannable, zoomable map of `data`: drag to pan; wheel, ctrl+wheel or
/// pinch to zoom. Fills its parent's space by default.
pub fn map(data: &Rc<MapData>) -> Map {
    Map {
        data: data.clone(),
        style: Style {
            size: Size {
                width: Dimension::percent(1.0),
                height: Dimension::percent(1.0),
            },
            ..Style::default()
        },
        start: None,
        focus: None,
        fit: None,
        on_tap: None,
        overlay: None,
    }
}

type TapFn = dyn Fn(&mut EventCx, Point, f32);
type OverlayFn = dyn Fn(&mut PaintCx, &dyn Fn(Point) -> Point);

/// See [`map`].
pub struct Map {
    data: Rc<MapData>,
    style: Style,
    start: Option<(Point, f32)>,
    focus: Option<(Point, f32)>,
    fit: Option<Rect>,
    on_tap: Option<Rc<TapFn>>,
    overlay: Option<Rc<OverlayFn>>,
}

impl Map {
    /// Opens centred on `center` (from [`MapBuilder::point`]) at `scale`
    /// logical px per metre, instead of fitting the whole map.
    pub fn start(mut self, center: Point, scale: f32) -> Self {
        self.start = Some((center, scale));
        self
    }

    /// Centres on `center` and zooms in to at least `scale` whenever this
    /// differs from the focus of the last build, e.g. to show a search result.
    pub fn focus(mut self, focus: Option<(Point, f32)>) -> Self {
        self.focus = focus;
        self
    }

    /// Fits `bounds` (in [`MapBuilder::point`] units) to the widget, zooming
    /// in or out, whenever they differ from the last build's, e.g. to show
    /// one region and then the whole map again.
    pub fn fit(mut self, bounds: Rect) -> Self {
        self.fit = Some(bounds);
        self
    }

    /// Paints over the map, given a function from map points to logical px,
    /// e.g. to draw markers that keep their size while zooming.
    pub fn overlay(mut self, f: impl Fn(&mut PaintCx, &dyn Fn(Point) -> Point) + 'static) -> Self {
        self.overlay = Some(Rc::new(f));
        self
    }

    /// Runs on a tap that didn't pan or pinch, with the map point under it
    /// in [`MapBuilder::point`] metres.
    pub fn on_tap(self, f: impl Fn(&mut EventCx, Point) + 'static) -> Self {
        self.on_tap_scaled(move |cx, p, _| f(cx, p))
    }

    /// [`Map::on_tap`], also given the scale in logical px per metre, e.g. to
    /// take taps within a few px of a marker.
    pub fn on_tap_scaled(mut self, f: impl Fn(&mut EventCx, Point, f32) + 'static) -> Self {
        self.on_tap = Some(Rc::new(f));
        self
    }
}

impl From<Map> for Element {
    fn from(w: Map) -> Self {
        Element::new(w)
    }
}

impl Styled for Map {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Map {
    fn style(&self) -> Style {
        self.style.clone()
    }

    fn paint(&self, cx: &mut PaintCx) {
        let (r, now) = (cx.rect, cx.now);
        let v = cx.state.get::<View>();
        v.fit(self, r, now);
        let gliding = v.step(now);
        let (s, c) = (v.scale, v.center);
        if gliding {
            cx.request_frame();
        }
        let visible = Rect::new(c[0] - r.w / 2.0 / s, c[1] - r.h / 2.0 / s, r.w / s, r.h / s);
        let screen = |b: Rect| {
            let x = r.x + r.w / 2.0 + (b.x - c[0]) * s;
            let y = r.y + r.h / 2.0 + (b.y - c[1]) * s;
            Rect::new(x, y, b.w * s, b.h * s)
        };
        let data = &*self.data;
        let (level, start) = (level_of(s), Instant::now());
        let mut pending = false;
        let seen = |c: Rect| {
            let hit = visible.intersect(&c);
            hit.w > 0.0 && hit.h > 0.0
        };
        let draw = |cx: &mut PaintCx, t: &Tile, clip: Rect| {
            if let Some((shape, view)) = data.shape(t) {
                cx.with_clip(screen(clip), |cx| cx.shape(shape, screen(*view), t.color));
            }
            t.stored.set(true);
        };
        cx.with_clip(r, |cx| {
            for levels in &data.layers {
                let tiles = data.near(&levels[level], visible);
                if tiles.first().is_none_or(|t| s < t.min_scale) {
                    continue;
                }
                let mut holes = Vec::new();
                for t in tiles.iter().filter(|t| seen(t.cell)) {
                    if t.stored.get() || start.elapsed() < BUDGET {
                        draw(cx, t, t.cell);
                    } else {
                        holes.push(t.cell);
                    }
                }
                if holes.is_empty() {
                    continue;
                }
                pending = true;
                // Coarser levels in view, finest first, skipping ones that
                // reuse a finer level's tiles.
                let mut coarser: Vec<Vec<&Tile>> = Vec::new();
                let mut used = vec![levels[level].clone()];
                for l in (0..level).rev() {
                    if !used.contains(&levels[l]) {
                        used.push(levels[l].clone());
                        let tiles = data.near(&levels[l], visible);
                        coarser.push(tiles.iter().filter(|t| seen(t.cell)).collect());
                    }
                }
                // The finest that has all it needs stored stands in, else the coarsest.
                for hole in holes {
                    let over = |t: &&&Tile| {
                        let hit = t.cell.intersect(&hole);
                        hit.w > 0.0 && hit.h > 0.0
                    };
                    let mut cover: Vec<&Tile> = Vec::new();
                    for tiles in &coarser {
                        let here: Vec<&Tile> = tiles.iter().filter(over).copied().collect();
                        if here.is_empty() {
                            continue;
                        }
                        cover = here;
                        if cover.iter().all(|t| t.stored.get()) {
                            break;
                        }
                    }
                    for t in cover {
                        draw(cx, t, t.cell.intersect(&hole));
                    }
                }
            }
            for t in data.near(&data.labels, visible) {
                if s < t.min_scale || !seen(t.cell) {
                    continue;
                }
                if t.stored.get() || start.elapsed() < BUDGET {
                    draw(cx, t, t.cell);
                } else {
                    pending = true;
                }
            }
            if let Some(f) = &self.overlay {
                let at = |p: Point| {
                    [
                        r.x + r.w / 2.0 + (p[0] - c[0]) * s,
                        r.y + r.h / 2.0 + (p[1] - c[1]) * s,
                    ]
                };
                f(cx, &at);
            }
        });
        if pending {
            cx.request_frame();
        }
    }

    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let (r, data, now) = (cx.rect, &*self.data, cx.now);
        let inside = |x, y| cx.rect.contains(x, y) && cx.clip.contains(x, y);
        let (mut capture, mut stop, mut tap) = (false, false, None);
        let v = cx.state.get::<View>();
        v.fit(self, r, now);
        match *ev {
            Event::PointerDown { x, y } if inside(x, y) => {
                v.press = Some(([x, y], [x, y]));
                v.panning = false;
            }
            Event::PointerMove { x, y } => {
                let Some((start, last)) = v.press else { return };
                v.panning |= (x - start[0]).hypot(y - start[1]) > SLOP;
                if v.panning {
                    v.glide = None;
                    let s = v.scale;
                    v.center = [
                        v.center[0] - (x - last[0]) / s,
                        v.center[1] - (y - last[1]) / s,
                    ];
                    v.press = Some((start, [x, y]));
                    capture = true;
                }
            }
            Event::PointerUp { x, y } => {
                if v.press.take().is_some() && !v.panning && inside(x, y) {
                    let s = v.scale;
                    let c = v.center;
                    let at = [
                        c[0] + (x - r.x - r.w / 2.0) / s,
                        c[1] + (y - r.y - r.h / 2.0) / s,
                    ];
                    tap = Some((at, s));
                }
            }
            Event::Cancel => v.press = None,
            Event::Wheel { x, y, dy, .. } if inside(x, y) => {
                v.zoom(data, r, x, y, (-dy / 200.0).exp());
                stop = true;
            }
            Event::Pinch { x, y, scale } if inside(x, y) || v.press.is_some() => {
                v.zoom(data, r, x, y, scale);
                // Two fingers held still are a pinch, not a tap.
                (v.panning, capture, stop) = (true, true, true);
            }
            _ => return,
        }
        if capture {
            cx.capture();
        }
        if stop {
            cx.stop();
        }
        if let (Some((p, s)), Some(f)) = (tap, &self.on_tap) {
            f(cx, p, s);
        }
        cx.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x: f32, y: f32, side: f32) -> Vec<Point> {
        vec![[x, y], [x + side, y], [x + side, y + side], [x, y + side]]
    }

    /// Nonzero winding of `rings` around `p`.
    fn covers(rings: &[Vec<Point>], p: Point) -> bool {
        let mut w = 0;
        for r in rings {
            for (k, &a) in r.iter().enumerate() {
                let b = r[(k + 1) % r.len()];
                let side = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
                if a[1] <= p[1] && b[1] > p[1] && side > 0.0 {
                    w += 1;
                } else if a[1] > p[1] && b[1] <= p[1] && side < 0.0 {
                    w -= 1;
                }
            }
        }
        w != 0
    }

    fn empty() -> Rc<MapData> {
        Rc::new(MapData::new(
            Vec::new(),
            Vec::new(),
            Rect::new(0.0, 0.0, 100.0, 100.0),
            &[],
        ))
    }

    #[test]
    fn lines_and_holes_wind_by_role() {
        let mut b = MapBuilder::new(48.0, 11.0);
        let l = b.layer(Color::BLACK, 0.0);
        let mut hole = square(0.0, 0.0, 5.0);
        hole.reverse();
        b.area(l, &[square(0.0, 0.0, 10.0)], &[hole]);
        b.line(l, &[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], 2.0);
        let signs: Vec<bool> = b.layers[0]
            .rings
            .iter()
            .map(|r| signed_area(r) > 0.0)
            .collect();
        assert_eq!(signs, [true, false]);
        let mut stroked = Vec::new();
        stroked.extend(b.layers[0].lines[0].stroke());
        // Two caps, two segments and a bevel outside the bend.
        assert_eq!(stroked.len(), 5);
        assert!(stroked.iter().all(|r| signed_area(r) > 0.0));
    }

    #[test]
    fn clip_cuts_contains_and_drops() {
        let sq = square(0.0, 0.0, 10.0);
        let half = clip(&sq, Rect::new(5.0, -1.0, 10.0, 12.0)).unwrap();
        assert_eq!(signed_area(&half), 100.0);
        let inner = clip(&sq, Rect::new(2.0, 2.0, 2.0, 2.0)).unwrap();
        assert_eq!(signed_area(&inner), 8.0);
        assert!(clip(&sq, Rect::new(20.0, 0.0, 5.0, 5.0)).is_none());
    }

    #[test]
    fn cut_lines_stroke_the_same_inside_the_margin() {
        let run = Run {
            points: vec![
                [-30.0, 5.0],
                [12.0, 18.0],
                [3.0, 26.0],
                [40.0, 31.0],
                [9.0, -20.0],
            ],
            width: 6.0,
            caps: [true, true],
            bevels: true,
        };
        let margin = Rect::new(0.0, 0.0, 30.0, 30.0);
        let pieces = cut_run(&run, grow(margin, run.width / 2.0));
        assert!(pieces.len() > 1);
        assert!(
            pieces.iter().all(|p| p.caps == [false, false]),
            "every end is a cut"
        );
        let (mut whole, mut cut) = (Vec::new(), Vec::new());
        whole.extend(run.stroke());
        for p in &pieces {
            cut.extend(p.stroke());
        }
        let cut: Vec<_> = cut.iter().filter_map(|r| clip(r, margin)).collect();
        for i in 0..60 {
            for j in 0..60 {
                let p = [0.25 + i as f32 * 0.5, 0.25 + j as f32 * 0.5];
                assert_eq!(covers(&whole, p), covers(&cut, p), "at {p:?}");
            }
        }
        let inside = cut_run(&run, Rect::new(-100.0, -100.0, 200.0, 200.0));
        assert_eq!(inside, [run]);
    }

    #[test]
    fn simplify_keeps_the_ends_and_the_corners() {
        let pts = [[0.0, 0.0], [5.0, 0.1], [10.0, 0.0], [10.0, 10.0]];
        assert_eq!(
            simplify(&pts, 0.25),
            [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]
        );
        assert_eq!(simplify(&pts, 0.05), pts);
    }

    #[test]
    fn coarse_levels_are_lighter() {
        let mut b = MapBuilder::new(48.0, 11.0);
        let l = b.layer(Color::BLACK, 0.0);
        let wiggle: Vec<Point> = (0..400)
            .map(|i| [i as f32 * 5.0, (i % 2) as f32 * 0.5])
            .collect();
        b.line(l, &wiggle, 3.0);
        b.area(l, &[square(0.0, 50.0, 2.0)], &[]);
        let data = b.build();
        let weight = |level: usize| {
            let tiles = &data.tiles[data.layers[0][level].tiles.clone()];
            let shapes = tiles.iter().filter_map(|t| data.shape(t));
            shapes
                .map(|(s, _)| s.path().contours.iter().map(Vec::len).sum::<usize>())
                .sum::<usize>()
        };
        assert!(
            weight(0) * 10 < weight(FULL),
            "{} vs {}",
            weight(0),
            weight(FULL)
        );
        let coarse = &data.tiles[data.layers[0][0].tiles.clone()];
        let geo = coarse.iter().map(|t| t.geo.as_ref().unwrap());
        assert!(
            geo.clone().all(|g| g.rings.is_empty()),
            "the 2 m square is too small"
        );
        assert!(
            geo.flat_map(|g| &g.runs)
                .all(|r| r.caps == [false, false] && !r.bevels)
        );
        assert_eq!(
            (level_of(0.01), level_of(0.25), level_of(5.0)),
            (0, 4, FULL)
        );
    }

    #[test]
    fn dense_layers_split_into_small_tiles() {
        let mut b = MapBuilder::new(48.0, 11.0);
        let l = b.layer(Color::BLACK, 0.0);
        for i in 0..20 {
            for j in 0..20 {
                b.area(l, &[square(i as f32 * 20.0, j as f32 * 20.0, 10.0)], &[]);
            }
        }
        let data = b.build();
        assert!(data.layers[0][FULL].tiles.len() > 1);
        for t in &data.tiles {
            let (shape, _) = data.shape(t).unwrap();
            let n: usize = shape.path().contours.iter().map(Vec::len).sum();
            assert!(n <= MAX_QUADS, "{n} segments");
        }
        assert_eq!(data.bounds, Rect::new(0.0, 0.0, 390.0, 390.0));
        // Too small for the coarsest level, too plain to simplify in between.
        assert!(data.layers[0][0].tiles.is_empty());
        assert_eq!(data.layers[0][3], data.layers[0][FULL]);
    }

    #[test]
    fn labels_run_along_upright_and_spaced() {
        let text = Text::default();
        let mut b = MapBuilder::new(48.0, 11.0);
        let l = b.layer(Color::BLACK, 0.0);
        let label =
            |b: &mut MapBuilder, pts: &[Point]| b.label(l, pts, &text, Font::DEFAULT, "Hi", 10.0);
        assert!(!label(&mut b, &[[0.0, 0.0], [10.0, 0.0]]), "too short");
        let bend = [[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]];
        assert!(!label(&mut b, &bend), "turns under the text");
        // Drawn right to left, still read left to right: 'H' comes first.
        assert!(label(&mut b, &[[100.0, 0.0], [0.0, 0.0]]));
        let t = &b.labels[0];
        let glyphs = &t.geo.as_ref().unwrap().glyphs;
        let v = bounds(glyphs.contours.iter().flatten().flatten()).unwrap();
        assert!((v.x + v.w / 2.0 - 50.0).abs() < 2.0);
        assert!((v.y + v.h / 2.0).abs() < 4.0, "centred on the line");
        assert!(glyphs.contours[0][0][0][0] < 50.0);
        assert_eq!(t.min_scale, MIN_LABEL_PX / 10.0);
        // The same text 100 m on waits until they're far enough apart on screen.
        assert!(label(&mut b, &[[100.0, 0.0], [200.0, 0.0]]));
        assert_eq!(b.labels[1].min_scale, LABEL_SPACING_PX / 100.0);
    }

    #[test]
    fn maps_round_trip_through_bytes() {
        let mut b = MapBuilder::new(52.37, 4.89);
        let roads = b.layer(Color::WHITE, 0.0);
        let names = b.layer(Color::BLACK, 0.5);
        b.line(roads, &[[0.0, 0.0], [300.0, 40.0], [600.0, 0.0]], 10.0);
        b.area(roads, &[square(100.0, 100.0, 50.0)], &[]);
        b.label(
            names,
            &[[0.0, 0.0], [600.0, 0.0]],
            &Text::default(),
            Font::DEFAULT,
            "Damrak",
            8.0,
        );
        let built = b.build();
        let bytes: &'static [u8] = Vec::leak(built.to_bytes());
        let read = MapData::from_bytes(bytes).unwrap();
        assert_eq!(read.to_bytes(), bytes);
        assert_eq!(
            (read.tiles(), read.segments()),
            (built.tiles(), built.segments())
        );
        assert_eq!(
            (read.layers.clone(), read.labels.clone()),
            (built.layers.clone(), built.labels.clone())
        );
        for (a, z) in built.tiles.iter().zip(&read.tiles) {
            assert_eq!(
                (a.cell, a.min_scale, a.level),
                (z.cell, z.min_scale, z.level)
            );
            let (va, vz) = (data_view(&built, a), data_view(&read, z));
            // Grid rounding moves edges met at a shallow angle by more.
            assert!((va.x - vz.x).abs() < 1e-2 && (va.w - vz.w).abs() < 1e-2);
        }
        assert!(MapData::from_bytes(&bytes[..bytes.len() - 1]).is_none());
    }

    fn data_view(d: &MapData, t: &Tile) -> Rect {
        d.shape(t).unwrap().1
    }

    #[test]
    fn panning_in_stores_the_tiles_in_view() {
        struct App(Rc<MapData>);
        impl dfui::App for App {
            fn view(&self) -> Element {
                map(&self.0).start([200.0, 200.0], 4.0).into()
            }
        }
        let mut b = MapBuilder::new(48.0, 11.0);
        let l = b.layer(Color::BLACK, 0.0);
        for i in 0..40 {
            b.line(
                l,
                &[[0.0, i as f32 * 10.0], [400.0, i as f32 * 10.0 + 3.0]],
                2.0,
            );
        }
        let data = Rc::new(b.build());
        let mut bench = dfui::Bench::new(App(data.clone()), (100.0, 100.0));
        bench.settle();
        let view = Rect::new(187.5, 187.5, 25.0, 25.0);
        let stored = |t: &Tile| t.stored.get();
        let shown = data.tiles[data.layers[0][FULL].tiles.clone()]
            .iter()
            .filter(|t| {
                let hit = t.cell.intersect(&view);
                hit.w > 0.0 && hit.h > 0.0
            });
        assert!(shown.clone().count() > 0 && shown.clone().all(stored));
        let off = data
            .tiles
            .iter()
            .filter(|t| t.cell.intersect(&view).w <= 0.0 || t.cell.intersect(&view).h <= 0.0);
        assert!(!off.clone().any(stored), "nothing out of view is stored");
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let data = empty();
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut v = View::default();
        v.fit(&map(&data), r, Instant::now());
        let at = |v: &View| {
            [
                v.center[0] + (80.0 - 50.0) / v.scale,
                v.center[1] + (20.0 - 50.0) / v.scale,
            ]
        };
        let before = at(&v);
        v.zoom(&data, r, 80.0, 20.0, 3.0);
        assert_eq!(v.scale, 3.0);
        let after = at(&v);
        assert!((before[0] - after[0]).abs() < 1e-4 && (before[1] - after[1]).abs() < 1e-4);
    }

    #[test]
    fn taps_report_the_map_point_and_pans_dont() {
        use std::cell::RefCell;

        struct App(Rc<MapData>, Rc<RefCell<Vec<(Point, f32)>>>);
        impl dfui::App for App {
            fn view(&self) -> Element {
                let taps = self.1.clone();
                map(&self.0)
                    .on_tap_scaled(move |_, p, s| taps.borrow_mut().push((p, s)))
                    .into()
            }
        }
        let taps = Rc::new(RefCell::new(Vec::new()));
        let mut b = dfui::Bench::new(App(empty(), taps.clone()), (100.0, 100.0));
        b.settle();
        b.event(Event::PointerDown { x: 80.0, y: 20.0 });
        b.event(Event::PointerUp { x: 80.0, y: 20.0 });
        b.event(Event::PointerDown { x: 50.0, y: 50.0 });
        b.event(Event::PointerMove { x: 10.0, y: 50.0 });
        b.event(Event::PointerUp { x: 10.0, y: 50.0 });
        assert_eq!(*taps.borrow(), [([80.0, 20.0], 1.0)]);
    }

    #[test]
    fn focus_moves_the_view_once_per_change() {
        let data = empty();
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut v = View::default();
        let t = Instant::now();
        let focused = map(&data).focus(Some(([10.0, 20.0], 5.0)));
        v.fit(&focused, r, t);
        v.step(t + GLIDE);
        assert_eq!((v.center, v.scale), ([10.0, 20.0], 5.0));
        v.center = [0.0, 0.0];
        v.fit(&focused, r, t);
        assert!(!v.step(t + GLIDE));
        assert_eq!(v.center, [0.0, 0.0]);
    }

    #[test]
    fn fit_zooms_out_as_well_as_in() {
        let data = empty();
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        let mut v = View::default();
        let t = Instant::now();
        v.fit(&map(&data).fit(Rect::new(0.0, 0.0, 10.0, 10.0)), r, t);
        v.step(t + GLIDE);
        assert_eq!((v.center, v.scale), ([5.0, 5.0], 5.0));
        v.fit(&map(&data).fit(Rect::new(0.0, 0.0, 400.0, 100.0)), r, t);
        // Halfway in time is halfway in zoom, geometrically.
        assert!(v.step(t + GLIDE / 2));
        assert!((v.scale - (5.0f32 * 0.25).sqrt()).abs() < 1e-4);
        assert!(!v.step(t + GLIDE));
        assert_eq!((v.center, v.scale), ([200.0, 50.0], 0.25));
    }
}
