# dfui-map (experiment)

A pannable, zoomable vector map widget built only on dfui's public API. dfui
itself doesn't depend on it; an app opts in by depending on `dfui-map`.

The app downloads and parses the data (the example uses OpenStreetMap through
the Overpass API) and hands projected rings and polylines to a `MapBuilder`.
The builder strokes lines into polygons, cuts everything into quadtree tiles
and stores each tile as a dfui `Shape`. A `Shape` lives on the GPU once and is
drawn into any rect, so panning and zooming only move the rects: no geometry
is rebuilt or uploaded per frame.

```sh
cargo run -p dfui-map --release --example osm                    # central Amsterdam
cargo run -p dfui-map --release --example osm -- 48.137 11.575 3 # lat lon km
cargo run -p dfui-map --release --example osm -- --bench
```

Drag to pan; wheel, ctrl+wheel, touchpad pinch or two-finger pinch to zoom.
Overpass instances are often at capacity; the example tries a few with
backoff and caches the response in the temp directory.

## How it draws

- Projection: equirectangular metres around the builder's origin, `f32`.
- Areas keep their holes by winding: outlines are stored positive, holes
  negative, and the shader fills by nonzero winding.
- Lines become one rectangle per segment plus a bevel triangle on the
  outside of every bend, with round caps (half-disks) so ways that meet at
  any angle close up. They are all wound the same way, so overlaps merge.
- Each layer is split into quadtree cells until a cell holds at most 128
  segments. Rings are cut to the cell plus a 2 m margin (Sutherland–Hodgman,
  exact for straight edges under nonzero fill). The tile is drawn clipped to
  the cell, so the cut edges sit outside the clip and adjacent tiles meet
  without antialiasing seams.
- Street names are geometry too: `Text::glyph_paths` gives the shaped
  glyph outlines, and each glyph is turned to the street under its middle
  and stored in metres, one shape per label. They scale with the map and
  show once the font reaches 7 px. The same name repeats only 300 px apart
  on screen: each repeat waits for the zoom where it's that far from the
  ones placed before it, so names of wide roads go first.
- `MapData::to_bytes` saves a built map so an app can bundle it instead of
  building at startup: an index of tiles, then each tile's outline with
  points on a millimetre grid as varint deltas, straight edges without a
  control point. `MapData::from_bytes` reads only the index; each tile is
  decoded the first time it's drawn. Innsbruck (4×4 km, 1.4M edges) is
  8.9 MB (6.8 MB gzipped) against 6.7 MB of stripped Overpass JSON (1 MB
  gzipped), and loads in ~5 ms instead of ~530 ms of building.
- Paint walks every tile, skips the ones off screen or below their layer's
  `min_scale`, and draws the rest with `PaintCx::shape`.

## Numbers

Central Amsterdam, 2×2 km of OSM data (15.8 MB of Overpass JSON). Eight layers,
10293 shapes (940 of them labels), 847k segments. Prepared in ~200 ms on the desktop. A phone-sized
target (1080×2400 at scale 2.625) on a Ryzen 5 3600 and RTX 4060 Ti, using
`--bench`. CPU is one paint of the scene; GPU is the median render pass from
timestamps.

| View | CPU paint | GPU frame |
|---|---|---|
| Whole map (~0.15 px/m, buildings and paths hidden) | 204 µs | 0.40 ms |
| 0.3 px/m (buildings and paths hidden) | 152 µs | 0.64 ms |
| 0.5 px/m (dense: every building) | 224 µs | 1.15 ms |
| 2 px/m | 43 µs | 0.74 ms |
| 10 px/m | 26 µs | 0.47 ms |
| 50 px/m | 23 µs | 0.05 ms |

For scale, dfui's `gpu/text_wall` is ~0.12 ms on the same machine. How the GPU
time came down, at 0.5 px/m:

| Tiling | Segments | GPU frame |
|---|---|---|
| Fixed 256 m grid, 256 segments per shape, unclipped | 259k | 4.71 ms |
| Quadtree, clipped, 256 per tile, 6 m margin | 764k | 2.12 ms |
| Quadtree, 128 per tile, 2 m margin | 551k | 1.42 ms |
| Same, plus quads clamped to their clip in dfui's vertex shader | 551k | 1.17 ms |

