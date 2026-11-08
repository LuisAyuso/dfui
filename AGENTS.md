# Working on dfui

Start with `README.md` for the layout and commands, `docs/architecture.md` for
how it fits together, and `TODO.md` for the roadmap. Tick off or add items in
`TODO.md` as part of a change.

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
- New widgets get an example in `site/examples/`, added to `WIDGETS` in
  `site/src/main.rs`, and a golden when they draw something new.
