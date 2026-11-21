//! Experimental vector maps for dfui: the app projects its geometry (e.g.
//! from OpenStreetMap) with a [`MapBuilder`], which packs it into tiled
//! [`Shape`]s, and [`map`] pans and zooms them. Shapes are stored on the GPU
//! once, so moving the map only moves the rects they're drawn into.

use std::collections::HashMap;
use std::rc::Rc;

use dfui::path::Point;
use dfui::taffy::{Dimension, Size, Style};
use dfui::widgets::Styled;
use dfui::{Color, Element, Event, EventCx, Font, PaintCx, Path, Rect, Shape, Text, Widget};

/// Most segments in one shape: the shader walks a band's segments per pixel.
const MAX_QUADS: usize = 128;
/// Cells aren't split below this side, metres.
const MIN_CELL: f32 = 16.0;
/// How far past its cell a tile's geometry is cut, metres, so the cut edge
/// stays outside the cell's clip and never antialiases into a seam. Exact
/// while it is half a px or more: from 0.25 px per metre.
const MARGIN: f32 = 2.0;
const EARTH_RADIUS: f64 = 6_371_000.0;
/// Edges of a half-disk line cap; flattening stays under 2% of the width.
const CAP_STEPS: usize = 8;
/// Font size, logical px, at which a label starts to show.
const MIN_LABEL_PX: f32 = 7.0;
/// Least screen distance between labels with the same text, logical px.
const LABEL_SPACING_PX: f32 = 300.0;
/// Sharpest turn between neighbouring glyphs of a label, radians.
const MAX_LABEL_TURN: f32 = 0.35;
/// Pointer travel before a press becomes a pan, logical px.
const SLOP: f32 = 6.0;

