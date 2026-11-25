# dfui-media

A workspace member beside `map`: video decoded in hardware and drawn by dfui
as a bitmap, with dfui controls over it. VA-API on Linux, MediaCodec on
Android. It replaces the [media-player study](media-player.md)'s libmpv
route: the first user is movies-thing, and since its server picks the codec
and the container, the player only has to play one narrow format well.

## The stream contract

The movies-thing server sends fragmented MP4 from ffmpeg
(`server/src/stream.rs`). The player asks for it with `codecs=h264,aac`, and
the server copies a track only when its codec is on that list, re-encoding
the rest:

| Part | Format |
| --- | --- |
| Container | Fragmented MP4 (`moov` then `moof`/`mdat` pairs), streamed over HTTP |
| Video | H.264, 8-bit 4:2:0; copied when the source already is, else transcoded. 10-bit H.264 is never copied |
| Audio | AAC at the source's rate and channels when copied; stereo when the server mixes or re-encodes |
| Image subtitles | Burned in by the server (`sub=`) |
| Text subtitles | WebVTT from `/subs/{id}/{key}`, drawn by the app |
| Seeking | A new stream with `start=`; the `X-Start` header says where a copy really starts (the keyframe before) |

Later codecs, in order: 8-bit HEVC (MIME and `hvcC` on Android, libavcodec
already decodes it on Linux), then 10-bit HEVC and AV1, which need P010
frames. HDR stays tone-mapped by the server until the convert pass learns
BT.2020. The player will ask its decoder what it supports and send that list.

## Pipeline

```
Read ──► fMP4 demux ──┬─► H.264 samples ──► decoder ──► NV12 frame ──► convert pass ──► RGBA texture ──► Bitmap
                      └─► AAC frames ──► symphonia ──► cpal ──► audio clock
                                                                    │
                             the widget's paint picks the frame due ◄┘
```

`Player::new(source, start)` takes any `Read + Send`, so HTTP stays the
app's job. Threads: demux (also decodes AAC), video decode (also runs the
convert pass), and the audio output, which owns the cpal stream because it
isn't `Send` everywhere. The video widget's `paint` picks the newest frame
whose pts the clock has passed and asks for another frame while playing.
Each converted frame is a new `Bitmap`, so dfui's damage diff sees it.

### Demux

Our own fMP4 reader (`mp4.rs`): `moov` once (`avcC`, the AAC config from
`esds`, timescales, edit lists, `trex` defaults), then each `moof` with the
`mdat` after it, read whole. Samples come out in decode order across tracks.
Tested against a fixture made with the server's movflags.

### Audio and clock

symphonia decodes AAC to f32 and cpal plays it at the stream's rate and
channels, or folded to stereo. The clock is frames played ÷ rate plus the
first sample's pts. Sound starts only when the first video frame is ready,
and stops (with the clock) while paused or starved. Streams without sound
use a wall clock.

### Decoders

One per platform, picked by `cfg` (no trait: there's never a second one on
the same platform):

| | Linux | Android |
| --- | --- | --- |
| Library | libavcodec (`ffmpeg-sys-the-third`) with a VA-API device; software when there's none | `ndk` MediaCodec |
| Input | `avcC` as extradata, length-prefixed NALs as they are | Annex B, SPS/PPS as `csd-0`/`csd-1` |
| Output | VA surface mapped to DRM PRIME: NV12 Y and UV planes as dmabufs | `AImageReader` in YUV_420_888, copied out as NV12 |
| Fallback | Copy with `av_hwframe_transfer_data` when mapping or importing fails | — |

libavcodec replaced cros-codecs: cros-codecs needs git pins of two repos and
patches to build against libva 2.24, and its placeholder 16×16 context fails
on NVIDIA's VA-API driver. libavcodec also handles any H.264 the server
copies, interlaced included.

### Convert pass

dfui-media owns RGBA textures the size of the video (a small pool) and runs
one full-screen WGSL pass per frame: NV12 to R'G'B', BT.709 or BT.601,
limited or full range, from the stream. The decode thread waits for the pass
before handing the frame's memory back, so no fences cross threads.

- Linux: the two dmabuf planes become `R8Unorm` and `Rg8Unorm` textures with
  wgpu-hal's `texture_from_dmabuf_fd`, one import per frame.
- Memory frames (Android, and Linux's fallback) are written into two plane
  textures with `write_texture`.

A GPU test decodes the fixture both ways and checks they match each other
and ffmpeg's own RGB conversion.

## What dfui core got

- `Bitmap::from_texture`: draws an app-owned `Rgba8Unorm` texture like any
  bitmap.
- `dfui::gpu()`: the window's device and queue, from any thread; it changes
  when the GPU is set up again (an Android resume) and the decode thread
  follows it.
- The device asks for `VULKAN_EXTERNAL_MEMORY_DMA_BUF` when the adapter has
  it; wgpu-hal then enables the dmabuf and DRM modifier extensions itself.

No device or pre-draw hook was needed.

## Status

Done: the server contract, the dfui changes, the crate (demux, audio, VA-API
with zero-copy, MediaCodec, convert pass, widget), examples (`play` on
desktop, `play_android` with an embedded clip), and the movies-thing player's
built-in engine (Settings → Player), with seeking, text subtitles, burned-in
image subtitles and the server's stereo mixes.

Checked on: NVIDIA RTX 4060 Ti with `nvidia-vaapi-driver` (zero-copy), and
the Android emulator (goldfish H.264 decoder). Not yet on the AMD TV
or a phone.

## Next

- Surround: the server re-encodes non-AAC audio to stereo, so 5.1 output
  needs the server to send multichannel AAC for players that ask.
- HDR: only the server's tone-mapping, as above.
- Android zero-copy: import the AHardwareBuffer with a YCbCr sampler (raw
  Vulkan, since wgpu has none) if copying shows at 4K.
- Cache dmabuf imports per VA surface if importing every frame shows.
- Audio track choice, and output latency in the clock.

## Risks

- **Modifiers.** VA-API on AMD often exports tiled modifiers; the Vulkan
  driver must accept them. When it doesn't, the decoder falls back to copies.
- **Build dependencies.** Linux builds need FFmpeg's headers (and libclang
  for bindgen); immutable distros like the TV's Bazzite need them layered or
  a toolbox.
- **wgpu hal API** changes between majors; it's one small module
  (`convert.rs`, `dmabuf`).
