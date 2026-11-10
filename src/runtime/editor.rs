//! The soft keyboard's way in: a Java view with a real `InputConnection`.
//! Its classes are embedded as dex (`scripts/android-dex.sh`) and loaded at
//! runtime, so apps need no Java or packaging of their own.

use std::ffi::c_void;
use std::sync::Mutex;

use jni::errors::{LogErrorAndDefault, Result};
use jni::objects::{Global, JClass, JObject, JString, JValue};
use jni::{Env, EnvUnowned, NativeMethod, jni_sig, jni_str};
use winit::platform::android::activity::{AndroidApp, AndroidAppWaker};

use crate::android::with_activity;
use crate::widget::Keyboard;

static DEX: &[u8] = include_bytes!("../../android/classes.dex");

pub(crate) enum Edit {
    Text(String),
    /// UTF-16 units before the caret.
    Delete(usize),
}

static EDITS: Mutex<Vec<Edit>> = Mutex::new(Vec::new());
static WAKER: Mutex<Option<AndroidAppWaker>> = Mutex::new(None);
static VIEW: Mutex<Option<Global<JObject<'static>>>> = Mutex::new(None);

fn push(e: Edit) {
    EDITS.lock().unwrap().push(e);
    if let Some(w) = &*WAKER.lock().unwrap() {
        w.wake();
    }
}

/// Edits from the keyboard since the last call.
pub(crate) fn take() -> Vec<Edit> {
    std::mem::take(&mut EDITS.lock().unwrap())
}

extern "system" fn text<'l>(mut env: EnvUnowned<'l>, _: JClass<'l>, s: JString<'l>) {
    env.with_env(|env| -> Result<()> {
        push(Edit::Text(s.try_to_string(env)?));
        Ok(())
    })
    .resolve::<LogErrorAndDefault>()
}

extern "system" fn backspace<'l>(_: EnvUnowned<'l>, _: JClass<'l>, n: i32) {
    push(Edit::Delete(n.max(0) as usize));
}

/// Adds the view to this activity; runs once per activity, before any [`set_keyboard`].
pub(crate) fn install(app: &AndroidApp) {
    *WAKER.lock().unwrap() = Some(app.create_waker());
    let a = app.clone();
    app.run_on_java_main_thread(Box::new(move || match with_activity(&a, create) {
        Ok(v) => *VIEW.lock().unwrap() = Some(v),
        Err(e) => log::error!("creating the editor view failed: {e}"),
    }));
}

/// Lets the destroyed activity go: the view references it, and the process
/// can outlive it.
pub(crate) fn uninstall() {
    *VIEW.lock().unwrap() = None;
    *WAKER.lock().unwrap() = None;
}

fn create(env: &mut Env, activity: &JObject) -> Result<Global<JObject<'static>>> {
    let bytes = env.byte_array_from_slice(DEX)?;
    let buffer = env
        .call_static_method(
            jni_str!("java/nio/ByteBuffer"),
            jni_str!("wrap"),
            jni_sig!("([B)Ljava/nio/ByteBuffer;"),
            &[JValue::Object(&bytes)],
        )?
        .l()?;
    let parent = env
        .call_method(
            activity,
            jni_str!("getClassLoader"),
            jni_sig!("()Ljava/lang/ClassLoader;"),
            &[],
        )?
        .l()?;
    let loader = env.new_object(
        jni_str!("dalvik/system/InMemoryDexClassLoader"),
        jni_sig!("(Ljava/nio/ByteBuffer;Ljava/lang/ClassLoader;)V"),
        &[JValue::Object(&buffer), JValue::Object(&parent)],
    )?;
    let name = env.new_string("dev.dfui.EditorView")?;
    let class = env
        .call_method(
            &loader,
            jni_str!("loadClass"),
            jni_sig!("(Ljava/lang/String;)Ljava/lang/Class;"),
            &[JValue::Object(&name)],
        )?
        .l()?;
    let class = env.cast_local::<JClass>(class)?;
    // SAFETY: each pointer matches the signature given beside it.
    let methods = unsafe {
        [
            NativeMethod::from_raw_parts(
                jni_str!("text"),
                jni_str!("(Ljava/lang/String;)V"),
                text as *mut c_void,
            ),
            NativeMethod::from_raw_parts(
                jni_str!("backspace"),
                jni_str!("(I)V"),
                backspace as *mut c_void,
            ),
        ]
    };
    // SAFETY: as above.
    unsafe { env.register_native_methods(&class, &methods)? };
    let view = env.new_object(
        &class,
        jni_sig!("(Landroid/content/Context;)V"),
        &[JValue::Object(activity)],
    )?;
    let params = env.new_object(
        jni_str!("android/view/ViewGroup$LayoutParams"),
        jni_sig!("(II)V"),
        &[JValue::Int(1), JValue::Int(1)],
    )?;
    env.call_method(
        activity,
        jni_str!("addContentView"),
        jni_sig!("(Landroid/view/View;Landroid/view/ViewGroup$LayoutParams;)V"),
        &[JValue::Object(&view), JValue::Object(&params)],
    )?;
    env.new_global_ref(view)
}

/// `InputType` flags for each keyboard.
fn input_type(k: Keyboard) -> i32 {
    match k {
        Keyboard::Text => 0x1,
        Keyboard::Number => 0x2,
        Keyboard::Email => 0x21,
        Keyboard::Password => 0x81,
    }
}

/// Shows the given keyboard, attached to the view, or hides it.
pub(crate) fn set_keyboard(app: &AndroidApp, keyboard: Option<Keyboard>) {
    let a = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        if let Err(e) = with_activity(&a, |env, activity| apply(env, activity, keyboard)) {
            log::error!("toggling the keyboard failed: {e}");
        }
    }));
}

fn apply(env: &mut Env, activity: &JObject, keyboard: Option<Keyboard>) -> Result<()> {
    let guard = VIEW.lock().unwrap();
    let Some(view) = guard.as_ref() else {
        return Ok(());
    };
    let service = env.new_string("input_method")?;
    let imm = env
        .call_method(
            activity,
            jni_str!("getSystemService"),
            jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
            &[JValue::Object(&service)],
        )?
        .l()?;
    if let Some(k) = keyboard {
        env.call_method(
            view,
            jni_str!("setInputType"),
            jni_sig!("(I)V"),
            &[JValue::Int(input_type(k))],
        )?;
        env.call_method(view, jni_str!("requestFocus"), jni_sig!("()Z"), &[])?;
        env.call_method(
            &imm,
            jni_str!("restartInput"),
            jni_sig!("(Landroid/view/View;)V"),
            &[JValue::Object(view)],
        )?;
        env.call_method(
            &imm,
            jni_str!("showSoftInput"),
            jni_sig!("(Landroid/view/View;I)Z"),
            &[JValue::Object(view), JValue::Int(0)],
        )?;
    } else {
        let token = env
            .call_method(
                view,
                jni_str!("getWindowToken"),
                jni_sig!("()Landroid/os/IBinder;"),
                &[],
            )?
            .l()?;
        env.call_method(
            &imm,
            jni_str!("hideSoftInputFromWindow"),
            jni_sig!("(Landroid/os/IBinder;I)Z"),
            &[JValue::Object(&token), JValue::Int(0)],
        )?;
    }
    Ok(())
}
