# Async and external state

Study of how external, possibly asynchronous state should reach the UI, and
how that interacts with the web (wasm) port. The executor part is implemented
(`src/task.rs`); the state design moved to `state-management.md`.

## Problem

- Apps need state that changes outside widget events: network responses,
  file watchers, timers, data owned by an existing tokio/async-std program.
- Today nothing outside a widget event can trigger an update: the event loop
  is `EventLoop<()>` (`lib.rs`), `App::update` only receives messages emitted
  by widgets, and `stateful` slots only change inside event handlers.
- The web port needs the same missing piece. GPU init is async
  (`request_adapter`/`request_device`) and can't be blocked on in the browser,
  so the `Renderer` must be built in a spawned future and handed back to the
  event loop through a user event. External state needs the same channel to
  wake the UI.

## Requirements

1. External code on any runtime or thread can cause UI changes.
2. Works on web, where everything is single-threaded and mostly `!Send`.
3. dfui depends on no async runtime.
4. Keeps the Elm model: state changes go through `update`, `view(&self)`
   reads it. No signals (`architecture.md`). Relaxed in
   `state-management.md`, which compares other architectures.

## Constraints

- **winit is callback-driven.** `ApplicationHandler` methods are sync and the
  OS (or browser) calls them; the UI loop can't be an `async fn`. Async work is
  spawned from a callback and its result comes back as a user event via
  `EventLoopProxy`.
- **winit owns the main thread** (required on macOS/iOS). A host runtime like
  tokio runs on worker threads, not via `#[tokio::main]` on the main thread.
- **Window before GPU.** The window is created in `resumed` (Android
  recreates it after every suspend); the surface needs the window, the adapter
  request needs the surface. Init therefore always starts inside a callback.
- **wgpu futures are runtime-free.** On native they're ready on first poll, so
  `pollster::block_on` never waits. On web they wrap JS promises that only the
  browser event loop resolves; `wasm_bindgen_futures::spawn_local` is the
  only executor that works. An app-supplied executor buys nothing for GPU init.
- **Reactor-bound futures.** A future that needs a runtime's reactor (tokio
  IO, timers) can be polled from another executor only if that runtime is
  running (multi-thread) and was entered (`rt.enter()`) when the future was
  created. Otherwise the app spawns the work on its runtime and hands over a
  channel/oneshot receiver, which is runtime-agnostic.

## Options

| | Model | Any runtime | Web | Elm | Cost |
|---|---|---|---|---|---|
| A. Msg sender | `tx.send(Msg)` → proxy → `update` → `view` | yes | needs `Msg: Send`; JS values aren't | yes | ~20 lines |
| B. Shared store + wake | `Arc<RwLock<Model>>` (`Rc<RefCell>` on web) mutated externally, then `wake()`; `view` reads it | yes | yes, with `Rc` | no: bypasses `update`, locks in `view`, no record of what changed | small, platform-split |
| C. Tasks + subscriptions | `update` returns `Future<Output = Msg>`s, `subscriptions()` returns `Stream<Msg>`s; dfui polls them on the UI thread | yes (see reactor note) | yes, no `Send` needed | yes | ~100–150 lines |
| D. App-supplied executor | as C, but the app passes an `Executor` impl (iced) | yes | no real choice: must be `spawn_local` | yes | trait + impl per runtime |
| E. Signals | widgets subscribe to observable values (Leptos) | yes | yes | no | rewrite |

## External state and widget state

With any Elm-style option the path is the same:

```
external event ──► Msg ──► App::update ──► App::view(&self) ──► widgets
```

- External data that affects the UI is app state by definition; widgets take
  it as `view` arguments.
- Ephemeral `stateful` state that external events must change (e.g. clear a
  field after a server reset) is lifted into `App`, making the widget
  "controlled". That's a per-widget API decision, not a state-system one.
- Externally addressing `stateful` slots by `Handle` is rejected: handles are
  tree positions, which would couple external code to tree shape.
- Global state is `App`. `view(&self)` already sees all of it, so no
  context/provider mechanism is needed. An app that owns a shared store
  elsewhere (B) sends `Msg::Refresh` and reads the store in `view`.

## Recommendation: C, built on one primitive

1. **Waker.** The runtime implements a `std::task::Waker` that posts an
   `EventLoopProxy` user event. It's `Send + Sync`, std, and understood by
   every runtime. The same user-event channel delivers the async `Renderer`
   on web.
2. **Local executor.** `Runtime` keeps pending
   `Pin<Box<dyn Future<Output = Msg>>>`s (and later streams) and polls them
   when the waker fires. It never leaves the UI thread, so nothing needs
   `Send`, and web needs no special casing.
3. **Tasks from `update`**, e.g. `fn update(&mut self, msg) -> Task<Msg>` or
   `cx.spawn(fut)` to keep the current signature. Resulting messages go
   through `update` like any other.
4. **Thread-safe sender** (A) falls out as a channel-backed subscription, for
   pushing messages from tokio tasks or OS threads.

Trade-off: dfui now polls futures, so a slow `poll` stalls the UI. Heavy work
belongs on the app's runtime, with results handed back over a channel.

Usage from a tokio host:

```rust
fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let _guard = rt.enter();
    dfui::run(MyApp::default());
}

// in update:
Msg::Load => Task::future(async {
    let rx = spawn_on_tokio(fetch()); // or poll reqwest directly under rt.enter()
    Msg::Loaded(rx.await.unwrap())
}),
```

## First step

Waker + local executor + `Task` from `update`, plus a demo example that
fetches data on a timer. Subscriptions later: a task that returns a new task
covers simple polling loops. This also unblocks the web port's async GPU init.

## Web port checklist

Independent of the state design, measured with
`cargo check --target wasm32-unknown-unknown`:

- `arboard` has no web backend: move under
  `cfg(not(any(target_os = "android", target_arch = "wasm32")))`; the
  clipboard falls back to the in-memory stub.
- `std::time::Instant` → `web_time::Instant` (already in the lockfile via
  winit); std's panics on wasm32 and winit's `WaitUntil` expects web-time's.
- Async GPU init via the user-event channel above.
- `EventLoopExtWebSys::spawn_app` instead of `run_app`; window into a canvas
  with `WindowAttributesExtWebSys::with_canvas`/`with_append`.
- WebGPU only: `shader.wgsl` reads `var<storage>` buffers in the fragment
  stage, which WebGL2 lacks. Docs pages can fall back to a `snapshot()` PNG.
- Web-only deps go under `[target.'cfg(target_arch = "wasm32")'.dependencies]`
  rather than a cargo feature: the target already decides, and native builds
  never compile them.
- One winit event loop per page and one window per `Runtime`: multiple live
  examples on one docs page use one iframe each.
