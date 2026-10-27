use jni::objects::JObject;
use jni::{Env, JValue, JavaVM, jni_sig, jni_str};

use crate::AndroidApp;

/// System bar + display cutout insets in physical px: top, right, bottom, left.
/// NativeActivity's content rect ignores them once the window is edge-to-edge.
pub(crate) fn insets(app: &AndroidApp) -> [i32; 4] {
    // `WindowInsets.Type` is API 30+; edge-to-edge is only enforced from 35.
    if AndroidApp::sdk_version() < 30 {
        return [0; 4];
    }
    // SAFETY: android-activity hands out a valid JavaVM pointer for the process.
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    vm.attach_current_thread(|env| -> jni::errors::Result<[i32; 4]> {
        let r = query(env, app);
        if env.exception_check() {
            env.exception_clear();
        }
        Ok(r.unwrap_or_else(|e| {
            log::warn!("reading window insets failed: {e}");
            [0; 4]
        }))
    })
    .unwrap_or([0; 4])
}

fn query(env: &mut Env, app: &AndroidApp) -> jni::errors::Result<[i32; 4]> {
    // SAFETY: the activity global ref outlives this call; JObject doesn't delete it.
    let activity = unsafe { JObject::from_raw(env, app.activity_as_ptr().cast()) };
    let window = env
        .call_method(
            &activity,
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
