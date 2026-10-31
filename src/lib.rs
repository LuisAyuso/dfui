//! Declarative (Elm-style) UI framework rendered entirely with distance fields.

#[cfg(target_os = "android")]
mod android;
pub mod anim;
mod layout;
mod outline;
pub mod path;
mod render;
mod runtime;
mod task;
mod text;
mod widget;
pub mod widgets;

pub use anim::Animated;
pub use path::Path;
pub use render::{Color, Rect, Shadow};
pub use runtime::{Bench, GpuBench, snapshot};
pub use taffy;
pub use task::{Sleep, sleep};
pub use text::{Font, Text};
pub use web_time::Instant;
pub use widget::{
    BuildCx, Element, Event, EventCx, Handle, Key, Layer, PaintCx, Scope, State, Widget,
};

/// Root of the UI. State lives in `stateful` widgets and in external sources
/// that builders read; `view` runs again whenever any of it changes.
pub trait App: 'static {
    fn view(&self) -> Element;
    /// Window background, read every frame.
    fn background(&self) -> Color {
        Color::WHITE
    }
}

pub fn run<A: App>(app: A) {
    let event_loop = winit::event_loop::EventLoop::new().expect("event loop");
    runtime::Runtime::new(app).run(event_loop);
}

#[cfg(target_os = "android")]
pub use winit::platform::android::activity::AndroidApp;

#[cfg(target_os = "android")]
pub fn run_android<A: App>(app: A, android: AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;
    let event_loop = winit::event_loop::EventLoop::builder()
        .with_android_app(android.clone())
        .build()
        .expect("event loop");
    let mut rt = runtime::Runtime::new(app);
    rt.android = Some(android);
    rt.run(event_loop);
}
