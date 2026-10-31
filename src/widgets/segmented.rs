use std::cell::OnceCell;
use std::time::Duration;

use taffy::util::ResolveOrZero;
use taffy::{AlignItems, Dimension, JustifyContent, LengthPercentage, Style};

use super::{Styled, container, text};
use crate::anim::{Animated, ease};
use crate::render::{Color, Rect, Shadow};
use crate::widget::{Element, Event, EventCx, PaintCx, State, Widget};

type OnChange<Msg> = Box<dyn Fn(&mut EventCx<Msg>, usize)>;

const SLIDE: Duration = Duration::from_millis(200);
const INK: Color = Color::hex(0x1d2030);
const MUTED: Color = Color::hex(0x6b7080);
const TRACK: Color = Color::hex(0xe8e9ef);
const DIVIDER: Color = Color::hex(0xdfe1e8);

#[derive(Clone, Copy, PartialEq)]
enum Look {
    Segmented,
    Tabs,
}

#[derive(Default)]
struct SegState {
    /// Option index pressed down, chosen if released over the same one.
    pressed: Option<usize>,
    /// Indicator position in option widths.
    at: Animated<f32>,
}

/// A row of equal-width options with a sliding indicator under `selected`.
pub struct Segmented<Msg> {
    labels: Vec<String>,
    selected: usize,
    look: Look,
    accent: Color,
    on_change: Option<OnChange<Msg>>,
    children: OnceCell<Vec<Element<Msg>>>,
    style: Style,
}

/// Pill-shaped segmented button.
pub fn segmented<Msg, S: Into<String>>(
    labels: impl IntoIterator<Item = S>,
    selected: usize,
) -> Segmented<Msg> {
    Segmented {
        labels: labels.into_iter().map(Into::into).collect(),
        selected,
        look: Look::Segmented,
        accent: Color::hex(0x3b6fe0),
        on_change: None,
        children: OnceCell::new(),
        style: Style {
            padding: taffy::Rect::length(3.0),
            ..Style::default()
        },
    }
}

/// Tab bar with an underline; the app shows the selected tab's content.
pub fn tabs<Msg, S: Into<String>>(
    labels: impl IntoIterator<Item = S>,
    selected: usize,
) -> Segmented<Msg> {
    Segmented {
        look: Look::Tabs,
        style: Style::default(),
        ..segmented(labels, selected)
    }
}

impl<Msg> Segmented<Msg> {
    /// Called with the tapped option's index when it isn't already selected.
    pub fn on_change(mut self, f: impl Fn(&mut EventCx<Msg>, usize) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }
    pub fn accent(mut self, c: Color) -> Self {
        self.accent = c;
        self
    }

    /// Content box and one option's width.
    fn slots(&self, r: Rect) -> (Rect, f32) {
        let p = self.style.padding;
        let res = |l: LengthPercentage| l.resolve_or_zero(Some(r.w), |_, _| 0.0);
        let (l, t) = (res(p.left), res(p.top));
        let c = Rect::new(
            r.x + l,
            r.y + t,
            r.w - l - res(p.right),
            r.h - t - res(p.bottom),
        );
        (c, c.w / self.labels.len().max(1) as f32)
    }

    fn index_at(&self, r: Rect, x: f32) -> usize {
        let (c, w) = self.slots(r);
        (((x - c.x) / w).max(0.0) as usize).min(self.labels.len().saturating_sub(1))
    }
}

impl<Msg: 'static> From<Segmented<Msg>> for Element<Msg> {
    fn from(w: Segmented<Msg>) -> Self {
        Element::new(w)
    }
}

impl<Msg> Styled for Segmented<Msg> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<Msg: 'static> Widget<Msg> for Segmented<Msg> {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn expand(&self, _id: u64, _state: &mut State) {
        self.children.get_or_init(|| {
            let pad = if self.look == Look::Tabs { 12.0 } else { 7.0 };
            self.labels
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let color = match (self.look, i == self.selected) {
                        (Look::Tabs, true) => self.accent,
                        (Look::Tabs, false) => MUTED,
                        (Look::Segmented, _) => INK,
                    };
                    let mut c = container(text(l).size(15.0).color(color))
                        .grow(1.0)
                        .padding_xy(12.0, pad)
                        .align(AlignItems::CENTER)
                        .justify(JustifyContent::CENTER);
                    // Equal widths whatever the label lengths.
                    c.style_mut().flex_basis = Dimension::length(0.0);
                    c.into()
                })
                .collect()
        });
    }
    fn children(&self) -> &[Element<Msg>] {
        self.children.get().map_or(&[], |c| c.as_slice())
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (now, r) = (cx.now, cx.rect);
        let s = cx.state.get::<SegState>();
        s.at.to(self.selected as f32, now, SLIDE, ease::out_cubic);
        let at = s.at.get(now);
        if s.at.running(now) {
            cx.request_frame();
        }
        let (c, w) = self.slots(r);
        let x = c.x + at * w;
        match self.look {
            Look::Segmented => {
                cx.fill(r, 9.0, TRACK);
                let ind = Rect::new(x, c.y, w, c.h);
                let sh = Shadow {
                    color: Color::BLACK.alpha(0.15),
                    offset: (0.0, 1.0),
                    blur: 3.0,
                };
                cx.shadow(ind, 7.0, sh);
                cx.fill(ind, 7.0, Color::WHITE);
            }
            Look::Tabs => {
                cx.fill(Rect::new(r.x, r.y + r.h - 1.0, r.w, 1.0), 0.0, DIVIDER);
                cx.fill(Rect::new(x, r.y + r.h - 3.0, w, 3.0), 1.5, self.accent);
            }
        }
    }
    fn event(&self, ev: &Event, cx: &mut EventCx<Msg>) {
        let r = cx.rect;
        match *ev {
            Event::PointerDown { x, y } if cx.hit(x, y) => {
                cx.state.get::<SegState>().pressed = Some(self.index_at(r, x));
            }
            Event::PointerUp { x, y } => {
                let pressed = cx.state.get::<SegState>().pressed.take();
                let i = self.index_at(r, x);
                if cx.hit(x, y)
                    && pressed == Some(i)
                    && i != self.selected
                    && let Some(f) = &self.on_change
                {
                    f(cx, i);
                }
            }
            Event::Cancel => cx.state.get::<SegState>().pressed = None,
            _ => {}
        }
    }
}
