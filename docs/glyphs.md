# Glyphs and icons

How text and icons become pixels. There's no atlas and no bitmap: a letter and
an icon are both filled outlines, and the fragment shader computes each
pixel's coverage straight from the curves. Icons go through the glyph path
unchanged. For the rest of the pipeline, see `rendering.md`.

The technique is Eric Lengyel's **Slug algorithm**: winding numbers from rays
cast through horizontal and vertical bands of curves, antialiased along each
ray and combined by weight. dfui differs in one place, how it decides which
curves a ray crosses:

```
                   Slug                              dfui
 which roots    lookup table on the signs of      curves pre-split at their x/y
 count          p0.y, p1.y, p2.y; curves are      extrema; a curve crosses iff its
                used as they are                  endpoints straddle the ray's line
```

The split costs more curves in exchange for a simpler shader test. See
References for the paper and the patent status.

## Overview

```
 CPU, once per glyph or shape                    GPU, every frame, per pixel
 ────────────────────────────                    ───────────────────────────
 font (read_fonts) ───┐
                      ├─► quadratic Béziers ─► monotonic split ─► bands
 Shape (Path + view) ─┘   (em units, y up)                          │
                                                                    ▼
                                         storage buffers: curves[], bands[]
                                                                    │
 paint: text / icon ─► Scene::glyph(rect, em box, color) ─► one Instance (quad)
                                                                    │
                                     fragment shader: ray_coverage ×2 ─► alpha
```

Each glyph on screen is one instance: a rect plus the offset of its band
header. Curves are stored once (`Text::glyphs`, `Text::shapes`) and reused
at every size.

## Outlines (`outline.rs`)

`Builder` receives a glyph's contours from `read_fonts` and stores them as
quadratic Béziers `[p0, control, p2]` in em units (1.0 = font size):

```
 line_to   ─►  quad with its control point at the midpoint (path::line)
 quad_to   ─►  as is (TrueType)
 curve_to  ─►  cubic split into 4 quads (CFF)
```

Fill is nonzero winding. Holes are contours that run the opposite way, so an
O is two contours whose windings cancel in the middle:

```
      ╭─────────╮        outer contour: winding +1
    ╭─╯ ╭─────╮ ╰─╮      inner contour: winding −1
    │   │     │   │
    │ 1 │  0  │ 1 │      ring:  +1       → filled
    │   │     │   │      hole:  +1 −1 = 0 → empty
    ╰─╮ ╰─────╯ ╭─╯
      ╰─────────╯
```

### Monotonic split

`split_monotonic` cuts each curve at its x and y extrema, so every piece only
moves one way along each axis:

```
 before:     ╭───╮              after:     ╭─┬─╮
            ╱     ╲                       ╱  │  ╲
          p0       p2                   p0   │   p2
                                       split at the y extremum
```

A monotonic piece crosses any horizontal or vertical line at most once, so
the shader can tell whether it crosses a ray from its endpoints alone.

### Bands

Testing every curve at every pixel would be too slow. The glyph's bounding
box is cut into `n` horizontal and `n` vertical bands (`n` ≈ curves / 2,
capped at 32), and each band lists the curves that touch it:

```
 horizontal bands (+x rays)          vertical bands (+y rays)
 ┌───────────────────────┐ 3         ┌─────┬─────┬─────┬─────┐
 ├───────────────────────┤ 2         │     │     │     │     │
 ├───────────────────────┤ 1         │  0  │  1  │  2  │  3  │
 ├───────────────────────┤ 0         │     │     │     │     │
 └───────────────────────┘           └─────┴─────┴─────┴─────┘
```

Each list is sorted by how far its curves reach along the ray, furthest
first, so the shader can stop at the first curve that lies entirely behind
the pixel.

GPU layout:

```
 curves: │ (p0.x p0.y p1.x p1.y) (p2.x p2.y 0 0) │ ... │   two vec4 per curve

 bands:  ... │ [start,count] × n_h │ [start,count] × n_v │ curve indices ... │
             ▲
             header (Instance params.x)
```

## Coverage (`shader.wgsl`, `ray_coverage`)

The vertex shader expands the instance to its quad plus 1px of padding, and
each fragment gets its position in em space (`uv`).

### Winding by ray

From the pixel, cast a ray toward +x through its horizontal band. For each
curve whose endpoints straddle the ray's line, solve the quadratic for `t`,
find the crossing's x, and if it's ahead of the pixel add ±1 by the curve's
direction:

```
 pixel in the hole:          ●────────────┼──────┼────►   inner −1, outer +1 → 0
 pixel in the left stroke: ●─────┼────────┼──────┼────►   inner +1 −1, outer +1 → 1
                           │     │        │      │
                         outer inner    inner  outer
                         left  left     right  right
```

### Antialiasing

