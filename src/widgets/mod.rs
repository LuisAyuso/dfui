//! Starter widget set. Composite widgets should be plain functions over these.

use taffy::{
    AlignItems, AvailableSpace, Dimension, Display, FlexDirection, FlexWrap, JustifyContent,
    LengthPercentage, Overflow, Point, Size, Style,
};

use crate::render::{Color, Shadow};
use crate::text::Text;
use crate::widget::{Element, Event, EventCx, PaintCx, State, Widget};

// A blanket `impl<W: Widget> From<W>` is rejected by coherence, so each
// widget opts in. Third-party widgets can do the same or use `Element::new`.
impl<Msg: 'static> From<Container<Msg>> for Element<Msg> {
    fn from(w: Container<Msg>) -> Self {
        Element::new(w)
    }
}
impl<Msg: 'static> From<Label> for Element<Msg> {
    fn from(w: Label) -> Self {
        Element::new(w)
    }
}
impl<Msg: Clone + 'static> From<Button<Msg>> for Element<Msg> {
    fn from(w: Button<Msg>) -> Self {
        Element::new(w)
    }
}
impl<Msg: 'static> From<Scroll<Msg>> for Element<Msg> {
    fn from(w: Scroll<Msg>) -> Self {
        Element::new(w)
    }
}

/// Flex-layout modifiers shared by every widget that owns a `Style`.
pub trait Styled: Sized {
    fn style_mut(&mut self) -> &mut Style;

    fn padding(mut self, p: f32) -> Self {
        self.style_mut().padding = taffy::Rect::length(p);
        self
    }
    fn padding_xy(mut self, x: f32, y: f32) -> Self {
        let (x, y) = (LengthPercentage::length(x), LengthPercentage::length(y));
        self.style_mut().padding = taffy::Rect {
            left: x,
            right: x,
            top: y,
            bottom: y,
        };
        self
    }
    fn gap(mut self, g: f32) -> Self {
        self.style_mut().gap = Size::length(g);
        self
    }
    fn grow(mut self, g: f32) -> Self {
        self.style_mut().flex_grow = g;
        self
    }
    fn shrink(mut self, s: f32) -> Self {
        self.style_mut().flex_shrink = s;
        self
    }
    fn width(mut self, w: f32) -> Self {
        self.style_mut().size.width = Dimension::length(w);
        self
    }
    fn height(mut self, h: f32) -> Self {
        self.style_mut().size.height = Dimension::length(h);
        self
    }
    fn fill_width(mut self) -> Self {
        self.style_mut().size.width = Dimension::percent(1.0);
        self
    }
    fn align(mut self, a: AlignItems) -> Self {
        self.style_mut().align_items = a;
        self
    }
    fn justify(mut self, j: JustifyContent) -> Self {
        self.style_mut().justify_content = j;
        self
    }
    fn wrap(mut self) -> Self {
        self.style_mut().flex_wrap = FlexWrap::Wrap;
        self
    }
}

/// Flex box with optional rounded, bordered, shadowed background.
pub struct Container<Msg> {
    style: Style,
    children: Vec<Element<Msg>>,
    bg: Color,
    radius: f32,
    border: (f32, Color),
    shadow: Option<Shadow>,
}

pub fn row<Msg>(children: impl IntoIterator<Item = Element<Msg>>) -> Container<Msg> {
    Container {
        style: Style {
            flex_direction: FlexDirection::Row,
            ..Style::default()
        },
        children: children.into_iter().collect(),
        bg: Color::TRANSPARENT,
        radius: 0.0,
        border: (0.0, Color::TRANSPARENT),
        shadow: None,
    }
}

pub fn column<Msg>(children: impl IntoIterator<Item = Element<Msg>>) -> Container<Msg> {
    let mut c = row(children);
    c.style.flex_direction = FlexDirection::Column;
    c
}

pub fn container<Msg>(child: impl Into<Element<Msg>>) -> Container<Msg> {
    column([child.into()])
}

/// Flexible empty space; `spacer().grow(1.)` pushes siblings apart.
pub fn spacer<Msg>() -> Container<Msg> {
    row([])
}

impl<Msg> Container<Msg> {
    pub fn child(mut self, c: impl Into<Element<Msg>>) -> Self {
        self.children.push(c.into());
        self
    }
    pub fn bg(mut self, c: Color) -> Self {
        self.bg = c;
        self
    }
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
    pub fn border(mut self, width: f32, c: Color) -> Self {
        self.border = (width, c);
        self
    }
    pub fn shadow(mut self, s: Shadow) -> Self {
        self.shadow = Some(s);
        self
    }
}

