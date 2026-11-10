# Image

`image(bitmap)` draws decoded pixels, stretched over its box. Sized on one
axis it keeps its aspect ratio; `radius` rounds it and `tint` multiplies
every pixel.

dfui doesn't decode files: build a `Bitmap` from RGBA8 with a crate such as
`image`, on any thread. Keep it and clone it, since each new `Bitmap` is a
new texture.