A crossing isn't counted as 0 or 1 but by where it falls inside the pixel
(a 1px box filter along the ray):

```
 ◄─────── 1 px ───────►
 │          ●    ┆     │     edge at r = +0.25 px from the centre
 │       centre  edge  │     contribution = clamp(r + 0.5, 0, 1) = 0.75
```

`fwidth(uv)` gives one screen pixel in em units, so this holds at any size or
scale.

### Two rays

A ray running parallel to an edge only sees a hard inside/outside step there.
So a second ray is cast toward +y through the vertical bands, and the two
are blended by how close each one's nearest crossing is:

```
              ▲ +y ray
              │
 ─────────────●──────────►  +x ray

 weight  = 1 − 2|r| of the nearest crossing   ("this ray sees an edge here")
 coverage = max((h·wh + v·wv) / (wh + wv), min(h, v))
```

Near a horizontal edge the vertical ray dominates, and vice versa. This is
Slug's weighting; why a plain average isn't enough is in `rendering.md`.

The result is the alpha: `color.a × coverage`, premultiplied, blended in
gamma space.

## Text (`text.rs`)

```
 "Hello" ─► harfrust shaping (kerning, ligatures, contextual forms)
         ─► positioned glyph ids, wrapped into lines
         ─► TextLayout, cached by (text, font, size, max width)
         ─► paint: per glyph, Text::glyph (cached) ─► Scene::glyph
```

`Glyph::plane` is the glyph's box relative to the pen on the baseline. Scaled
by the font size and offset by the pen position, it's the instance rect:

```
        ┌───┐   plane × size
        │ H │
 pen ●──┴───┴──────── baseline
```

Characters the font lacks fall back to `Font::DEFAULT`.

## Icons (`path.rs`, `widgets/icon.rs`)

A `Shape` is a `Path` (contours of quads, logical px, y down) plus a `view`
box, like an SVG `viewBox`. `Text::draw_shape` stores it once per shape id
in the same buffers as glyphs:

```
 view box, y down          unit box, y up            any target rect
 ┌────────────┐            ┌────────────┐            ┌──────────────────┐
 │     ★      │  ───────►  │     ★      │  ───────►  │                  │
 │            │  normalize │            │ Scene::    │        ★         │
 └────────────┘  (once)    └────────────┘ glyph      │                  │
   24 × 24                   0..1                    └──────────────────┘
                                                            48 × 48
```

- A 12px and a 48px star share the same curves; only the instance rect
  differs. The stored box is padded by 0.02 so edges that touch the view box
  keep their antialiasing.
- `icon` draws a shape in one color. `icon_button` adds padding and the
  press/spring animation, which only changes the rect.
- `PaintCx::path` is for paths that change every frame (morphs, polygons).
  They go in a fixed transient region at the front of the buffers (512
  curves, 4096 band entries) that `Outlines::begin_frame` resets.
- SVG isn't parsed; convert its paths to a `Path` with a parser such as usvg.

## Trade-offs

| | atlas | outlines |
|---|---|---|
| sharp at any size or scale | no, re-rasterize | yes |
| memory | textures per size | a few KB of curves |
| per-pixel cost | one texture fetch | loop over a band's curves, two rays |
| requires | any GPU | fragment-stage storage buffers (Vulkan / GLES 3.1+) |

The per-pixel loop is why glyphs dominate GPU time in the benchmarks, and
why partial redraws scissor to the damaged area.

## References

- Eric Lengyel, ["GPU-Centered Font Rendering Directly from Glyph Outlines"](https://jcgt.org/published/0006/02/02/),
  *Journal of Computer Graphics Techniques* 6(2), 2017. The Slug paper.
- Eric Lengyel, ["A Decade of Slug"](https://terathon.com/blog/decade-slug.html),
  2026. Retrospective with reference shader code. The Slug patent
  (US 10,373,352) was dedicated to the public domain on 2026-03-17, so the
  technique is free to use.
- Nehab and Hoppe, ["Random-Access Rendering of General Vector Graphics"](https://hhoppe.com/proj/ravg/),
  SIGGRAPH Asia 2008. Per-cell curve lists and per-pixel winding; the
  ancestor of bands.
- Loop and Blinn, ["Resolution Independent Curve Rendering using Programmable Graphics Hardware"](https://www.microsoft.com/en-us/research/publication/resolution-independent-curve-rendering-using-programmable-graphics-hardware/),
  SIGGRAPH 2005. The triangulation-based alternative.
- Will Dobbie, ["GPU text rendering with vector textures"](https://wdobbie.com/post/gpu-text-rendering-with-vector-textures/),
  2016. A readable precursor: grid cells and one antialiased ray.
- Sebastian Lague, ["Coding Adventure: Rendering Text"](https://www.youtube.com/watch?v=SO83KQuuZvg),
  2024. Visual walkthrough ending in a Slug-like ray caster.
