//! Entry point shared by the docs site's examples.

use dfui::{App, Color, Element};

struct Live(fn() -> Element);

impl App for Live {
    fn view(&self) -> Element {
        (self.0)()
    }
    fn background(&self) -> Color {
        Color::hex(0xf2f3f7)
    }
}

/// Runs `view` as the whole app: in a canvas on the web, in a window natively.
pub fn run(view: fn() -> Element) {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();
    dfui::run(Live(view));
}
