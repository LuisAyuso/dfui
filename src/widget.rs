use std::any::Any;
use std::marker::PhantomData;
use web_time::Instant;

use taffy::{AvailableSpace, Size, Style};

use crate::path::Path;
use crate::render::{Color, Rect, Scene, Shadow};
use crate::task::Tasks;
use crate::text::{Font, Text};

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
    /// The pointer is gone: finger lifted or cursor left the window.
    Leave,
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
    SelectAll,
    Copy,
    Cut,
}

/// A retained node description. `view` rebuilds the tree; framework-owned
/// per-widget state survives rebuilds, keyed by tree position or `key`.
pub struct Element {
    pub(crate) widget: Box<dyn Widget>,
    pub(crate) key: Option<u64>,
}

impl Element {
    pub fn new(w: impl Widget + 'static) -> Self {
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

/// What a layer widget is laid out over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Its parent's box, e.g. a dropdown menu or tooltip.
    Parent,
    /// The window, inside the system bars, e.g. a dialog or drawer.
    Window,
}

/// Primitive building block. Composite widgets are plain functions returning
/// `Element`; implement this only for new layout/paint/input behavior.
pub trait Widget {
    fn style(&self) -> Style {
        Style::default()
    }

    fn children(&self) -> &[Element] {
        &[]
    }

    /// Runs before `children` with this node's id, state slot and providers;
    /// lets a widget build its subtree from framework-owned state.
    fn expand(&self, _cx: &mut BuildCx) {}

    /// Takes this subtree out of the layout flow: it covers `Layer`'s box,
    /// isn't clipped by ancestors, paints above and gets input before the
    /// rest of the tree.
    fn layer(&self) -> Option<Layer> {
        None
    }

    /// Value made available to descendants through `Scope::get`.
    fn provide(&self) -> Option<&dyn Any> {
        None
    }

    /// Intrinsic size of a leaf (children-less) widget.
    fn measure(
        &self,
        _known: Size<Option<f32>>,
        _available: Size<AvailableSpace>,
        _text: &Text,
    ) -> Size<f32> {
        Size::ZERO
    }

    /// Hash of everything `measure` reads besides its arguments. Layout is
    /// kept across rebuilds and a leaf is only re-measured when this changes,
    /// so widgets whose size depends on their data must override it.
    fn measure_key(&self) -> u64 {
        0
    }

    /// Children are clipped to this widget's `rect` and shifted by the
    /// returned offset, as of frame time `now`.
    fn scroll(&self, _state: &mut State, _rect: Rect, _now: Instant) -> Option<(f32, f32)> {
        None
    }

    fn paint(&self, _cx: &mut PaintCx) {}

    fn event(&self, _ev: &Event, _cx: &mut EventCx) {}
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

/// Values provided by ancestors (`provide`), innermost last.
#[derive(Clone, Copy)]
pub struct Scope<'a>(pub(crate) &'a [&'a dyn Any]);

impl<'a> Scope<'a> {
    /// The nearest ancestor-provided `T`.
    pub fn get<T: 'static>(&self) -> Option<&'a T> {
        self.0.iter().rev().find_map(|v| v.downcast_ref())
    }
}

pub struct BuildCx<'a> {
    pub id: u64,
    pub state: &'a mut State,
    pub scope: Scope<'a>,
    pub(crate) tasks: &'a mut Tasks,
}

pub struct PaintCx<'a> {
    /// Frame time, shared by every widget painted this frame.
    pub now: Instant,
    pub rect: Rect,
    pub clip: Rect,
    pub focused: bool,
    pub state: &'a mut State,
    pub(crate) scene: &'a mut Scene,
    pub(crate) text: &'a mut Text,
    pub(crate) frame: &'a mut bool,
    pub(crate) wake: &'a mut Option<Instant>,
    pub(crate) place: &'a mut bool,
}

impl PaintCx<'_> {
    /// Paint again next frame, for animations or live readouts.
    pub fn request_frame(&mut self) {
        *self.frame = true;
    }

    /// Re-run placement (`Widget::scroll`) and paint again next frame, for
    /// animated scroll offsets.
    pub fn request_placement(&mut self) {
        *self.frame = true;
        *self.place = true;
    }

    /// Paint again at `t`, for slow animations like a caret blink; the
    /// runtime sleeps until then instead of painting every frame.
    pub fn request_frame_at(&mut self, t: Instant) {
        *self.wake = Some(self.wake.map_or(t, |w| w.min(t)));
    }

    /// Fills `path` through the glyph renderer.
    pub fn path(&mut self, path: &Path, color: Color) {
        let quads = path.contours.iter().flatten();
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in quads.clone().flatten() {
            lo = [lo[0].min(p[0]), lo[1].min(p[1])];
            hi = [hi[0].max(p[0]), hi[1].max(p[1])];
        }
        let span = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        if !span.is_finite() || span <= 0.0 {
            return;
        }
        // Stored like a glyph: y up, scaled to a unit box, which keeps the
        // shader's line test and precision where glyphs have them.
        let s = 1.0 / span;
        let unit = quads
            .map(|q| q.map(|[x, y]| [(x - lo[0]) * s, (hi[1] - y) * s]))
            .collect();
        // 1px padding keeps the edge antialiasing.
        let Some(g) = self.text.outlines.transient(unit, s) else {
            return;
        };
        let [x0, y0, x1, y1] = g.bbox;
        let r = Rect::new(lo[0] + x0 / s, hi[1] - y1 / s, (x1 - x0) / s, (y1 - y0) / s);
        let bands = [g.header as f32, g.bands_h as f32, g.bands_v as f32];
        self.scene
            .glyph(r, [x0, y1, x1, y0], color, self.clip, bands);
    }

    /// Outline of `c` at `size` px, pen at the origin on the baseline.
    pub fn glyph_path(&self, font: Font, c: char, size: f32) -> Option<Path> {
        self.text.glyph_path(font, c, size)
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
        font: Font,
        size: f32,
        color: Color,
        max_width: Option<f32>,
        origin: (f32, f32),
    ) {
        self.text.draw(
            s, font, size, max_width, origin, color, self.clip, self.scene,
        );
    }
}

pub struct EventCx<'a> {
    /// When the event was dispatched.
    pub now: Instant,
    pub rect: Rect,
    pub clip: Rect,
    /// Extent of laid-out children, relative to this widget's origin.
    pub content: Size<f32>,
    pub focused: bool,
    /// Shift is held: arrows and clicks extend a selection.
    pub shift: bool,
    pub state: &'a mut State,
    pub(crate) text: &'a Text,
    pub(crate) updates: &'a mut Vec<Update>,
    pub(crate) copied: &'a mut Option<String>,
    pub(crate) capture: bool,
    pub(crate) focus: bool,
    pub(crate) stop: bool,
    pub(crate) redraw: bool,
}

impl EventCx<'_> {
    pub fn hit(&self, x: f32, y: f32) -> bool {
        self.rect.contains(x, y) && self.clip.contains(x, y)
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

    /// Put `text` on the clipboard after this event.
    pub fn copy(&mut self, text: impl Into<String>) {
        *self.copied = Some(text.into());
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
