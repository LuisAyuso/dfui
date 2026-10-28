# Rendering

How dfui turns a `Scene` into pixels, the decisions behind it, and rendering
work in progress. For how the scene is produced, see `architecture.md`.

## Pipeline

One render pass, one pipeline, one instanced draw per frame:

- Widgets paint into a `Scene`: a flat `Vec<Instance>` in painter's order
  (logical px). Each instance is a rect + params: rounded box (radius,
  border, colors), shadow (box with blur), or glyph (band header, em-space
  quad).
- The whole list is uploaded each frame and drawn as `draw(0..6, 0..n)`. The
  vertex shader expands each instance to a quad, padded by blur + 1px so AA
  and shadows aren't cut off.
- The fragment shader branches on kind and applies the instance's clip rect
  (scroll views) by discarding.
- Output is premultiplied alpha, blended in painter's order. No depth buffer,
  no batching by material: there is only one material.

## Shapes

Signed distance to a rounded box (`sd_round_box`). Coverage is
`clamp(0.5 - d / aa)` with `aa = length(dpdx(d), dpdy(d))`, the exact
per-pixel rate of the distance. (`fwidth` overestimates it by up to √2 on
diagonals and blurs corners unevenly.) Borders mix fill and border color
on a second distance `d + border`. Shadows use `smoothstep(-blur, blur, d)`.

## Text

Glyphs are drawn directly from their outlines, with no atlas
(`outline.rs`, shader `ray_coverage`):

- Contours are converted to monotonic quadratic Béziers. Cubics (CFF) get a
  fixed 4-way split. Each glyph gets horizontal and vertical band lists of
  curve indices, sorted by how far they reach along the ray.
- Per pixel, a ray is cast along +x through the horizontal band and along
  +y through the vertical band. Each crossing adds winding with a 1px box
  filter along the ray, and the scan stops at the first curve behind the
  pixel.
- The two rays are combined with Slug-style weighting. Each ray reports how
  close its nearest crossing is (`1 - 2|r|`), and coverage is the
  weighted average, floored by `min(h, v)`. A ray running parallel to an
  edge only sees a hard inside/outside step there, so a plain 50/50 average
  left a half-amplitude stair-step on every curve. This was visible at 1×,
  and hidden by 3× density on phones.
- Requires fragment-stage storage buffers (Vulkan / GLES 3.1+).

## Color and blending

Colors are specified in sRGB and passed to the GPU unconverted. The surface
is drawn through a non-sRGB view (`remove_srgb_suffix`), so blending and AA
happen in gamma space, as in browsers and Skia. Linear-space blending put 50%
coverage about two-thirds of the way to the background color, making edges
look thin and stepped. Side effect: shadows and dark-on-light text look slightly
heavier, which matches CSS.

MSAA was considered and rejected. Every primitive computes analytic coverage
in the fragment shader, and quad edges sit in transparent padding, so MSAA
would cost about 4× fill for no visible change.

## Timing

`Renderer` logs fps, CPU time (excluding vsync wait) and GPU time every two
seconds. GPU time comes from timestamp queries around the render pass, with
one sample in flight. Baselines are in `TODO.md`.

## Partial redraws

In `render.rs` / `runtime.rs`: a scissored retained canvas, per-node damage
from a scene diff, and idle frames skipped entirely. Not yet measured on the
phone.

### Idea

GPU time is dominated by fragment shading, mostly glyph ray casting. Vertex
work is 6 vertices per instance and trivial. So keep the full instance list
and the single instanced draw, but set a scissor rect to the damaged area.
The rasterizer drops fragments outside it before shading, and painter's order
inside the damage stays correct because every instance is still submitted.

### The catch: swapchain images don't keep contents

Swapchain images rotate (2–3 deep), and with `LoadOp::Load` their contents
are undefined or several frames stale, especially on Android Vulkan. Platform
"buffer age" extensions fix this, but wgpu doesn't expose them. So:

```
canvas texture (ours, persistent, surface format)
  └─ pass: LoadOp::Load, scissor = damage, draw ALL instances
copy canvas → swapchain image (whole frame)
```

- The scene always starts with a full-viewport background instance, because
  `LoadOp::Clear` would clear the whole attachment, not just the scissor.
