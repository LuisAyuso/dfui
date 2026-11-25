use std::collections::VecDeque;
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::audio::{self, Audio};
use crate::convert::{Converter, Shown};
use crate::mp4::{Kind, Reader};

#[cfg(target_os = "android")]
use crate::mediacodec::Decoder as Hw;
#[cfg(target_os = "linux")]
use crate::vaapi::Decoder as Hw;

/// Converted frames waiting for their time; more would only hold GPU memory.
const AHEAD: usize = 3;

/// Plays one fragmented MP4 stream (H.264 + AAC): demuxes, decodes in
/// hardware and plays the sound, on its own threads. Cheap to clone; playback
/// stops when the last clone drops. Seeking is a new player on a new stream.
#[derive(Clone)]
pub struct Player(Arc<Handle>);

struct Handle(Arc<Shared>);

impl Drop for Handle {
    fn drop(&mut self) {
        self.0.stop.store(true, Ordering::Relaxed);
        self.0.cond.notify_all();
    }
}

struct Shared {
    /// Where the stream starts in the movie, seconds.
    start: f64,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    volume: Arc<AtomicU32>,
    audio: OnceLock<Option<Arc<Audio>>>,
    frames: Mutex<Frames<Shown>>,
    cond: Condvar,
    error: Mutex<Option<String>>,
}

trait Timed {
    fn pts(&self) -> f64;
}

impl Timed for Shown {
    fn pts(&self) -> f64 {
        self.pts
    }
}

/// What the audio output says, for [`Frames::advance`].
struct Sound {
    /// Stream time it has played up to; `None` before it starts.
    clock: Option<f64>,
    /// It has played everything it will get.
    drained: bool,
}

/// Decoded frames and the clock that picks which one shows.
struct Frames<T> {
    queue: VecDeque<T>,
    current: Option<T>,
    /// The clock without sound: pts at `since`, or frozen at `.0` while paused.
    /// Streams with sound switch to it once the sound runs out before the picture.
    wall: Option<(f64, Option<Instant>)>,
    demuxed: bool,
    decoded: bool,
}

impl<T> Default for Frames<T> {
    fn default() -> Self {
        Frames {
            queue: VecDeque::new(),
            current: None,
            wall: None,
            demuxed: false,
            decoded: false,
        }
    }
}

impl<T: Timed + Clone> Frames<T> {
    /// Stream time now, `None` before playback starts.
    fn clock(&self, sound: Option<&Sound>) -> Option<f64> {
        let wall = self
            .wall
            .map(|(t, since)| t + since.map_or(0.0, |s| s.elapsed().as_secs_f64()));
        match sound {
            Some(s) if self.wall.is_none() => s.clock,
            _ => wall,
        }
    }

    /// Moves to the frame due now and returns it.
    fn advance(&mut self, sound: Option<&Sound>, paused: bool) -> Option<T> {
        if let Some(s) = sound
            && self.wall.is_none()
            && self.demuxed
            && s.drained
            && let Some(t) = s.clock
        {
            self.wall = Some((t, (!paused).then(Instant::now)));
        }
        match self.clock(sound) {
            None => {
                if self.current.is_none() {
                    self.current = self.queue.pop_front();
                }
            }
            Some(t) => {
                while self.queue.front().is_some_and(|n| n.pts() <= t) {
                    self.current = self.queue.pop_front();
                }
            }
        }
        self.current.clone()
    }

    fn set_paused(&mut self, paused: bool) {
        if let Some((t, since)) = &mut self.wall {
            match (paused, *since) {
                (true, Some(at)) => (*t, *since) = (*t + at.elapsed().as_secs_f64(), None),
                (false, None) => *since = Some(Instant::now()),
                _ => {}
            }
        }
    }

    /// Drops frames made on a GPU that's gone.
    fn clear(&mut self) {
        self.queue.clear();
        self.current = None;
    }
}

enum Msg {
    Sample(Vec<u8>, f64),
    End,
}

impl Shared {
    fn fail(&self, e: String) {
        log::error!("playback: {e}");
        self.error.lock().unwrap().get_or_insert(e);
        self.stop.store(true, Ordering::Relaxed);
        self.cond.notify_all();
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn sound(&self) -> Option<Sound> {
        let a = self.audio.get()?.as_ref()?;
        Some(Sound {
            clock: a.go.load(Ordering::Relaxed).then(|| a.clock()).flatten(),
            drained: a.is_empty(),
        })
    }
}

impl Player {
    /// Starts playing `source`, a stream that begins `start` seconds into the movie.
    pub fn new(source: impl Read + Send + 'static, start: f64) -> Player {
        let shared = Arc::new(Shared {
            start,
            stop: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            volume: Arc::new(AtomicU32::new(1f32.to_bits())),
            audio: OnceLock::new(),
            frames: Mutex::new(Frames::default()),
            cond: Condvar::new(),
            error: Mutex::new(None),
        });
        let s = shared.clone();
        std::thread::Builder::new()
            .name("demux".into())
            .spawn(move || {
                if let Err(e) = demux(&s, source) {
                    s.fail(e);
                }
                s.frames.lock().unwrap().demuxed = true;
            })
            .expect("spawn demux thread");
        Player(Arc::new(Handle(shared)))
    }

