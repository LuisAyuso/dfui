//! A dfui app inside someone else's frame loop and renderer, e.g. a game's HUD.

use std::sync::Arc;

use winit::event::WindowEvent;

use super::task::{self, LoopWaker};
use super::{Runtime, pointer_at};
use crate::App;
use crate::render::{Embed, Spans};
use crate::widget::Event;

/// A dfui app drawn into the host's texture with the host's wgpu device: the
/// host forwards window events and records [`render`](Self::render) once a
/// frame. Draws a premultiplied overlay where the app's background is
/// transparent.
pub struct Embedded<A: App> {
    rt: Runtime<A>,
    gpu: Embed,
    wake: Arc<LoopWaker>,
    scale: f32,
    /// What each node painted last frame, in paint order.
    spans: Spans,
}

impl<A: App> Embedded<A> {
    /// `size` is physical px and `format` the format of the views given to `render`.
    pub fn new(
        app: A,
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        size: (u32, u32),
        scale: f32,
    ) -> Self {
        let mut rt = Runtime::new(app);
        let wake = LoopWaker::new(|| {});
        rt.waker = wake.clone().into();
        rt.loop_waker = Some(wake.clone());
        rt.viewport = (size.0 as f32 / scale, size.1 as f32 / scale);
        rt.drive();
        rt.relayout();
        Embedded {
            rt,
            gpu: Embed::new(device, queue, format, size),
            wake,
            scale,
            spans: Vec::new(),
        }
    }

    /// Physical px, and physical px per logical px.
    pub fn resize(&mut self, size: (u32, u32), scale: f32) {
        self.scale = scale;
        self.gpu.resize(size);
        self.rt.viewport = (size.0 as f32 / scale, size.1 as f32 / scale);
        self.rt.relayout();
    }

    /// Returns whether the UI took the event, so the host should ignore it:
    /// the pointer is over something the UI drew or captured by it, or a
    /// widget has keyboard focus.
    pub fn window_event(&mut self, event: WindowEvent) -> bool {
        let s = self.scale;
        let touch = match &event {
            WindowEvent::Touch(t) => Some((t.location.x as f32 / s, t.location.y as f32 / s)),
            _ => None,
        };
        let pointer = touch.is_some()
            || matches!(
                event,
                WindowEvent::CursorMoved { .. }
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
                    | WindowEvent::PinchGesture { .. }
            );
        let took = self.rt.input(event, s);
        took || pointer && self.over_ui(touch.unwrap_or(self.rt.cursor))
    }

    /// Sends an event in logical px; returns whether the UI took it, as
    /// [`window_event`](Self::window_event) does.
    pub fn event(&mut self, ev: Event) -> bool {
        let at = pointer_at(&ev);
        self.rt.dispatch(ev) || self.rt.captured.is_some() || at.is_some_and(|p| self.over_ui(p))
    }

    /// Whether a node that painted last frame covers `(x, y)`, logical px.
    fn over_ui(&self, (x, y): (f32, f32)) -> bool {
        let rt = &self.rt;
        rt.order.iter().zip(&self.spans).any(|(&i, (id, r))| {
            let Some(p) = rt.placed.get(i) else {
                return false;
            };
            rt.ids.get(i) == Some(id)
                && !r.is_empty()
                && p.rect.contains(x, y)
                && p.clip.contains(x, y)
        })
    }

    /// Changes the app, e.g. with new game state, and rebuilds the view.
    pub fn app(&mut self, f: impl FnOnce(&mut A)) {
        f(&mut self.rt.app);
        self.rt.rebuild();
        self.rt.drive();
        self.rt.relayout();
    }

    /// Runs due timers and woken tasks, then records the frame into `enc`. The
    /// target must have the size and format given earlier.
    // ponytail: redraws every frame, as games do; skip on empty damage if the HUD shows in GPU profiles.
    pub fn render(&mut self, enc: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        task::fire_timers(task::now());
        if self.wake.pending() {
            self.rt.woken();
        }
        let (scene, spans) = self.rt.scene(&mut false);
        self.rt.rebuild_if_due();
        self.gpu
            .render(enc, target, &scene, &mut self.rt.text, self.scale);
        self.spans = spans;
    }
}
