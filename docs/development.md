# Development

Running the demo, the tests and the benchmarks from a checkout.

```sh
cargo run -p dfui-demo --release   # desktop
cargo test                         # GPU tests: cargo test -- --include-ignored
cargo test --test patterns         # usual app patterns, driven by input events
cargo bench --bench cpu            # criterion CPU frame benches
cargo bench --bench gpu            # criterion GPU benches (timestamp-measured)
scripts/android-bench.sh gpu       # same benches on the connected phone (or cpu)
```

`scripts/android-bench.sh` needs the Android setup in [android.md](android.md).

## Golden images

`tests/golden.rs` renders widgets offscreen on a virtual clock and compares
them with the PNGs in `tests/golden/`, which come from the reference machine
(CI leaves them out for that reason). After an intended change:

```sh
DFUI_BLESS=1 cargo test --test golden
```

## Debugging switches

- `DFUI_SHOW_DAMAGE=1` tints the area redrawn each frame.
- `DFUI_SHOW_COST=1` shows what each pixel costs: red counts the curves the
  glyph shader tested, green the instances drawn there (see
  [rendering.md](rendering.md)).
- `RUST_LOG=dfui=info` logs frame stats (`fps … cpu … gpu …`) every 2 s.

## Releasing

dfui, dfui-map, dfui-media, dfui-bevy and dfui-plot share one version
(`[workspace.package]` in `Cargo.toml`, and the `dfui` line under
`[workspace.dependencies]`). A `v*` tag that matches it runs the tests and
publishes them all
(`.github/workflows/release.yml`).
