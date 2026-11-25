//! Hardware-decoded video for dfui: plays fragmented MP4 (H.264 or HEVC, with
//! AAC sound), as the movies-thing server streams it, drawing frames as dfui
//! bitmaps. VA-API through libavcodec on Linux, MediaCodec on Android. HDR
//! films go out as HDR when [`dfui::hdr`] is on and are tone-mapped otherwise.

mod audio;
mod convert;
#[cfg(target_os = "android")]
mod mediacodec;
pub mod mp4;
mod player;
#[cfg(target_os = "linux")]
mod vaapi;
mod widget;

pub use convert::Shown;

/// What the server may send this player as is, for a stream request's
/// `codecs`: H.264 and AAC always, HEVC (10-bit too) where the hardware
/// decodes it or the CPU is fast enough to. The answer is worked out once.
pub fn codecs() -> &'static str {
    static CODECS: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    CODECS.get_or_init(|| {
        #[cfg(target_os = "linux")]
        if vaapi::Decoder::hevc10() {
            return "h264,hevc,aac";
        }
        "h264,aac"
    })
}
pub use player::Player;
pub use widget::{Video, video};

/// A decoded picture, before conversion to RGBA.
pub(crate) struct Frame {
    /// Seconds.
    pts: f64,
    width: u32,
    height: u32,
    /// Pixel aspect ratio.
    sar: f32,
    color: Color,
    planes: Planes,
}

/// How a frame's samples become colors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Color {
    matrix: Matrix,
    transfer: Transfer,
    full_range: bool,
    /// Bits per sample: 8, or 10 in the high bits of 16-bit (P010) planes.
    bits: u8,
    /// The brightest the film gets, nits, from its HDR metadata; 0 when unknown.
    peak: f32,
}

/// Y'CbCr to R'G'B' coefficients. BT.2020 also means BT.2020 primaries.
// Android decodes 8-bit SDR only so far.
#[cfg_attr(target_os = "android", allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Matrix {
    Bt601,
    Bt709,
    Bt2020,
}

#[cfg_attr(target_os = "android", allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Transfer {
    /// BT.709/sRGB-like gamma, shown as is.
    Sdr,
    /// SMPTE ST 2084 (HDR10).
    Pq,
    /// ARIB STD-B67 hybrid log-gamma.
    Hlg,
}

pub(crate) enum Planes {
    /// NV12 (or P010, two bytes a sample): full-size luma rows, then half-size
    /// rows of interleaved Cb Cr. Strides are in bytes.
    Cpu {
        y: Vec<u8>,
        y_stride: usize,
        uv: Vec<u8>,
        uv_stride: usize,
    },
    #[cfg(target_os = "linux")]
    Dmabuf(Dmabuf),
}

#[cfg(target_os = "linux")]
pub(crate) struct Dmabuf {
    /// Y, then interleaved UV.
    planes: [DmaPlane; 2],
    /// Keeps the fds open and the decoder off this memory.
    _keep: Box<dyn Send>,
}

#[cfg(target_os = "linux")]
pub(crate) struct DmaPlane {
    fd: std::os::fd::RawFd,
    offset: u64,
    pitch: u64,
    modifier: u64,
}
