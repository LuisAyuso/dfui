//! H.264 through MediaCodec into an ImageReader, copied out as NV12.
// ponytail: one copy per frame through the CPU; import the AHardwareBuffer with a
// YCbCr sampler if 4K on phones needs it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use ndk::media::image_reader::{AcquireResult, Image, ImageFormat, ImageReader};
use ndk::media::media_codec::{
    DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection,
};
use ndk::media::media_format::MediaFormat;

use crate::{Frame, Planes};

const END_OF_STREAM: u32 = 4;

pub(crate) struct Decoder {
    codec: MediaCodec,
    reader: ImageReader,
    /// Bytes in each NAL's length prefix.
    nal_len: usize,
    bt709: bool,
    full_range: bool,
    out: VecDeque<Frame>,
    annexb: Vec<u8>,
}

/// (NAL length size, SPS and PPS as Annex B) from an `avcC` payload.
fn parse_avcc(avcc: &[u8]) -> Option<(usize, Vec<u8>, Vec<u8>)> {
    let nal_len = (*avcc.get(4)? & 3) as usize + 1;
    let mut at = 5;
    let mut sets = |count_mask: u8| -> Option<Vec<u8>> {
        let n = *avcc.get(at)? & count_mask;
        at += 1;
        let mut out = Vec::new();
        for _ in 0..n {
            let len = u16::from_be_bytes(avcc.get(at..at + 2)?.try_into().ok()?) as usize;
            out.extend([0, 0, 0, 1]);
            out.extend(avcc.get(at + 2..at + 2 + len)?);
            at += 2 + len;
        }
        Some(out)
    };
    let sps = sets(0x1f)?;
    let pps = sets(0xff)?;
    Some((nal_len, sps, pps))
}

/// Length-prefixed NALs as Annex B, into `out`.
fn to_annexb(data: &[u8], nal_len: usize, out: &mut Vec<u8>) {
    out.clear();
    let mut at = 0;
    while at + nal_len <= data.len() {
        let len = data[at..at + nal_len]
            .iter()
            .fold(0usize, |v, &b| v << 8 | b as usize);
        at += nal_len;
        let end = (at + len).min(data.len());
        out.extend([0, 0, 0, 1]);
        out.extend(&data[at..end]);
        at = end;
    }
}

fn err(what: &str, e: impl std::fmt::Display) -> String {
    format!("{what}: {e}")
}

impl Decoder {
    pub fn new(avcc: &[u8], width: u32, height: u32) -> Result<Decoder, String> {
        let (nal_len, sps, pps) = parse_avcc(avcc).ok_or("broken avcC")?;
        let mut format = MediaFormat::new();
        format.set_str("mime", "video/avc");
        format.set_i32("width", width as i32);
        format.set_i32("height", height as i32);
        format.set_buffer("csd-0", &sps);
        format.set_buffer("csd-1", &pps);
        let reader = ImageReader::new(width as i32, height as i32, ImageFormat::YUV_420_888, 4)
            .map_err(|e| err("image reader", e))?;
        let codec = MediaCodec::from_decoder_type("video/avc").ok_or("no H.264 decoder")?;
        let window = reader.window().map_err(|e| err("image reader", e))?;
        codec
            .configure(&format, Some(&window), MediaCodecDirection::Decoder)
            .map_err(|e| err("configuring the H.264 decoder", e))?;
        codec
            .start()
            .map_err(|e| err("starting the H.264 decoder", e))?;
        Ok(Decoder {
            codec,
            reader,
            nal_len,
            bt709: height >= 720,
            full_range: false,
            out: VecDeque::new(),
            annexb: Vec::new(),
        })
    }

    fn queue(&mut self, data: Option<&[u8]>, pts: f64) -> Result<(), String> {
        loop {
            match self
                .codec
                .dequeue_input_buffer(Duration::from_millis(10))
                .map_err(|e| err("decoding", e))?
            {
                DequeuedInputBufferResult::Buffer(mut b) => {
                    let (len, flags) = match data {
                        Some(d) => {
                            let dst = b.buffer_mut();
                            let n = d.len().min(dst.len());
                            for (o, i) in dst.iter_mut().zip(&d[..n]) {
                                o.write(*i);
                            }
                            (n, 0)
                        }
                        None => (0, END_OF_STREAM),
                    };
                    let us = (pts * 1e6).max(0.0) as u64;
                    return self
                        .codec
                        .queue_input_buffer(b, 0, len, us, flags)
                        .map_err(|e| err("decoding", e));
                }
                DequeuedInputBufferResult::TryAgainLater => self.drain(Duration::ZERO)?,
            }
        }
    }

