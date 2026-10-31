# Architecture

## Goals

- Declarative API: `view() -> Element` rebuilt from state owned by
  `stateful` widgets, provided values and async sources. No messages, no
  signals, no retained widget objects owned by the user.
- One rendering primitive: every pixel comes from one instanced draw whose
  fragment shader evaluates either a rounded-rect distance field or glyph
  coverage from Bézier outlines. No texture atlases, no tessellation.
- Mobile first: Android via `android-activity`, edge-to-edge with system bar
  insets, 60 fps on a mid-range Mali GPU (baseline numbers in `TODO.md`).
- Small: few dependencies (wgpu, winit, taffy, read-fonts, harfrust), little code.

## Frame loop

```
input event ──► dispatch ──► state updates ──┐
task wake ────► poll tasks ──► results ───────┴──► App::view ──► new Element tree
                                                                        │
RedrawRequested ◄── request_redraw ◄── measure (taffy) + place ◄───────┘
      │                       ▲
      │                       └── place only: redraw without a rebuild
      │                           (hover, press, scroll, caret)
      │
      ▼
 paint every node ──► Scene (flat instance list) ──► Renderer (one pass) ──► surface
```

`Runtime` (`src/runtime.rs`) is the winit `ApplicationHandler` that owns the
app, the current element tree, per-widget state, the text system and the
renderer. On Android the renderer is dropped on `suspended` and rebuilt on
`resumed`, since the native window is destroyed when backgrounded.

## Modules

| File | Role |
|------|------|
| `lib.rs` | Public API: `App`, `run`, `run_android`, re-exports. |
| `widget.rs` | `Element`, `Widget` trait, `Event`, `State`, `PaintCx`, `EventCx`. |
| `widgets/mod.rs` | Widget infrastructure: the `Styled` flex modifiers, re-exports. |
| `widgets/*.rs` | One file per built-in: `container` (`row`/`column`/`container`/`spacer`), `label` (`text`), `button`, `scroll`, `stateful`, `provide`, `builder` (`future_builder`/`stream_builder`), `text_input`, `toggle`, `slider`, `radio`, `segmented` (`segmented`/`tabs`), `overlay` (`overlay`/`popup` and the `dialog`, `drawer`, `bottom_sheet`, `snackbar`, `tooltip`, `dropdown` built on them). |
| `anim.rs` | `Animated<T>` transitions, `Lerp`, `ease` curves; driven by `PaintCx::now` and `request_frame`. `Morph` between two `Path`s (e.g. glyphs), cutting segments of the shorter contour into as many pieces as needed. |
| `path.rs` | `Path`: contours of quadratic segments, filled by `PaintCx::path` through the glyph renderer (per-frame shapes use a reserved region of the outline buffers). |
| `layout.rs` | Flattens the tree to pre-order nodes; `measure` runs taffy, `place` produces absolute rects, clips and scroll offsets. |
| `task.rs` | Widget-owned futures/streams, polling, the event-loop waker. |
| `runtime.rs` | Event loop, input → `Event` translation, dispatch, pointer capture, frame stats, `snapshot`. |
| `render.rs` | `Scene`, `Color`/`Rect`/`Shadow`, wgpu setup, the single render pass, GPU timestamp timing, offscreen rendering. |
| `shader.wgsl` | Vertex expands instances to quads; fragment does SDF shapes (kind 0) and outline glyphs (kind 1). |
| `text.rs` | `Font` handles (bundled DejaVu Sans as default and fallback), per-word shaping with harfrust, greedy word wrap, lazy glyph outline building. |
| `outline.rs` | Converts glyph contours to monotonic quadratics and per-glyph band lists for the shader. |
| `android.rs` | JNI query for system bar / cutout insets. |

## Widgets and state

- `view` rebuilds the whole `Element` tree after every state update or task
  result. Elements are cheap boxed descriptions.
- Composite widgets are plain functions returning `Element`. Implement
  `Widget` only for new layout, paint or input behavior.
- Framework-owned per-widget state (`State`, a typed `Box<dyn Any>` slot) is
  keyed by a hash of the parent id plus child slot index, or `Element::key`
  when set. State for ids missing from the new tree is dropped after each
  update. Use `key` for reorderable lists.
