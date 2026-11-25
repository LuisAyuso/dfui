//! Reader for the fragmented MP4 that ffmpeg writes with
//! `-movflags frag_keyframe+empty_moov+default_base_moof`, from a stream that
//! can't seek: `moov` first, then `moof`/`mdat` pairs.

use std::collections::HashMap;
use std::io::{self, Read};

#[derive(Clone, Debug)]
pub struct Track {
    pub id: u32,
    pub timescale: u32,
    pub kind: Kind,
}

#[derive(Clone, Debug)]
pub enum Kind {
    /// H.264; `avcc` is the `avcC` box's payload (SPS, PPS, NAL length size).
    Video {
        width: u32,
        height: u32,
        avcc: Vec<u8>,
    },
    /// AAC; `config` is the AudioSpecificConfig.
    Audio {
        rate: u32,
        channels: u16,
        config: Vec<u8>,
    },
    Other,
}

#[derive(Debug)]
pub struct Sample {
    pub track: u32,
    /// Presentation time, seconds.
    pub pts: f64,
    dts: f64,
    pub key: bool,
    /// For video, NAL units each prefixed by their length.
    pub data: Vec<u8>,
}

/// A box's type, the offset of its first byte, and its payload.
type RawBox = ([u8; 4], u64, Vec<u8>);

#[derive(Clone, Copy, Default)]
struct Defaults {
    duration: u32,
    size: u32,
    flags: u32,
}

pub struct Reader<R> {
    r: R,
    /// Bytes read so far, for `trun` data offsets.
    pos: u64,
    pub tracks: Vec<Track>,
    trex: HashMap<u32, Defaults>,
    /// Ticks to subtract from each track's times, from its edit list.
    shift: HashMap<u32, i64>,
    ready: std::collections::VecDeque<Sample>,
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Big-endian reads from a box payload; short payloads read as zeros.
struct Buf<'a>(&'a [u8]);

impl Buf<'_> {
    fn take(&mut self, n: usize) -> &[u8] {
        let n = n.min(self.0.len());
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        a
    }
    fn uint(&mut self, n: usize) -> u64 {
        self.take(n).iter().fold(0, |v, &b| v << 8 | b as u64)
    }
    fn u8(&mut self) -> u8 {
        self.uint(1) as u8
    }
    fn u16(&mut self) -> u16 {
        self.uint(2) as u16
    }
    fn u32(&mut self) -> u32 {
        self.uint(4) as u32
    }
    fn u64(&mut self) -> u64 {
        self.uint(8)
    }
    fn skip(&mut self, n: usize) {
        self.take(n);
    }
}

/// The child boxes in `data`: (type, payload).
fn boxes(mut data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    std::iter::from_fn(move || {
        if data.len() < 8 {
            return None;
        }
        let mut b = Buf(data);
        let size = b.u32() as u64;
        let kind: [u8; 4] = b.take(4).try_into().ok()?;
        let (head, size) = match size {
            1 => (16, b.u64()),
            0 => (8, data.len() as u64),
            s => (8, s),
        };
        if size < head || size > data.len() as u64 {
            return None;
        }
        let payload = &data[head as usize..size as usize];
        data = &data[size as usize..];
        Some((kind, payload))
    })
}

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(data).find(|(k, _)| k == kind).map(|(_, p)| p)
}

/// An MPEG-4 descriptor's tag and body.
fn descriptor<'a>(b: &mut Buf<'a>) -> (u8, &'a [u8]) {
    let tag = b.u8();
    let mut len = 0usize;
    for _ in 0..4 {
        let c = b.u8();
        len = len << 7 | (c & 0x7f) as usize;
        if c & 0x80 == 0 {
            break;
        }
    }
    let n = len.min(b.0.len());
    let (body, rest) = b.0.split_at(n);
    b.0 = rest;
    (tag, body)
}

