//! Hardware-decoded video for dfui: plays the fragmented MP4 (H.264 + AAC)
//! that the movies-thing server streams, drawing frames as dfui bitmaps.
//! VA-API through libavcodec on Linux, MediaCodec on Android.

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
    /// BT.709 colours; else BT.601.
    bt709: bool,
    full_range: bool,
    planes: Planes,
}

pub(crate) enum Planes {
    /// NV12: full-size luma rows, then half-size rows of interleaved Cb Cr.
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
