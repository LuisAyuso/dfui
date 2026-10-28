use taffy::{Display, Overflow, Point, Style};

use super::Styled;
use crate::widget::{Element, Event, EventCx, State, Widget};

const DRAG_SLOP: f32 = 6.0;

#[derive(Default)]
struct ScrollState {
    offset: f32,
    drag: Option<(f32, f32)>,
    dragging: bool,
}

/// Vertical scroll view; drag (touch) or wheel. Children are clipped.
pub struct Scroll<Msg> {
    child: [Element<Msg>; 1],
    style: Style,
}

pub fn scroll<Msg>(child: impl Into<Element<Msg>>) -> Scroll<Msg> {
    Scroll {
        child: [child.into()],
        style: Style {
            display: Display::Block,
            overflow: Point {
                x: Overflow::Hidden,
                y: Overflow::Scroll,
            },
            scrollbar_width: 0.0,
            ..Style::default()
        },
    }
}

impl<Msg: 'static> From<Scroll<Msg>> for Element<Msg> {
    fn from(w: Scroll<Msg>) -> Self {
        Element::new(w)
    }
}

impl<Msg> Styled for Scroll<Msg> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<Msg> Widget<Msg> for Scroll<Msg> {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element<Msg>] {
        &self.child
    }
    fn scroll(&self, state: &mut State) -> Option<(f32, f32)> {
        Some((0.0, state.get::<ScrollState>().offset))
    }
    fn event(&self, ev: &Event, cx: &mut EventCx<Msg>) {
        let max = (cx.content.height - cx.rect.h).max(0.0);
        let hit = |x, y| cx.hit(x, y);
        let (mut capture, mut redraw, mut stop) = (false, false, false);
        match *ev {
            Event::PointerDown { x, y } if hit(x, y) => {
                let s = cx.state.get::<ScrollState>();
                s.drag = Some((y, s.offset));
            }
            Event::PointerMove { y, .. } => {
                let s = cx.state.get::<ScrollState>();
                if let Some((start, from)) = s.drag {
                    if !s.dragging && (y - start).abs() > DRAG_SLOP {
                        s.dragging = true;
                        capture = true;
                    }
                    if s.dragging {
                        s.offset = (from - (y - start)).clamp(0.0, max);
                        redraw = true;
                    }
                }
            }
            Event::PointerUp { .. } | Event::Cancel => {
                let s = cx.state.get::<ScrollState>();
                s.drag = None;
                s.dragging = false;
            }
            Event::Wheel { x, y, dy } if hit(x, y) => {
                let s = cx.state.get::<ScrollState>();
                s.offset = (s.offset + dy).clamp(0.0, max);
                redraw = true;
                stop = true;
            }
            _ => {}
        }
        if capture {
            cx.capture();
        }
        if redraw {
            cx.request_redraw();
        }
        if stop {
            cx.stop();
        }
    }
}
