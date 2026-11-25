//! AAC through symphonia to the default output through cpal. The frames the
//! output has played are the playback clock.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_AAC, CodecParameters, DecoderOptions};
use symphonia::core::formats::Packet;

pub(crate) struct Aac {
    dec: Box<dyn symphonia::core::codecs::Decoder>,
    buf: Option<SampleBuffer<f32>>,
}

impl Aac {
    pub fn new(config: &[u8]) -> Result<Aac, String> {
        let mut params = CodecParameters::new();
        params
            .for_codec(CODEC_TYPE_AAC)
            .with_extra_data(config.into());
        let dec = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|e| format!("AAC decoder: {e}"))?;
        Ok(Aac { dec, buf: None })
    }

    /// One AAC frame as interleaved samples and their channel count; empty if it's broken.
    pub fn decode(&mut self, data: &[u8]) -> (&[f32], usize) {
        let decoded = match self.dec.decode(&Packet::new_from_slice(0, 0, 0, data)) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("AAC frame: {e}");
                return (&[], 1);
            }
        };
        let spec = *decoded.spec();
        let frames = decoded.capacity() as u64;
        let buf = match &mut self.buf {
            Some(b) if b.capacity() >= frames as usize * spec.channels.count() => b,
            b => b.insert(SampleBuffer::new(frames, spec)),
        };
        buf.copy_interleaved_ref(decoded);
        (buf.samples(), spec.channels.count())
    }
}

/// Folds `src` (interleaved, `from` channels, in the FL FR FC LFE … order
/// symphonia uses) into `to` channels.
pub(crate) fn remix(src: &[f32], from: usize, to: usize, out: &mut impl Extend<f32>) {
    const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
    for f in src.chunks_exact(from.max(1)) {
        let c = |i: usize| f.get(i).copied().unwrap_or(0.0);
        match (from, to) {
            (a, b) if a == b => out.extend(f.iter().copied()),
            (1, _) => out.extend((0..to).map(|_| f[0])),
            // ponytail: plain fold of centre and surrounds, no LFE; the server's mixes do better.
            (_, 2) => {
                let s = 1.0 / (1.0 + 2.0 * H);
                out.extend([
                    (c(0) + H * c(2) + H * c(4)) * s,
                    (c(1) + H * c(2) + H * c(5)) * s,
                ]);
            }
            _ => out.extend((0..to).map(c)),
        }
    }
}

/// Samples between the decoder and the output, and what the output has played.
pub(crate) struct Audio {
    ring: Mutex<VecDeque<f32>>,
    space: Condvar,
    pub channels: usize,
    pub rate: u32,
    played: AtomicU64,
    /// Pts of the first sample, as f64 bits; NaN until one arrives.
    first: AtomicU64,
    /// Plays once the first video frame is ready and while not paused.
    pub go: AtomicBool,
    pub paused: Arc<AtomicBool>,
    pub volume: Arc<AtomicU32>,
}

impl Audio {
    /// Seconds played, from the first sample's pts; `None` before any sample.
    pub fn clock(&self) -> Option<f64> {
        let first = f64::from_bits(self.first.load(Ordering::Acquire));
        (!first.is_nan())
            .then(|| first + self.played.load(Ordering::Relaxed) as f64 / self.rate as f64)
    }

    pub fn is_empty(&self) -> bool {
        self.ring.lock().unwrap().is_empty()
    }

    /// Queues `samples` (already in the output's layout), waiting while a second
    /// is queued once playback has started. Before that it never waits: the
    /// first video frame can lie further into the stream than a second of sound,
    /// and sound starts only with it.
    pub fn push(&self, pts: f64, samples: &[f32], stop: &AtomicBool) {
        let _ = self.first.compare_exchange(
            f64::NAN.to_bits(),
            pts.to_bits(),
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
        let cap = self.rate as usize * self.channels;
        let mut ring = self.ring.lock().unwrap();
        while ring.len() >= cap && self.go.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed)
        {
            ring = self
                .space
                .wait_timeout(ring, std::time::Duration::from_millis(100))
                .unwrap()
                .0;
        }
        ring.extend(samples);
    }

    fn fill(&self, out: &mut [f32]) {
        let mut n = 0;
        if self.go.load(Ordering::Relaxed) && !self.paused.load(Ordering::Relaxed) {
            let v = f32::from_bits(self.volume.load(Ordering::Relaxed));
            let mut ring = self.ring.lock().unwrap();
            // Whole frames only, so channels never swap.
            n = out.len().min(ring.len()) / self.channels * self.channels;
            for (o, s) in out.iter_mut().zip(ring.drain(..n)) {
                *o = s * v;
            }
            self.space.notify_one();
        }
        out[n..].fill(0.0);
        self.played
            .fetch_add((n / self.channels) as u64, Ordering::Relaxed);
    }
}

