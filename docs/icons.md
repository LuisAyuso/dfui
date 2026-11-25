# Icons

dfui has no SVG parser. `svg2path` converts icons ahead of time into `.path`
files that the app bundles; commit them, and rerun it when an SVG changes.

```sh
cargo run -p svg2path -- icons/ assets/icons/   # a file or a directory
cargo install --path tools/svg2path             # for use outside this repo
```

Build each `Shape` once and keep it, for example in the `App`, to avoid parsing
it again in `view`:

```rust,no_run
use dfui::widgets::icon_button;
use dfui::{App, Element, Shape};

struct MyApp { close: Shape }

impl App for MyApp {
    fn view(&self) -> Element {
        icon_button(&self.close).on_click(|_| {}).into()
    }
}

fn main() {
    let bytes = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/close.path"));
    let close = Shape::from_bytes(bytes).unwrap();
    dfui::run(MyApp { close });
}
```

An icon is one shape in one color, set with `.color()`; the SVG's colors are
dropped. Fills and strokes convert. `<text>` and embedded images are skipped
without a warning (an SVG of only text gives an empty shape), and a directory
is read without its subdirectories.
