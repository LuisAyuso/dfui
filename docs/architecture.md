# Architecture

## Goals

- Elm-style API: app state + `update(Msg)` + `view() -> Element`. No signals,
  no retained widget objects owned by the user.
- One rendering primitive: every pixel comes from one instanced draw whose
  fragment shader evaluates either a rounded-rect distance field or glyph
  coverage from Bézier outlines. No texture atlases, no tessellation.
- Mobile first: Android via `android-activity`, edge-to-edge with system bar
  insets, 60 fps on a mid-range Mali GPU (baseline numbers in `TODO.md`).
- Small: few dependencies (wgpu, winit, taffy, ttf-parser), little code.

## Frame loop

```
input event ──► dispatch ──► Msg* ──► App::update ──► App::view ──► new Element tree
                                                                        │
RedrawRequested ◄── request_redraw ◄── relayout (taffy) ◄───────────────┘
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
| `widgets/*.rs` | One file per built-in: `container` (`row`/`column`/`container`/`spacer`), `label` (`text`), `button`, `scroll`. |
| `layout.rs` | Flattens the tree to pre-order nodes, runs taffy, produces absolute rects, clips and scroll offsets. |
| `runtime.rs` | Event loop, input → `Event` translation, dispatch, pointer capture, frame stats, `snapshot`. |
| `render.rs` | `Scene`, `Color`/`Rect`/`Shadow`, wgpu setup, the single render pass, GPU timestamp timing, offscreen rendering. |
| `shader.wgsl` | Vertex expands instances to quads; fragment does SDF shapes (kind 0) and outline glyphs (kind 1). |
| `text.rs` | Font loading (bundled DejaVu Sans), kerning, greedy word wrap, lazy glyph outline building. |
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

`flatten` produces a pre-order `Vec<Node>`; `layout` builds a taffy tree from
each widget's `style()`/`measure()`, then walks it to place nodes absolutely,
applying `scroll()` offsets and intersecting clip rects. The root fills the
viewport, with insets added to its padding so backgrounds bleed under system
bars. Full relayout happens on every change (see `ponytail:` note).

## Input

Events are offered in reverse pre-order (deepest / topmost first). A widget can
`stop()` propagation or `capture()` the pointer; on capture every other widget
receives `Event::Cancel` and subsequent pointer events go only to the captor
until pointer up. Touch is single-pointer.

## Rendering

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
- Colors are specified in sRGB and converted to linear for blending.
- `Renderer` reports CPU time (minus vsync wait) and GPU time via timestamp
  queries, logged every two seconds.

## Testing

- Unit tests sit next to the code (`layout`, `outline`, `text`, `runtime`
  dispatch).
- `dfui::snapshot(app, size, scale)` renders one frame offscreen to an
  `RgbaImage` for golden-image tests (not automated yet).

## Known limits

Tracked in `TODO.md` and as `ponytail:` comments in the code: no text input,
no shaping/bidi, single font, single pointer, full relayout per frame, all
outlines re-uploaded when a glyph is added, linear hit-testing.
