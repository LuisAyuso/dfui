# Getting started

## Add the dependency

dfui isn't on crates.io yet; depend on the repository:

```toml
[dependencies]
dfui = { git = "https://github.com/LuisAyuso/dfui" }
```

## A first app

An app implements `App`: `view` returns the element tree, and is called again
whenever state changes. State lives in `stateful` widgets; handlers change it
through the `Copy` handle they capture.

```rust
use dfui::widgets::{Styled, button, column, stateful, text};
use dfui::{App, Color, Element};

struct Counter;

impl App for Counter {
    fn view(&self) -> Element {
        stateful(|count: &u32, h| {
            column([
                text(format!("Clicked {count} times")).size(20.0).into(),
                button(text("Click").color(Color::WHITE))
                    .on_click(move |cx| cx.update(h, |n: &mut u32| *n += 1))
                    .into(),
            ])
            .gap(12.0)
            .padding(24.0)
        })
        .into()
    }
}

fn main() {
    dfui::run(Counter);
}
```

`dfui::run` opens a window and blocks until it closes (on the web it returns
and the page keeps running).

## External and async state

Anything the app owns, such as an HTTP client, a channel or a store, is
handed to the UI with `provide`. `future_builder` and `stream_builder` start
any std `Future` or `Stream` from it and rebuild as results arrive. They are
polled on the UI thread, so they work with any runtime, and they are
cancelled when their widget leaves the tree.

```rust
provide(api.clone(), future_builder(
    user_id,
    move |scope: Scope| scope.get::<Api>().unwrap().clone().user(user_id),
    |snap: &Snapshot<Option<User>>| match &snap.data {
        Some(user) => text(&user.name),
        None => text("Loading…"),
    },
))
```

From a program that already runs tokio, keep tokio on its worker threads and
give dfui the main thread:

```rust
fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    dfui::run(MyApp::new());
}
```

`dfui::sleep` is a timer driven by dfui's own event loop, for delays inside
builders on every platform, including the web.

## Desktop

```sh
cargo run --release
```

Windows, macOS and Linux (X11 and Wayland) run through winit and wgpu.

## Android

Apps are NativeActivity libraries built with `cargo-apk`. Make the crate a
`cdylib` and add an entry point:

```rust
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: dfui::AndroidApp) {
    dfui::run_android(MyApp::new(), app);
}
```

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk
cargo apk run --lib --release
```

The repository README covers the SDK and NDK setup.

## Web

dfui builds for `wasm32-unknown-unknown` and draws with WebGPU. Build the app
as a binary or `cdylib`, generate the JavaScript glue with `wasm-bindgen`
(the same version as in your `Cargo.lock`), and load it from a page whose CSS
gives the canvas its size:

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir pkg target/wasm32-unknown-unknown/release/my_app.wasm
```

```html
<style>
  html, body { margin: 0; height: 100%; }
  canvas { display: block; width: 100%; height: 100%; }
</style>
<script type="module">
  import init from "./pkg/my_app.js";
  await init();
</script>
```

The examples on this site are built exactly this way; see `scripts/site.sh`
in the repository.
