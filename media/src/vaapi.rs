//! H.264 through libavcodec, decoded by VA-API when the machine has it and in
//! software when it doesn't.

use std::collections::VecDeque;
use std::ffi::c_int;
use std::ptr;

use ffmpeg_sys_the_third as ff;

use crate::{DmaPlane, Dmabuf, Frame, Planes};

pub(crate) struct Decoder {
    ctx: *mut ff::AVCodecContext,
    pkt: *mut ff::AVPacket,
    frame: *mut ff::AVFrame,
    out: VecDeque<Frame>,
    /// Hand frames over as dmabufs; cleared when mapping or importing fails.
    zero_copy: bool,
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
    /// `avcc` is the `avcC` box payload; samples are then length-prefixed NALs.
    /// The size comes from the stream itself.
    pub fn new(avcc: &[u8], _width: u32, _height: u32) -> Result<Decoder, String> {
        unsafe {
            let codec = ff::avcodec_find_decoder(ff::AVCodecID::H264);
            if codec.is_null() {
                return Err("libavcodec has no H.264 decoder".into());
            }
            let ctx = ff::avcodec_alloc_context3(codec);
            let pad = ff::AV_INPUT_BUFFER_PADDING_SIZE as usize;
            let extra = ff::av_mallocz(avcc.len() + pad) as *mut u8;
            ptr::copy_nonoverlapping(avcc.as_ptr(), extra, avcc.len());
            (*ctx).extradata = extra;
            (*ctx).extradata_size = avcc.len() as c_int;
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
            };
            if r < 0 {
                return Err(err("opening the H.264 decoder", r));
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
            let bt709 = match f.colorspace {
                ff::AVColorSpace::BT470BG | ff::AVColorSpace::SMPTE170M => false,
                ff::AVColorSpace::UNSPECIFIED => h >= 720,
                _ => true,
            };
            let full_range = f.color_range == ff::AVColorRange::JPEG
                || f.format == ff::AVPixelFormat::YUVJ420P.0;
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
                bt709,
                full_range,
                planes,
            }))
        }
    }

    pub fn frame(&mut self) -> Option<Frame> {
        self.out.pop_front()
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

/// An NV12 or I420 frame in memory, as NV12.
unsafe fn copy(f: &ff::AVFrame) -> Option<Planes> {
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

/// A VA-API frame's NV12 planes as dmabufs, in order Y then UV.
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
    use crate::mp4::{Kind, Reader};

    #[test]
    fn decodes_the_clips_video() {
        let mut r = Reader::new(&include_bytes!("../tests/clip.mp4")[..]).unwrap();
        let (id, avcc) = r
            .tracks
            .iter()
            .find_map(|t| match &t.kind {
                Kind::Video { avcc, .. } => Some((t.id, avcc.clone())),
                _ => None,
            })
            .unwrap();
        let mut dec = Decoder::new(&avcc, 64, 48).unwrap();
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
}
