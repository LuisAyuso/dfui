# HDR video

What dfui and dfui-media need to show HDR films as HDR, and to tone-map them
well when the screen can't. The first user is the movies-thing player on the
TV (Steam's gaming mode under gamescope, an HDR TV). See
[media.md](media.md) for the pipeline this extends.

## Today

The player asks the server for H.264 with `tonemap=true`. For an HDR film the
server decodes the 10-bit HEVC, tone-maps it to SDR BT.709 (libplacebo, or
zscale on the CPU) and encodes H.264, so every HDR film costs a 4K transcode
on the server and reaches the TV as SDR. dfui then draws everything in
8-bit sRGB.

The library's HDR films are mostly HDR10: The Creator is HEVC Main 10, BT.2020
primaries, PQ transfer, a 1000-nit mastering display and an unknown MaxCLL.
The server's probe also tags HLG, HDR10+ and Dolby Vision (`probe.rs`).

## What HDR needs, layer by layer

| Layer | Needs | State |
| --- | --- | --- |
| Server | Copy 10-bit HEVC (and its HDR metadata) when the player asks for `hevc` and no tone-mapping | `codecs` and `tonemap` already exist; the player just has to ask |
| Demux | `hvc1`/`hev1` sample entries and `hvcC` | `mp4.rs` reads `avc1` only |
| Decode | HEVC Main 10 into P010 frames | libavcodec decodes it; the VA-API driver must too (NVIDIA: yes; AMD VCN: unchecked). MediaCodec needs `video/hevc` and a P010 ImageReader (API 33) |
| Frames | P010 planes as `R16Unorm` / `Rg16Unorm` textures (dmabuf import or copy) | needs `TEXTURE_FORMAT_16BIT_NORM`, which this GPU has; the decoder must report transfer, primaries and mastering/CLL side data |
| Convert | PQ/HLG BT.2020 to either SDR (tone-mapped) or an HDR working space | the pass does BT.709/601 only |
| dfui output | A surface in `Bt2100Pq` on `Rgb10a2Unorm`, with the UI at SDR reference white | wgpu 30 has the colour spaces and reports them per format; dfui always configures 8-bit sRGB |
| HDR metadata | ST 2086 mastering and MaxCLL for the display's own tone mapping | wgpu doesn't write it; `vkSetHdrMetadataEXT` through wgpu-hal |
| Compositor | Pass HDR10 through to the TV | gamescope does, through its Vulkan WSI layer (mpv used it with `--target-colorspace-hint`); GNOME 51 offers HDR10 on this NVIDIA desktop and maps it to the SDR monitor |

## The design

### One working space

dfui blends in gamma-encoded sRGB on purpose (`gpu.rs`: linear blending makes
anti-aliased edges look thin). HDR shouldn't change how the UI looks, so the
UI keeps that, and HDR video joins it in an extended version of the same
encoding:

- The canvas, which dfui already keeps for partial redraws, is
  `Rgba16Float` for SDR and HDR alike (8-bit only where the GPU can't render
  and blend half floats), so both take the same path. It holds sign-preserving
  sRGB-encoded BT.709, where 1.0 is SDR reference white (203 nits, ITU-R
  BT.2408). UI colours land in 0..1 exactly as now. Video highlights go above
  1, and BT.2020 colours outside BT.709 go negative.
- Video frames convert into `Rgba16Float` textures in the same encoding: PQ
  or HLG decoded to linear nits, divided by 203, BT.2020 → BT.709 matrix,
  then extended sRGB encoding. `Bitmap::from_texture` accepts that format
  too.
- The copy from canvas to swapchain is a pass (`present.wgsl`) for both. With
  its `pq` constant off it writes the canvas as is (clamped); on, it decodes to
  linear, × 203 nits, BT.709 → BT.2020, PQ-encodes and writes `Rgb10a2Unorm`
  in `Bt2100Pq`. The main shader doesn't change at all, and `DFUI_SHOW_DAMAGE`
  draws its flash in this pass. On this desktop, at 1080×2400, the pass costs
  about 47 µs and the float canvas a few percent more than an 8-bit one.

Apps opt in (`App::hdr() -> bool`), and the whole window, UI and video, then
goes out as HDR10 when the surface lists `Bt2100Pq` for a format; otherwise
it's SDR. `dfui::hdr()` says which, for video that should tone-map when it
isn't.

### Tone mapping in the player

When the output is SDR (no opt-in, no HDR display, or HDR off in Steam), the
convert pass tone-maps instead: PQ/HLG to linear, BT.2390 EETF from the
mastering peak (MaxCLL when known) down to SDR white, gamut-map BT.2020 to
BT.709, sRGB encode. This lets the server copy HDR films for every screen,
which removes the 4K transcode, and it's the first step because it needs none
of the dfui output work.

### What the player asks for

| Screen | Request | Player does |
| --- | --- | --- |
| HDR output active | `codecs=h264,hevc,aac` | HDR → working space |
| SDR | `codecs=h264,hevc,aac` | tone-maps |
| Decoder without HEVC 10-bit | `codecs=h264,aac&tonemap=true` (today) | nothing |

The player checks its decoder for HEVC Main 10 before asking for `hevc`.

## Phases

1. **HEVC Main 10 with client tone mapping.** Done in dfui-media: `hvcC`
   demux, P010 frames (zero-copy into `R16Unorm`/`Rg16Unorm`, or copied),
   colour info and MaxCLL / mastering peak from libavcodec, and the convert
   pass, which either keeps HDR (`Rgba16Float`, 203 nits = 1.0) or tone-maps
   with the BT.2390 EETF on the brightest channel from the film's peak to SDR
   white. `dfui_media::codecs()` adds `hevc` where VA-API decodes 10-bit HEVC
   (probed with a 9 KB HDR10 clip). Checks: grey ramps at known PQ codes, the
   HDR10 clip through dmabuf and copy, and libplacebo's own BT.2390 on the
   same clip (same brightness within 5%; ours is closer to the clip's SDR
   source). Android stays on H.264 until it has a P010 ImageReader.
2. **HDR output in dfui.** Done: `App::hdr()`, surface choice, the float
   canvas, the present pass, `Rgba16Float` bitmaps. A GPU test checks the PQ
   codes for 203 and 1000 nits. On this desktop GNOME 51 takes the HDR10
   surface (and maps it to the SDR monitor), and nested gamescope
   (`gamescope --hdr-enabled --hdr-debug-force-output -- env
   ENABLE_GAMESCOPE_WSI=1 app`) gets an HDR10 swapchain as on the TV. The
   picture itself still needs an HDR screen.
3. **Metadata.** Mastering display and MaxCLL through `vkSetHdrMetadataEXT`,
   from the film while it plays.
4. **The TV.** Run under gamescope's WSI layer (`ENABLE_GAMESCOPE_WSI=1`,
   HDR on in Steam), check the TV's VA-API decodes HEVC Main 10, and look at
   it.
5. **Later.** HLG, Android (P010 ImageReader, HDR10 swapchains on phones),
   Dolby Vision profile 5 (no HDR10 base layer: keep transcoding those),
   HDR10+ dynamic metadata.

## Open questions

- Does the TV's VA-API (Cyan Skillfish, VCN) decode HEVC Main 10, and export
  P010 dmabufs with modifiers its Vulkan driver imports? If not, phase 1
  falls back to the copy path, or to the server's transcode.
- Does gamescope pass `Bt2100Pq` through from a non-Steam Vulkan app the way
  it did for mpv, and does it want the metadata?
- Is one full-screen PQ pass per frame cheap enough on the TV at 4K? The UI
  redraws little while a film plays, but the video changes every frame.
- Subtitles and the overlay sit at SDR white; bright UI over a dark HDR
  scene may need a lower white (the 203 nits could be a setting).
