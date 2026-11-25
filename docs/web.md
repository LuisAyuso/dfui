# Web

Note: this framework is not intended for web development, but I like the idea of providing a page with functional examples of the snippets, therefore the web build exist.

The framework builds for `wasm32-unknown-unknown` and draws with WebGPU
(WebGL2 lacks the fragment-stage storage buffers the text shader reads).

## The docs site

The [docs site](https://luisayuso.github.io/dfui/) in `site/` is static HTML,
deployed to GitHub Pages by `.github/workflows/pages.yml` on every push to
`main`:

- `site/pages/*.md`: hand-written pages (introduction, getting started,
  concepts); `docs/architecture.md` is published as is.
- `examples/<name>.rs` (linked as `site/examples`): one standalone program per
  widget, `cargo run --example <name>` on desktop, built to its
  own wasm and shown below the live frame, under the text of `<name>.md`. The
  widget's API docs include the same file (`#[doc = example!("<name>")]`), so
  `cargo test --doc` compiles it. New examples are added to `WIDGETS` in
  `site/src/main.rs`.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129   # must match Cargo.lock
scripts/site.sh                                    # writes site/dist
python3 -m http.server -d site/dist                # open http://localhost:8000
cargo run -p dfui-site --example slider           # one example, natively
```

## The map demo

dfui-map's `osm` example builds for the web with a stripped copy of the
Overpass JSON (tags and geometry only, 5.7 MB) embedded through
`DFUI_MAP_OSM`. The wasm-bindgen CLI must match the version in `Cargo.lock`:

```sh
DFUI_MAP_OSM=/path/to/amsterdam.json cargo build -p dfui-map --example osm \
    --profile web --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir out \
    target/wasm32-unknown-unknown/web/examples/osm.wasm
```

Then load `out/osm.js` from a page as `site/dist/run.html` does.
