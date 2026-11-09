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

## Motivation

This is the only human-written section in the project. With it I want to explain why I started this project and where it is going. You may disagree with the reasons or methods, and I invite you to open an issue to let me know where I got it wrong.

I started working on this framework out of curiosity and because I am quite fed up with the feature creep which invades every aspect of commercial software. I needed a cool framework to develop apps which I want to use. There is no intent to turn this into a product, for two reasons: I believe anyone can tailor a best-fit framework to whatever they need with the use of AI, and I do not have the energy to deal with the overhead of curating all the corner cases.

At work I usually try to prioritize tasks and model development after the following statement: first stability, followed by features (because they pay the bills), and lastly performance (unless performance is a feature, which it not always is). Usually managers come around and mess up the priorities: features (because they are already sold), performance, and then stability (because they do not know better). This project is for me, for my own enjoyment, and therefore I model it in the following order:

- Stability:
  - I chose Rust as a programming language because I think it interacts very well with agentic development (vibecoding if you must), thanks to the strict compiler requirements and the fast feedback that the model gets from them. I would not necessarily write a UI framework in Rust because of the complexity, but this project is not aimed at facilitating human development; this tool is directed at agents.
  - The graphics created by this framework are analytical to avoid interpolation or aliasing nonsense when modifying the size. As I get older I am beginning to make the fonts larger, and I want buttery smooth rendering at any size.
  - Declarative versus immediate mode: lately immediate mode UI has been getting quite some traction. I can see the value of simplifying horrific UI frameworks which need a PhD to be understood. Nevertheless, I foresee a complexity issue with immediate mode when the state (both global and ephemeral) has to be managed by the app and the only thing that prevents chaos is your own programming hygiene. This framework is targeted at coding agents, and currently I doubt that they can keep things scalable and clean without constant intervention in the architecture. I believe in abstractions, and I think that these apply to agents as well as they apply to humans, or better.
  - As few dependencies as I can manage. Dependency hell is a reality even when you have something which works as well as Cargo. I tolerate some small packages, and I lean heavily on other packages to provide the technological foundation (Taffy and wgpu).
- Performance:
  - Although precomputed glyphs rendered via quads could be faster, that approach is rather boring and limits the design to something which already exists. Nevertheless, performance is at the core of the development, because I got very frustrated with the loading times of other frameworks and the general wasteful thinking of frameworks. It seems like everyone is moving away from Electron now: I wonder why.
  - Multiple CPU and GPU benchmarks can be run on the development machine or on the phone. These benchmarks assist the agent in making performance decisions.
  - Multiple optimizations, because in the end I may just be hiding that I chose not to pre-render glyphs.
  - It is fun. I enjoy tuning performance way more than cannibalizing my API to squeeze in more features.
- Features:
  - I free myself from the burden of having to support any corner cases. The framework is cut-throat pragmatic. I may indulge myself in developing some oddity (like the morph animation), but those are driven purely by the need to make this project a fun thing. After all, I am doing this in my free time.
  - The choice of not having an SVG parser in the framework is deliberate. I do not see the need to force every app to contain extra dependencies, but I compromised by facilitating the creation of icons with a development-time tool which converts SVG into usable dfui paths.
  - No mipmapping: I was recently exposed to some discussion about better antialiasing algorithms when downscaling, and decided not to force mipmap generation into the framework.
  - Networking, data sources, databases, multimedia... blah blah blah. None of this has anything to do with a UI, and it belongs in the app. I see no value in forcing these into downstream developments.

I do not expect you to use this framework (after all, I am making it for myself), but I thought that maybe you would come across this site and wonder why I did it (I would).

## Running

```sh
cargo run -p dfui-demo --release   # desktop
cargo test                         # GPU tests: cargo test -- --include-ignored
cargo test --test patterns         # usual app patterns, driven by input events
cargo bench --bench cpu            # criterion CPU frame benches
cargo bench --bench gpu            # criterion GPU benches (timestamp-measured)
scripts/android-bench.sh gpu       # same benches on the connected phone (or cpu)
```

`DFUI_SHOW_DAMAGE=1` tints the area redrawn each frame (desktop debugging).

## Performance

Measured at commit `8f187ba` with the commands above, release builds. Desktop
is a Ryzen 5 3600 with an RTX 4060 Ti; the phone has a Mali-G715 GPU.
Phone numbers drift between sessions with GPU clock scaling, so compare builds
back to back.

CPU per frame, without the GPU's work (400×800 logical px):

| Bench | Desktop, 100 cards | Phone, 100 cards | Desktop, 1000 cards | Phone, 1000 cards |
|---|---|---|---|---|
| Full frame (`frame/cards`) | 503 µs | 582 µs | 8.77 ms | 11.6 ms |
| One label changes (`frame/edit`) | 562 µs | 643 µs | 9.78 ms | 12.6 ms |
| Scroll (`frame/scroll`) | 131 µs | 133 µs | 1.29 ms | 1.71 ms |
| Pointer move (`frame/pointer_move`) | 0.64 µs | 0.87 µs | 4.8 µs | 6.4 µs |

GPU per frame, render pass only (1080×2400, scale 2.625):

| Bench | RTX 4060 Ti | Mali-G715 |
|---|---|---|
| 10 cards (`gpu/cards_10`) | 133 µs | 3.01 ms |
| Screen of text (`gpu/text_wall`) | 113 µs | 3.65 ms |
| Same, redrawing the FPS meter area (`gpu/text_wall_partial`) | 10.9 µs | 65.8 µs |

Boot:

| | Desktop | Phone |
|---|---|---|
| Runtime and first layout, no window or GPU (`startup/cards_10`) | 644 µs | 578 µs |
| Cold start of the demo (median of 10, 103–121 ms) | not measured | 112 ms |

Cold start is `TotalTime` from `adb shell am start -W`, so it ends when Android
reports the activity displayed, which may be before dfui presents its first frame.

## Icons

dfui has no SVG parser. `svg2path` converts icons ahead of time into `.path`
files that the app bundles; commit them, and rerun it when an SVG changes.

```sh
cargo run -p svg2path -- icons/ assets/icons/   # a file or a directory
cargo install --path tools/svg2path             # for use outside this repo
```

Build each `Shape` once and keep it, for example in the `App`, to avoid parsing
it again in `view`:

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

Note: this framework is not intended for web development, but I like the idea of providing a page with functional examples of the snippets, therefore the web build exist.

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
