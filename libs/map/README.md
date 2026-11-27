# dfui-map

[![crates.io](https://img.shields.io/crates/v/dfui-map.svg)](https://crates.io/crates/dfui-map)
[![docs.rs](https://img.shields.io/docsrs/dfui-map)](https://docs.rs/dfui-map)
[![tests](https://github.com/LuisAyuso/dfui/actions/workflows/test.yml/badge.svg)](https://github.com/LuisAyuso/dfui/actions/workflows/test.yml)
[![lint](https://github.com/LuisAyuso/dfui/actions/workflows/lint.yml/badge.svg)](https://github.com/LuisAyuso/dfui/actions/workflows/lint.yml)
[![examples](https://img.shields.io/badge/examples-live-blue)](https://luisayuso.github.io/dfui/)
[![license](https://img.shields.io/crates/l/dfui-map.svg)](https://github.com/LuisAyuso/dfui/blob/main/LICENSE)

A pannable, zoomable vector map widget built only on [dfui](https://github.com/LuisAyuso/dfui)'s public API. dfui
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
- Lines are kept by their middle and stroked when their tile is first
  drawn: one rectangle per segment plus a bevel triangle on the outside of
  every bend, with round caps (half-disks) so ways that meet at any angle
  close up. They are all wound the same way, so overlaps merge.
- **Levels of detail.** Each layer is prepared for zoom bands that start at
  1/32, 1/16, 1/8, 1/4, 1/2 and 1 px/m, plus full detail. A level drops
  points that move a line by under a quarter pixel at its largest scale
  (Douglas–Peucker), leaves out features under 0.75 px and draws lines
  under 1.5 px without caps or bevels. A level that doesn't halve the
  segments of the finer one isn't kept; that one draws in its place.
- Each level is split into quadtree cells until a cell holds at most 128
  segments and spans at most 256 px at the level's largest scale (64 m at
  full detail): a tile costs every pixel it covers, so a sparse layer, a
  single bus line, mustn't spread over a large cell. Rings are cut to the
  cell plus a 2 m margin (Sutherland–Hodgman, exact for straight edges under
  nonzero fill); lines are cut further out, by half their width, and their
  stroke is cut to the margin, which is the same as cutting the stroke of
  the whole line. The tile is drawn clipped to the cell, so the cut edges
  sit outside the clip and adjacent tiles meet without antialiasing seams.
- Street names are geometry too: `Text::glyph_paths` gives the shaped
  glyph outlines, and each glyph is turned to the street under its middle
  and stored in metres, one shape per label. They scale with the map and
  show once the font reaches 7 px. The same name repeats only 300 px apart
  on screen: each repeat waits for the zoom where it's that far from the
  ones placed before it, so names of wide roads go first.
- **Streaming.** Paint draws the level for the current scale. A tile is
  decoded, stroked and stored on the GPU the first time it's drawn, up to
  4 ms of that a frame; dfui then uploads only the new outlines. Where a
  tile has to wait, the finest coarser level that's stored draws in its
  place, clipped to its cell, and the map asks for another frame. Tiles are
  sorted by their left edge, so paint looks at the strip of them under the
  view rather than at all of them.
- `MapData::to_bytes` saves a built map so an app can bundle it instead of
  building at startup: a table of layers, an index of tiles, then each
  tile's geometry with points on a millimetre grid as varint deltas: areas
  as rings, lines by their middle, labels as glyph outlines.
  `MapData::from_bytes` reads only the index.

## Numbers

Central Amsterdam, 2×2 km of OSM data (15.8 MB of Overpass JSON). Eight layers,
17726 shapes across all levels, 1.03M segments. Prepared in ~150 ms on the desktop. A phone-sized
target (1080×2400 at scale 2.625) on a Ryzen 5 3600 and RTX 4060 Ti, using
`--bench`. CPU is one paint of the scene once its tiles are stored; GPU is
the median render pass from timestamps.

| View | CPU paint | GPU frame |
|---|---|---|
| Whole map (~0.15 px/m, buildings and paths hidden) | 183 µs | 0.27 ms |
| 0.3 px/m (buildings and paths hidden) | 228 µs | 0.53 ms |
| 0.5 px/m (dense: every building) | 271 µs | 0.94 ms |
| 2 px/m | 33 µs | 0.72 ms |
| 10 px/m | 6 µs | 0.43 ms |
| 50 px/m | 3.5 µs | 0.05 ms |

On the phone, the Innsbruck map of the bus app (10×7.5 km, 24 layers: one
per bus line), on a Pixel 10a (Mali-G715) at its 1080×2400:

| View | GPU frame, one tile per layer per 128 segments | Now |
|---|---|---|
| Whole city (0.04 px/m) | 10.3 ms | 8.3 ms |
| 0.2 px/m | 26 ms | 11.8 ms |
| 0.7 px/m | 40 ms | 10.3 ms |
| 1.5 px/m | 49 ms | 10.3 ms |
| 5 px/m | 50 ms | 8.3 ms |

Most of that was the tile size: a bus line's tiles were up to 3 km across
and each covered the whole screen. Frames that store new tiles take 4–9 ms
of CPU on the phone while they stream in. Its saved map is 19.7 MB (13 MB
gzipped), of which labels are 4.5 MB and the index 5.2 MB; it was 23 MB
with roads stored stroked and no levels. Rendered side by side, views from
0.7 px/m up match the stroked map's pixels (no channel off by more than
8/255); coarser ones differ on ~1% of pixels, mostly in the shape of small
circles.

How the GPU time came down, Amsterdam at 0.5 px/m, before levels:

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

## Limitations

- **Cost per pixel, not per feature.** Every covered pixel evaluates the
  curves of each layer's tile under it; that loop is ~90% of the GPU time
  (see `DFUI_SHOW_COST` in [rendering.md](https://github.com/LuisAyuso/dfui/blob/main/docs/rendering.md)). Partial repaint doesn't help, since
  a pan changes every pixel of the map.
- **Pop-in.** A layer with no coarser level (its levels didn't simplify
  enough to keep) has nothing to stand in while its tiles stream, so it
  appears a frame or two late; buildings do this when zooming in.
- **GPU memory grows with what was seen.** 32 bytes a segment plus band
  lists, resident while drawn; shapes are freed 600 frames after they were
  last drawn. Decoded tiles stay in memory on the CPU side.
- **Labels are large.** They're stored as glyph outlines, a quarter of the
  saved map. Storing the text and laying it out on first draw would shrink
  them.
- **Projection is for cities.** Equirectangular `f32` metres around one
  origin: fine for tens of km, wrong for countries or the whole world.
  The levels of detail and tile sizes are tuned in metres too: the coarsest
  level starts at 1/32 px/m, about 10 km across a phone. For a region or a
  country (borders and a few cities), pass kilometres instead of metres to
  `MapBuilder` and everywhere else a map takes units, as the meteo app does
  for Spain and Austria.
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

## Web and Android

The [web](https://luisayuso.github.io/dfui/) and Android builds of the
example embed the OSM data at build time; see [web.md](https://github.com/LuisAyuso/dfui/blob/main/docs/web.md) and
[android.md](https://github.com/LuisAyuso/dfui/blob/main/docs/android.md).

Released with dfui under the same version.

Map data © OpenStreetMap contributors, under the ODbL.