/// Collects areas and lines in metres around an origin and packs them into
/// [`MapData`]. Layers draw in the order they were added.
pub struct MapBuilder {
    origin: (f64, f64),
    cos: f64,
    layers: Vec<LayerData>,
    /// One shape each, drawn over every layer.
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
        });
        Layer(self.layers.len() - 1)
    }

    /// A filled area, e.g. an OSM multipolygon: `outers` filled, `holes` cut
    /// out of them. Rings may wind either way.
    pub fn area(&mut self, layer: Layer, outers: &[Vec<Point>], holes: &[Vec<Point>]) {
        for r in outers {
            self.ring(layer, r.clone(), true);
        }
        for r in holes {
            self.ring(layer, r.clone(), false);
        }
    }

    /// A line `width` metres wide through `points`, with bevelled joins.
    pub fn line(&mut self, layer: Layer, points: &[Point], width: f32) {
        let h = width / 2.0;
        let normal = |a: Point, b: Point| {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len = dx.hypot(dy);
            (len > 0.0).then(|| [-dy / len * h, dx / len * h])
        };
        let add = |p: Point, n: Point, s: f32| [p[0] + n[0] * s, p[1] + n[1] * s];
        let mut pts = points.to_vec();
        pts.dedup();
        let n = pts.len();
        if n < 2 {
            return;
        }
        // Round caps: OSM splits roads into ways that meet at any angle.
        for (a, b) in [(pts[0], pts[1]), (pts[n - 1], pts[n - 2])] {
            let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
            let k = h / dx.hypot(dy);
            let (ux, uy) = (dx * k, dy * k);
            let cap = (0..=CAP_STEPS)
                .map(|i| {
                    let t = std::f32::consts::PI * (i as f32 / CAP_STEPS as f32 - 0.5);
                    let (s, c) = t.sin_cos();
                    [a[0] + ux * c - uy * s, a[1] + uy * c + ux * s]
                })
                .collect();
            self.ring(layer, cap, true);
        }
        let mut last: Option<Point> = None;
        for w in pts.windows(2) {
            let Some(n) = normal(w[0], w[1]) else {
                continue;
            };
            let quad = vec![
                add(w[0], n, 1.0),
                add(w[1], n, 1.0),
                add(w[1], n, -1.0),
                add(w[0], n, -1.0),
            ];
            self.ring(layer, quad, true);
            // Only the outside of a bend has a notch; overlaps fill as one
            // under nonzero winding.
            if let Some(m) = last {
                let turn = m[0] * n[1] - m[1] * n[0];
                if turn != 0.0 {
                    let s = -turn.signum();
                    self.ring(layer, vec![w[0], add(w[0], m, s), add(w[0], n, s)], true);
                }
            }
            last = Some(n);
        }
    }

    /// `s` at `size` metres along the polyline, e.g. a street's name inside
    /// it: centred, reading left to right, each glyph turned to the line
    /// under its middle. Shows once `size` reaches [`MIN_LABEL_PX`] and the
    /// same text placed earlier is [`LABEL_SPACING_PX`] away; false,
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
        let path = Path { contours };
        let Some(view) = bounds(path.contours.iter().flatten().flatten()) else {
            return false;
        };
        let (c, _) = at(length / 2.0);
        let near = self.placed.entry(s.to_owned()).or_default();
        let d = near.iter().map(|p| (p[0] - c[0]).hypot(p[1] - c[1]));
        let spaced = LABEL_SPACING_PX / d.fold(f32::MAX, f32::min);
        near.push(c);
        let l = &self.layers[layer.0];
        let pad = size / 4.0;
        self.labels.push(Tile {
            shape: Shape::new(path, view),
            view,
            cell: Rect::new(
                view.x - pad,
                view.y - pad,
                view.w + 2.0 * pad,
                view.h + 2.0 * pad,
            ),
            color: l.color,
            min_scale: l.min_scale.max(MIN_LABEL_PX / size).max(spaced),
        });
        true
    }

    fn ring(&mut self, layer: Layer, mut ring: Vec<Point>, outer: bool) {
        let area = signed_area(&ring);
        if ring.len() < 3 || area == 0.0 {
            return;
        }
        if (area > 0.0) != outer {
            ring.reverse();
        }
        self.layers[layer.0].rings.push(ring);
    }

    /// Cuts every layer into quadtree cells of at most [`MAX_QUADS`] segments.
    pub fn build(self) -> MapData {
        let mut tiles = Vec::new();
        for l in &self.layers {
            if let Some(b) = bounds(l.rings.iter().flatten()) {
                split(l, b, l.rings.clone(), &mut tiles);
            }
        }
        let bounds = tiles.iter().map(|t| t.cell).reduce(|a, b| a.union(&b));
        tiles.extend(self.labels);
        MapData {
            tiles,
            bounds: bounds.unwrap_or_default(),
        }
    }
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