/// The AudioSpecificConfig inside an `esds` payload.
fn esds_config(esds: &[u8]) -> Option<Vec<u8>> {
    let mut b = Buf(esds);
    b.skip(4);
    let (tag, es) = descriptor(&mut b);
    if tag != 3 {
        return None;
    }
    let mut es = Buf(es);
    es.skip(2);
    let flags = es.u8();
    if flags & 0x80 != 0 {
        es.skip(2);
    }
    if flags & 0x40 != 0 {
        let n = es.u8() as usize;
        es.skip(n);
    }
    if flags & 0x20 != 0 {
        es.skip(2);
    }
    let (tag, dc) = descriptor(&mut es);
    if tag != 4 {
        return None;
    }
    let mut dc = Buf(dc);
    dc.skip(13);
    let (tag, asc) = descriptor(&mut dc);
    (tag == 5).then(|| asc.to_vec())
}

fn track(trak: &[u8]) -> Option<(Track, i64)> {
    let mut tkhd = Buf(child(trak, b"tkhd")?);
    let id = match tkhd.u8() {
        1 => {
            tkhd.skip(3 + 16);
            tkhd.u32()
        }
        _ => {
            tkhd.skip(3 + 8);
            tkhd.u32()
        }
    };
    let mdia = child(trak, b"mdia")?;
    let mut mdhd = Buf(child(mdia, b"mdhd")?);
    let timescale = match mdhd.u8() {
        1 => {
            mdhd.skip(3 + 16);
            mdhd.u32()
        }
        _ => {
            mdhd.skip(3 + 8);
            mdhd.u32()
        }
    };
    let shift = child(trak, b"edts")
        .and_then(|e| child(e, b"elst"))
        .and_then(|elst| {
            let mut b = Buf(elst);
            let v = b.u8();
            b.skip(3);
            // An empty edit (media_time -1) delays the track; it comes before the real one.
            (0..b.u32()).find_map(|_| {
                let (_dur, time) = match v {
                    1 => (b.u64(), b.u64() as i64),
                    _ => (b.u32() as u64, b.u32() as i32 as i64),
                };
                b.skip(4);
                (time >= 0).then_some(time)
            })
        })
        .unwrap_or(0);
    let stsd = child(child(child(mdia, b"minf")?, b"stbl")?, b"stsd")?;
    let (entry_kind, entry) = boxes(stsd.get(8..)?).next()?;
    let kind = match &entry_kind {
        b"avc1" | b"avc3" => {
            let mut b = Buf(entry);
            b.skip(24);
            let (width, height) = (b.u16() as u32, b.u16() as u32);
            let avcc = child(entry.get(78..)?, b"avcC")?.to_vec();
            Kind::Video {
                width,
                height,
                avcc,
            }
        }
        b"mp4a" => {
            let mut b = Buf(entry);
            b.skip(16);
            let channels = b.u16();
            b.skip(6);
            let rate = b.u32() >> 16;
            let config = esds_config(child(entry.get(28..)?, b"esds")?)?;
            Kind::Audio {
                rate,
                channels,
                config,
            }
        }
        _ => Kind::Other,
    };
    Some((
        Track {
            id,
            timescale,
            kind,
        },
        shift,
    ))
}

impl<R: Read> Reader<R> {
    /// Reads up to the end of `moov`, which lists the tracks.
    pub fn new(r: R) -> io::Result<Reader<R>> {
        let mut me = Reader {
            r,
            pos: 0,
            tracks: Vec::new(),
            trex: HashMap::new(),
            shift: HashMap::new(),
            ready: Default::default(),
        };
        loop {
            let Some((kind, _, payload)) = me.next_box()? else {
                return Err(bad("no moov"));
            };
            if &kind != b"moov" {
                continue;
            }
            for (k, p) in boxes(&payload) {
                match &k {
                    b"trak" => {
                        if let Some((t, shift)) = track(p) {
                            me.shift.insert(t.id, shift);
                            me.tracks.push(t);
                        }
                    }
                    b"mvex" => {
                        for (_, trex) in boxes(p).filter(|(k, _)| k == b"trex") {
                            let mut b = Buf(trex);
                            b.skip(4);
                            let id = b.u32();
                            b.skip(4);
                            let d = Defaults {
                                duration: b.u32(),
                                size: b.u32(),
                                flags: b.u32(),
                            };
                            me.trex.insert(id, d);
                        }
                    }
                    _ => {}
                }
            }
            return Ok(me);
        }
    }