- UI state lives in `stateful(|s: &S, h: Handle<S>| ...)` widgets, including
  app-wide state in a root `stateful`. Handlers capture the `Copy` handle,
  never the state, and call `cx.update(h, |s: &mut S| ...)`; the runtime
  lends the state during the call, so there are no borrow problems.
- `stateful` builds its subtree lazily in `flatten` via `Widget::expand`, with
  `S` read from its state slot. Queued updates are applied after dispatch,
  then the tree is rebuilt.
- External state stays outside the framework. The app owns it and hands it
  down with `provide(value, child)`; descendants read the nearest one with
  `Scope::get::<T>()` during `expand`. `flatten` keeps the stack of provided
  values while it walks children.
- `Button::on_click(|cx| ..)` is the general handler.

## Async

- `future_builder(deps, |scope| fut, build)` and
  `stream_builder(deps, |scope| stream, build)` build their subtree from a
  `Snapshot` (`data`, `done`) that the source writes. Sources are any std
  `Future` or `futures_core::Stream`, so they can come from any runtime.
  Streams collect into `S: Default + Extend<Item>` (`Vec`, `String`, ...).
- In `expand` a builder starts its source as a task keyed by node id
  (`src/task.rs`), or keeps the running one when the hash of `deps` matches.
  New deps reset the snapshot and replace the task. Tasks are dropped with
  their node in the same pass that drops state, which cancels them.
- Tasks are polled on the UI thread, so sources need not be `Send`. One
  `Waker` posts a winit user event (deduplicated); `user_event` polls every
  task, draining ready items so a burst costs one rebuild, and rebuilds
  until no task changes (new tasks get their first poll that way). Without
  an event loop (tests, benches) the waker is a no-op and `drive` is called
  directly.

## Web

- Builds for `wasm32-unknown-unknown`; WebGPU only. `run` uses winit's
  `spawn_app`, and the window is a canvas appended to the page, sized by CSS.
- GPU setup is async on every platform: `resumed` stores `Renderer::new` as
  a future and polls it with the task waker. Native adapters resolve on the
  first poll; in the browser the waker fires when the promises settle.
- `dfui::sleep` registers deadlines on the UI thread; `about_to_wait` wakes
  due ones and sleeps the loop until the next (`ControlFlow::WaitUntil`), so
  timed async work needs no threads or runtime.