    fn shared(&self) -> &Shared {
        &self.0.0
    }

    /// Position in the movie, seconds.
    pub fn position(&self) -> f64 {
        let s = self.shared();
        let sound = s.sound();
        let f = s.frames.lock().unwrap();
        let t = f
            .clock(sound.as_ref())
            .or(f.current.as_ref().map(|c| c.pts))
            .unwrap_or(0.0);
        s.start + t.max(0.0)
    }

    pub fn paused(&self) -> bool {
        self.shared().paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        let s = self.shared();
        s.paused.store(paused, Ordering::Relaxed);
        s.frames.lock().unwrap().set_paused(paused);
    }

    /// Linear gain, 1 is unchanged.
    pub fn set_volume(&self, v: f32) {
        self.shared().volume.store(v.to_bits(), Ordering::Relaxed);
    }

    /// Why playback stopped, if it failed.
    pub fn error(&self) -> Option<String> {
        self.shared().error.lock().unwrap().clone()
    }

    /// Everything in the stream has been shown and heard, or playback failed.
    pub fn ended(&self) -> bool {
        let s = self.shared();
        let f = s.frames.lock().unwrap();
        let heard = match s.audio.get() {
            Some(Some(a)) => a.is_empty(),
            _ => true,
        };
        s.error.lock().unwrap().is_some() || f.demuxed && f.decoded && f.queue.is_empty() && heard
    }

