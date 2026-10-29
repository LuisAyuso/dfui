//! Declarative (Elm-style) UI framework rendered entirely with distance fields.

#[cfg(target_os = "android")]
mod android;
mod layout;
mod outline;
mod render;
mod runtime;
mod text;
mod widget;
pub mod widgets;

pub use render::{Color, Rect, Shadow};
pub use runtime::{Bench, GpuBench, snapshot};
pub use taffy;
pub use text::Text;
pub use widget::{Element, Event, EventCx, Handle, Key, PaintCx, State, Widget};

pub trait App: 'static {
    type Msg: 'static;
    fn update(&mut self, msg: Self::Msg);
    fn view(&self) -> Element<Self::Msg>;
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
