# dfui

[![crates.io](https://img.shields.io/crates/v/dfui.svg)](https://crates.io/crates/dfui)
[![docs.rs](https://img.shields.io/docsrs/dfui)](https://docs.rs/dfui)
[![examples](https://img.shields.io/badge/examples-live-blue)](https://luisayuso.github.io/dfui/)
[![tests](https://github.com/LuisAyuso/dfui/actions/workflows/test.yml/badge.svg)](https://github.com/LuisAyuso/dfui/actions/workflows/test.yml)
[![license](https://img.shields.io/crates/l/dfui.svg)](LICENSE)

A small declarative UI framework in Rust that renders everything —
shapes, shadows, text and icons — analytically in a single wgpu shader, with
no glyph atlas. Text and icons are drawn straight from their Bézier outlines
with the [Slug algorithm](docs/glyphs.md). Primary target is Android
(NativeActivity, Vulkan); desktop runs through the same winit event loop, and
the [examples page](https://luisayuso.github.io/dfui/) runs every widget live
in the browser.

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

## Crates

| Crate | |
|---|---|
| [dfui](https://crates.io/crates/dfui) | The framework. |
| [dfui-map](map/) | Pannable, zoomable vector maps, built on dfui's public API. |
| [dfui-media](media/) | Hardware-decoded video (VA-API, MediaCodec), HDR. |

All three release together under one version.

## Documentation

- [Examples and guides](https://luisayuso.github.io/dfui/): getting started,
  concepts, and a live page per widget.
- [API reference](https://docs.rs/dfui).
- Design: [architecture](docs/architecture.md), [rendering](docs/rendering.md),
  [glyphs](docs/glyphs.md), [state management](docs/state-management.md),
  [theming](docs/theming.md), [HDR](docs/hdr.md).
- [Icons](docs/icons.md): converting SVGs with `svg2path`.
- [Development](docs/development.md): the demo, tests, benches, releasing.
- [Performance](docs/performance.md): measured numbers, desktop and phone.
- [Android](docs/android.md) and [Web](docs/web.md) builds.

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

## License

[MIT](LICENSE). The bundled fonts `assets/DejaVuSans*.ttf` keep their own
license, the DejaVu Fonts License derived from Bitstream Vera:
[`assets/DejaVuSans-LICENSE.txt`](assets/DejaVuSans-LICENSE.txt).
