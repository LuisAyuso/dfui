# dfui-media

A workspace member beside `map`: video decoded in hardware and drawn by dfui
as a texture, with dfui controls over it. VA-API on Linux, MediaCodec on
Android. It replaces the [media-player study](media-player.md)'s libmpv
route: the first user is movies-thing, and since its server picks the codec
and the container, the player only has to play one narrow format well.

## The stream contract

The movies-thing server already sends fragmented MP4 from ffmpeg
(`server/src/stream.rs`). For dfui-media it sends exactly:

| Part | Format |
| --- | --- |
| Container | Fragmented MP4 (`moov` then `moof`/`mdat` pairs), on stdout over HTTP |
| Video | H.264, 8-bit 4:2:0, High profile; copied when the source already is, else transcoded |
| Audio | AAC-LC stereo 48 kHz; the server does the downmix and `Mix` as today |
| Image subtitles | Burned in by the server, as today |
| Text subtitles | Fetched from the server's subtitle endpoint, drawn by dfui |
| Seeking | Reopen the stream with `start=`; no range requests |

The player sends the codecs it decodes, `codecs=h264,aac`, on the stream
request; the server copies a track only when its codec is on that list and
re-encodes the rest. 10-bit H.264 is never copied.

Later codecs, in order: 8-bit HEVC (MIME and `hvcC` on Android, cros-codecs'
H.265 decoder on Linux), then 10-bit HEVC and AV1, which need P010 frames.
HDR stays tone-mapped by the server until the convert pass learns BT.2020.
The player will ask its decoder what it supports and send that list.

What this removes from the player: container sniffing, MKV, codec
negotiation, audio formats beyond AAC, multichannel layouts, libass, and
seek tables.

## Pipeline

```
HTTP ──► fMP4 demux ──┬─► H.264 access units ──► Decoder (HAL) ──► Frame ──► convert pass ──► RGBA wgpu texture ──► dfui
                      └─► AAC frames ──► symphonia ──► cpal ──► audio clock
                                                                    │
                                     frame queue picks by pts ◄─────┘
```

Threads: one for HTTP + demux, one for decode, cpal's callback for audio.
The UI thread only picks the frame whose pts is due on the audio clock and
records the convert pass.

### Demux

Our own fMP4 reader, a few hundred lines: `ftyp`/`moov` once (`avcC` for
SPS/PPS, `esds` for the AAC config, timescales), then for each fragment
`tfhd`/`tfdt`/`trun` give sample sizes, durations and offsets into `mdat`.
Length-prefixed NALs go to the decoder as they are (MediaCodec) or as
Annex B (VA-API parser). No crate: the format is fixed by our own muxer.

### Audio and clock

`symphonia` (AAC only) decodes to f32, `cpal` plays it. The clock is
samples played ÷ 48000 plus the first pts, from cpal's callback. Both
platforms use this path; no MediaCodec for audio.

### HAL

```rust
pub trait Decoder: Send {
    /// Queues one access unit; frames come out of `frame` in display order.
    fn decode(&mut self, au: &[u8], pts: Duration) -> Result<(), Error>;
    fn frame(&mut self) -> Option<Frame>;
    fn flush(&mut self);
}
```

Two implementations, one per platform, picked by `cfg`. `Frame` holds a
platform handle the convert pass can import, not pixels:

| | Linux: VA-API | Android: MediaCodec |
| --- | --- | --- |
| Crate | `cros-codecs` (`vaapi` feature): H.264 parser, DPB, VA-API stateless decode | `ndk::media::MediaCodec` (stateful: feed NALs, get buffers) |
| Output | VA surface, exported with `vaExportSurfaceHandle` as DRM PRIME 2: one dmabuf fd, NV12 Y and UV planes with offsets and modifier | `AImageReader` with `GPU_SAMPLED_IMAGE` usage, set as the codec's output surface: one `AHardwareBuffer` per frame |
| Vulkan import | `VK_EXT_external_memory_dma_buf` + `VK_EXT_image_drm_format_modifier` | `VK_ANDROID_external_memory_android_hardware_buffer` |
| Sampling | Two plain images on the same memory: Y as `R8Unorm`, UV as `Rg8Unorm` | `VkSamplerYcbcrConversion` with the buffer's external format (vendor-defined layout) |
| Release | Surface back to the decoder's pool when the frame drops | `AImage_delete` when the frame drops |

