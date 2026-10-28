use std::any::Any;
use std::marker::PhantomData;

use taffy::{AvailableSpace, Size, Style};

use crate::render::{Color, Rect, Scene, Shadow};
use crate::text::Text;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    PointerDown {
        x: f32,
        y: f32,
    },
    PointerMove {
        x: f32,
        y: f32,
    },
    PointerUp {
        x: f32,
        y: f32,
    },
    /// Mouse wheel, logical pixels, positive scrolls content up.
    Wheel {
        x: f32,
        y: f32,
        dy: f32,
    },
    /// Another widget captured the pointer; drop any in-progress gesture.
    Cancel,
    /// Text typed while this widget has focus.
    Text(String),
    /// Editing key pressed while this widget has focus.
    Key(Key),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
}

/// A retained node description. `view` rebuilds the tree; framework-owned
/// per-widget state survives rebuilds, keyed by tree position or `key`.
pub struct Element<Msg> {
    pub(crate) widget: Box<dyn Widget<Msg>>,
    pub(crate) key: Option<u64>,
}

impl<Msg> Element<Msg> {
    pub fn new(w: impl Widget<Msg> + 'static) -> Self {
        Element {
            widget: Box::new(w),
            key: None,
        }
    }

    /// Stable identity for state across rebuilds, e.g. items in a reorderable list.
    pub fn key(mut self, key: u64) -> Self {
        self.key = Some(key);
        self
    }
}

/// Primitive building block. Composite widgets are plain functions returning
/// `Element`; implement this only for new layout/paint/input behavior.
pub trait Widget<Msg> {
    fn style(&self) -> Style {
        Style::default()
    }

    fn children(&self) -> &[Element<Msg>] {
        &[]
    }

    /// Runs before `children` with this node's id and state slot; lets a widget
    /// build its subtree from framework-owned state.
    fn expand(&self, _id: u64, _state: &mut State) {}

    /// Intrinsic size of a leaf (children-less) widget.
    fn measure(
        &self,
        _known: Size<Option<f32>>,
        _available: Size<AvailableSpace>,
        _text: &Text,
    ) -> Size<f32> {
        Size::ZERO
    }

    /// Children are clipped to this widget's rect and shifted by the returned offset.
    fn scroll(&self, _state: &mut State) -> Option<(f32, f32)> {
        None
    }

    fn paint(&self, _cx: &mut PaintCx) {}

    fn event(&self, _ev: &Event, _cx: &mut EventCx<Msg>) {}
}

/// Framework-owned per-widget state slot.
#[derive(Default)]
pub struct State(Option<Box<dyn Any>>);

impl State {
    pub fn get<T: Default + 'static>(&mut self) -> &mut T {
        if !self.0.as_ref().is_some_and(|b| b.is::<T>()) {
            self.0 = Some(Box::new(T::default()));
        }
        self.0.as_mut().unwrap().downcast_mut().unwrap()
    }
}

/// Typed reference to a stateful widget's state, cheap to capture in handlers.
pub struct Handle<S> {
    pub(crate) id: u64,
    _s: PhantomData<fn() -> S>,
}

impl<S> Handle<S> {
    pub(crate) fn new(id: u64) -> Self {
        Handle {
            id,
            _s: PhantomData,
        }
    }
}

impl<S> Clone for Handle<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S> Copy for Handle<S> {}

pub(crate) type Update = (u64, Box<dyn FnOnce(&mut State)>);

pub struct PaintCx<'a> {
    pub rect: Rect,
    pub clip: Rect,
    pub focused: bool,
    pub state: &'a mut State,
    pub(crate) scene: &'a mut Scene,
    pub(crate) text: &'a mut Text,
    pub(crate) frame: &'a mut bool,
}

impl PaintCx<'_> {
    /// Paint again next frame, for animations or live readouts.
    pub fn request_frame(&mut self) {
        *self.frame = true;
    }

    pub fn fill(&mut self, r: Rect, radius: f32, color: Color) {
        self.scene
            .shape(r, radius, color, 0.0, Color::TRANSPARENT, self.clip);
    }

    pub fn fill_bordered(
        &mut self,
        r: Rect,
        radius: f32,
        color: Color,
        border: f32,
        border_color: Color,
    ) {
        self.scene
            .shape(r, radius, color, border, border_color, self.clip);
    }

    pub fn shadow(&mut self, r: Rect, radius: f32, s: Shadow) {
        self.scene.shadow(r, radius, s, self.clip);
    }

    pub fn text(
        &mut self,
        s: &str,
        size: f32,
        color: Color,
        max_width: Option<f32>,
        origin: (f32, f32),
    ) {
        self.text
            .draw(s, size, max_width, origin, color, self.clip, self.scene);
    }
}

pub struct EventCx<'a, Msg> {
    pub rect: Rect,
    pub clip: Rect,
    /// Extent of laid-out children, relative to this widget's origin.
    pub content: Size<f32>,
    pub focused: bool,
    pub state: &'a mut State,
    pub(crate) text: &'a Text,
    pub(crate) msgs: &'a mut Vec<Msg>,
    pub(crate) updates: &'a mut Vec<Update>,
    pub(crate) capture: bool,
    pub(crate) focus: bool,
    pub(crate) stop: bool,
    pub(crate) redraw: bool,
}

impl<Msg> EventCx<'_, Msg> {
    pub fn hit(&self, x: f32, y: f32) -> bool {
        self.rect.contains(x, y) && self.clip.contains(x, y)
    }

    pub fn emit(&mut self, msg: Msg) {
        self.msgs.push(msg);
    }

    /// Modify a stateful widget's state after this event; its subtree is rebuilt.
    pub fn update<S: Default + 'static>(&mut self, h: Handle<S>, f: impl FnOnce(&mut S) + 'static) {
        self.updates
            .push((h.id, Box::new(move |st: &mut State| f(st.get::<S>()))));
    }

    /// Route all pointer events to this widget until pointer up; others get `Cancel`.
    pub fn capture(&mut self) {
        self.capture = true;
    }

    /// Take keyboard focus; on touch devices this shows the soft keyboard.
    pub fn focus(&mut self) {
        self.focus = true;
    }

    /// Don't offer this event to ancestors.
    pub fn stop(&mut self) {
        self.stop = true;
    }

    pub fn request_redraw(&mut self) {
        self.redraw = true;
    }
}
