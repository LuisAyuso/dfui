/// The system clipboard on desktop; an in-app one on Android, the web and in tests.
// ponytail: Android could use ClipboardManager over JNI once there's a touch selection UI.
#[derive(Default)]
pub(super) struct Clipboard {
    #[cfg(not(any(test, target_os = "android", target_arch = "wasm32")))]
    sys: Option<arboard::Clipboard>,
    #[cfg(any(test, target_os = "android", target_arch = "wasm32"))]
    text: Option<String>,
}

#[cfg(not(any(test, target_os = "android", target_arch = "wasm32")))]
impl Clipboard {
    fn sys(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.sys.is_none() {
            self.sys = arboard::Clipboard::new()
                .inspect_err(|e| log::warn!("clipboard unavailable: {e}"))
                .ok();
        }
        self.sys.as_mut()
    }

    pub fn get(&mut self) -> Option<String> {
        self.sys()?.get_text().ok()
    }

    pub fn set(&mut self, t: String) {
        if let Some(c) = self.sys()
            && let Err(e) = c.set_text(t)
        {
            log::warn!("clipboard write failed: {e}");
        }
    }
}

#[cfg(any(test, target_os = "android", target_arch = "wasm32"))]
impl Clipboard {
    pub fn get(&mut self) -> Option<String> {
        self.text.clone()
    }

    pub fn set(&mut self, t: String) {
        self.text = Some(t);
    }
}
