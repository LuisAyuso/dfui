# dfui

[![crates.io](https://img.shields.io/crates/v/dfui.svg)](https://crates.io/crates/dfui)
[![docs.rs](https://img.shields.io/docsrs/dfui)](https://docs.rs/dfui)
[![tests](https://github.com/LuisAyuso/dfui/actions/workflows/test.yml/badge.svg)](https://github.com/LuisAyuso/dfui/actions/workflows/test.yml)
[![lint](https://github.com/LuisAyuso/dfui/actions/workflows/lint.yml/badge.svg)](https://github.com/LuisAyuso/dfui/actions/workflows/lint.yml)
[![examples](https://img.shields.io/badge/examples-live-blue)](https://luisayuso.github.io/dfui/)
[![rust](https://img.shields.io/badge/rust-1.90%2B-orange.svg)](https://www.rust-lang.org)
[![license](https://img.shields.io/crates/l/dfui.svg)](https://github.com/LuisAyuso/dfui/blob/main/LICENSE)

A declarative UI framework for Rust, made for Android phones first and running
the same code on the desktop and in the browser. It draws everything (boxes,
borders, shadows, text and icons) analytically in a single wgpu shader. Text
and icons come straight from their Bézier outlines with the
[Slug algorithm](https://github.com/LuisAyuso/dfui/blob/main/docs/glyphs.md), without a glyph atlas, so
they stay sharp at any size and zoom, and glyphs can morph: a label turns
bold glyph by glyph.

```rust,no_run
use dfui::widgets::{button, stateful, text};
use dfui::{App, Element};

struct Counter;

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

The [examples page](https://luisayuso.github.io/dfui/) runs every widget live
in the browser.

## What it is

- **Views as functions of state.** `App::view` builds a tree of elements;
  `stateful` keeps a widget's state across rebuilds, and `watch` with a
  `Notifier` rebuilds when worker threads change the data a view reads.
- **Flexbox layout** through [Taffy](https://github.com/DioxusLabs/taffy).
- **Widgets:** buttons, text and number inputs, toggles, sliders, radios,
  segmented buttons and tabs, dropdowns, virtualized lists, tables and
  trees, pages with back navigation, dialogs, drawers, bottom sheets,
  snackbars, tooltips, images, progress bars and spinners, all themed
  through color roles.
- **Android first:** a NativeActivity on Vulkan with edge-to-edge drawing
  under the system bars, the back button, the soft keyboard and HDR output.
  The desktop runs through winit; the web build uses WebGPU.
- **Small:** no SVG parser or image codecs in the framework.
  [`svg2path`](https://github.com/LuisAyuso/dfui/tree/main/tools/svg2path) turns SVG icons into shapes at
  build time, and apps decode their own images.
- **Tested by rendering:** an off-screen `Bench` drives apps with events on a
  virtual clock, and golden images pin how each widget draws.

## Crates

| Crate | |
|---|---|
| [dfui](https://crates.io/crates/dfui) | The framework. |
| [dfui-map](https://github.com/LuisAyuso/dfui/tree/main/libs/map) | Pannable, zoomable vector maps. |
| [dfui-media](https://github.com/LuisAyuso/dfui/tree/main/libs/media) | Hardware-decoded video (VA-API, MediaCodec), HDR. |
| [dfui-plot](https://github.com/LuisAyuso/dfui/tree/main/libs/plot) | ggplot-style plots: lines, bars, boxes, candles. |
| [dfui-bevy](https://github.com/LuisAyuso/dfui/tree/main/libs/bevy) | A dfui app as the HUD of a Bevy game. |

The four in `libs/` are built only on dfui's public API, and dfui doesn't
depend on them. All five release together under one version.

## Apps built with it

- [meteo](https://github.com/LuisAyuso/meteo): daily and hourly weather for
  Spain (AEMET) and Austria (GeoSphere) on a map of their regions.
- [busapp](https://github.com/LuisAyuso/busapp): Innsbruck's buses on a map,
  with trip planning.

## Status

dfui is young and below 1.0: a minor version (0.2 → 0.3) may break the API,
a patch version doesn't. Most of the code is written with coding agents;
[`AGENTS.md`](https://github.com/LuisAyuso/dfui/blob/main/AGENTS.md) and the generated
[API index](https://github.com/LuisAyuso/dfui/blob/main/docs/api.md), every public item on one line, are
kept for them.

## Documentation

- [Examples and guides](https://luisayuso.github.io/dfui/): getting started,
  concepts, and a live page per widget.
- [API reference](https://docs.rs/dfui) and the
  [API index](https://github.com/LuisAyuso/dfui/blob/main/docs/api.md).
- Design: [architecture](https://github.com/LuisAyuso/dfui/blob/main/docs/architecture.md),
  [rendering](https://github.com/LuisAyuso/dfui/blob/main/docs/rendering.md),
  [glyphs](https://github.com/LuisAyuso/dfui/blob/main/docs/glyphs.md),
  [state management](https://github.com/LuisAyuso/dfui/blob/main/docs/state-management.md),
  [theming](https://github.com/LuisAyuso/dfui/blob/main/docs/theming.md), [HDR](https://github.com/LuisAyuso/dfui/blob/main/docs/hdr.md).
- [Icons](https://github.com/LuisAyuso/dfui/blob/main/docs/icons.md): converting SVGs with `svg2path`.
- [Development](https://github.com/LuisAyuso/dfui/blob/main/docs/development.md): the demo, tests, benches,
  releasing.
- [Performance](https://github.com/LuisAyuso/dfui/blob/main/docs/performance.md): measured numbers, desktop
  and phone.
- [Android](https://github.com/LuisAyuso/dfui/blob/main/docs/android.md) and
  [Web](https://github.com/LuisAyuso/dfui/blob/main/docs/web.md) builds.

## License

[MIT](https://github.com/LuisAyuso/dfui/blob/main/LICENSE). The bundled fonts `assets/DejaVuSans*.ttf` keep
their own license, the DejaVu Fonts License derived from Bitstream Vera:
[`assets/DejaVuSans-LICENSE.txt`](https://github.com/LuisAyuso/dfui/blob/main/assets/DejaVuSans-LICENSE.txt).
