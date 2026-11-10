//! Declarative UI framework that draws shapes, text and icons analytically in one wgpu shader.

pub mod anim;
pub mod kinetic;
mod layout;
pub mod model;
mod outline;
pub mod path;
mod render;
mod runtime;
mod text;
mod theme;
mod widget;
pub mod widgets;

pub use anim::Animated;
pub use path::{Path, Shape};
pub use render::{Bitmap, Color, Pixels, Rect, Shadow};
pub use runtime::task::{Notifier, Sleep, sleep};
// Hooks for the benches and golden tests, outside the semver guarantee.
#[doc(hidden)]
pub use runtime::{Bench, GpuBench, snapshot};
/// Layout styles. Its version is part of dfui's: a taffy upgrade is a breaking change.
pub use taffy;
pub use text::{Font, Text, TextLayout};
pub use theme::{Ink, Role, Theme};
pub use web_time::Instant;
pub use widget::{
    BuildCx, Element, Event, EventCx, Handle, Key, Keyboard, Layer, PaintCx, Placement, Scope,
    State, Widget,
};

// Compiles the README's examples as doc tests without publishing it as docs.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

/// Root of the UI. State lives in `stateful` widgets and in external sources
/// that builders read; `view` runs again whenever any of it changes.
pub trait App: 'static {
    /// Builds the whole UI. Runs on every rebuild, so keep it to building
    /// elements; parse assets and start work elsewhere.
    fn view(&self) -> Element;
    /// Colors for the whole app, read every frame.
    fn theme(&self) -> Theme {
        Theme::default()
    }
    /// Window background, read every frame.
    fn background(&self) -> Color {
        self.theme().background
    }
}

/// A function returning the view is an app with the default theme:
/// `dfui::run(view)`.
impl<F: Fn() -> Element + 'static> App for F {
    fn view(&self) -> Element {
        self()
    }
}

/// Opens a window showing `app` and runs until it's closed. On the web it
/// returns at once and the browser drives the app; on Android use `run_android`.
pub fn run<A: App>(app: A) {
    // Without it a panic in wasm only reports "unreachable executed".
    #[cfg(target_arch = "wasm32")]
    std::panic::set_hook(Box::new(|p| {
        web_sys::console::error_1(&p.to_string().into())
    }));
    let event_loop = winit::event_loop::EventLoop::new()
        .unwrap_or_else(|e| runtime::startup_failed(&format!("can't start the event loop: {e}")));
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
