use std::time::Duration;

use taffy::{AlignItems, LengthPercentage, Style};

use super::Styled;
use super::button::Tap;
use crate::anim::{Animated, Lerp, ease};
use crate::render::{Color, Rect};
use crate::widget::{Element, Event, EventCx, PaintCx, Widget};

type Handler = Box<dyn Fn(&mut EventCx)>;

const GROW: Duration = Duration::from_millis(140);
const RING: Color = Color::hex(0x9a9fb0);
const DOT: f32 = 20.0;
const GAP: f32 = 10.0;

#[derive(Default)]
struct RadioState {
    tap: Tap,
    /// 0 unselected, 1 selected.
    dot: Animated<f32>,
}

/// One option of a group: a ring before `child`, filled when `selected`.
/// The whole row is the tap target.
pub struct Radio {
    child: [Element; 1],
    selected: bool,
    accent: Color,
    on_select: Option<Handler>,
    style: Style,
}

pub fn radio(selected: bool, child: impl Into<Element>) -> Radio {
    Radio {
        child: [child.into()],
        selected,
        accent: Color::hex(0x3b6fe0),
        on_select: None,
        style: Style {
            padding: taffy::Rect {
                left: LengthPercentage::length(DOT + GAP),
                right: LengthPercentage::length(0.0),
                top: LengthPercentage::length(6.0),
                bottom: LengthPercentage::length(6.0),
            },
            min_size: taffy::Size {
                width: taffy::LengthPercentageAuto::auto(),
                height: taffy::LengthPercentageAuto::length(DOT + 12.0),
            },
            align_items: AlignItems::CENTER,
            ..Style::default()
        },
    }
}

impl Radio {
    /// Runs when tapped while not selected.
    pub fn on_select(mut self, f: impl Fn(&mut EventCx) + 'static) -> Self {
        self.on_select = Some(Box::new(f));
        self
    }
    pub fn accent(mut self, c: Color) -> Self {
        self.accent = c;
        self
    }
}

impl From<Radio> for Element {
    fn from(w: Radio) -> Self {
        Element::new(w)
    }
}

impl Styled for Radio {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Radio {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element] {
        &self.child
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (now, r) = (cx.now, cx.rect);
        let s = cx.state.get::<RadioState>();
        let target = if self.selected { 1.0 } else { 0.0 };
        s.dot.to(target, now, GROW, ease::out_cubic);
        let t = s.dot.get(now);
        if s.dot.running(now) {
            cx.request_frame();
        }
        let ring = Rect::new(r.x, r.y + (r.h - DOT) / 2.0, DOT, DOT);
        let c = RING.lerp(self.accent, t);
        cx.fill_bordered(ring, DOT / 2.0, Color::TRANSPARENT, 2.0, c);
        let d = 10.0 * t;
        if d > 0.0 {
            let o = (DOT - d) / 2.0;
            let dot = Rect::new(ring.x + o, ring.y + o, d, d);
            cx.fill(dot, d / 2.0, self.accent);
        }
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let mut tap = std::mem::take(&mut cx.state.get::<RadioState>().tap);
        let fire = tap.event(ev, cx);
        cx.state.get::<RadioState>().tap = tap;
        if fire
            && !self.selected
            && let Some(f) = &self.on_select
        {
            f(cx);
        }
    }
}