Unclipped tiles overlapped: a pixel in a dense cell was covered by around ten
shapes, each walking its band. A view of empty land at 50 px/m still cost
2.5 ms. Smaller tiles cost CPU (more tiles to walk) and duplicate more
geometry into the margins, so 128 segments per tile is the balance.

Not measured on the phone yet. dfui's other GPU benches run 22–32× slower on
the Mali-G715 than on the desktop. If the map scales the same way, that is
6–37 ms a frame, so the dense views would miss 60 fps.

## Limitations

- **Storing shapes is slow.** The first frame that draws a tile builds its
  band lists; the whole map at once costs 4.4 s on the phone.
- **New tiles re-upload everything.** dfui uploads the whole outline store
  whenever any shape is added (a `ponytail:` in `render/gpu.rs`). Opening
  zoomed in and panning stores tiles as they appear, and each such frame
  copies the store again (up to ~17 MB of curves here, plus band lists).
  Expect hitches while panning into new areas. Fix in dfui: write only the
  ranges that changed.
- **Cost per pixel, not per feature.** Every covered pixel evaluates the
  curves of each layer's tile under it; that loop is ~90% of the GPU time
  (see `DFUI_SHOW_COST` in `docs/rendering.md`). Partial repaint doesn't help, since
  a pan changes every pixel of the map.
- **No levels of detail.** Only per-layer `min_scale`. Zoomed out over a
  bigger area, curves per pixel grow without bound. Needs simplified
  geometry per zoom level, or rasterized tiles for far zooms.
- **GPU memory grows with area.** 32 bytes a segment plus band lists, all
  resident while drawn (shapes are freed 600 frames after they were last
  drawn). There is no streaming of data in or out.
- **Paint walks every tile** each frame (10.3k here, up to ~0.2 ms). A
  quadtree walk would make it proportional to what's on screen.
- **Projection is for cities.** Equirectangular `f32` metres around one
  origin: fine for tens of km, wrong for countries or the whole world.
- **Cartography is minimal.** No casings, dashes or one-way arrows. Bevel
  joins only; caps are polygons. OSM relations keep only member ways that
  close by themselves.
- **Labels are per OSM way.** A name longer than its way (one block, often)
  isn't shown; joining a street's ways would fit more. Different names can
  overlap at junctions, and dual carriageways label each side off the
  road's middle. Areas (squares, canals) have no names yet.
- **Seams** are exact while the 2 m margin is at least half a pixel
  (≥ 0.25 px/m). Below that, faint lines could show in large areas; none
  were visible at the whole-map zoom.
- **Gestures.** No fling after a pan. A two-finger touch held still and
  lifted taps whatever is under it unless a widget captured the pointer.

## Web demo

The wasm build embeds a stripped copy of the Overpass JSON (tags and
geometry only, 5.7 MB) through `DFUI_MAP_OSM`. The wasm-bindgen CLI must
match the version in `Cargo.lock`:

```sh
DFUI_MAP_OSM=/path/to/amsterdam.json cargo build -p dfui-map --example osm \
    --profile web --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir out \
    target/wasm32-unknown-unknown/web/examples/osm.wasm
```

Then load `out/osm.js` from a page as `site/dist/run.html` does.

## Android

`osm_android` is the same example as an APK, with the data embedded the same
way (set up `cargo-apk` as in the top-level README):

```sh
DFUI_MAP_OSM=/path/to/amsterdam.json cargo apk run -p dfui-map \
    --example osm_android --release
```

Preparing the data takes ~190 ms on the Pixel. The first frame takes 4.4 s
(0.65 s on the desktop): it stores every tile's shape, since the whole map is
in view.

## Releasing

Published to crates.io on its own: bump `version` in `map/Cargo.toml`,
then push a `map-v<version>` tag (`.github/workflows/map-release.yml`). It
depends on the published dfui, so release dfui first when it needs a new one.

Map data © OpenStreetMap contributors, under the ODbL.
