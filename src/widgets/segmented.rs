use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use taffy::util::ResolveOrZero;
use taffy::{AvailableSpace, LengthPercentage, Size, Style};

use super::Styled;
use super::label::weight;
use crate::anim::{Animated, Lerp, ease};
use crate::render::{Color, Rect, Shadow};
use crate::text::{Font, Text};
use crate::widget::{Element, Event, EventCx, PaintCx, Widget};

type OnChange = Box<dyn Fn(&mut EventCx, usize)>;

const SLIDE: Duration = Duration::from_millis(200);
/// Regular to bold and back, for tab labels.
const WEIGHT: Duration = Duration::from_millis(220);
const SIZE: f32 = 15.0;
/// Room on each side of a tab's regular label, enough for it to turn bold
/// (DejaVu Sans Bold is up to ~8 px wider at this size); spare width is
/// shared out on top.
const TAB_PAD: f32 = 5.0;
const INK: Color = Color::hex(0x1d2030);
const MUTED: Color = Color::hex(0x6b7080);
const TRACK: Color = Color::hex(0xe8e9ef);
const DIVIDER: Color = Color::hex(0xdfe1e8);

#[derive(Clone, Copy, PartialEq, Hash)]
enum Look {
    Segmented,
    Tabs,
}

impl Look {
    /// Space above and below the labels.
    fn pad_y(self) -> f32 {
        match self {
            Look::Segmented => 7.0,
            Look::Tabs => 12.0,
        }
    }
}

#[derive(Default)]
struct SegState {
    /// Option index pressed down, chosen if released over the same one.
    pressed: Option<usize>,
    /// Indicator position in options.
    at: Animated<f32>,
    /// Each tab label's weight: 0 regular, 1 bold.
    weight: Vec<Animated<f32>>,
}

/// A row of options with a sliding indicator under `selected`.
pub struct Segmented {
    labels: Vec<String>,
    selected: usize,
    look: Look,
    accent: Color,
    on_change: Option<OnChange>,
    style: Style,
}

/// Pill-shaped segmented button with equal-width options.
pub fn segmented<S: Into<String>>(
    labels: impl IntoIterator<Item = S>,
    selected: usize,
) -> Segmented {
    Segmented {
        labels: labels.into_iter().map(Into::into).collect(),
        selected,
        look: Look::Segmented,
        accent: Color::hex(0x3b6fe0),
        on_change: None,
        style: Style {
            padding: taffy::Rect::length(3.0),
            ..Style::default()
        },
    }
}

/// Tab bar with an underline; the app shows the selected tab's content. Each
/// tab is as wide as its label, plus an equal share of the spare width; the
/// selected label turns bold, morphing glyph by glyph.
pub fn tabs<S: Into<String>>(labels: impl IntoIterator<Item = S>, selected: usize) -> Segmented {
    Segmented {
        look: Look::Tabs,
        style: Style::default(),
        ..segmented(labels, selected)
    }
}

