//! Android event loop on android-activity directly: unlike winit's, it
//! returns on `onDestroy` and can run again when the activity is recreated
//! in the same process.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use web_time::Instant;
use winit::event::TouchPhase;
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey};
use winit::platform::android::activity::input::{
    InputEvent, KeyAction, KeyEvent, KeyMapChar, Keycode, MetaState, MotionAction,
};
use winit::platform::android::activity::ndk::native_window::NativeWindow;
use winit::platform::android::activity::{AndroidApp, InputStatus, MainEvent, PollEvent, ndk_sys};
use winit::raw_window_handle::{
    AndroidDisplayHandle, AndroidNdkWindowHandle, RawDisplayHandle, RawWindowHandle,
};

use super::Runtime;
use super::task::{self, LoopWaker};
use crate::App;
use crate::render::{Host, Renderer};
use crate::widget::Event;

struct Window {
    app: AndroidApp,
    native: NativeWindow,
    redraw: Rc<Cell<bool>>,
}

impl Host for Window {
    fn surface(&self, instance: &wgpu::Instance) -> wgpu::Surface<'static> {
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Android(AndroidDisplayHandle::new())),
            raw_window_handle: RawWindowHandle::AndroidNdk(AndroidNdkWindowHandle::new(
                self.native.ptr().cast(),
            )),
        };
        // SAFETY: `Renderer` drops the surface before this window, whose
        // `NativeWindow` holds a reference to the ANativeWindow.
        unsafe { instance.create_surface_unsafe(target) }.expect("create surface")
    }
    fn size(&self) -> (u32, u32) {
        (self.native.width() as u32, self.native.height() as u32)
    }
    fn scale(&self) -> f32 {
        self.app
            .config()
            .density()
            .map_or(1.0, |d| d as f32 / 160.0)
    }
    fn request_redraw(&self) {
        self.redraw.set(true);
    }
    fn set_ime(&self, on: bool) {
        if on {
            self.app.show_soft_input(true);
        } else {
            self.app.hide_soft_input(true);
        }
    }
}

pub(crate) fn run<A: App>(mut rt: Runtime<A>, app: AndroidApp) {
    let waker = app.create_waker();
    let w = LoopWaker::new(move || waker.wake());
    rt.waker = w.clone().into();
    rt.loop_waker = Some(w);
    rt.android = Some(app.clone());
    rt.drive();
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(desc.with_env());
    let redraw = Rc::new(Cell::new(false));
    let mut light = None;
    let mut destroyed = false;
    while !destroyed {
        let now = Instant::now();
        if rt.next_frame.is_some_and(|t| now >= t) {
            rt.next_frame = None;
            redraw.set(true);
        }
        let timer = task::fire_timers(now);
        let timeout = if redraw.get() && rt.renderer.is_some() {
            Some(Duration::ZERO)
        } else {
            (rt.next_frame.into_iter().chain(timer).min()).map(|t| t.saturating_duration_since(now))
        };
        app.poll_events(timeout, |ev| match ev {
            PollEvent::Main(MainEvent::InitWindow { .. }) => {
                if let Some(native) = app.native_window() {
                    let window = Window {
                        app: app.clone(),
                        native,
                        redraw: redraw.clone(),
                    };
                    let r = Renderer::new(instance.clone(), Box::new(window));
                    rt.pending_renderer = Some(Box::pin(r));
                    rt.poll_renderer();
                }
            }
            // The window is gone once this callback returns.
            PollEvent::Main(MainEvent::TerminateWindow { .. }) => {
                rt.renderer = None;
                rt.pending_renderer = None;
            }
            PollEvent::Main(MainEvent::WindowResized { .. } | MainEvent::ConfigChanged { .. }) => {
                if let Some(n) = app.native_window() {
                    rt.resized(n.width() as u32, n.height() as u32);
                }
            }
            PollEvent::Main(MainEvent::RedrawNeeded { .. }) => redraw.set(true),
            PollEvent::Main(MainEvent::InsetsChanged { .. }) => {
                rt.update_insets();
                rt.redraw();
            }
            PollEvent::Main(MainEvent::Destroy) => destroyed = true,
            _ => {}
        });
        if let Ok(mut events) = app.input_events_iter() {
            while events.next(|ev| input(&mut rt, &app, ev)) {}
        }
        // A wake can arrive as any event, not only `PollEvent::Wake`; missing
        // one would leave the waker pending and silence it for good.
        if rt.loop_waker.as_ref().is_some_and(|w| w.pending()) {
            rt.woken();
        }
        if rt.renderer.is_some() && redraw.take() {
            rt.paint();
            let bg = rt.app.background();
            let l = 0.299 * bg.r + 0.587 * bg.g + 0.114 * bg.b > 0.5;
            if light != Some(l) {
                light = Some(l);
                crate::android::set_light_bars(&app, l);
            }
        }
    }
}

