//! H.264 through libavcodec, decoded by VA-API when the machine has it and in
//! software when it doesn't.

use std::collections::VecDeque;
use std::ffi::c_int;
use std::ptr;

use ffmpeg_sys_the_third as ff;

use crate::mp4::{Codec, Kind, Reader};
use crate::{Color, DmaPlane, Dmabuf, Frame, Matrix, Planes, Transfer};

/// libavutil's `AVMasteringDisplayMetadata`, which the bindings leave out.
#[repr(C)]
struct Mastering {
    primaries: [[ff::AVRational; 2]; 3],
    white: [ff::AVRational; 2],
    min_luminance: ff::AVRational,
    max_luminance: ff::AVRational,
    has_primaries: c_int,
    has_luminance: c_int,
}

/// libavutil's `AVContentLightMetadata`.
#[repr(C)]
struct LightLevel {
    max_cll: std::ffi::c_uint,
    max_fall: std::ffi::c_uint,
}

pub(crate) struct Decoder {
    ctx: *mut ff::AVCodecContext,
    pkt: *mut ff::AVPacket,
    frame: *mut ff::AVFrame,
    out: VecDeque<Frame>,
    /// Hand frames over as dmabufs; cleared when mapping or importing fails.
    zero_copy: bool,
    /// The film's peak from its HDR metadata, which only some frames carry.
    peak: f32,
    /// A frame came out of VA-API, not the software decoder.
    hardware: bool,
}

// SAFETY: the codec context and frames are only used by the thread owning the decoder.
unsafe impl Send for Decoder {}

const US: ff::AVRational = ff::AVRational {
    num: 1,
    den: 1_000_000,
};

unsafe extern "C" fn get_format(
    ctx: *mut ff::AVCodecContext,
    fmts: *const ff::AVPixelFormat,
) -> ff::AVPixelFormat {
    unsafe {
        let mut p = fmts;
        while *p != ff::AVPixelFormat::NONE {
            if *p == ff::AVPixelFormat::VAAPI {
                return *p;
            }
            p = p.add(1);
        }
        log::warn!("no VA-API for this stream, decoding in software");
        ff::avcodec_default_get_format(ctx, fmts)
    }
}

fn err(what: &str, code: c_int) -> String {
    let mut buf = [0 as std::ffi::c_char; 128];
    unsafe { ff::av_strerror(code, buf.as_mut_ptr(), buf.len()) };
    let msg = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
    format!("{what}: {}", msg.to_string_lossy())
}

impl Decoder {
    /// `config` is the `avcC` or `hvcC` box payload; samples are then
    /// length-prefixed NALs. The size comes from the stream itself.
    pub fn new(kind: Codec, config: &[u8], _width: u32, _height: u32) -> Result<Decoder, String> {
        let (id, name) = match kind {
            Codec::H264 => (ff::AVCodecID::H264, "H.264"),
            Codec::Hevc => (ff::AVCodecID::HEVC, "HEVC"),
        };
        unsafe {
            let codec = ff::avcodec_find_decoder(id);
            if codec.is_null() {
                return Err(format!("libavcodec has no {name} decoder"));
            }
            let ctx = ff::avcodec_alloc_context3(codec);
            let pad = ff::AV_INPUT_BUFFER_PADDING_SIZE as usize;
            let extra = ff::av_mallocz(config.len() + pad) as *mut u8;
            ptr::copy_nonoverlapping(config.as_ptr(), extra, config.len());
            (*ctx).extradata = extra;
            (*ctx).extradata_size = config.len() as c_int;
            (*ctx).pkt_timebase = US;
            let mut device = ptr::null_mut();
            let r = ff::av_hwdevice_ctx_create(
                &mut device,
                ff::AVHWDeviceType::VAAPI,
                ptr::null(),
                ptr::null_mut(),
                0,
            );
            if r < 0 {
                log::warn!("{}; decoding in software", err("no VA-API device", r));
                (*ctx).thread_count = 0;
            } else {
                (*ctx).hw_device_ctx = device;
                (*ctx).get_format = Some(get_format);
            }
            let r = ff::avcodec_open2(ctx, codec, ptr::null_mut());
            let me = Decoder {
                ctx,
                pkt: ff::av_packet_alloc(),
                frame: ff::av_frame_alloc(),
                out: VecDeque::new(),
                zero_copy: true,
                peak: 0.0,
                hardware: false,
            };
            if r < 0 {
                return Err(err(&format!("opening the {name} decoder"), r));
            }
            Ok(me)
        }
    }