- The canvas is recreated on resize, and the next frame is full (`fresh`).
- If the surface doesn't support `COPY_DST`, there is no canvas and every
  frame is a full redraw straight to the swapchain.

### Damage: per-node scene diff

While painting, the runtime records which instance range each node produced
(`Spans`, keyed by node id). Each paint diffs the new scene against the last
drawn one (`render::damage`):

- Nodes are matched by id, not instance index, so inserting a card doesn't
  damage the siblings after it.
- A node whose instance bytes differ, or that was added or removed,
  contributes its old and/or new bounds. Bounds are the quad plus padding
  (shadow blur + 1px, or 1px AA for glyphs), clipped to the instance's clip
  rect.
- The union, plus a 2px margin rounded out to physical pixels, becomes the
  scissor. An empty diff skips the frame entirely (see below).
- Full redraws happen only when the canvas is fresh (first frame, resume,
  resize). Events, rebuilds and relayouts all go through the diff, so a
  hover damages one button and typing damages one field.
- `prev` only advances when a frame is actually drawn, so a skipped frame
  (surface outdated or lost) can't leave the canvas missing changes.

Because the diff is on painted output, widgets need no invalidation API, and
`request_frame` only schedules another paint. An animation that paints the
same pixels again costs nothing on the GPU: the Bench FPS meter repaints
every frame but only damages when its text changes, twice a second.

Known gap: a pure z-order swap of overlapping nodes with byte-identical
instances isn't detected (marked `ponytail:`).

### Idle frames

When the diff is empty, the window already shows the canvas, so `render`
returns `Outcome::Idle` without acquiring, copying or presenting. That
removes the copy, the floor cost of every frame, whenever nothing visibly
changed.

Idle frames don't block on vsync, so a `request_frame` loop would spin. When
a requested frame turns out idle, the runtime paces the next one with
`ControlFlow::WaitUntil` at the monitor's refresh interval (60 Hz fallback).
Presented frames still pace themselves on vsync. While the window is
occluded, idle animation frames stop entirely and resume when it's visible.

Exceptions: a fresh canvas always draws, and with the damage overlay on, the
frame after a tinted one presents so the tint clears.

On the Bench screen (desktop) the FPS meter keeps painting at 61 fps while
the window presents about once a second, only when the readout changes.

### Seeing it

Run with `DFUI_SHOW_DAMAGE=1` to tint each frame's redrawn area with a
cycling color. The tint is drawn onto the swapchain image after the canvas
copy, so it never accumulates in the canvas; a tinted frame is followed by
one clean empty-damage frame, so tints flash rather than linger. On the Bench
screen only the FPS readout flashes, when its text changes. On the main screen, hover, taps
and typing flash only the widget involved. Stale pixels show up as content that changes without being
tinted.

### Results

Bench screen, desktop (2560×1440 display, default window size), release,
steady state. Only the FPS meter animates:

| | render pass GPU | CPU |
|---|---|---|
| full redraw | 0.43 ms | ~0.33 ms |
| scissored | 0.06 ms | ~0.32 ms |

Not yet measured:
- **The canvas → swapchain copy.** The timer only covers the render pass.
  It's a full-frame bandwidth cost each frame.
- **Phone (Mali).** The baseline is about 5.1 ms GPU on the bench. With
  `LoadOp::Load`, a tiler reads every tile back from memory, which adds
  bandwidth that a full clear avoids. Measure GPU time and, ideally, power.

### Open questions / next steps

- **Caret blink** is now cheap on the GPU (damage = the caret), but it
  keeps the CPU rebuilding the scene every frame while a field is focused.
- **One rect vs several.** A union of distant rects (corner button + caret)
  can be large. Several scissored draws of the same buffer are cheap on the
  vertex side; decide by measurement.
- **Skip the copy.** `VK_KHR_incremental_present` or buffer age would let us
  draw straight into the swapchain; that needs wgpu support. Until then the
  copy is the floor cost of any frame.
- **CPU side** (rebuild, layout, paint ~2.2 ms on phone) is untouched by
  this. It's the other half of the partial-redraw study in `TODO.md`.