/// Unhandled events (Back, volume) get the system's default behavior.
fn input<A: App>(rt: &mut Runtime<A>, app: &AndroidApp, ev: &InputEvent) -> InputStatus {
    match ev {
        InputEvent::MotionEvent(m) => {
            let phase = match m.action() {
                MotionAction::Down | MotionAction::PointerDown => TouchPhase::Started,
                MotionAction::Up | MotionAction::PointerUp => TouchPhase::Ended,
                MotionAction::Move => TouchPhase::Moved,
                MotionAction::Cancel => TouchPhase::Cancelled,
                _ => return InputStatus::Unhandled,
            };
            let pointers: Vec<_> = match phase {
                TouchPhase::Started | TouchPhase::Ended => {
                    vec![m.pointer_at_index(m.pointer_index())]
                }
                _ => m.pointers().collect(),
            };
            let s = rt.scale();
            for p in pointers {
                rt.touch(p.pointer_id() as u64, phase, p.x() / s, p.y() / s);
            }
            InputStatus::Handled
        }
        InputEvent::KeyEvent(k) if k.action() == KeyAction::Down => {
            let meta = k.meta_state();
            rt.mods.set(ModifiersState::SHIFT, meta.shift_on());
            rt.mods.set(ModifiersState::CONTROL, meta.ctrl_on());
            rt.mods.set(ModifiersState::ALT, meta.alt_on());
            if k.key_code() == Keycode::Back {
                return match rt.dispatch(Event::Back) {
                    true => InputStatus::Handled,
                    false => InputStatus::Unhandled,
                };
            }
            let named = match k.key_code() {
                Keycode::Del => Some(NamedKey::Backspace),
                Keycode::ForwardDel => Some(NamedKey::Delete),
                Keycode::DpadLeft => Some(NamedKey::ArrowLeft),
                Keycode::DpadRight => Some(NamedKey::ArrowRight),
                Keycode::MoveHome => Some(NamedKey::Home),
                Keycode::MoveEnd => Some(NamedKey::End),
                _ => None,
            };
            let key: WKey = match named {
                Some(n) => WKey::Named(n),
                None => match char_of(app, k) {
                    Some(c) => WKey::Character(c.to_string().into()),
                    None => return InputStatus::Unhandled,
                },
            };
            let text = match &key {
                WKey::Character(c) => Some(c.to_string()),
                _ => None,
            };
            rt.key_pressed(&key, text.as_deref());
            InputStatus::Handled
        }
        _ => InputStatus::Unhandled,
    }
}

/// The key's character ignoring Ctrl, so shortcuts still see their letter.
// ponytail: dead keys (combining accents) are dropped; combine them via KeyCharacterMap.getDeadChar if hardware keyboards need it.
fn char_of(app: &AndroidApp, k: &KeyEvent) -> Option<char> {
    let ctrl = ndk_sys::AMETA_CTRL_ON | ndk_sys::AMETA_CTRL_LEFT_ON | ndk_sys::AMETA_CTRL_RIGHT_ON;
    let meta = MetaState(k.meta_state().0 & !ctrl);
    let map = app.device_key_character_map(k.device_id()).ok()?;
    match map.get(k.key_code(), meta).ok()? {
        KeyMapChar::Unicode(c) => Some(c),
        _ => None,
    }
}