    /// The frame to show now, moving on as the clock passes the next one's time.
    pub fn frame(&self) -> Option<Shown> {
        let s = self.shared();
        let sound = s.sound();
        let shown = s
            .frames
            .lock()
            .unwrap()
            .advance(sound.as_ref(), self.paused());
        s.cond.notify_all();
        shown
    }
}

fn demux(s: &Arc<Shared>, source: impl Read) -> Result<(), String> {
    let mut r = Reader::new(source).map_err(|e| format!("reading the stream: {e}"))?;
    let video = r.tracks.iter().find_map(|t| match &t.kind {
        Kind::Video {
            avcc,
            width,
            height,
        } => Some((t.id, avcc.clone(), (*width, *height))),
        _ => None,
    });
    let sound = r.tracks.iter().find_map(|t| match &t.kind {
        Kind::Audio {
            rate,
            channels,
            config,
        } => Some((t.id, *rate, *channels as usize, config.clone())),
        _ => None,
    });
    let mut aac = None;
    let audio = match &sound {
        Some((_, rate, channels, config)) => {
            aac = Some(audio::Aac::new(config)?);
            let out = audio::open(
                *rate,
                *channels,
                s.paused.clone(),
                s.volume.clone(),
                s.stop.clone(),
            );
            // A missing or busy output shouldn't stop the picture.
            out.inspect_err(|e| log::warn!("playing without sound: {e}"))
                .ok()
        }
        None => None,
    };
    let _ = s.audio.set(audio.clone());
    let tx = match &video {
        Some((_, avcc, size)) => {
            let (tx, rx) = sync_channel(256);
            let (s2, avcc, size) = (s.clone(), avcc.clone(), *size);
            std::thread::Builder::new()
                .name("video decode".into())
                .spawn(move || {
                    if let Err(e) = decode(&s2, &avcc, size, rx) {
                        s2.fail(e);
                    }
                    s2.frames.lock().unwrap().decoded = true;
                })
                .map_err(|e| e.to_string())?;
            Some(tx)
        }
        None => {
            s.frames.lock().unwrap().decoded = true;
            if let Some(a) = &audio {
                a.go.store(true, Ordering::Relaxed);
            }
            None
        }
    };
    let send = |tx: &SyncSender<Msg>, m| tx.send(m).map_err(|_| ());
    let mut pcm = Vec::new();
    while !s.stopped() {
        let Some(sample) = r.sample().map_err(|e| format!("reading the stream: {e}"))? else {
            break;
        };
        if let Some(tx) = &tx
            && video.as_ref().is_some_and(|v| v.0 == sample.track)
        {
            if send(tx, Msg::Sample(sample.data, sample.pts)).is_err() {
                break;
            }
        } else if let (Some(a), Some(dec), Some(snd)) = (&audio, &mut aac, &sound)
            && snd.0 == sample.track
        {
            let (samples, ch) = dec.decode(&sample.data);
            pcm.clear();
            audio::remix(samples, ch, a.channels, &mut pcm);
            a.push(sample.pts, &pcm, &s.stop);
        }
    }
    if let Some(tx) = &tx {
        let _ = send(tx, Msg::End);
    }
    Ok(())
}

fn decode(
    s: &Arc<Shared>,
    avcc: &[u8],
    (width, height): (u32, u32),
    rx: Receiver<Msg>,
) -> Result<(), String> {
    // The window sets the GPU up shortly after start.
    let (device, queue) = loop {
        if s.stopped() {
            return Ok(());
        }
        if let Some(g) = dfui::gpu() {
            break g;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut conv = Converter::new(device, queue);
    let mut dec = Hw::new(avcc, width, height)?;
    for msg in rx {
        if s.stopped() {
            break;
        }
        match msg {
            Msg::Sample(data, pts) => dec.decode(&data, pts)?,
            Msg::End => dec.finish()?,
        }
        while let Some(frame) = dec.frame() {
            if let Some((d, q)) = dfui::gpu()
                && d != conv.device
            {
                conv = Converter::new(d, q);
                s.frames.lock().unwrap().clear();
            }
            let Some(shown) = conv.convert(&frame) else {
                if conv.import_failed {
                    dec.copy_frames();
                }
                continue;
            };
            let mut f = s.frames.lock().unwrap();
            while f.queue.len() >= AHEAD && !s.stopped() {
                f = s
                    .cond
                    .wait_timeout(f, Duration::from_millis(100))
                    .unwrap()
                    .0;
            }
            if f.wall.is_none() && !matches!(s.audio.get(), Some(Some(_))) {
                let since = (!s.paused.load(Ordering::Relaxed)).then(Instant::now);
                f.wall = Some((shown.pts, since));
            }
            f.queue.push_back(shown);
            if let Some(Some(a)) = s.audio.get() {
                a.go.store(true, Ordering::Relaxed);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Timed for f64 {
        fn pts(&self) -> f64 {
            *self
        }
    }

    fn frames(pts: &[f64]) -> Frames<f64> {
        Frames {
            queue: pts.iter().copied().collect(),
            ..Frames::default()
        }
    }

    fn sound(clock: f64, drained: bool) -> Sound {
        Sound {
            clock: Some(clock),
            drained,
        }
    }

    #[test]
    fn shows_the_first_frame_before_the_clock_starts() {
        let mut f = frames(&[0.1, 0.2]);
        let silent = Sound {
            clock: None,
            drained: false,
        };
        assert_eq!(f.advance(Some(&silent), false), Some(0.1));
        assert_eq!(f.advance(Some(&silent), false), Some(0.1));
    }

    #[test]
    fn follows_the_audio_clock_and_skips_late_frames() {
        let mut f = frames(&[0.0, 0.04, 0.08, 0.12]);
        assert_eq!(f.advance(Some(&sound(0.05, false)), false), Some(0.04));
        assert_eq!(f.advance(Some(&sound(0.13, false)), false), Some(0.12));
        assert!(f.queue.is_empty());
    }

    #[test]
    fn waits_for_sound_that_is_still_coming() {
        // Starved mid-stream: the picture holds with the sound.
        let mut f = frames(&[0.0, 1.0]);
        assert_eq!(f.advance(Some(&sound(0.5, true)), false), Some(0.0));
        assert!(f.wall.is_none());
    }

    #[test]
    fn plays_past_the_end_of_the_sound() {
        let mut f = frames(&[0.0, 1.0, 1.02]);
        f.demuxed = true;
        assert_eq!(f.advance(Some(&sound(0.98, true)), false), Some(0.0));
        assert!(f.wall.is_some(), "switched to the wall clock");
        std::thread::sleep(Duration::from_millis(60));
        // The sound output keeps reporting where it stopped.
        assert_eq!(f.advance(Some(&sound(0.98, true)), false), Some(1.02));
        assert!(f.queue.is_empty());
    }

    #[test]
    fn the_wall_clock_stops_while_paused() {
        let mut f = frames(&[0.0, 0.03]);
        f.wall = Some((0.0, None));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(f.advance(None, true), Some(0.0));
        f.set_paused(false);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(f.advance(None, false), Some(0.03));
    }

    #[test]
    fn a_new_gpu_drops_old_frames() {
        let mut f = frames(&[0.0, 0.1]);
        f.advance(None, false);
        f.clear();
        assert_eq!(f.advance(None, false), None);
    }
}
