# Working on dfui

Start with `README.md` for the commands, `docs/architecture.md` for how it fits
together, and `TODO.md` for the roadmap. Tick off or add items in `TODO.md` as
part of a change.

## Layout

- `src/` — the framework crate (`dfui`)
- `demo/` — demo app, desktop binary and Android `cdylib`
- `site/` — docs site published to GitHub Pages: guide pages and a live example per widget
- `tools/svg2path/` — converts SVG icons to shapes an app can bundle (see Icons in `README.md`)
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
- A `ponytail:` comment marks a deliberate shortcut and its ceiling.
- Commit directly to `main`. Title up to 50 characters saying what changed;
  a short body saying why.
- New widgets get an example in `site/examples/` (`<name>.rs` code, `<name>.md`
  page text), added to `WIDGETS` in `site/src/main.rs` and to the widget's docs
  with `#[doc = example!("<name>")]`, and a golden when they draw something new.