    /// The next box: type, offset of its first byte, payload. `None` at a clean end.
    fn next_box(&mut self) -> io::Result<Option<RawBox>> {
        let start = self.pos;
        let mut head = [0; 8];
        match self.r.read_exact(&mut head) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        self.pos += 8;
        let mut size = u32::from_be_bytes(head[..4].try_into().unwrap()) as u64;
        let kind: [u8; 4] = head[4..].try_into().unwrap();
        let mut header = 8;
        if size == 1 {
            let mut large = [0; 8];
            self.r.read_exact(&mut large)?;
            self.pos += 8;
            size = u64::from_be_bytes(large);
            header = 16;
        }
        let payload = match size {
            0 => {
                let mut v = Vec::new();
                self.r.read_to_end(&mut v)?;
                v
            }
            s if s < header => return Err(bad("box smaller than its header")),
            // ponytail: a whole fragment in memory; ffmpeg's are one GOP, a few MB at most.
            s if s - header > 1 << 30 => return Err(bad("box over 1 GiB")),
            s => {
                let mut v = vec![0; (s - header) as usize];
                self.r.read_exact(&mut v)?;
                v
            }
        };
        self.pos += payload.len() as u64;
        Ok(Some((kind, start, payload)))
    }

    /// The next sample in decode order across tracks; `None` at the end of the stream.
    pub fn sample(&mut self) -> io::Result<Option<Sample>> {
        while self.ready.is_empty() {
            let Some((kind, start, moof)) = self.next_box()? else {
                return Ok(None);
            };
            if &kind != b"moof" {
                continue;
            }
            let Some((kind, mdat_start, mdat)) = self.next_box()? else {
                return Err(bad("moof without the mdat after it"));
            };
            if &kind != b"mdat" {
                return Err(bad("moof without the mdat after it"));
            }
            let mdat_data = mdat_start + 8;
            let mut samples = self.fragment(&moof, start, mdat_data, &mdat)?;
            samples.sort_by(|a, b| a.dts.total_cmp(&b.dts));
            self.ready.extend(samples);
        }
        Ok(self.ready.pop_front())
    }

