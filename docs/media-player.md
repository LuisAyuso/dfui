# Media player

> Superseded: dfui-media decodes with VA-API and MediaCodec instead; see
> [media.md](media.md).

A study for a dfui media player subproject: a widget that shows video, and
the controls drawn over it by dfui. The first user is the movies-thing
player, which today runs mpv as a separate window. This page curates the
engines and libraries that could back it. No HDR output is needed.

## What a player needs

| Stage | Job |
| --- | --- |
| Input | HTTP with headers (session cookie), seeking by range requests |
| Demux | MKV and MP4 containers; video, audio and subtitle tracks |
| Video decode | H.264, HEVC (also 10-bit) and AV1, in hardware |
| Present | The decoded frame as a texture on dfui's wgpu device |
| Audio | AAC, AC-3, E-AC-3, DTS, Opus and FLAC; downmix to the configured layout |
| Sync | An audio-driven clock that video frames follow |
| Subtitles | SRT and ASS, from inside the file and from the server |

The present stage is set by dfui. It runs wgpu 30 on the **Vulkan backend
only** (`winit_app.rs`, `activity.rs`). `Bitmap` is a CPU RGBA buffer,
uploaded once and cached by id. A player needs two things dfui doesn't
have yet:

- A frame source the app updates every frame, either a texture or a CPU
  upload into a texture that already exists, plus NV12 (YUV) to RGB
  conversion in the shader or in a pass before it.
- For zero-copy import, access to the wgpu device at the hal level, and a
  device created with the external-memory extensions. dfui creates the
  device today, so this needs a hook.

## Engines

Each one covers some of the stages above. Ordered from most done for us
to most built by us.

### 1. libmpv (the engine we already use)

The whole pipeline, with mpv's options, `sub-add` and the audio filters
the player already passes. Rust: [`libmpv2`][libmpv2] (5.x, render
context included; needs libmpv 2.0 / mpv 0.35+).

The render API has only OpenGL, software and (Windows) D3D11 backends.
Upstream mpv has no Vulkan render API, only an [open request][mpv-vk] and
patched forks. So with dfui on Vulkan, the ways to get frames are:

- **Software render (`MPV_RENDER_API_TYPE_SW`)**: mpv writes RGBA into our
  buffer, and we upload it to a wgpu texture with `queue.write_texture`.
  It's simple and works now. The cost is the CPU, which does YUV to RGB,
  scaling and the upload: fine at 1080p, heavy at 4K. Hardware decode
  still works with `hwdec=auto-copy`.
- **OpenGL render, then dmabuf to Vulkan**: run a hidden EGL context and
  have mpv render into an FBO there. Export the texture with
  `EGL_MESA_image_dma_buf_export` and import it into wgpu with
  [`wgpu-external-frame`][wef] (Linux DMA-BUF over Vulkan external memory).
  This is zero-copy, but needs a fence between EGL and Vulkan, and the
  device extensions described above.
- **Patched Vulkan render API**: ruled out; it means maintaining an mpv fork.

On Android, libmpv builds exist (mpv-android), but the NDK cross-build is
a project of its own.

**Verdict:** the fastest path to a working embedded player. Start with
the software render and move to dmabuf when 4K needs it.

### 2. GStreamer

Also the whole pipeline: `playbin3` does HTTP, demuxing, VA-API decoding
(`vah264dec`/`vah265dec`/`vaav1dec`), audio output and the clock. Subtitles
come through `subtitleoverlay`, or `assrender` for libass. Rust:
[`gstreamer`][gst-rs] 0.25 and `gstreamer-app` (`appsink`).

To present, take frames from `appsink`, either in system memory to upload
or as DMA-BUF caps to import through `wgpu-external-frame`.
[`iced_video_player`][ivp] does exactly this for iced, copying frames into
a wgpu texture with its own pipeline, so it's the reference to read.

On Android, official GStreamer builds exist, but they are large and need
an NDK integration.

**Verdict:** a good engine with less magic than mpv. It's a heavy system
dependency, and its pipelines and caps are verbose to debug.

### 3. FFmpeg, built up from parts