/// One tile for `rings` (already cut to `cell` plus [`MARGIN`]), or four
/// for its quarters while it holds too many segments.
fn split(l: &LayerData, cell: Rect, rings: Vec<Vec<Point>>, tiles: &mut Vec<Tile>) {
    let n: usize = rings.iter().map(Vec::len).sum();
    let Some(view) = bounds(rings.iter().flatten()) else {
        return;
    };
    if n <= MAX_QUADS || cell.w.max(cell.h) < MIN_CELL {
        let contours = rings
            .iter()
            .flat_map(|r| Path::polygon(r).contours)
            .collect();
        tiles.push(Tile {
            shape: Shape::new(Path { contours }, view),
            view,
            cell,
            color: l.color,
            min_scale: l.min_scale,
        });
        return;
    }
    let (w, h) = (cell.w / 2.0, cell.h / 2.0);
    for (i, j) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let c = Rect::new(cell.x + i * w, cell.y + j * h, w, h);
        let m = Rect::new(
            c.x - MARGIN,
            c.y - MARGIN,
            w + 2.0 * MARGIN,
            h + 2.0 * MARGIN,
        );
        let cut = rings.iter().filter_map(|r| clip(r, m)).collect();
        split(l, c, cut, tiles);
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

struct Tile {
    shape: Shape,
    /// The shape's view box, metres.
    view: Rect,
    /// What the tile covers and is clipped to when drawn, metres.
    cell: Rect,
    color: Color,
    min_scale: f32,
}

/// Geometry ready to draw, from [`MapBuilder::build`]. Build it once and
/// share it, e.g. in the `App`.
pub struct MapData {
    tiles: Vec<Tile>,
    bounds: Rect,
}

impl MapData {
    /// The shapes stored, for gauging how heavy a map is.
    pub fn tiles(&self) -> usize {
        self.tiles.len()
    }

    /// The segments across all shapes, each 32 bytes on the GPU.
    pub fn segments(&self) -> usize {
        let quads = self
            .tiles
            .iter()
            .map(|t| t.shape.path().contours.iter().map(Vec::len));
        quads.flatten().sum()
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
}

impl View {
    fn fit(&mut self, map: &Map, r: Rect) {
        let b = map.data.bounds;
        if self.scale <= 0.0 && b.w > 0.0 {
            (self.center, self.scale) = map.start.unwrap_or((
                [b.x + b.w / 2.0, b.y + b.h / 2.0],
                (r.w / b.w).min(r.h / b.h),
            ));
        }
        if map.focus != self.focused {
            if let Some((center, scale)) = map.focus {
                (self.center, self.scale) = (center, self.scale.max(scale));
            }
            self.focused = map.focus;
        }
    }

    /// Zooms by `k` keeping the metre under `(x, y)` in place.
    fn zoom(&mut self, data: &MapData, r: Rect, x: f32, y: f32, k: f32) {
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
        on_tap: None,
        overlay: None,
    }
}

type TapFn = dyn Fn(&mut EventCx, Point);
type OverlayFn = dyn Fn(&mut PaintCx, &dyn Fn(Point) -> Point);

/// See [`map`].
pub struct Map {
    data: Rc<MapData>,
    style: Style,
    start: Option<(Point, f32)>,
    focus: Option<(Point, f32)>,
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

    /// Paints over the map, given a function from map points to logical px,
    /// e.g. to draw markers that keep their size while zooming.
    pub fn overlay(mut self, f: impl Fn(&mut PaintCx, &dyn Fn(Point) -> Point) + 'static) -> Self {
        self.overlay = Some(Rc::new(f));
        self
    }

    /// Runs on a tap that didn't pan or pinch, with the map point under it
    /// in [`MapBuilder::point`] metres.
    pub fn on_tap(mut self, f: impl Fn(&mut EventCx, Point) + 'static) -> Self {
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
        let r = cx.rect;
        let v = cx.state.get::<View>();
        v.fit(self, r);
        let (s, c) = (v.scale, v.center);
        let visible = Rect::new(c[0] - r.w / 2.0 / s, c[1] - r.h / 2.0 / s, r.w / s, r.h / s);
        let screen = |b: Rect| {
            let x = r.x + r.w / 2.0 + (b.x - c[0]) * s;
            let y = r.y + r.h / 2.0 + (b.y - c[1]) * s;
            Rect::new(x, y, b.w * s, b.h * s)
        };
        cx.with_clip(r, |cx| {
            for t in &self.data.tiles {
                let hit = visible.intersect(&t.cell);
                if s < t.min_scale || hit.w <= 0.0 || hit.h <= 0.0 {
                    continue;
                }
                cx.with_clip(screen(t.cell), |cx| {
                    cx.shape(&t.shape, screen(t.view), t.color);
                });
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
    }

    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let (r, data) = (cx.rect, &*self.data);
        let inside = |x, y| cx.rect.contains(x, y) && cx.clip.contains(x, y);
        let (mut capture, mut stop, mut tap) = (false, false, None);
        let v = cx.state.get::<View>();
        v.fit(self, r);
        match *ev {
            Event::PointerDown { x, y } if inside(x, y) => {
                v.press = Some(([x, y], [x, y]));
                v.panning = false;
            }
            Event::PointerMove { x, y } => {
                let Some((start, last)) = v.press else { return };
                v.panning |= (x - start[0]).hypot(y - start[1]) > SLOP;
                if v.panning {
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
                    tap = Some([
                        c[0] + (x - r.x - r.w / 2.0) / s,
                        c[1] + (y - r.y - r.h / 2.0) / s,
                    ]);
                }
            }
            Event::Cancel => v.press = None,
            Event::Wheel { x, y, dy } if inside(x, y) => {
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
        if let (Some(p), Some(f)) = (tap, &self.on_tap) {
            f(cx, p);
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
        // Outline, hole, two caps, two segments and a bevel outside the bend.
        assert_eq!(signs, [true, false, true, true, true, true, true]);
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
    fn dense_layers_split_into_small_tiles() {
        let mut b = MapBuilder::new(48.0, 11.0);
        let l = b.layer(Color::BLACK, 0.0);
        for i in 0..20 {
            for j in 0..20 {
                b.area(l, &[square(i as f32 * 20.0, j as f32 * 20.0, 10.0)], &[]);
            }
        }
        let data = b.build();
        assert!(data.tiles() > 1);
        for t in &data.tiles {
            let n: usize = t.shape.path().contours.iter().map(Vec::len).sum();
            assert!(n <= MAX_QUADS, "{n} segments");
        }
        assert_eq!(data.bounds, Rect::new(0.0, 0.0, 390.0, 390.0));
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
        assert!((t.view.x + t.view.w / 2.0 - 50.0).abs() < 2.0);
        assert!(
            (t.view.y + t.view.h / 2.0).abs() < 4.0,
            "centred on the line"
        );
        let first = t.shape.path().contours[0][0][0][0];
        assert!(first < 50.0);
        assert_eq!(t.min_scale, MIN_LABEL_PX / 10.0);
        // The same text 100 m on waits until they're far enough apart on screen.
        assert!(label(&mut b, &[[100.0, 0.0], [200.0, 0.0]]));
        assert_eq!(b.labels[1].min_scale, LABEL_SPACING_PX / 100.0);
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        let data = Rc::new(MapData {
            tiles: Vec::new(),
            bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
        });
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut v = View::default();
        v.fit(&map(&data), r);
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

        struct App(Rc<MapData>, Rc<RefCell<Vec<Point>>>);
        impl dfui::App for App {
            fn view(&self) -> Element {
                let taps = self.1.clone();
                map(&self.0)
                    .on_tap(move |_, p| taps.borrow_mut().push(p))
                    .into()
            }
        }
        let data = Rc::new(MapData {
            tiles: Vec::new(),
            bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
        });
        let taps = Rc::new(RefCell::new(Vec::new()));
        let mut b = dfui::Bench::new(App(data, taps.clone()), (100.0, 100.0));
        b.settle();
        b.event(Event::PointerDown { x: 80.0, y: 20.0 });
        b.event(Event::PointerUp { x: 80.0, y: 20.0 });
        b.event(Event::PointerDown { x: 50.0, y: 50.0 });
        b.event(Event::PointerMove { x: 10.0, y: 50.0 });
        b.event(Event::PointerUp { x: 10.0, y: 50.0 });
        assert_eq!(*taps.borrow(), [[80.0, 20.0]]);
    }

    #[test]
    fn focus_moves_the_view_once_per_change() {
        let data = Rc::new(MapData {
            tiles: Vec::new(),
            bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
        });
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let mut v = View::default();
        let focused = map(&data).focus(Some(([10.0, 20.0], 5.0)));
        v.fit(&focused, r);
        assert_eq!((v.center, v.scale), ([10.0, 20.0], 5.0));
        v.center = [0.0, 0.0];
        v.fit(&focused, r);
        assert_eq!(v.center, [0.0, 0.0]);
    }
}
