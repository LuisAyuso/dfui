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
- `TODO.md` — roadmap and current performance baseline

## Running

```sh
cargo run -p dfui-demo                 # desktop
cargo apk run -p dfui-demo --release   # Android (aarch64, min SDK 26)
cargo test
```
