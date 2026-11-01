//! Declarative (Elm-style) UI framework rendered entirely with distance fields.

pub mod anim;
pub mod kinetic;
mod layout;
mod outline;
pub mod path;
mod render;
mod runtime;
mod text;
mod widget;
pub mod widgets;

pub use anim::Animated;
pub use path::Path;
pub use render::{Bitmap, Color, Rect, Shadow};
pub use runtime::task::{Sleep, sleep};
pub use runtime::{Bench, GpuBench, snapshot};
pub use taffy;
pub use text::{Font, Text, TextLayout};
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
pub use runtime::android;
#[cfg(target_os = "android")]
pub use winit::platform::android::activity::AndroidApp;

/// Runs `app` until the activity is destroyed; call it from `android_main`.
/// The process may outlive the activity (a service, a download) and start
/// `android_main` again for a new one.
#[cfg(target_os = "android")]
pub fn run_android<A: App>(app: A, android: AndroidApp) {
    runtime::activity::run(runtime::Runtime::new(app), android);
}
