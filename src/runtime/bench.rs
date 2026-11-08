use std::time::Duration;

use web_time::Instant;

use crate::App;
use crate::render::{Offscreen, Pixels, Rect, Scene, damage, render_offscreen};
use crate::text::Text;
use crate::widget::Event;

use super::{Runtime, task};

/// CPU side of frames for benchmarks and golden tests: rebuild, layout, paint
/// and damage diff, through the real runtime but without a window, on a
/// virtual clock that only moves through `advance` and `settle`.
#[doc(hidden)]
pub struct Bench<A: App>(Runtime<A>);

impl<A: App> Bench<A> {
    pub fn new(app: A, viewport: (f32, f32)) -> Self {
        task::set_clock(Some(Instant::now()));
        let mut rt = Runtime::new(app);
        rt.viewport = viewport;
        rt.relayout();
        let mut b = Bench(rt);
        b.frame();
        b
    }

    /// One full CPU frame as after a state change; returns the instance count.
    pub fn frame(&mut self) -> usize {
        let rt = &mut self.0;
        rt.set_root(rt.app.view());
        rt.relayout();
        self.paint()
    }

    /// Changes the app, then one full CPU frame.
    pub fn update(&mut self, f: impl FnOnce(&mut A)) -> usize {
        f(&mut self.0.app);
        self.frame()
    }

    /// Paint and damage diff only, as after an event that kept the tree.
    pub fn paint(&mut self) -> usize {
        let rt = &mut self.0;
        let (scene, spans) = rt.scene(&mut false);
        if let Some((ps, pn)) = &rt.prev {
            std::hint::black_box(damage(ps, pn, &scene, &spans));
        }
        let n = scene.len();
        rt.prev = Some((scene, spans));
        n
    }

    pub fn event(&mut self, ev: Event) {
        self.0.dispatch(ev);
    }

    /// Moves the clock forward by `d`, through every frame, wake and timer
    /// due on the way, as a window would.
    pub fn advance(&mut self, d: Duration) {
        let end = task::now() + d;
        while let Some(t) = self.next_event().filter(|&t| t < end) {
            self.step_to(t);
        }
        self.step_to(end);
    }

    /// Advances until no frame, wake or timer is pending.
    pub fn settle(&mut self) {
        for _ in 0..10_000 {
            let Some(t) = self.next_event() else { return };
            self.step_to(t);
        }
        panic!("still animating after 10000 frames");
    }

    /// Paints, then returns when the next frame (60 Hz while animating), wake or timer is due.
    fn next_event(&mut self) -> Option<Instant> {
        let mut frame = false;
        self.0.scene(&mut frame);
        self.0.rebuild_if_due();
        let now = task::now();
        match frame {
            true => Some(now + Duration::from_micros(16_667)),
            false => self.0.wake.into_iter().chain(task::fire_timers(now)).min(),
        }
    }

    fn step_to(&mut self, t: Instant) {
        task::set_clock(Some(t));
        task::fire_timers(t);
        self.0.woken();
        self.0.reposition();
    }

    /// Renders the current frame offscreen at `scale` physical px per logical px.
    pub fn shot(&mut self, scale: f32) -> Pixels {
        let rt = &mut self.0;
        let size = (
            (rt.viewport.0 * scale).round() as u32,
            (rt.viewport.1 * scale).round() as u32,
        );
        let (scene, _) = rt.scene(&mut false);
        render_offscreen(&[(&scene, None)], &mut rt.text, size, scale)
    }
}

/// GPU side of frames for benchmarks: one scene of `app`, drawn offscreen at
/// `size` physical pixels, timed with GPU timestamps where available.
#[doc(hidden)]
pub struct GpuBench {
    scene: Scene,
    text: Text,
    off: Offscreen,
    scale: f32,
}

impl GpuBench {
    pub fn new<A: App>(app: A, size: (u32, u32), scale: f32) -> Self {
        let mut rt = Runtime::new(app);
        rt.viewport = (size.0 as f32 / scale, size.1 as f32 / scale);
        rt.relayout();
        let (scene, _) = rt.scene(&mut false);
        let mut g = GpuBench {
            scene,
            text: rt.text,
            off: Offscreen::new(size),
            scale,
        };
        // Builds and uploads glyph outlines before anything is timed.
        g.draw(None);
        g
    }

    /// Redraws everything, or only `damage` (logical px) as a partial frame.
    pub fn draw(&mut self, damage: Option<Rect>) -> Duration {
        let s = self.scale;
        let scissor = damage.map(|d| {
            let px = |v: f32| (v * s).round() as u32;
            [px(d.x), px(d.y), px(d.w).max(1), px(d.h).max(1)]
        });
        self.off.timed_draw(&self.scene, &mut self.text, s, scissor)
    }
}

/// Renders one frame of `app` offscreen at `size` physical pixels, e.g. for golden tests.
pub fn snapshot<A: App>(app: A, size: (u32, u32), scale: f32) -> Pixels {
    let mut rt = Runtime::new(app);
    rt.viewport = (size.0 as f32 / scale, size.1 as f32 / scale);
    rt.relayout();
    let (scene, _) = rt.scene(&mut false);
    render_offscreen(&[(&scene, None)], &mut rt.text, size, scale)
}