FFmpeg demuxes and decodes, and we write the rest: the clock, the frame
queue, seeking, audio output and subtitles. Rust:

- [`rsmpeg`][rsmpeg]: FFmpeg 6–8, close to the C API and actively updated.
- [`ffmpeg-the-third`][ff3]: an `ffmpeg-next` fork, tracks FFmpeg 8.
- `ffmpeg-next`: the most used, but in maintenance mode.

Hardware decode is VA-API through `hwaccel`. Map frames to DRM PRIME
(`av_hwframe_map`) for multi-plane NV12 DMA-BUFs, then import them with
`wgpu-external-frame`. Its examples are single-plane, so the NV12 part is
ours to write. Audio goes through `cpal` (or `rodio`, which the music app
already uses), with `swresample` for the downmix.

**Verdict:** the most control and the most code, since the A/V sync and
seeking logic is the part that mpv and GStreamer took years to get right.

### 4. Pure Rust: gpu-video

[`gpu-video`][gpuv] (Software Mansion, formerly `vk-video`) decodes
through Vulkan Video straight into wgpu NV12 textures, with no FFmpeg. It
targets wgpu 30, the same as dfui. It needs a wgpu device created with the
Vulkan Video extensions, which is the same dfui hook again. Before 0.3 it
decoded H.264 only, and current codec coverage is unverified.

The rest would also be pure Rust: `matroska-demuxer` or `mp4` for
containers, and [`symphonia`][symphonia] with `cpal` for audio. Symphonia
has no AC-3, E-AC-3 or DTS, which most movie files use. That rules out
pure Rust audio for movies.

**Verdict:** worth watching, but not yet a base for a movie player.

## Shared pieces

| Need | Library | Notes |
| --- | --- | --- |
| Frame import (Linux) | [`wgpu-external-frame`][wef] | DMA-BUF over Vulkan; has a wgpu 30 fix |
| Frame import (Android) | `ndk` `HardwareBuffer` + `wgpu-external-frame` (AHardwareBuffer import in progress) | |
| Android decode | `ndk` `media::MediaCodec` | `mediacodec` crate unmaintained since 2022 |
| Audio out | `cpal`, `rodio` | rodio already in movies-thing's music app |
| Audio decode (no AC-3/DTS) | `symphonia` | only for engine 4 |
| Resampling | `rubato` | only if not using FFmpeg/GStreamer |
| ASS subtitles | libass (system), [`rassa`][rassa] (pure Rust rewrite) | mpv and GStreamer bring libass themselves |

## Android

None of the desktop engines carries over cheaply. The native route is
MediaCodec decoding into an `ImageReader`, which gives AHardwareBuffers to
import into Vulkan. The other route plays with ExoPlayer through JNI, like
the music app's `PlaybackService`, but then the video is an Android
`SurfaceView` under the dfui surface, not a dfui texture. That needs a
transparent hole in the dfui surface for the controls overlay.

## Recommendation

1. Add a video frame source to dfui: a texture the app updates each frame
   from a CPU buffer, drawn like an image. Leave NV12 out at first.
2. Embed libmpv through `libmpv2` with the software render API in a new
   workspace member. Keep mpv's own options, subtitles and audio filters
   from the movies-thing player, and draw the controls with dfui.
3. If the CPU cost shows at 4K: switch to mpv's OpenGL render with dmabuf
   export, then a `wgpu-external-frame` import. That's when dfui adds the
   device hook and NV12.
4. Look at Android only after the desktop works, starting from MediaCodec
   and AHardwareBuffer.

[libmpv2]: https://docs.rs/libmpv2
[mpv-vk]: https://github.com/mpv-player/mpv/issues/18343
[wef]: https://github.com/water-rs/wgpu-external-frame
[gst-rs]: https://crates.io/crates/gstreamer
[ivp]: https://github.com/jazzfool/iced_video_player
[rsmpeg]: https://lib.rs/crates/rsmpeg
[ff3]: https://lib.rs/crates/ffmpeg-sys-the-third
[gpuv]: https://lib.rs/crates/gpu-video
[symphonia]: https://crates.io/crates/symphonia
[rassa]: https://docs.rs/crate/rassa/0.5.1
