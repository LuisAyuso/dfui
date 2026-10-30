# Architecture

## Goals

- Elm-style API: app state + `update(Msg)` + `view() -> Element`. No signals,
  no retained widget objects owned by the user.
- One rendering primitive: every pixel comes from one instanced draw whose
  fragment shader evaluates either a rounded-rect distance field or glyph
  coverage from Bézier outlines. No texture atlases, no tessellation.
- Mobile first: Android via `android-activity`, edge-to-edge with system bar
  insets, 60 fps on a mid-range Mali GPU (baseline numbers in `TODO.md`).
- Small: few dependencies (wgpu, winit, taffy, read-fonts, harfrust), little code.

## Frame loop

```
input event ──► dispatch ──► Msg* ──► App::update ──► App::view ──► new Element tree
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
| `widgets/*.rs` | One file per built-in: `container` (`row`/`column`/`container`/`spacer`), `label` (`text`), `button`, `scroll`, `stateful`, `text_input`. |
| `anim.rs` | `Animated<T>` transitions, `Lerp`, `ease` curves; driven by `PaintCx::now` and `request_frame`. `Morph` between two `Path`s (e.g. glyphs), cutting segments of the shorter contour into as many pieces as needed. |
| `path.rs` | `Path`: contours of quadratic segments, filled by `PaintCx::path` through the glyph renderer (per-frame shapes use a reserved region of the outline buffers). |
| `layout.rs` | Flattens the tree to pre-order nodes; `measure` runs taffy, `place` produces absolute rects, clips and scroll offsets. |
| `runtime.rs` | Event loop, input → `Event` translation, dispatch, pointer capture, frame stats, `snapshot`. |
| `render.rs` | `Scene`, `Color`/`Rect`/`Shadow`, wgpu setup, the single render pass, GPU timestamp timing, offscreen rendering. |
| `shader.wgsl` | Vertex expands instances to quads; fragment does SDF shapes (kind 0) and outline glyphs (kind 1). |
| `text.rs` | `Font` handles (bundled DejaVu Sans as default and fallback), per-word shaping with harfrust, greedy word wrap, lazy glyph outline building. |
| `outline.rs` | Converts glyph contours to monotonic quadratics and per-glyph band lists for the shader. |
| `android.rs` | JNI query for system bar / cutout insets. |

## Widgets and state

- `view` rebuilds the whole `Element` tree every time a message is handled.
  Elements are cheap boxed descriptions.
- Composite widgets are plain functions returning `Element`. Implement
  `Widget` only for new layout, paint or input behavior.
- Framework-owned per-widget state (`State`, a typed `Box<dyn Any>` slot) is
  keyed by a hash of the parent id plus child slot index, or `Element::key`
  when set. State for ids missing from the new tree is dropped after each
  update. Use `key` for reorderable lists.
- Two kinds of state: app/domain state lives in `App` and changes through
  `Msg`; ephemeral UI state (expanded, tab index, field contents) lives in a
  `stateful(|s: &S, h: Handle<S>| ...)` widget. Handlers capture the `Copy`
  handle, never the state, and call `cx.update(h, |s: &mut S| ...)`; the
  runtime lends the state during the call, so there are no borrow problems.
- `stateful` builds its subtree lazily in `flatten` via `Widget::expand`, with
  `S` read from its state slot. Queued updates are applied after dispatch,
  before messages, then the tree is rebuilt.
- `Button::on_click(|cx| ..)` is the general handler; `on_press(msg)` is sugar
  for emitting a message.

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

Events are offered in reverse pre-order (deepest / topmost first). A widget can
`stop()` propagation or `capture()` the pointer; on capture every other widget
receives `Event::Cancel` and subsequent pointer events go only to the captor
until pointer up. Touch is single-pointer.

One widget at a time holds keyboard focus, claimed with `cx.focus()` on
pointer down; a tap no widget claims blurs it. `Event::Text` and `Event::Key`
go only to the focused widget, which sees `cx.focused` in event and paint. On
Android, focus toggles the soft keyboard via `set_ime_allowed`; NativeActivity
delivers its input as key events.

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