    /// Queues one sample; its frames come out of [`frame`](Self::frame).
    pub fn decode(&mut self, data: &[u8], pts: f64) -> Result<(), String> {
        unsafe {
            let r = ff::av_new_packet(self.pkt, data.len() as c_int);
            if r < 0 {
                return Err(err("allocating a packet", r));
            }
            ptr::copy_nonoverlapping(data.as_ptr(), (*self.pkt).data, data.len());
            (*self.pkt).pts = (pts * 1e6).round() as i64;
            let sent = self.send(self.pkt);
            ff::av_packet_unref(self.pkt);
            sent
        }
    }

    /// Flushes the frames held back for reordering, at the end of the stream.
    pub fn finish(&mut self) -> Result<(), String> {
        self.send(ptr::null_mut())
    }

    fn send(&mut self, pkt: *mut ff::AVPacket) -> Result<(), String> {
        loop {
            let r = unsafe { ff::avcodec_send_packet(self.ctx, pkt) };
            if r == ff::AVERROR(libc::EAGAIN) {
                self.receive()?;
                continue;
            }
            // A broken slice in the middle of a movie shouldn't end it.
            if r < 0 && r != ff::AVERROR_EOF {
                log::warn!("{}", err("decoding", r));
            }
            return self.receive();
        }
    }

    fn receive(&mut self) -> Result<(), String> {
        loop {
            let r = unsafe { ff::avcodec_receive_frame(self.ctx, self.frame) };
            if r == ff::AVERROR(libc::EAGAIN) || r == ff::AVERROR_EOF {
                return Ok(());
            }
            if r < 0 {
                return Err(err("decoding", r));
            }
            let f = unsafe { self.take() };
            unsafe { ff::av_frame_unref(self.frame) };
            self.out.extend(f?);
        }
    }

    /// `self.frame` as a [`Frame`]; `None` for frames that can't be shown.
    unsafe fn take(&mut self) -> Result<Option<Frame>, String> {
        unsafe {
            let f = &*self.frame;
            let (w, h) = (f.width as u32, f.height as u32);
            let sar = match f.sample_aspect_ratio {
                ff::AVRational { num, den } if num > 0 && den > 0 => num as f32 / den as f32,
                _ => 1.0,
            };
            let matrix = match f.colorspace {
                ff::AVColorSpace::BT2020_NCL | ff::AVColorSpace::BT2020_CL => Matrix::Bt2020,
                ff::AVColorSpace::BT470BG | ff::AVColorSpace::SMPTE170M => Matrix::Bt601,
                ff::AVColorSpace::UNSPECIFIED if h < 720 => Matrix::Bt601,
                _ => Matrix::Bt709,
            };
            let transfer = match f.color_trc {
                ff::AVColorTransferCharacteristic::SMPTE2084 => Transfer::Pq,
                ff::AVColorTransferCharacteristic::ARIB_STD_B67 => Transfer::Hlg,
                _ => Transfer::Sdr,
            };
            self.peak = peak(self.frame).unwrap_or(self.peak);
            // A VA-API frame's own format says VAAPI; its surfaces' tells the depth.
            let format = match f.format == ff::AVPixelFormat::VAAPI.0 && !f.hw_frames_ctx.is_null()
            {
                true => (*((*f.hw_frames_ctx).data as *const ff::AVHWFramesContext)).sw_format,
                false => ff::AVPixelFormat(f.format),
            };
            let ten =
                format == ff::AVPixelFormat::P010LE || format == ff::AVPixelFormat::YUV420P10LE;
            let color = Color {
                matrix,
                transfer,
                full_range: f.color_range == ff::AVColorRange::JPEG
                    || f.format == ff::AVPixelFormat::YUVJ420P.0,
                bits: if ten { 10 } else { 8 },
                peak: self.peak,
            };
            self.hardware |= f.format == ff::AVPixelFormat::VAAPI.0;
            let planes = if f.format == ff::AVPixelFormat::VAAPI.0 {
                match self.zero_copy.then(|| map(self.frame)).flatten() {
                    Some(d) => Planes::Dmabuf(d),
                    None => {
                        self.zero_copy = false;
                        let sw = ff::av_frame_alloc();
                        let r = ff::av_hwframe_transfer_data(sw, self.frame, 0);
                        let p = (r >= 0).then(|| copy(&*sw)).flatten();
                        ff::av_frame_free(&mut { sw });
                        match p {
                            Some(p) => p,
                            None => return Err(err("reading a VA-API frame", r)),
                        }
                    }
                }
            } else {
                match copy(f) {
                    Some(p) => p,
                    None => {
                        log::warn!("unsupported pixel format {}", f.format);
                        return Ok(None);
                    }
                }
            };
            Ok(Some(Frame {
                pts: f.best_effort_timestamp as f64 / 1e6,
                width: w,
                height: h,
                sar,
                color,
                planes,
            }))
        }
    }

