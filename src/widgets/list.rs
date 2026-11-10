use std::cell::{OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use taffy::{
    Dimension, Display, FlexDirection, LengthPercentageAuto, Overflow, Point, Position, Style,
};
use web_time::Instant;

use super::scroll::{ScrollState, drag_scroll};
use super::{Styled, column};
use crate::anim::ease;
use crate::model::ListModel;
use crate::runtime::task;
use crate::widget::{BuildCx, Element, Event, EventCx, PaintCx, Placement, State, Widget};

/// Rows slide in, out and to new places over this long.
const MOTION: Duration = Duration::from_millis(450);
/// Row height assumed before any row has been measured.
const ESTIMATE: f32 = 48.0;
/// Viewport assumed before the first placement.
const VIEWPORT: f32 = 800.0;

/// Builds one row from a copy of its item, so a removed row can still be
/// drawn while it slides out.
pub(crate) type Thunk = Rc<dyn Fn() -> Element>;

/// What a [`List`] shows: row count, each row's key, and its builder.
pub(crate) trait Rows {
    fn len(&self) -> usize;
    fn key(&self, i: usize) -> u64;
    fn row(&self, i: usize) -> Thunk;
}

struct ModelRows<M, F> {
    model: M,
    build: Rc<F>,
}

impl<M, F> Rows for ModelRows<M, F>
where
    M: ListModel,
    M::Item: Clone + 'static,
    F: Fn(&M::Item) -> Element + 'static,
{
    fn len(&self) -> usize {
        self.model.len()
    }
    fn key(&self, i: usize) -> u64 {
        self.model.key(i)
    }
    fn row(&self, i: usize) -> Thunk {
        let item = self.model.with(i, Clone::clone);
        let build = self.build.clone();
        Rc::new(move || build(&item))
    }
}

/// Measured row heights by key, written by each row as it is placed.
type Heights = Rc<RefCell<HashMap<u64, f32>>>;

/// Offset animation: x in widths of the row, y in px, as `scroll` returns it.
#[derive(Clone, Copy)]
struct Anim {
    from: (f32, f32),
    to: (f32, f32),
    since: Instant,
}

impl Anim {
    fn at(&self, now: Instant) -> (f32, f32) {
        let t = (now.saturating_duration_since(self.since).as_secs_f32() / MOTION.as_secs_f32())
            .min(1.0);
        let e = ease::in_out_cubic(t);
        let mix = |a: f32, b: f32| a + (b - a) * e;
        (mix(self.from.0, self.to.0), mix(self.from.1, self.to.1))
    }
    fn running(&self, now: Instant) -> bool {
        now < self.since + MOTION
    }
}

/// A removed row, drawn where it was while it slides out.
struct Ghost {
    key: u64,
    row: Thunk,
    /// Content y, px.
    y: f32,
    since: Instant,
}

#[derive(Default)]
struct ListState {
    scroll: ScrollState,
    viewport: f32,
    heights: Heights,
    /// First visible row: key, index, px of it above the viewport's top.
    anchor: Option<(u64, usize, f32)>,
    built: Range<usize>,
    /// Content y of the first built row.
    top: f32,
    /// Built rows in order, with their builders.
    shown: Vec<(u64, Thunk)>,
    /// Screen y (relative to the list's top) of each built row at the last paint.
    painted: HashMap<u64, f32>,
    /// Every key at the last build.
    keys: HashSet<u64>,
    anims: HashMap<u64, Anim>,
    ghosts: Vec<Ghost>,
    built_once: bool,
    /// Anchor key and the content y the last build gave it from estimates.
    expected: Option<(u64, f32)>,
}

impl ListState {
    fn height(&self, key: u64, estimate: f32) -> f32 {
        self.heights.borrow().get(&key).copied().unwrap_or(estimate)
    }
    fn estimate(&self) -> f32 {
        let h = self.heights.borrow();
        match h.len() {
            0 => ESTIMATE,
            n => h.values().sum::<f32>() / n as f32,
        }
    }
    fn viewport(&self) -> f32 {
        if self.viewport > 0.0 {
            self.viewport
        } else {
            VIEWPORT
        }
    }
}

/// Virtualized vertical list: builds only the rows near the viewport, keeps
/// the first visible row in place across rebuilds, and slides rows in, out
/// and to new positions by key. Bound its height like a `scroll`'s.
pub struct List {
    rows: Box<dyn Rows>,
    style: Style,
    animate: bool,
    content: OnceCell<[Element; 1]>,
}

/// A virtualized list of `model`'s items, each drawn by `row`.
pub fn list<M, F, E>(model: M, row: F) -> List
where
    M: ListModel + 'static,
    M::Item: Clone + 'static,
    F: Fn(&M::Item) -> E + 'static,
    E: Into<Element>,
{
    List::new(ModelRows {
        model,
        build: Rc::new(move |it: &M::Item| row(it).into()),
    })
}

impl List {
    pub(crate) fn new(rows: impl Rows + 'static) -> Self {
        List {
            rows: Box::new(rows),
            style: Style {
                display: Display::Block,
                overflow: Point {
                    x: Overflow::Hidden,
                    y: Overflow::Scroll,
                },
                scrollbar_width: 0.0,
                ..Style::default()
            },
            animate: true,
            content: OnceCell::new(),
        }
    }

    /// Whether inserts, removals and moves animate (on by default).
    pub fn animate(mut self, on: bool) -> Self {
        self.animate = on;
        self
    }

    fn build(&self, s: &mut ListState, now: Instant) -> Element {
        let rows = &*self.rows;
        let n = rows.len();
        let keys: Vec<u64> = (0..n).map(|i| rows.key(i)).collect();
        let index: HashMap<u64, usize> = keys.iter().enumerate().map(|(i, &k)| (k, i)).collect();
        let est = s.estimate();
        let mut ys = Vec::with_capacity(n + 1);
        ys.push(0.0);
        for &k in &keys {
            ys.push(ys.last().unwrap() + s.height(k, est));
        }
        let vh = s.viewport();

        // Keep the anchor row where it was, whatever changed above it.
        let old = s.scroll.offset;
        s.expected = None;
        if let Some((k, i, dy)) = s.anchor
            && n > 0
        {
            let i = index.get(&k).copied().unwrap_or(i.min(n - 1));
            s.scroll.offset = ys[i] + dy;
            s.expected = Some((keys[i], ys[i]));
        }
        s.scroll.offset = s.scroll.offset.clamp(0.0, (ys[n] - vh).max(0.0));
        s.scroll.drag.shift(s.scroll.offset - old);
        let offset = s.scroll.offset;

        let (lo, hi) = (offset - vh, offset + 2.0 * vh);
        let first = ys[1..].partition_point(|&b| b <= lo);
        let last = ys[..n].partition_point(|&t| t < hi).max(first);

        if self.animate && s.built_once {
            self.diff(s, &index, &ys, offset, now);
        }
        s.anims.retain(|_, a| a.running(now));
        s.ghosts
            .retain(|g| now < g.since + MOTION && !index.contains_key(&g.key));
        let ghosts: HashSet<u64> = s.ghosts.iter().map(|g| g.key).collect();
        s.heights
            .borrow_mut()
            .retain(|k, _| index.contains_key(k) || ghosts.contains(k));

        // Ghosts first: they slide out behind the rows that take their place.
        let mut children = Vec::new();
        for g in &s.ghosts {
            let anim = Anim {
                from: (0.0, 0.0),
                to: (-1.0, 0.0),
                since: g.since,
            };
            children.push(slot(g.key, &s.heights, Some(anim), (g.row)(), Some(g.y)));
        }
        let spacer = |h: f32| column([]).height(h).into();
        children.push(spacer(ys[first]));
        s.shown.clear();
        for (i, &key) in keys.iter().enumerate().take(last).skip(first) {
            let row = rows.row(i);
            children.push(slot(
                key,
                &s.heights,
                s.anims.get(&key).copied(),
                row(),
                None,
            ));
            s.shown.push((key, row));
        }
        children.push(spacer(ys[n] - ys[last]));
        s.built = first..last;
        s.top = ys[first];
        s.keys = keys.into_iter().collect();
        s.built_once = true;
        column(children).into()
    }

    /// Starts animations for what changed on screen since the last paint.
    fn diff(
        &self,
        s: &mut ListState,
        index: &HashMap<u64, usize>,
        ys: &[f32],
        offset: f32,
        now: Instant,
    ) {
        let vh = s.viewport();
        let est = s.estimate();
        let visible: Vec<(u64, f32)> = s
            .shown
            .iter()
            .filter_map(|(k, _)| Some((*k, *s.painted.get(k)?)))
            .filter(|&(k, y)| y + s.height(k, est) > 0.0 && y < vh)
            .collect();
        let kept = visible
            .iter()
            .filter(|(k, _)| index.contains_key(k))
            .count();
        // A reload that replaces most of the screen just cuts to it.
        if visible.is_empty() || kept * 2 < visible.len() {
            s.anims.clear();
            s.ghosts.clear();
            return;
        }
        for &(key, y) in &visible {
            if !index.contains_key(&key) && !s.ghosts.iter().any(|g| g.key == key) {
                let row = s.shown.iter().find(|r| r.0 == key).unwrap().1.clone();
                s.ghosts.push(Ghost {
                    key,
                    row,
                    y: y + offset,
                    since: now,
                });
            }
        }
        for (&key, &i) in index {
            let y = ys[i] - offset;
            if y + s.height(key, est) <= 0.0 || y >= vh {
                continue;
            }
            let cur = s
                .anims
                .get(&key)
                .filter(|a| a.running(now))
                .map_or((0.0, 0.0), |a| a.at(now));
            let from = match s.painted.get(&key) {
                Some(&was) if (was - y).abs() > 0.5 => (cur.0, cur.1 + y - was),
                Some(_) => continue,
                None if !s.keys.contains(&key) => (-1.0, 0.0),
                None => continue,
            };
            s.anims.insert(
                key,
                Anim {
                    from,
                    to: (0.0, 0.0),
                    since: now,
                },
            );
        }
    }
}

/// One row: reports its height and plays its offset animation, unclipped.
struct Slot {
    key: u64,
    heights: Heights,
    anim: Option<Anim>,
    child: [Element; 1],
    style: Style,
}

fn slot(
    key: u64,
    heights: &Heights,
    anim: Option<Anim>,
    child: Element,
    y: Option<f32>,
) -> Element {
    let mut style = Style {
        flex_direction: FlexDirection::Column,
        ..Style::default()
    };
    if let Some(y) = y {
        style.position = Position::Absolute;
        style.inset.top = LengthPercentageAuto::length(y);
        style.inset.left = LengthPercentageAuto::length(0.0);
        style.size.width = Dimension::percent(1.0);
    }
    Element::new(Slot {
        key,
        heights: heights.clone(),
        anim,
        child: [child],
        style,
    })
    .key(key)
}

impl Widget for Slot {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element] {
        &self.child
    }
    fn clips(&self) -> bool {
        false
    }
    // Only ghosts are absolutely positioned.
    fn inert(&self) -> bool {
        self.style.position == Position::Absolute
    }
    fn scroll(&self, _: &mut State, at: Placement, now: Instant) -> Option<(f32, f32)> {
        if self.style.position != Position::Absolute {
            self.heights.borrow_mut().insert(self.key, at.rest.h);
        }
        let (x, y) = self.anim?.at(now);
        Some((x * at.rest.w, y))
    }
    fn paint(&self, cx: &mut PaintCx) {
        if self.anim.is_some_and(|a| a.running(cx.now)) {
            cx.request_placement();
        }
    }
}