- Time is `web_time::Instant` throughout (std's clock panics on wasm32);
  natively it is std's type. The clipboard is in-app on the web.
- `site/` builds each example as its own wasm; `run.html?example=NAME`
  hosts one, embedded by its widget page in an iframe so its canvas and
  event loop don't take over the docs page.

## Layout

`flatten` produces a pre-order `Vec<Node>`, once per rebuilt tree; other
passes re-walk it from cached ids (`tree.md`). `measure` builds a taffy tree from
each widget's `style()`/`measure()` and keeps each node's parent-relative box;
`place` turns those into absolute rects, applying `scroll()` offsets and
intersecting clip rects. The runtime keeps `measure`'s result until the tree
is rebuilt or the viewport changes, so redraws without a rebuild only rerun
`place`. The taffy tree itself is kept across rebuilds, matched by node id:
only nodes whose style, children or `measure_key()` changed are marked
dirty, and taffy's cache recomputes just their path to the root. Widgets
whose `measure` depends on their own data must override `measure_key`. The root fills the
viewport, with insets added to its padding so backgrounds bleed under system
bars. Text layouts are cached in `Text` by (string, size, width) and evicted
after a frame that doesn't use them, so repeated `measure` calls and
per-frame rebuilds of the same strings are cheap.

## Input

Events are offered in reverse paint order (deepest / topmost first). A widget can
`stop()` propagation or `capture()` the pointer; on capture every other widget
receives `Event::Cancel` and subsequent pointer events go only to the captor
until pointer up. Touch is single-pointer.

`scroll` captures the pointer once a drag passes a 6px slop. On release it
flings: the offset speed (from the last 100ms of pointer samples) decays as
e^(-t/0.35s) until it drops below 10 px/s or hits an edge. The offset is
computed in `Widget::scroll` from the frame time, and the scroll view's
paint calls `request_placement` so placement reruns every frame while the
fling lasts, without a relayout.

One widget at a time holds keyboard focus, claimed with `cx.focus()` on
pointer down; a tap no widget claims blurs it. `Event::Text` and `Event::Key`
go only to the focused widget, which sees `cx.focused` in event and paint,
and `cx.shift` for extending selections. Ctrl/Cmd+A, C and X arrive as
`Key::SelectAll`, `Key::Copy` and `Key::Cut`; a widget hands text to the
clipboard with `cx.copy`. Paste arrives as `Event::Text` with the
clipboard's contents. On
Android, focus toggles the soft keyboard via `set_ime_allowed`; NativeActivity
delivers its input as key events.

## Layers

A widget whose `layer()` returns `Some` floats over the tree instead of
taking part in its flow: `measure` makes it `position: absolute` over its
parent (`Layer::Parent`, which is made `relative`) or `position: fixed` over
the window inside the system bars (`Layer::Window`), using taffy's
containing blocks. `place` gives its subtree the viewport as clip, so it
escapes scroll views, and leaves it out of the parent's scrollable content.
Paint order is pre-order, except that each layer's subtree comes after
everything it covers (sorted by layer depth, computed once per tree in
`Runtime::order`); dispatch walks that order in reverse, so layers get input
first.

`Overlay` is the built-in layer. Its content sits in a `Sink` that stops
pointer events hitting it, so any press that reaches the overlay landed
outside: that is `on_dismiss`, and `modal` also stops everything else and
paints a scrim. Tooltips and snackbars hide themselves (hover, timeout) by
scrolling their content out of the overlay's clip, so showing and hiding
needs no rebuild. Dialogs, drawers, sheets and snackbars slide the same way:
`scroll` eases an `Animated` between closed and open and offsets the content
by that fraction of the overlay's rect, and the scrim fades with it.

A removed node can't animate out, so `overlays(|ov| ..)` hosts the ones
opened from handlers. It is a `stateful` holding a builder per entry;
`ov.show(cx, build)` adds one and `Shown::close` marks it closing, which
the overlay animates out. Each entry sits in a `future_builder` whose future
sleeps until the entry's removal time (`dfui::sleep`) and then builds
nothing, so removal is an ordinary task result and rebuild: the tree's shape
still only changes in `set_root`. The deps are (key, closing), so closing
restarts the timer while the overlay below keeps its state. Removed entries
are pruned from the list on the next `show`.

## Rendering

Details and design decisions, including partial redraws: `rendering.md`.

- Painting appends instances to a `Scene`: shapes (rect, radius, border,
  colors), shadows (shapes with blur), and glyph quads. Each carries a clip
  rect evaluated in the fragment shader.
- All instances go into one vertex buffer and one draw call per frame.
- Shapes: rounded-box signed distance → antialiased fill, border and blurred
  shadow.
- Text: glyph outlines are stored in two storage buffers (curves, bands).
  The fragment shader casts horizontal and vertical rays through the band
  containing the pixel and counts antialiased winding crossings, giving
  resolution-independent text with no atlas. Curves are sorted by reach so
  the scan stops early. This requires fragment-stage storage buffers
  (Vulkan / GLES 3.1+).
- Colors are specified in sRGB and blended in sRGB (gamma) space.
- `Renderer` reports CPU time (minus vsync wait) and GPU time via timestamp
  queries, logged every two seconds.

## Testing

- Unit tests sit next to the code (`layout`, `outline`, `text`, `runtime`
  dispatch).
- `dfui::snapshot(app, size, scale)` renders one frame offscreen to an
  `RgbaImage` for golden-image tests (not automated yet).

## Known limits

Tracked in `TODO.md` and as `ponytail:` comments in the code: no IME composition on desktop,
no bidi, fallback only to the default font, single pointer, full `view` rebuild and paint per change, all
outlines re-uploaded when a glyph is added, linear hit-testing.