### Convert pass

The decoded frame never reaches dfui's shader. dfui-media owns an RGBA
texture the size of the video, and each new frame runs one full-screen
pass into it: NV12 to RGB (BT.709, limited range), at video size. dfui
draws that texture like a bitmap, so scaling, clipping, rounded corners
and the controls overlay all come from dfui unchanged.

- Linux: the pass is plain wgpu. Both plane images are wrapped with
  `Device::texture_from_raw` and `create_texture_from_hal`, and a WGSL
  shader does the matrix.
- Android: wgpu can't make a YCbCr-conversion sampler, so this pass is raw
  Vulkan (`ash`, reached through `as_hal`) with an immutable sampler, writing
  into the same RGBA texture wgpu owns. It is the only raw Vulkan code.

Sync: the pass is recorded into dfui's frame before dfui's own draw, on
dfui's queue; a frame is released one present later. VA-API decode is
finished before the surface is handed out (`vaSyncSurface`); MediaCodec
hands an acquire fence with the image, waited on with a semaphore import.

## What dfui core needs

1. **Device hook.** dfui creates the device with plain `request_device`. Add
   a way for the app to name extra Vulkan device extensions, applied through
   wgpu-hal's `Adapter::open_with_callback` and `create_device_from_hal`.
   Desktop wgpu must stay on the Vulkan backend for this; today only
   Android forces it.
2. **GPU textures.** A `Bitmap` built from a `wgpu::Texture` the app owns
   (`Bitmap::from_texture`), drawn exactly like an uploaded one, plus access
   to the `wgpu::Device` and `Queue` (`App::gpu_ready` or similar).
3. **Pre-draw hook.** A callback per frame that gets the command encoder
   before dfui's pass, where the convert pass is recorded.
4. **Animation.** A playing video asks for every frame; a `request_frame`
   from any thread through the `Notifier` already covers waking.

## Phases

Each one ends with something that plays, and gets its own commit.

1. **Server contract.** The `codecs` list on movies-thing's stream request.
   Done.
2. **dfui hooks.** Device extensions, `Bitmap::from_texture`, pre-draw hook.
   An example that fills a texture from a compute or render pass each frame.
3. **Audio and demux.** `dfui-media` crate: fMP4 reader, AAC through
   symphonia and cpal, the clock. An example plays a movies-thing stream's
   sound with a time readout. Demux gets a test on a small fMP4 fixture.
4. **VA-API with a copy.** Decode through cros-codecs, read surfaces back
   with `vaGetImage`, upload the two planes with `write_texture`, convert.
   This proves decode, A/V sync and seeking before any zero-copy.
5. **Zero-copy Linux.** dmabuf export and import; drop the readback.
6. **movies-thing player.** Replace the mpv window with the dfui-media
   widget; controls, tracks and subtitles in dfui.
7. **Android.** MediaCodec, AImageReader, AHardwareBuffer import, the raw
   Vulkan convert pass.

## Risks

- **cros-codecs on crates.io is 0.0.6** and development moved into the
  ChromiumOS tree. Pin a git revision if the crates.io one lags; the
  fallback is `cros-libva` plus our own H.264 slice-header parsing and DPB,
  which is the bulk of a stateless decoder.
- **Modifiers.** VA-API on AMD often exports tiled modifiers; the Vulkan
  driver must accept them (`vkGetPhysicalDeviceImageFormatProperties2` with
  the modifier). Intel and AMD Mesa both do for NV12; check early.
- **Android external formats** vary by vendor, which is why that pass is a
  YCbCr sampler and not our own shader math.
- **wgpu hal API** changes between majors; the hal code stays in two small
  files so a wgpu bump touches only those.
