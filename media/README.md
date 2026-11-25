# dfui-media

Experimental hardware-decoded video for [dfui](https://github.com/LuisAyuso/dfui):
plays fragmented MP4 (H.264 + AAC) and draws the frames as dfui bitmaps, with
VA-API through libavcodec on Linux and MediaCodec on Android. Built for the
streams the movies-thing server sends; see `docs/media.md` for the design.

```rust
let player = dfui_media::Player::new(http_response_or_file, 0.0);
// In the view:
dfui_media::video(&player)
```

Linux builds need FFmpeg's development headers and libclang.

```sh
cargo run -p dfui-media --example play -- movie.mp4   # Space pauses
DFUI_MEDIA_CLIP=movie.mp4 cargo apk run -p dfui-media --example play_android
```

The file must be fragmented, as ffmpeg writes with
`-movflags frag_keyframe+empty_moov+default_base_moof`.
