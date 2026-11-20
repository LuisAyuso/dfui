# Icon

A `Shape` is a `Path` stored on the GPU once, like a glyph, then drawn
into any rect: the same icon is sharp at every size and costs nothing to
scale. Its `view` box maps onto the drawn rect, like an SVG `viewBox`.

`icon` draws a shape in one color. `icon_button` adds `on_click` and a
touch-sized padding; the icon shrinks while pressed and springs back
past its size on release. dfui doesn't parse SVG files: `tools/svg2path`
converts them (strokes included) for `Shape::from_bytes`.