    pub fn decode(&mut self, data: &[u8], pts: f64) -> Result<(), String> {
        let mut annexb = std::mem::take(&mut self.annexb);
        to_annexb(data, self.nal_len, &mut annexb);
        let r = self.queue(Some(&annexb), pts);
        self.annexb = annexb;
        r?;
        self.drain(Duration::ZERO)
    }

    pub fn finish(&mut self) -> Result<(), String> {
        self.queue(None, 0.0)?;
        self.drain(Duration::from_millis(100))
    }

    /// Takes the frames the codec has ready, waiting up to `wait` for each.
    fn drain(&mut self, wait: Duration) -> Result<(), String> {
        loop {
            let (us, eos) = match self
                .codec
                .dequeue_output_buffer(wait)
                .map_err(|e| err("decoding", e))?
            {
                DequeuedOutputBufferInfoResult::Buffer(b) => {
                    let info = *b.info();
                    // Rendering sends the picture to the image reader.
                    self.codec
                        .release_output_buffer(b, info.size() > 0)
                        .map_err(|e| err("decoding", e))?;
                    if info.size() == 0 {
                        return Ok(());
                    }
                    (
                        info.presentation_time_us(),
                        info.flags() & END_OF_STREAM != 0,
                    )
                }
                DequeuedOutputBufferInfoResult::OutputFormatChanged => {
                    let f = self.codec.output_format();
                    // MediaFormat's COLOR_STANDARD_BT709 is 1, COLOR_RANGE_FULL is 1.
                    if let Some(s) = f.i32("color-standard") {
                        self.bt709 = s == 1;
                    }
                    self.full_range = f.i32("color-range") == Some(1);
                    continue;
                }
                DequeuedOutputBufferInfoResult::OutputBuffersChanged => continue,
                DequeuedOutputBufferInfoResult::TryAgainLater => return Ok(()),
            };
            let deadline = Instant::now() + Duration::from_millis(100);
            loop {
                match self.reader.acquire_next_image() {
                    Ok(AcquireResult::Image(img)) => {
                        if let Some(f) = self.copy(&img, us as f64 / 1e6) {
                            self.out.push_back(f);
                        }
                        break;
                    }
                    _ if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(1)),
                    _ => {
                        log::warn!("decoded frame never reached the image reader");
                        break;
                    }
                }
            }
            if eos {
                return Ok(());
            }
        }
    }

    /// A YUV_420_888 image's crop as NV12.
    fn copy(&self, img: &Image, pts: f64) -> Option<Frame> {
        let crop = img.crop_rect().ok()?;
        let (x0, y0) = (crop.left.max(0) as usize, crop.top.max(0) as usize);
        let (w, h) = (
            (crop.right - crop.left).max(0) as usize,
            (crop.bottom - crop.top).max(0) as usize,
        );
        let plane = |i| {
            Some((
                img.plane_data(i).ok()?,
                img.plane_row_stride(i).ok()? as usize,
                img.plane_pixel_stride(i).ok()? as usize,
            ))
        };
        let (yd, ys, yp) = plane(0)?;
        let (ud, us, up) = plane(1)?;
        let (vd, vs, vp) = plane(2)?;
        let mut y = Vec::with_capacity(w * h);
        for r in y0..y0 + h {
            for c in x0..x0 + w {
                y.push(*yd.get(r * ys + c * yp)?);
            }
        }
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let mut uv = Vec::with_capacity(cw * ch * 2);
        for r in y0 / 2..y0 / 2 + ch {
            for c in x0 / 2..x0 / 2 + cw {
                uv.push(*ud.get(r * us + c * up)?);
                uv.push(*vd.get(r * vs + c * vp)?);
            }
        }
        Some(Frame {
            pts,
            width: w as u32,
            height: h as u32,
            sar: 1.0,
            bt709: self.bt709,
            full_range: self.full_range,
            planes: Planes::Cpu {
                y,
                y_stride: w,
                uv,
                uv_stride: cw * 2,
            },
        })
    }

    pub fn frame(&mut self) -> Option<Frame> {
        self.out.pop_front()
    }

    /// Frames are always copies here.
    pub fn copy_frames(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avcc_and_samples_to_annexb() {
        let avcc = [
            1, 0x64, 0, 0x1f, 0xff, 0xe1, 0, 2, 0x67, 0xaa, 1, 0, 1, 0x68,
        ];
        let (n, sps, pps) = parse_avcc(&avcc).unwrap();
        assert_eq!(n, 4);
        assert_eq!(sps, [0, 0, 0, 1, 0x67, 0xaa]);
        assert_eq!(pps, [0, 0, 0, 1, 0x68]);
        let mut out = Vec::new();
        to_annexb(&[0, 0, 0, 1, 0x65, 0, 0, 0, 2, 0x41, 7], 4, &mut out);
        assert_eq!(out, [0, 0, 0, 1, 0x65, 0, 0, 0, 1, 0x41, 7]);
    }
}