impl From<List> for Element {
    fn from(w: List) -> Self {
        Element::new(w)
    }
}

impl Styled for List {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for List {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn expand(&self, cx: &mut BuildCx) {
        let s = cx.state.get::<ListState>();
        self.content.get_or_init(|| [self.build(s, task::now())]);
    }
    fn children(&self) -> &[Element] {
        self.content.get().map_or(&[], |c| c.as_slice())
    }
    fn scroll(&self, state: &mut State, at: Placement, now: Instant) -> Option<(f32, f32)> {
        let s = state.get::<ListState>();
        s.viewport = at.rest.h;
        if let Some(o) = s.scroll.drag.fling(now, s.scroll.max) {
            s.scroll.offset = o;
        }
        Some((0.0, s.scroll.offset))
    }
    fn paint(&self, cx: &mut PaintCx) {
        let now = cx.now;
        let s = cx.state.get::<ListState>();
        let est = s.estimate();
        let mut corrected = false;
        // Rows built from estimates measured differently: move the content
        // so the anchor row stays put.
        // ponytail: corrected a frame late; the rebuilt frame shows the error once.
        if let Some((key, want)) = s.expected.take()
            && let Some(j) = s.shown.iter().position(|r| r.0 == key)
        {
            let got = s.top + s.shown[..j].iter().map(|r| s.height(r.0, est)).sum::<f32>();
            if (got - want).abs() > 0.5 {
                s.scroll.offset += got - want;
                s.scroll.drag.shift(got - want);
                corrected = true;
            }
        }
        let (offset, vh) = (s.scroll.offset, s.viewport());
        let mut y = s.top - offset;
        let mut anchor = None;
        let mut painted = HashMap::with_capacity(s.shown.len());
        for (j, &(key, _)) in s.shown.iter().enumerate() {
            let h = s.height(key, est);
            if anchor.is_none() && y + h > 0.0 {
                anchor = Some((key, s.built.start + j, -y));
            }
            painted.insert(key, y);
            y += h;
        }
        s.painted = painted;
        // None when scrolled past every built row: the offset stands as is.
        s.anchor = anchor;
        // Scrolled past half the margin of rows built beyond the viewport.
        let (top, bottom) = (s.top - offset, y);
        let n = self.rows.len();
        let short =
            (s.built.start > 0 && top > -vh / 2.0) || (s.built.end < n && bottom < 1.5 * vh);
        let expired = s.ghosts.iter().any(|g| now >= g.since + MOTION);
        let flinging = s.scroll.drag.flinging();
        if short || expired {
            cx.request_rebuild();
        } else if !s.ghosts.is_empty() {
            cx.request_frame();
        }
        if flinging || corrected {
            cx.request_placement();
        }
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        drag_scroll(ev, cx, |st| &mut st.get::<ListState>().scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Keyed;
    use crate::widgets::{button, text};
    use crate::{App, Bench};

    #[derive(Clone)]
    struct It(u64);
    impl Keyed for It {
        fn key(&self) -> u64 {
            self.0
        }
    }

    thread_local! {
        /// Where each row was painted in the last frame.
        static PAINTED: RefCell<HashMap<u64, (f32, f32)>> = RefCell::default();
    }

    struct Probe(u64);
    impl Widget for Probe {
        fn style(&self) -> Style {
            Style {
                size: taffy::Size {
                    width: Dimension::auto(),
                    height: Dimension::length(50.0),
                },
                ..Style::default()
            }
        }
        fn paint(&self, cx: &mut PaintCx) {
            PAINTED.with(|p| p.borrow_mut().insert(self.0, (cx.rect.x, cx.rect.y)));
        }
    }

    struct L(Rc<RefCell<Vec<It>>>);
    impl App for L {
        fn view(&self) -> Element {
            column([list(self.0.clone(), |it: &It| Element::new(Probe(it.0)))
                .height(200.0)
                .into()])
            .into()
        }
    }

    fn painted(b: &mut Bench<L>) -> HashMap<u64, (f32, f32)> {
        PAINTED.with(|p| p.borrow_mut().clear());
        b.paint();
        PAINTED.with(|p| p.take())
    }

    #[test]
    fn rows_sliding_out_take_no_taps() {
        thread_local!(static TAPS: RefCell<Vec<u64>> = RefCell::default());
        struct T(Rc<RefCell<Vec<It>>>);
        impl App for T {
            fn view(&self) -> Element {
                let row = |it: &It| {
                    let k = it.0;
                    button(text("row"))
                        .on_click(move |_| TAPS.with(|t| t.borrow_mut().push(k)))
                        .height(50.0)
                };
                column([list(self.0.clone(), row).height(200.0).into()]).into()
            }
        }
        let items = Rc::new(RefCell::new((0..6).map(It).collect::<Vec<_>>()));
        let mut b = Bench::new(T(items.clone()), (100.0, 200.0));
        b.settle();
        items.borrow_mut().retain(|it| it.0 != 1);
        b.frame();
        b.advance(MOTION / 2);
        b.paint();
        b.event(Event::PointerDown { x: 75.0, y: 75.0 });
        b.event(Event::PointerUp { x: 75.0, y: 75.0 });
        assert_eq!(
            TAPS.with(|t| t.take()),
            [2],
            "only the row taking its place"
        );
    }

    #[test]
    fn fling_builds_rows_where_it_stops() {
        let items = Rc::new(RefCell::new((0..1000).map(It).collect::<Vec<_>>()));
        let mut b = Bench::new(L(items.clone()), (100.0, 200.0));
        b.settle();
        let ms = std::time::Duration::from_millis(8);
        b.event(Event::PointerDown { x: 10.0, y: 190.0 });
        for i in 1..=8 {
            b.advance(ms);
            b.event(Event::PointerMove {
                x: 10.0,
                y: 190.0 - i as f32 * 22.0,
            });
        }
        b.event(Event::PointerUp { x: 10.0, y: 14.0 });
        b.settle();
        let p = painted(&mut b);
        let mut ys: Vec<f32> = p
            .values()
            .map(|r| r.1)
            .filter(|y| (-50.0..200.0).contains(y))
            .collect();
        ys.sort_by(f32::total_cmp);
        assert!(ys.len() >= 4, "viewport covered: {ys:?}");
        assert!(!p.contains_key(&0), "flung away from the top");
    }

    #[test]
    fn builds_near_rows_keeps_anchor_and_slides_removals_out() {
        let items = Rc::new(RefCell::new((0..1000).map(It).collect::<Vec<_>>()));
        let mut b = Bench::new(L(items.clone()), (100.0, 200.0));
        b.settle();
        let p = painted(&mut b);
        assert!(p.len() < 30, "built {} rows", p.len());
        assert_eq!(p[&0].1, 0.0);

        b.event(Event::Wheel {
            x: 10.0,
            y: 10.0,
            dy: 5000.0,
        });
        b.settle();
        let p = painted(&mut b);
        assert_eq!(
            p.get(&100).map(|r| r.1),
            Some(0.0),
            "scrolled past the built rows"
        );
        assert!(!p.contains_key(&0));

        items
            .borrow_mut()
            .splice(0..0, [It(2000), It(2001), It(2002)]);
        b.frame();
        b.settle();
        assert_eq!(painted(&mut b)[&100].1, 0.0, "insert above keeps the view");

        items.borrow_mut().retain(|it| it.0 != 101);
        b.frame();
        b.advance(MOTION / 2);
        let p = painted(&mut b);
        let (gx, gy) = p[&101];
        assert!(
            gx > 0.0 && gy == 50.0,
            "ghost slides right in place: {:?}",
            p[&101]
        );
        let y = p[&102].1;
        assert!(y > 50.0 && y < 100.0, "next row slides up: {y}");
        b.settle();
        let p = painted(&mut b);
        assert!(!p.contains_key(&101), "ghost gone");
        assert_eq!(p[&102].1, 50.0);
    }
}