impl Segmented {
    /// Called with the tapped option's index when it isn't already selected.
    pub fn on_change(mut self, f: impl Fn(&mut EventCx, usize) + 'static) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }
    pub fn accent(mut self, c: Color) -> Self {
        self.accent = c;
        self
    }

    fn content(&self, r: Rect) -> Rect {
        let p = self.style.padding;
        let res = |l: LengthPercentage| l.resolve_or_zero(Some(r.w), |_, _| 0.0);
        let (l, t) = (res(p.left), res(p.top));
        Rect::new(
            r.x + l,
            r.y + t,
            r.w - l - res(p.right),
            r.h - t - res(p.bottom),
        )
    }

    /// Width each option needs: tabs fit their label, segments the widest one.
    fn natural(&self, text: &Text) -> Vec<f32> {
        let width = |l: &str, f| text.layout(l, f, SIZE, None).width;
        match self.look {
            Look::Tabs => self
                .labels
                .iter()
                .map(|l| width(l, Font::DEFAULT) + 2.0 * TAB_PAD)
                .collect(),
            Look::Segmented => {
                let w = self.labels.iter().map(|l| width(l, Font::DEFAULT) + 24.0);
                vec![w.fold(0.0, f32::max); self.labels.len()]
            }
        }
    }

    /// Content box and each option's (x, width). Spare width is shared out
    /// equally; when the options don't fit, they overflow (and are clipped).
    // ponytail: no horizontal scrolling for tab bars wider than the window yet.
    fn slots(&self, text: &Text, r: Rect) -> (Rect, Vec<(f32, f32)>) {
        let c = self.content(r);
        let n = self.labels.len().max(1) as f32;
        let natural = match self.look {
            Look::Tabs => self.natural(text),
            Look::Segmented => vec![0.0; self.labels.len()],
        };
        let extra = ((c.w - natural.iter().sum::<f32>()) / n).max(0.0);
        let mut x = c.x;
        let slots = natural
            .into_iter()
            .map(|w| {
                let s = (x, w + extra);
                x += w + extra;
                s
            })
            .collect();
        (c, slots)
    }

    fn index_at(&self, text: &Text, r: Rect, x: f32) -> usize {
        let (_, slots) = self.slots(text, r);
        let i = slots.iter().position(|&(sx, w)| x < sx + w);
        i.unwrap_or(slots.len()).min(slots.len().saturating_sub(1))
    }

    /// A tab's label at weight `t`, centered in its slot.
    fn tab_label(&self, cx: &mut PaintCx, l: &str, t: f32, slot: (f32, f32), y: f32) {
        weight(cx, l, SIZE, MUTED.lerp(self.accent, t), t, slot, y);
    }
}

impl From<Segmented> for Element {
    fn from(w: Segmented) -> Self {
        Element::new(w)
    }
}

impl Styled for Segmented {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl Widget for Segmented {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn measure(&self, known: Size<Option<f32>>, _: Size<AvailableSpace>, text: &Text) -> Size<f32> {
        let h = text.line_height(Font::DEFAULT, SIZE) + 2.0 * self.look.pad_y();
        Size {
            width: known.width.unwrap_or(self.natural(text).iter().sum()),
            height: known.height.unwrap_or(h),
        }
    }
    fn measure_key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        (&self.labels, self.look).hash(&mut h);
        h.finish()
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (now, r) = (cx.now, cx.rect);
        let (c, slots) = self.slots(cx.text, r);
        let s = cx.state.get::<SegState>();
        s.at.to(self.selected as f32, now, SLIDE, ease::out_cubic);
        s.weight.resize_with(self.labels.len(), Animated::default);
        let mut moving = s.at.running(now);
        let weights: Vec<f32> = (s.weight.iter_mut().enumerate())
            .map(|(i, w)| {
                let bold = if i == self.selected { 1.0 } else { 0.0 };
                w.to(bold, now, WEIGHT, ease::out_cubic);
                moving |= w.running(now);
                w.get(now)
            })
            .collect();
        let at = s.at.get(now);
        if moving {
            cx.request_frame();
        }
        let Some(&last) = slots.last() else { return };
        let (i, f) = (at.floor() as usize, at.fract());
        let (ax, aw) = slots.get(i).copied().unwrap_or(last);
        let (bx, bw) = slots.get(i + 1).copied().unwrap_or((ax, aw));
        let (x, w) = (ax.lerp(bx, f), aw.lerp(bw, f));
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
        let y = c.y + self.look.pad_y();
        cx.with_clip(r, |cx| {
            for ((l, &slot), t) in self.labels.iter().zip(&slots).zip(weights) {
                match self.look {
                    Look::Tabs => self.tab_label(cx, l, t, slot, y),
                    Look::Segmented => {
                        let lw = cx.text.layout(l, Font::DEFAULT, SIZE, None).width;
                        let lx = slot.0 + (slot.1 - lw) / 2.0;
                        cx.text(l, Font::DEFAULT, SIZE, INK, None, (lx, y));
                    }
                }
            }
        });
    }
    fn event(&self, ev: &Event, cx: &mut EventCx) {
        let r = cx.rect;
        match *ev {
            Event::PointerDown { x, y } if cx.hit(x, y) => {
                let i = self.index_at(cx.text, r, x);
                cx.state.get::<SegState>().pressed = Some(i);
            }
            Event::PointerUp { x, y } => {
                let pressed = cx.state.get::<SegState>().pressed.take();
                let i = self.index_at(cx.text, r, x);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_slots_hold_their_labels_and_taps_pick_them() {
        // The demo's tabs on a 411 dp phone (a 371 dp bar), which used to
        // overflow equal slots.
        let labels = ["Basics", "Controls", "Motion", "Cards", "Async", "Images"];
        let t = tabs(labels, 0);
        let text = Text::default();
        let r = Rect::new(20.0, 0.0, 371.0, 44.0);
        let (_, slots) = t.slots(&text, r);
        let (first, last) = (slots[0], slots[slots.len() - 1]);
        assert!(
            first.0 >= r.x && last.0 + last.1 <= r.x + r.w + 0.01,
            "fits"
        );
        for (i, (l, &(x, w))) in labels.iter().zip(&slots).enumerate() {
            let label = text.layout(l, Font::BOLD, SIZE, None).width;
            assert!(w >= label, "{l} fits its slot, bold");
            // The middle and both ends of the drawn label pick this tab.
            let mid = x + w / 2.0;
            for px in [mid, mid - label / 2.0 + 1.0, mid + label / 2.0 - 1.0] {
                assert_eq!(t.index_at(&text, r, px), i, "{l} at {px}");
            }
        }
    }
}