impl<Msg> Styled for Container<Msg> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<Msg> Widget<Msg> for Container<Msg> {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element<Msg>] {
        &self.children
    }
    fn paint(&self, cx: &mut PaintCx) {
        if let Some(s) = self.shadow {
            cx.shadow(cx.rect, self.radius, s);
        }
        if self.bg.a > 0.0 || self.border.0 > 0.0 {
            cx.fill_bordered(cx.rect, self.radius, self.bg, self.border.0, self.border.1);
        }
    }
}

pub struct Label {
    text: String,
    size: f32,
    color: Color,
    style: Style,
}

pub fn text(s: impl Into<String>) -> Label {
    Label {
        text: s.into(),
        size: 16.0,
        color: Color::BLACK,
        style: Style::default(),
    }
}

impl Label {
    pub fn size(mut self, s: f32) -> Self {
        self.size = s;
        self
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
}

impl Styled for Label {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<Msg> Widget<Msg> for Label {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn measure(
        &self,
        known: Size<Option<f32>>,
        available: Size<AvailableSpace>,
        text: &Text,
    ) -> Size<f32> {
        let max = known.width.or(match available.width {
            AvailableSpace::Definite(w) => Some(w),
            AvailableSpace::MinContent => Some(0.0),
            AvailableSpace::MaxContent => None,
        });
        let l = text.layout(&self.text, self.size, max);
        Size {
            width: known.width.unwrap_or(l.width),
            height: known.height.unwrap_or(l.height),
        }
    }
    fn paint(&self, cx: &mut PaintCx) {
        let (r, s, c) = (cx.rect, self.size, self.color);
        cx.text(&self.text, s, c, Some(r.w + 1.0), (r.x, r.y));
    }
}

#[derive(Default)]
struct ButtonState {
    pressed: bool,
    hovered: bool,
}

pub struct Button<Msg> {
    child: [Element<Msg>; 1],
    on_press: Option<Msg>,
    style: Style,
    color: Color,
    radius: f32,
}

pub fn button<Msg>(child: impl Into<Element<Msg>>) -> Button<Msg> {
    Button {
        child: [child.into()],
        on_press: None,
        style: Style {
            padding: taffy::Rect {
                left: LengthPercentage::length(16.0),
                right: LengthPercentage::length(16.0),
                top: LengthPercentage::length(10.0),
                bottom: LengthPercentage::length(10.0),
            },
            align_items: AlignItems::CENTER,
            justify_content: JustifyContent::CENTER,
            ..Style::default()
        },
        color: Color::hex(0x3b6fe0),
        radius: 8.0,
    }
}

impl<Msg> Button<Msg> {
    pub fn on_press(mut self, msg: Msg) -> Self {
        self.on_press = Some(msg);
        self
    }
    pub fn color(mut self, c: Color) -> Self {
        self.color = c;
        self
    }
    pub fn radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }
}

impl<Msg> Styled for Button<Msg> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

impl<Msg: Clone> Widget<Msg> for Button<Msg> {
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn children(&self) -> &[Element<Msg>] {
        &self.child
    }
    fn paint(&self, cx: &mut PaintCx) {
        let s = cx.state.get::<ButtonState>();
        let c = self.color;
        let k = if s.pressed {
            0.75
        } else if s.hovered {
            0.9
        } else {
            1.0
        };
        let fill = Color {
            r: c.r * k,
            g: c.g * k,
            b: c.b * k,
            a: c.a,
        };
        if !s.pressed {
            let sh = Shadow {
                color: Color::BLACK.alpha(0.25),
                offset: (0.0, 2.0),
                blur: 4.0,
            };
            cx.shadow(cx.rect, self.radius, sh);
        }
        cx.fill(cx.rect, self.radius, fill);
    }
    fn event(&self, ev: &Event, cx: &mut EventCx<Msg>) {
        let hit = match *ev {
            Event::PointerDown { x, y }
            | Event::PointerMove { x, y }
            | Event::PointerUp { x, y } => cx.hit(x, y),
            _ => false,
        };
        let before = (
            cx.state.get::<ButtonState>().pressed,
            cx.state.get::<ButtonState>().hovered,
        );
        let s = cx.state.get::<ButtonState>();
        match ev {
            Event::PointerDown { .. } if hit => s.pressed = true,
            Event::PointerMove { .. } => s.hovered = hit,
            Event::PointerUp { .. } => {
                let fire = s.pressed && hit;
                s.pressed = false;
                if fire && let Some(m) = self.on_press.clone() {
                    cx.emit(m);
                }
            }
            Event::Cancel => s.pressed = false,
            _ => {}
        }
        let s = cx.state.get::<ButtonState>();
        if (s.pressed, s.hovered) != before {
            cx.request_redraw();
        }
    }
}

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

impl<Msg> Styled for Scroll<Msg> {
    fn style_mut(&mut self) -> &mut Style {
        &mut self.style
    }
}

const DRAG_SLOP: f32 = 6.0;

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