    fn fragment(
        &self,
        moof: &[u8],
        moof_start: u64,
        mdat_start: u64,
        mdat: &[u8],
    ) -> io::Result<Vec<Sample>> {
        let mut out = Vec::new();
        for (_, traf) in boxes(moof).filter(|(k, _)| k == b"traf") {
            let Some(tfhd) = child(traf, b"tfhd") else {
                continue;
            };
            let mut b = Buf(tfhd);
            let flags = b.u32() & 0xff_ffff;
            let id = b.u32();
            let Some(track) = self.tracks.iter().find(|t| t.id == id) else {
                continue;
            };
            let mut d = self.trex.get(&id).copied().unwrap_or_default();
            // Without an explicit base, offsets count from the moof (default-base-is-moof).
            let base = if flags & 0x1 != 0 {
                b.u64()
            } else {
                moof_start
            };
            if flags & 0x2 != 0 {
                b.skip(4);
            }
            if flags & 0x8 != 0 {
                d.duration = b.u32();
            }
            if flags & 0x10 != 0 {
                d.size = b.u32();
            }
            if flags & 0x20 != 0 {
                d.flags = b.u32();
            }
            let mut dts = child(traf, b"tfdt").map_or(0, |t| {
                let mut b = Buf(t);
                match b.u8() {
                    1 => {
                        b.skip(3);
                        b.u64()
                    }
                    _ => {
                        b.skip(3);
                        b.u32() as u64
                    }
                }
            }) as i64;
            let scale = track.timescale.max(1) as f64;
            let shift = self.shift.get(&id).copied().unwrap_or(0);
            for (_, trun) in boxes(traf).filter(|(k, _)| k == b"trun") {
                let mut b = Buf(trun);
                let head = b.u32();
                let (version, tf) = (head >> 24, head & 0xff_ffff);
                let count = b.u32();
                let mut at = base;
                if tf & 0x1 != 0 {
                    at = base.wrapping_add(b.u32() as i32 as i64 as u64);
                }
                let first_flags = (tf & 0x4 != 0).then(|| b.u32());
                for i in 0..count {
                    let duration = if tf & 0x100 != 0 { b.u32() } else { d.duration };
                    let size = if tf & 0x200 != 0 { b.u32() } else { d.size };
                    let mut sflags = if tf & 0x400 != 0 { b.u32() } else { d.flags };
                    if i == 0
                        && let Some(f) = first_flags
                    {
                        sflags = f;
                    }
                    let cts = match (tf & 0x800 != 0, version) {
                        (false, _) => 0,
                        (true, 0) => b.u32() as i64,
                        (true, _) => b.u32() as i32 as i64,
                    };
                    let from = at
                        .checked_sub(mdat_start)
                        .ok_or_else(|| bad("sample before its mdat"))?
                        as usize;
                    let data = mdat
                        .get(from..from + size as usize)
                        .ok_or_else(|| bad("sample past its mdat"))?;
                    out.push(Sample {
                        track: id,
                        pts: (dts + cts - shift) as f64 / scale,
                        dts: (dts - shift) as f64 / scale,
                        key: sflags & 0x1_0000 == 0,
                        data: data.to_vec(),
                    });
                    at += size as u64;
                    dts += duration as i64;
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made by ffmpeg with the server's movflags: 64x48 H.264 with B-frames at 24 fps,
    /// stereo AAC at 48 kHz, one second.
    const CLIP: &[u8] = include_bytes!("../tests/clip.mp4");

    #[test]
    fn reads_the_servers_fragments() {
        let mut r = Reader::new(CLIP).unwrap();
        let video = r
            .tracks
            .iter()
            .find_map(|t| match &t.kind {
                Kind::Video {
                    width,
                    height,
                    avcc,
                } => Some((t.id, *width, *height, avcc.clone())),
                _ => None,
            })
            .unwrap();
        assert_eq!((video.1, video.2), (64, 48));
        assert_eq!(video.3[0], 1, "avcC version");
        let audio = r
            .tracks
            .iter()
            .find_map(|t| match &t.kind {
                Kind::Audio {
                    rate,
                    channels,
                    config,
                } => Some((t.id, *rate, *channels, config.clone())),
                _ => None,
            })
            .unwrap();
        assert_eq!((audio.1, audio.2), (48000, 2));
        // AAC-LC (2) at 48 kHz (index 3), two channels.
        assert_eq!(audio.3[..2], [0x11, 0x90]);

        let mut all = Vec::new();
        while let Some(s) = r.sample().unwrap() {
            all.push(s);
        }
        let v: Vec<&Sample> = all.iter().filter(|s| s.track == video.0).collect();
        let a: Vec<&Sample> = all.iter().filter(|s| s.track == audio.0).collect();
        assert_eq!((v.len(), a.len()), (24, 48));
        assert!(v[0].key && !v[1].key);
        // Decode order, with B-frames shown before the P-frame decoded ahead of them (from ffprobe).
        let pts: Vec<f64> = v[..4].iter().map(|s| s.pts).collect();
        for (p, want) in pts.iter().zip([0.083333, 0.208333, 0.125, 0.166667]) {
            assert!((p - want).abs() < 1e-4, "{pts:?}");
        }
        // The first NAL's length prefix covers the rest of a one-NAL sample at most.
        let len = u32::from_be_bytes(v[0].data[..4].try_into().unwrap()) as usize;
        assert!(len + 4 <= v[0].data.len());
        // Tracks are interleaved by decode time, not one after the other.
        let first_audio = all.iter().position(|s| s.track == audio.0).unwrap();
        assert!(first_audio < 4, "audio starts at {first_audio}");
    }

    #[test]
    fn a_cut_stream_ends_with_an_error() {
        let mut r = Reader::new(&CLIP[..CLIP.len() - 100]).unwrap();
        let res = std::iter::from_fn(|| r.sample().transpose()).find(|s| s.is_err());
        assert!(res.is_some());
    }
}
