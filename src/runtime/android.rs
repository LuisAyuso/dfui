//! Calling into Java from app code.
//!
//! Threads started by native code use the system class loader, so
//! `FindClass` can't see the app's own classes from them; reach those by
//! name through the activity (e.g. `Intent.setClassName`) or its
//! `getClassLoader().loadClass(..)`.

use jni::objects::JObject;
use jni::{Env, JValue, JavaVM, jni_sig, jni_str};

pub use jni;

use crate::AndroidApp;

/// Runs `f` on the current thread, attached to the JVM, with the activity.
/// Any thread works, but only while the activity lives (until `run_android`
/// returns). A pending Java exception is cleared and reported as an error.
pub fn with_activity<R>(
    app: &AndroidApp,
    f: impl FnOnce(&mut Env, &JObject) -> jni::errors::Result<R>,
) -> jni::errors::Result<R> {
    // SAFETY: android-activity hands out a valid JavaVM pointer for the process.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    vm.attach_current_thread(|env| {
        // SAFETY: the activity global ref lives until onDestroy; JObject doesn't delete it.
        let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
        let r = f(env, &activity);
        if env.exception_check() {
            env.exception_clear();
            return r.and(Err(jni::errors::Error::JavaException));
        }
        r
    })
}

/// System bar + display cutout insets in physical px: top, right, bottom, left.
/// NativeActivity's content rect ignores them once the window is edge-to-edge.
pub(crate) fn insets(app: &AndroidApp) -> [i32; 4] {
    // `WindowInsets.Type` is API 30+; edge-to-edge is only enforced from 35.
    if AndroidApp::sdk_version() < 30 {
        return [0; 4];
    }
    with_activity(app, query).unwrap_or_else(|e| {
        log::warn!("reading window insets failed: {e}");
        [0; 4]
    })
}

/// Dark status and navigation bar icons when `light` (a light background
/// behind them), light icons otherwise.
pub(crate) fn set_light_bars(app: &AndroidApp, light: bool) {
    // `WindowInsetsController` is API 30+.
    if AndroidApp::sdk_version() < 30 {
        return;
    }
    let a = app.clone();
    // Views may only be touched from the Java UI thread.
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(e) = with_activity(&a, |env, activity| light_bars(env, activity, light)) {
            log::warn!("setting system bar appearance failed: {e}");
        }
    }));
}

fn light_bars(env: &mut Env, activity: &JObject, light: bool) -> jni::errors::Result<()> {
    // WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS | APPEARANCE_LIGHT_NAVIGATION_BARS
    const MASK: i32 = 0x8 | 0x10;
    let window = env
        .call_method(
            activity,
            jni_str!("getWindow"),
            jni_sig!("()Landroid/view/Window;"),
            &[],
        )?
        .l()?;
    let controller = env
        .call_method(
            &window,
            jni_str!("getInsetsController"),
            jni_sig!("()Landroid/view/WindowInsetsController;"),
            &[],
        )?
        .l()?;
    if controller.is_null() {
        return Ok(());
    }
    env.call_method(
        &controller,
        jni_str!("setSystemBarsAppearance"),
        jni_sig!("(II)V"),
        &[JValue::Int(if light { MASK } else { 0 }), JValue::Int(MASK)],
    )?;
    Ok(())
}

fn query(env: &mut Env, activity: &JObject) -> jni::errors::Result<[i32; 4]> {
    let window = env
        .call_method(
            activity,
            jni_str!("getWindow"),
            jni_sig!("()Landroid/view/Window;"),
            &[],
        )?
        .l()?;
    let decor = env
        .call_method(
            &window,
            jni_str!("getDecorView"),
            jni_sig!("()Landroid/view/View;"),
            &[],
        )?
        .l()?;
    let root = env
        .call_method(
            &decor,
            jni_str!("getRootWindowInsets"),
            jni_sig!("()Landroid/view/WindowInsets;"),
            &[],
        )?
        .l()?;
    if root.is_null() {
        return Ok([0; 4]);
    }
    let types = jni_str!("android/view/WindowInsets$Type");
    let bars = env
        .call_static_method(types, jni_str!("systemBars"), jni_sig!("()I"), &[])?
        .i()?;
    let cutout = env
        .call_static_method(types, jni_str!("displayCutout"), jni_sig!("()I"), &[])?
        .i()?;
    let ins = env
        .call_method(
            &root,
            jni_str!("getInsets"),
            jni_sig!("(I)Landroid/graphics/Insets;"),
            &[JValue::Int(bars | cutout)],
        )?
        .l()?;
    let mut field = |name| env.get_field(&ins, name, jni_sig!("I")).and_then(|v| v.i());
    Ok([
        field(jni_str!("top"))?,
        field(jni_str!("right"))?,
        field(jni_str!("bottom"))?,
        field(jni_str!("left"))?,
    ])
}
