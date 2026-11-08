# dfui

A small declarative UI framework in Rust that renders everything —
shapes, shadows, text and icons — analytically in a single wgpu shader, with
no glyph atlas. Text and icons are drawn straight from their Bézier outlines
with the [Slug algorithm](docs/glyphs.md). Primary target is Android (NativeActivity, Vulkan); desktop runs
through the same winit event loop.

```rust
impl App for Counter {
    fn view(&self) -> Element {
        stateful(|n: &i32, h| {
            button(text(format!("+ {n}"))).on_click(move |cx| cx.update(h, |n: &mut i32| *n += 1))
        })
        .into()
    }
}

fn main() { dfui::run(Counter); }
```

## Layout

- `src/` — the framework crate (`dfui`)
- `demo/` — demo app, desktop binary and Android `cdylib`
- `site/` — docs site published to GitHub Pages: guide pages and a live example per widget
- `tools/svg2path/` — converts SVG icons to shapes an app can bundle (see Icons)
- `docs/architecture.md` — how it fits together
- `docs/rendering.md` — renderer details and design decisions
- `docs/glyphs.md` — how text and icons are drawn from outlines (Slug)
- `docs/tree.md` — the flattened node list and its id cache
- `docs/async-and-state.md` — study: external/async state and the web port
- `docs/state-management.md` — study: state architectures across UI frameworks
- `docs/data-models.md` — list and tree models, virtualized lists, tables and trees
- `docs/theming.md` — the `Theme`, color roles and how other frameworks do it
- `TODO.md` — roadmap and current performance baseline

## Running

```sh
cargo run -p dfui-demo --release   # desktop
cargo test                         # GPU tests: cargo test -- --include-ignored
cargo bench --bench cpu            # criterion CPU frame benches
cargo bench --bench gpu            # criterion GPU benches (timestamp-measured)
scripts/android-bench.sh gpu       # same benches on the connected phone (or cpu)
```

`DFUI_SHOW_DAMAGE=1` tints the area redrawn each frame (desktop debugging).

## Icons

dfui has no SVG parser. `svg2path` converts icons ahead of time into `.path`
files that the app bundles; commit them, and rerun it when an SVG changes.

```sh
cargo run -p svg2path -- icons/ assets/icons/   # a file or a directory
cargo install --path tools/svg2path             # for use outside this repo
```

Build each `Shape` once and keep it, for example in the `App`. Every new
`Shape` is stored again and never freed, so don't build one in `view`:

```rust
struct MyApp { close: Shape }

impl App for MyApp {
    fn view(&self) -> Element {
        icon_button(&self.close).on_click(|_| {}).into()
    }
}

fn main() {
    let close = Shape::from_bytes(include_bytes!("../assets/icons/close.path"));
    dfui::run(MyApp { close });
}
```

An icon is one shape in one color, set with `.color()`; the SVG's colors are
dropped. Fills and strokes convert. `<text>` and embedded images are skipped
without a warning (an SVG of only text gives an empty shape), and a directory
is read without its subdirectories.

## Web

The framework builds for `wasm32-unknown-unknown` and draws with WebGPU
(WebGL2 lacks the fragment-stage storage buffers the text shader reads).

The docs site in `site/` is static HTML, deployed to GitHub Pages by
`.github/workflows/pages.yml` on every push to `main`:

- `site/pages/*.md`: hand-written pages (introduction, getting started,
  concepts); `docs/architecture.md` is published as is.
- `site/examples/*.rs`: one standalone program per widget, built to its own
  wasm. Its `//!` header is the widget page's text; its code is shown below
  the live frame. New examples are added to `WIDGETS` in `site/src/main.rs`.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129   # must match Cargo.lock
scripts/site.sh                                    # writes site/dist
python3 -m http.server -d site/dist                # open http://localhost:8000
cargo run -p dfui-site --example slider           # one example, natively
```

## Android

The demo is a NativeActivity app built with `cargo-apk`, aarch64 only,
min SDK 26, target SDK 35.

One-time setup:

```sh
rustup target add aarch64-linux-android
cargo install cargo-apk
# Android SDK with platform-tools, build-tools, platform android-35 and an NDK
export ANDROID_HOME=/opt/android-sdk
export NDK_HOME=$ANDROID_HOME/ndk/<version>
# Release builds need a keystore; the debug one is fine for development.
export CARGO_APK_RELEASE_KEYSTORE=$HOME/.android/debug.keystore
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
```

`cargo-apk` is capped at platform 35, so `android-35` must be installed even
if newer platforms are present.

Run on a device (USB debugging on, visible in `adb devices`):

```sh
cargo apk run -p dfui-demo --lib --release
adb logcat | grep -E 'dfui|fps'   # logs; frame stats every 2 s
```

`--lib` is needed because the demo crate also has a desktop binary, which
makes `cargo-apk` panic ("Bin is not compatible with Cdylib") after building.
The APK lands in `target/release/apk/dfui-demo.apk`; `cargo apk build`
instead of `run` only builds it. For real distribution, point the keystore
variables at a release keystore. Frame stats (`fps … cpu … gpu …`) are how the
baselines in `TODO.md` are measured; use the demo's Bench screen.

In an app, `android_main` calls `dfui::run_android`, which returns when the
activity is destroyed. If the process lives on (a service, a download),
Android calls `android_main` again for the next activity, and it can run
again. `eprintln!` output is lost on Android, so log with `log` and
`android_logger` (see `android_main` in `demo/src/lib.rs`) and read the logs
with `adb logcat`. `dfui::android::with_activity` runs JNI calls with the
activity from any thread.

## License

[WTFPL](LICENSE). The bundled fonts `assets/DejaVuSans*.ttf` keep their own
license, the DejaVu Fonts License derived from Bitstream Vera:
[`assets/DejaVuSans-LICENSE.txt`](assets/DejaVuSans-LICENSE.txt).
