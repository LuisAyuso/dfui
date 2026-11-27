# Working on dfui

Start with `README.md` for the commands, `docs/architecture.md` for how it fits
together, and `TODO.md` for the roadmap. Tick off or add items in `TODO.md` as
part of a change.

Before writing a helper, a layout workaround or a widget, look it up in
`docs/api.md`: every public function, method and type, one line each. Apps
on dfui (busapp, meteo) live in their own repos and point here too.

## Layout

- `src/` — the framework crate (`dfui`)
- `demo/` — demo app, desktop binary and Android `cdylib`
- `libs/` — optional crates on dfui's public API, released with it:
  - `libs/map/` — vector map widget (`dfui-map`)
  - `libs/media/` — hardware-decoded video (`dfui-media`)
  - `libs/plot/` — ggplot-style plots (`dfui-plot`)
  - `libs/bevy/` — dfui as a Bevy game's HUD (`dfui-bevy`), on `dfui::Embedded`
- `site/` — docs site published to GitHub Pages: guide pages and a live example per widget
- `tools/api-index/` — writes `docs/api.md` from the sources
- `tools/svg2path/` — converts SVG icons to shapes an app can bundle (see Icons in `README.md`)
- `docs/api.md` — generated index of the public API, one line per item
- `docs/architecture.md` — how it fits together
- `docs/rendering.md` — renderer details and design decisions
- `docs/glyphs.md` — how text and icons are drawn from outlines (Slug)
- `docs/tree.md` — the flattened node list and its id cache
- `docs/async-and-state.md` — study: external/async state and the web port
- `docs/state-management.md` — study: state architectures across UI frameworks
- `docs/data-models.md` — list and tree models, virtualized lists, tables and trees
- `docs/theming.md` — the `Theme`, color roles and how other frameworks do it
- `TODO.md` — roadmap and current performance baseline

## Checks

```sh
cargo test --workspace
cargo clippy --workspace --all-targets
```

Golden images in `tests/golden/` are compared against PNGs from the reference
machine. On a mismatch, the actual image and a diff land in `target/golden/`.
Look at them first; rewrite the goldens with
`DFUI_BLESS=1 cargo test --test golden` only for an intended visual change.
Tests marked `#[ignore]` need a GPU: `cargo test -- --include-ignored`.

## Conventions

- Keep comments to a minimum: only a non-obvious why, never past behavior.
  Docstrings are one short paragraph.
- When the public API or its docs change, run `cargo run -p api-index`; its
  test fails while `docs/api.md` is out of date. When an app hand-rolls
  something dfui has, fix the docs where it was missed, not only `TODO.md`.
- A `ponytail:` comment marks a deliberate shortcut and its ceiling.
- Commit directly to `main`. Title up to 50 characters saying what changed;
  a short body saying why.
- New widgets get an example in `examples/` (`<name>.rs` code, `<name>.md`
  page text), added to `WIDGETS` in `site/src/main.rs` and to the widget's docs
  with `#[doc = example!("<name>")]`, and a golden when they draw something new.
