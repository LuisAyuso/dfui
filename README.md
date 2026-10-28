# dfui

A small declarative (Elm-style) UI framework in Rust that renders everything —
shapes, shadows and text — as distance fields / analytic coverage in a single
wgpu shader. Primary target is Android (NativeActivity, Vulkan); desktop runs
through the same winit event loop.

```rust
impl App for Counter {
    type Msg = i32;
    fn update(&mut self, d: i32) { self.0 += d; }
    fn view(&self) -> Element<i32> {
        column([button(text("+")).on_press(1).into()]).into()
    }
}

fn main() { dfui::run(Counter(0)); }
```

## Layout

- `src/` — the framework crate (`dfui`)
- `demo/` — demo app, desktop binary and Android `cdylib`
- `docs/architecture.md` — how it fits together
- `docs/rendering.md` — renderer details and design decisions
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

## License

[WTFPL](LICENSE). The bundled font `assets/DejaVuSans.ttf` keeps its own
license, the DejaVu Fonts License derived from Bitstream Vera:
[`assets/DejaVuSans-LICENSE.txt`](assets/DejaVuSans-LICENSE.txt).