    pub fn frame(&mut self) -> Option<Frame> {
        self.out.pop_front()
    }

    /// Whether 10-bit HEVC plays here: the first frame of a tiny HDR10 clip
    /// decodes, through VA-API or on a CPU with threads enough for 4K.
    pub fn hevc10() -> bool {
        // ponytail: thread count as the speed test; a BC-250 (12 threads) plays 4K
        // HDR10 on about 2.5 cores. Time a real 4K decode if weaker CPUs stutter.
        let fast_cpu = std::thread::available_parallelism().is_ok_and(|n| n.get() >= 8);
        let probe = || -> Option<bool> {
            let mut r = Reader::new(&include_bytes!("../tests/hdr10.mp4")[..]).ok()?;
            let Kind::Video { config, .. } = &r.tracks.first()?.kind else {
                return None;
            };
            let mut dec = Decoder::new(Codec::Hevc, config, 256, 144).ok()?;
            while let Some(s) = r.sample().ok()? {
                dec.decode(&s.data, s.pts).ok()?;
                if dec.frame().is_some() {
                    return Some(dec.hardware || fast_cpu);
                }
            }
            None
        };
        probe().unwrap_or(false)
    }

    /// Hands over frames in memory from now on, for GPUs that can't import dmabufs.
    pub fn copy_frames(&mut self) {
        self.zero_copy = false;
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            ff::av_frame_free(&mut self.frame);
            ff::av_packet_free(&mut self.pkt);
            ff::avcodec_free_context(&mut self.ctx);
        }
    }
}

/// The film's peak brightness, nits, if `frame` carries HDR metadata:
/// MaxCLL when known, else the mastering display's peak.
unsafe fn peak(frame: *const ff::AVFrame) -> Option<f32> {
    unsafe {
        let side = |t| {
            ff::av_frame_get_side_data(frame, t)
                .as_ref()
                .map(|d| d.data)
        };
        if let Some(d) = side(ff::AVFrameSideDataType::CONTENT_LIGHT_LEVEL) {
            let l = &*(d as *const LightLevel);
            if l.max_cll > 0 {
                return Some(l.max_cll as f32);
            }
        }
        let m = &*(side(ff::AVFrameSideDataType::MASTERING_DISPLAY_METADATA)? as *const Mastering);
        let ff::AVRational { num, den } = m.max_luminance;
        (m.has_luminance != 0 && num > 0 && den > 0).then(|| num as f32 / den as f32)
    }
}

