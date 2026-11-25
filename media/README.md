# dfui-media

Experimental hardware-decoded video for [dfui](https://github.com/LuisAyuso/dfui).
It plays fragmented MP4 (H.264 or HEVC, 8- or 10-bit, with AAC sound) and
draws the frames as dfui bitmaps: VA-API through libavcodec on Linux, with
zero-copy frames when the GPU imports dmabufs, and MediaCodec on Android
(H.264 only for now). HDR10 and HLG films go out as HDR when the app opts
into it (`dfui::App::hdr`) and the screen takes HDR10, and are tone-mapped
(ITU-R BT.2390) otherwise.

It was built for the streams the
[movies-thing](https://github.com/LuisAyuso/movies-thing) server sends, and
plays exactly that shape: one fragmented MP4 over any `Read`, seeking by
opening a new stream. See the
[design](https://github.com/LuisAyuso/dfui/blob/main/docs/media.md) and the
[HDR notes](https://github.com/LuisAyuso/dfui/blob/main/docs/hdr.md).

```rust,ignore
let player = dfui_media::Player::new(http_response_or_file, 0.0);
// In the view:
dfui_media::video(&player)
```

`dfui_media::codecs()` tells a server what to send as is (`h264,aac`, plus
`hevc` where the hardware decodes 10-bit HEVC).

## Building

Linux needs ALSA's and FFmpeg's development files and libclang, e.g. on
Debian or Ubuntu:

```sh
sudo apt install libasound2-dev libavcodec-dev libavutil-dev libclang-dev
```

Examples, from the dfui repository:

```sh
cargo run -p dfui-media --example play -- movie.mp4   # Space pauses
DFUI_MEDIA_CLIP=movie.mp4 cargo apk run -p dfui-media --example play_android
```

Files must be fragmented, as ffmpeg writes with
`-movflags frag_keyframe+empty_moov+default_base_moof`.

## License

MIT. Two dependencies have other terms: AAC is decoded by
[symphonia](https://github.com/pdeljanov/Symphonia), under MPL-2.0, whose
copyleft covers its own files only; and on Linux the crate links the
system's FFmpeg (LGPL, or GPL depending on how it was built).