/// Opens the default output for `rate` and `channels` (or stereo), playing
/// from the returned [`Audio`] until `stop`.
pub(crate) fn open(
    rate: u32,
    channels: usize,
    paused: Arc<AtomicBool>,
    volume: Arc<AtomicU32>,
    stop: Arc<AtomicBool>,
) -> Result<Arc<Audio>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    // cpal streams aren't Send everywhere, so the stream lives on its own thread.
    std::thread::Builder::new()
        .name("audio out".into())
        .spawn(move || {
            let res = (|| {
                let device = cpal::default_host()
                    .default_output_device()
                    .ok_or("no audio output")?;
                let fits = |ch: usize| {
                    device.supported_output_configs().ok()?.find(|c| {
                        c.channels() as usize == ch
                            && c.sample_format() == cpal::SampleFormat::F32
                            && (c.min_sample_rate()..=c.max_sample_rate()).contains(&rate)
                    })
                };
                // ponytail: no resampling; PipeWire and AAudio take any rate, plain ALSA may not.
                let out_channels = match (fits(channels), fits(2)) {
                    (Some(_), _) => channels,
                    (None, Some(_)) => 2,
                    _ => return Err(format!("the audio output can't play {rate} Hz")),
                };
                let audio = Arc::new(Audio {
                    ring: Mutex::new(VecDeque::new()),
                    space: Condvar::new(),
                    channels: out_channels,
                    rate,
                    played: AtomicU64::new(0),
                    first: AtomicU64::new(f64::NAN.to_bits()),
                    go: AtomicBool::new(false),
                    paused,
                    volume,
                });
                let a = audio.clone();
                let stream = device
                    .build_output_stream(
                        cpal::StreamConfig {
                            channels: out_channels as u16,
                            sample_rate: rate,
                            buffer_size: cpal::BufferSize::Default,
                        },
                        move |out: &mut [f32], _| a.fill(out),
                        |e| log::warn!("audio output: {e}"),
                        None,
                    )
                    .map_err(|e| format!("audio output: {e}"))?;
                stream.play().map_err(|e| format!("audio output: {e}"))?;
                Ok((audio, stream))
            })();
            match res {
                Ok((audio, _stream)) => {
                    let _ = tx.send(Ok(audio));
                    while !stop.load(Ordering::Relaxed) {
                        std::thread::park_timeout(std::time::Duration::from_millis(100));
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    rx.recv().map_err(|_| "audio thread died".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remix_keeps_frames_whole() {
        let mut out = Vec::new();
        remix(&[0.5, 0.25], 1, 2, &mut out);
        assert_eq!(out, [0.5, 0.5, 0.25, 0.25]);
        out.clear();
        // 5.1: dialogue in the centre reaches both sides equally.
        remix(&[0.0, 0.0, 1.0, 0.0, 0.0, 0.0], 6, 2, &mut out);
        assert_eq!(out.len(), 2);
        assert!(out[0] > 0.2 && out[0] == out[1]);
    }

    #[test]
    fn decodes_the_clips_aac() {
        let mut r = crate::mp4::Reader::new(&include_bytes!("../tests/clip.mp4")[..]).unwrap();
        let (id, config) = r
            .tracks
            .iter()
            .find_map(|t| match &t.kind {
                crate::mp4::Kind::Audio { config, .. } => Some((t.id, config.clone())),
                _ => None,
            })
            .unwrap();
        let mut aac = Aac::new(&config).unwrap();
        let (mut frames, mut peak) = (0, 0f32);
        while let Some(s) = r.sample().unwrap() {
            if s.track == id {
                let (pcm, ch) = aac.decode(&s.data);
                assert_eq!(ch, 2);
                frames += pcm.len() / 2;
                peak = pcm.iter().fold(peak, |m, x| m.max(x.abs()));
            }
        }
        // 48 AAC frames of 1024 samples; the 440 Hz sine is loud.
        assert_eq!(frames, 48 * 1024);
        assert!(peak > 0.1, "{peak}");
    }

    fn silent_output() -> Audio {
        Audio {
            ring: Mutex::new(VecDeque::new()),
            space: Condvar::new(),
            channels: 2,
            rate: 48000,
            played: AtomicU64::new(0),
            first: AtomicU64::new(f64::NAN.to_bits()),
            go: AtomicBool::new(false),
            paused: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(AtomicU32::new(1f32.to_bits())),
        }
    }

    #[test]
    fn sound_queues_freely_until_playback_starts() {
        let audio = Arc::new(silent_output());
        let a = audio.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let stop = AtomicBool::new(false);
            // Three seconds of sound before the first video frame: more than the ring holds.
            for i in 0..3 {
                a.push(i as f64, &vec![0.0; 48000 * 2], &stop);
            }
            tx.send(()).unwrap();
        });
        let done = rx.recv_timeout(std::time::Duration::from_secs(2));
        assert!(done.is_ok(), "push blocked before playback started");
        assert_eq!(audio.ring.lock().unwrap().len(), 3 * 48000 * 2);
    }
}
