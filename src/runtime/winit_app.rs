//! Desktop and web: `Runtime` as a winit `ApplicationHandler`.

use std::sync::Arc;

use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use super::Runtime;
use super::task::{self, LoopWaker};
use crate::App;
use crate::render::{Host, Renderer};
use crate::widget::Event;

impl<A: App> Runtime<A> {
    pub fn run(mut self, event_loop: EventLoop<()>) {
        let proxy = std::sync::Mutex::new(event_loop.create_proxy());
        // Fails only once the loop has exited.
        let w = LoopWaker::new(move || {
            let _ = proxy.lock().unwrap().send_event(());
        });
        self.waker = w.clone().into();
        self.loop_waker = Some(w);
        self.drive();
        #[cfg(target_arch = "wasm32")]
        winit::platform::web::EventLoopExtWebSys::spawn_app(event_loop, self);
        #[cfg(not(target_arch = "wasm32"))]
        event_loop.run_app(&mut self).expect("event loop");
    }
}

impl Host for Arc<Window> {
    fn surface(&self, instance: &wgpu::Instance) -> wgpu::Surface<'static> {
        instance
            .create_surface(self.clone())
            .expect("create surface")
    }
    fn size(&self) -> (u32, u32) {
        self.inner_size().into()
    }
    fn scale(&self) -> f32 {
        self.scale_factor() as f32
    }
    fn request_redraw(&self) {
        Window::request_redraw(self);
    }
    fn pre_present(&self) {
        self.pre_present_notify();
    }
    fn refresh_mhz(&self) -> Option<u32> {
        self.current_monitor()?.refresh_rate_millihertz()
    }
}

impl<A: App> ApplicationHandler for Runtime<A> {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        #[allow(unused_mut)]
        let mut attrs = Window::default_attributes().with_title("dfui");
        // The page sizes the canvas with CSS.
        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowAttributesExtWebSys;
            attrs = attrs.with_append(true);
        }
        let window = Arc::new(el.create_window(attrs).expect("create window"));
        let instance = self.instance.get_or_insert_with(|| {
            let mut desc = wgpu::InstanceDescriptor::new_with_display_handle(Box::new(
                el.owned_display_handle(),
            ));
            if cfg!(target_os = "android") {
                desc.backends = wgpu::Backends::VULKAN;
            }
            wgpu::Instance::new(desc.with_env())
        });
        let size = window.inner_size().to_logical::<f32>(window.scale_factor());
        self.viewport = (size.width, size.height);
        self.pending_renderer = Some(Box::pin(Renderer::new(instance.clone(), Box::new(window))));
        self.poll_renderer();
    }

    fn user_event(&mut self, _: &ActiveEventLoop, (): ()) {
        self.woken();
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if self.next_frame.is_some_and(|t| now >= t) {
            self.next_frame = None;
            if let Some(r) = &self.renderer {
                r.window().request_redraw();
            }
        }
        let timer = task::fire_timers(now);
        el.set_control_flow(match self.next_frame.into_iter().chain(timer).min() {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
    }

    // Android destroys the native window when backgrounded.
    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.renderer = None;
        self.pending_renderer = None;
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let scale = self.scale();
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => self.resized(size.width, size.height),
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::Occluded(o) => {
                self.occluded = o;
                if !o && let Some(r) = &self.renderer {
                    r.window().request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32 / scale, position.y as f32 / scale);
                let (x, y) = self.cursor;
                self.dispatch(Event::PointerMove { x, y });
            }
            WindowEvent::CursorLeft { .. } => self.dispatch(Event::Leave),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.cursor;
                self.dispatch(match state {
                    ElementState::Pressed => Event::PointerDown { x, y },
                    ElementState::Released => Event::PointerUp { x, y },
                });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * 40.0,
                    MouseScrollDelta::PixelDelta(p) => -p.y as f32 / scale,
                };
                let (x, y) = self.cursor;
                self.dispatch(Event::Wheel { x, y, dy });
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.key_pressed(&event.logical_key, event.text.as_deref());
            }
            WindowEvent::Ime(Ime::Commit(t)) => self.dispatch(Event::Text(t)),
            WindowEvent::Touch(t) => self.touch(
                t.id,
                t.phase,
                t.location.x as f32 / scale,
                t.location.y as f32 / scale,
            ),
            _ => {}
        }
    }
}