/// An NV12, I420, P010 or 10-bit I420 frame in memory, as NV12 or P010.
unsafe fn copy(f: &ff::AVFrame) -> Option<Planes> {
    let fmt = ff::AVPixelFormat(f.format);
    if fmt == ff::AVPixelFormat::P010LE || fmt == ff::AVPixelFormat::YUV420P10LE {
        return unsafe { copy10(f) };
    }
    let (w, h) = (f.width as usize, f.height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let rows = |p: usize, n: usize, len: usize| -> Vec<&[u8]> {
        (0..n)
            .map(|r| unsafe {
                std::slice::from_raw_parts(
                    f.data[p].offset(r as isize * f.linesize[p] as isize),
                    len,
                )
            })
            .collect()
    };
    let y = rows(0, h, w).concat();
    let fmt = ff::AVPixelFormat(f.format);
    let uv = if fmt == ff::AVPixelFormat::NV12 {
        rows(1, ch, cw * 2).concat()
    } else if fmt == ff::AVPixelFormat::YUV420P || fmt == ff::AVPixelFormat::YUVJ420P {
        let (u, v) = (rows(1, ch, cw), rows(2, ch, cw));
        let mut uv = Vec::with_capacity(cw * ch * 2);
        for (u, v) in u.iter().zip(&v) {
            for (a, b) in u.iter().zip(v.iter()) {
                uv.extend([*a, *b]);
            }
        }
        uv
    } else {
        return None;
    };
    Some(Planes::Cpu {
        y,
        y_stride: w,
        uv,
        uv_stride: cw * 2,
    })
}

/// A 10-bit frame as P010: samples in the high bits of little-endian u16s.
unsafe fn copy10(f: &ff::AVFrame) -> Option<Planes> {
    let (w, h) = (f.width as usize, f.height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    let row = |p: usize, r: usize, n: usize| -> &[u16] {
        unsafe {
            std::slice::from_raw_parts(
                f.data[p].offset(r as isize * f.linesize[p] as isize) as *const u16,
                n,
            )
        }
    };
    let bytes = |v: &mut Vec<u8>, s: u16| v.extend(s.to_le_bytes());
    let mut y = Vec::with_capacity(w * h * 2);
    let mut uv = Vec::with_capacity(cw * ch * 4);
    if ff::AVPixelFormat(f.format) == ff::AVPixelFormat::P010LE {
        (0..h).for_each(|r| row(0, r, w).iter().for_each(|&s| bytes(&mut y, s)));
        (0..ch).for_each(|r| row(1, r, cw * 2).iter().for_each(|&s| bytes(&mut uv, s)));
    } else {
        // Planar, in the low bits.
        (0..h).for_each(|r| row(0, r, w).iter().for_each(|&s| bytes(&mut y, s << 6)));
        for r in 0..ch {
            for (u, v) in row(1, r, cw).iter().zip(row(2, r, cw)) {
                bytes(&mut uv, u << 6);
                bytes(&mut uv, v << 6);
            }
        }
    }
    Some(Planes::Cpu {
        y,
        y_stride: w * 2,
        uv,
        uv_stride: cw * 4,
    })
}

/// Owns a DRM PRIME mapping of a VA-API frame; the surface stays out of the
/// decoder's pool until this drops.
struct Mapped(*mut ff::AVFrame);

// SAFETY: the mapping is only read, then freed by whichever thread drops it.
unsafe impl Send for Mapped {}

impl Drop for Mapped {
    fn drop(&mut self) {
        unsafe { ff::av_frame_free(&mut self.0) };
    }
}

/// A VA-API frame's NV12 or P010 planes as dmabufs, in order Y then UV.
unsafe fn map(frame: *mut ff::AVFrame) -> Option<Dmabuf> {
    unsafe {
        let drm = ff::av_frame_alloc();
        (*drm).format = ff::AVPixelFormat::DRM_PRIME.0;
        let keep = Mapped(drm);
        let r = ff::av_hwframe_map(drm, frame, ff::AV_HWFRAME_MAP_READ.0 as c_int);
        if r < 0 {
            log::warn!(
                "{}; copying frames instead",
                err("mapping a VA-API frame", r)
            );
            return None;
        }
        let d = &*((*drm).data[0] as *const ff::AVDRMFrameDescriptor);
        let mut planes = Vec::new();
        for l in &d.layers[..d.nb_layers.clamp(0, 4) as usize] {
            for p in &l.planes[..l.nb_planes.clamp(0, 4) as usize] {
                let o = d.objects.get(p.object_index as usize)?;
                planes.push(DmaPlane {
                    fd: o.fd,
                    offset: p.offset as u64,
                    pitch: p.pitch as u64,
                    modifier: o.format_modifier,
                });
            }
        }
        let [y, uv] = <[DmaPlane; 2]>::try_from(planes).ok()?;
        Some(Dmabuf {
            planes: [y, uv],
            _keep: Box::new(keep),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_clips_video() {
        let mut r = Reader::new(&include_bytes!("../tests/clip.mp4")[..]).unwrap();
        let (id, avcc) = r
            .tracks
            .iter()
            .find_map(|t| match &t.kind {
                Kind::Video { config, .. } => Some((t.id, config.clone())),
                _ => None,
            })
            .unwrap();
        let mut dec = Decoder::new(Codec::H264, &avcc, 64, 48).unwrap();
        let mut frames = Vec::new();
        while let Some(s) = r.sample().unwrap() {
            if s.track == id {
                dec.decode(&s.data, s.pts).unwrap();
                frames.extend(std::iter::from_fn(|| dec.frame()));
            }
        }
        dec.finish().unwrap();
        frames.extend(std::iter::from_fn(|| dec.frame()));
        assert_eq!(frames.len(), 24);
        assert!(frames.iter().all(|f| (f.width, f.height) == (64, 48)));
        assert!(
            frames.windows(2).all(|w| w[0].pts < w[1].pts),
            "display order"
        );
        assert!((frames[0].pts - 1.0 / 12.0).abs() < 1e-3);
        let kind = match &frames[0].planes {
            Planes::Cpu { y, .. } => {
                assert_eq!(y.len(), 64 * 48);
                "memory"
            }
            Planes::Dmabuf(d) => {
                assert!(d.planes.iter().all(|p| p.fd >= 0 && p.pitch >= 64));
                "dmabuf"
            }
        };
        eprintln!("frames came as {kind}");
    }

    /// The HDR10 fixture: 256x144 HEVC Main 10, BT.2020 and PQ, mastered for
    /// 1000 nits, half a second at 24 fps.
    fn hdr10(copy: bool) -> Vec<Frame> {
        let mut r = Reader::new(&include_bytes!("../tests/hdr10.mp4")[..]).unwrap();
        let (id, config) = r
            .tracks
            .iter()
            .find_map(|t| match &t.kind {
                Kind::Video { config, .. } => Some((t.id, config.clone())),
                _ => None,
            })
            .unwrap();
        let mut dec = Decoder::new(Codec::Hevc, &config, 256, 144).unwrap();
        if copy {
            dec.copy_frames();
        }
        let mut frames = Vec::new();
        while let Some(s) = r.sample().unwrap() {
            if s.track == id {
                dec.decode(&s.data, s.pts).unwrap();
                frames.extend(std::iter::from_fn(|| dec.frame()));
            }
        }
        dec.finish().unwrap();
        frames.extend(std::iter::from_fn(|| dec.frame()));
        frames
    }

    #[test]
    fn decodes_hdr10_hevc_with_its_colors() {
        for copy in [false, true] {
            let frames = hdr10(copy);
            assert_eq!(frames.len(), 12);
            let c = frames[0].color;
            assert_eq!(
                (c.matrix, c.transfer, c.bits),
                (Matrix::Bt2020, Transfer::Pq, 10)
            );
            assert!(!c.full_range);
            assert_eq!(c.peak, 1000.0, "MaxCLL");
            if let Planes::Cpu { y, y_stride, .. } = &frames[0].planes {
                assert_eq!((*y_stride, y.len()), (512, 256 * 144 * 2), "P010");
            }
        }
    }

    #[test]
    fn this_machine_plays_hevc10() {
        // The answer is only printed: it depends on the machine.
        eprintln!("hevc10: {}", Decoder::hevc10());
    }
}
