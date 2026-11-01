use std::time::Duration;

use crate::App;
use crate::render::{Offscreen, Rect, Scene, damage, render_offscreen};
use crate::text::Text;
use crate::widget::Event;

use super::Runtime;

/// CPU side of frames for benchmarks: rebuild, layout, paint and damage diff,
/// through the real runtime but without a window or GPU.
#[doc(hidden)]
pub struct Bench<A: App>(Runtime<A>);

impl<A: App> Bench<A> {
    pub fn new(app: A, viewport: (f32, f32)) -> Self {
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
pub fn snapshot<A: App>(app: A, size: (u32, u32), scale: f32) -> image::RgbaImage {
    let mut rt = Runtime::new(app);
    rt.viewport = (size.0 as f32 / scale, size.1 as f32 / scale);
    rt.relayout();
    let (scene, _) = rt.scene(&mut false);
    render_offscreen(&[(&scene, None)], &mut rt.text, size, scale)
}
