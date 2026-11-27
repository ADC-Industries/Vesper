//! Audio decoding: one read-ahead thread per played file (a video's soundtrack or an audio
//! file) reading IDN's `AudioReader` at the bus format (48 kHz, 32-bit float, interleaved,
//! [`AudioOptions::channels`] channels; mono copied to every channel) into a bounded lock-free
//! ring that the audio engine reads without ever waiting ([`AudioTap`]). IDN decodes and
//! resamples; it reads on the caller's thread, so it is never called from the audio engine.
//!
//! ## Timeline
//! Samples are addressed in **timeline samples** at 48 kHz, like video frames' timeline time:
//! sample `n` plays `n / 48000` s after the start of playback, continuous across loops (IDN's
//! reader timeline: sample 0 is the in point, plays follow one another). Every pass over the
//! in → out range has the same length in samples ([`AudioDecoder::region_len`]): the out point
//! when given, else (for a video's soundtrack) the length the caller fixes (the video's),
//! else the file's sound. A shorter soundtrack is padded with silence and a longer one is
//! cut, so the sound of pass *k* starts exactly where the picture of pass *k* starts.
//!
//! ## Seeking
//! [`AudioTap::seek`] (called by the consumer) starts a new *generation*: the read-ahead
//! thread reads from the new position (IDN seeks the file and decodes forward to the exact
//! sample) and tags what it queues with the new generation; blocks of older generations are
//! dropped by the reader.

use crate::player::{Loops, plays};
use anyhow::anyhow;
use idn_ffmpeg::media as idn;
use std::cell::UnsafeCell;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{JoinHandle, Thread};
use std::time::Duration;

/// The bus sample rate.
pub const AUDIO_RATE: u32 = 48_000;
/// Frames per ring block.
const BLOCK: usize = 1024;
/// Ring blocks (≈ 1 s at 48 kHz).
const SLOTS: usize = 48;

/// How an audio stream is decoded and played.
#[derive(Clone, Debug)]
pub struct AudioOptions {
    /// Output channels (mono is copied to every channel, others are remixed by IDN).
    pub channels: usize,
    /// Start of the played range, seconds from file start.
    pub in_point: f64,
    /// End of the played range (`None` = end of the stream).
    pub out_point: Option<f64>,
    /// Passes over the range.
    pub loops: Loops,
    /// Timeline time to start at, seconds.
    pub start: f64,
}

impl Default for AudioOptions {
    fn default() -> Self {
        AudioOptions {
            channels: 2,
            in_point: 0.0,
            out_point: None,
            loops: Loops::Once,
            start: 0.0,
        }
    }
}

/// Facts about a file's (best) audio stream.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioInfo {
    /// Codec (`aac`, `mp3`, `pcm_s16le`).
    pub codec: String,
    /// Source sample rate, Hz.
    pub sample_rate: u32,
    /// Source channels.
    pub channels: u32,
    /// Duration of the file, seconds.
    pub duration: Option<f64>,
}

struct Block {
    generation: u64,
    start: i64,
    frames: usize,
    data: Vec<f32>,
}

/// Single-producer single-consumer ring of blocks. The producer (decode thread) writes the
/// slot at `tail` and publishes it; the consumer (engine thread) reads slots in
/// `head..tail` and releases them. Neither ever waits.
struct Ring {
    slots: Box<[UnsafeCell<Block>]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

// SAFETY: a slot is accessed by the producer only while it is outside `head..tail` (not yet
// published), and by the consumer only inside it; publication goes through the Release store
// of `tail` / Acquire load in the consumer (and the reverse for `head`).
unsafe impl Sync for Ring {}
unsafe impl Send for Ring {}

impl Ring {
    fn new(channels: usize) -> Ring {
        Ring {
            slots: (0..SLOTS)
                .map(|_| {
                    UnsafeCell::new(Block {
                        generation: 0,
                        start: 0,
                        frames: 0,
                        data: vec![0.0; BLOCK * channels],
                    })
                })
                .collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    fn free(&self) -> usize {
        SLOTS - (self.tail.load(Ordering::Relaxed) - self.head.load(Ordering::Acquire))
    }
}

/// State shared by the decode thread and its handles.
struct Ctl {
    seek_gen: AtomicU64,
    seek_target: AtomicI64,
    stop: AtomicBool,
    /// Timeline sample where playback ends (`i64::MAX` = not known yet / loops forever).
    end: AtomicI64,
    /// Samples per pass (`-1` = not known yet).
    len: AtomicI64,
    decoded: AtomicU64,
    thread: OnceLock<Thread>,
    error: Mutex<Option<String>>,
}

impl Ctl {
    fn wake(&self) {
        if let Some(t) = self.thread.get() {
            t.unpark();
        }
    }

    /// Nothing more is read past timeline sample `at` (until a seek): publishes it as the end,
    /// unless an end is known already. `whole_play`: `at` is also a single play's length.
    fn publish_end(&self, at: i64, whole_play: bool) {
        if self.end.load(Ordering::Acquire) != i64::MAX {
            return;
        }
        self.end.store(at, Ordering::Release);
        if whole_play && self.len.load(Ordering::Acquire) < 0 {
            self.len.store(at, Ordering::Release);
        }
    }
}

/// The reading end of an audio stream (owned by the audio engine's source). Reading never
/// waits: what the decoder has not delivered yet reads as silence.
pub struct AudioTap {
    ring: Arc<Ring>,
    ctl: Arc<Ctl>,
    generation: u64,
    channels: usize,
}

impl std::fmt::Debug for AudioTap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioTap")
            .field("generation", &self.generation)
            .field("channels", &self.channels)
            .finish()
    }
}

impl AudioTap {
    /// Channels (interleaved) of what [`AudioTap::read`] returns.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The current generation (bumped by every seek).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Restarts decoding at timeline sample `sample`; data queued for the old position is
    /// dropped.
    pub fn seek(&mut self, sample: i64) {
        self.ctl.seek_target.store(sample.max(0), Ordering::Release);
        self.generation = self.ctl.seek_gen.fetch_add(1, Ordering::AcqRel) + 1;
        self.discard_stale();
        self.ctl.wake();
    }

    /// Drops queued blocks of older generations (frees room for the decoder).
    pub fn discard_stale(&mut self) {
        loop {
            let head = self.ring.head.load(Ordering::Relaxed);
            if head >= self.ring.tail.load(Ordering::Acquire) {
                return;
            }
            // SAFETY: `head < tail`: the slot is published and owned by the consumer.
            let b = unsafe { &*self.ring.slots[head % SLOTS].get() };
            if b.generation == self.generation {
                return;
            }
            self.ring.head.store(head + 1, Ordering::Release);
        }
    }

    /// Timeline range `(first, end)` of the queued data of the current generation.
    pub fn buffered(&self) -> Option<(i64, i64)> {
        let head = self.ring.head.load(Ordering::Relaxed);
        let tail = self.ring.tail.load(Ordering::Acquire);
        let mut out: Option<(i64, i64)> = None;
        for i in head..tail {
            // SAFETY: published slot, consumer side.
            let b = unsafe { &*self.ring.slots[i % SLOTS].get() };
            if b.generation != self.generation {
                continue;
            }
            let e = b.start + b.frames as i64;
            out = Some(match out {
                None => (b.start, e),
                Some((s, _)) => (s, e),
            });
        }
        out
    }

    /// Fills `out` (interleaved, at least `frames × channels`) with timeline samples
    /// `pos .. pos + frames`. Queued data before `pos` is released; samples not decoded yet
    /// are silence. Returns how many frames came from decoded data.
    pub fn read(&mut self, pos: i64, out: &mut [f32], frames: usize) -> usize {
        let ch = self.channels;
        out[..frames * ch].fill(0.0);
        let end = pos + frames as i64;
        let mut covered = 0;
        loop {
            let head = self.ring.head.load(Ordering::Relaxed);
            if head >= self.ring.tail.load(Ordering::Acquire) {
                break;
            }
            // SAFETY: published slot, consumer side.
            let b = unsafe { &*self.ring.slots[head % SLOTS].get() };
            let (bs, be) = (b.start, b.start + b.frames as i64);
            if b.generation != self.generation || be <= pos {
                self.ring.head.store(head + 1, Ordering::Release);
                continue;
            }
            if bs >= end {
                break;
            }
            let from = bs.max(pos);
            let to = be.min(end);
            let n = (to - from) as usize;
            let src = (from - bs) as usize * ch;
            let dst = (from - pos) as usize * ch;
            out[dst..dst + n * ch].copy_from_slice(&b.data[src..src + n * ch]);
            covered += n;
            if be <= end {
                self.ring.head.store(head + 1, Ordering::Release);
            } else {
                break;
            }
        }
        covered
    }

    /// Timeline sample where playback ends, once the decoder got there.
    pub fn end(&self) -> Option<i64> {
        match self.ctl.end.load(Ordering::Acquire) {
            i64::MAX => None,
            e => Some(e),
        }
    }

    /// Samples per pass, once known.
    pub fn region_len(&self) -> Option<i64> {
        match self.ctl.len.load(Ordering::Acquire) {
            l if l < 0 => None,
            l => Some(l),
        }
    }
}

/// A running audio decode thread. Dropping it stops (and joins) the thread.
pub struct AudioDecoder {
    ctl: Arc<Ctl>,
    thread: Option<JoinHandle<()>>,
    info: AudioInfo,
}

impl std::fmt::Debug for AudioDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioDecoder")
            .field("info", &self.info)
            .finish()
    }
}

impl AudioDecoder {
    /// Stream facts.
    pub fn info(&self) -> &AudioInfo {
        &self.info
    }

    /// Timeline sample where playback ends, once known.
    pub fn end(&self) -> Option<i64> {
        match self.ctl.end.load(Ordering::Acquire) {
            i64::MAX => None,
            e => Some(e),
        }
    }

    /// Samples per pass, once known.
    pub fn region_len(&self) -> Option<i64> {
        match self.ctl.len.load(Ordering::Acquire) {
            l if l < 0 => None,
            l => Some(l),
        }
    }

    /// Frames decoded so far.
    pub fn decoded(&self) -> u64 {
        self.ctl.decoded.load(Ordering::Relaxed)
    }

    /// The last decode error.
    pub fn error(&self) -> Option<String> {
        self.ctl
            .error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl Drop for AudioDecoder {
    fn drop(&mut self) {
        self.ctl.stop.store(true, Ordering::Release);
        self.ctl.wake();
        if let Some(h) = self.thread.take() {
            let _ = h.join();
        }
    }
}

/// Stream facts of a file's audio (`Ok(None)`: the file has no audio stream).
pub fn probe_audio(path: &Path) -> anyhow::Result<Option<AudioInfo>> {
    let m = crate::probe::probe_file(path)?;
    Ok(m.audio.map(|a| AudioInfo {
        codec: a.codec,
        sample_rate: a.sample_rate,
        channels: a.channels,
        duration: m.duration.map(|d| d.as_secs_f64()),
    }))
}

fn secs(t: f64) -> Duration {
    Duration::from_secs_f64(if t.is_finite() { t.max(0.0) } else { 0.0 })
}

/// Starts reading `path`'s audio. `Ok(None)` when the file has no audio stream.
/// `region_len`: seconds per pass when the caller fixes it (a video's length), else the out
/// point, else the file's sound.
pub fn open_audio(
    path: &Path,
    options: AudioOptions,
    region_len: Option<f64>,
) -> anyhow::Result<Option<(AudioDecoder, AudioTap)>> {
    let m = crate::probe::probe_file(path)?;
    let Some(a) = m.audio else {
        return Ok(None);
    };
    let info = AudioInfo {
        codec: a.codec,
        sample_rate: a.sample_rate,
        channels: a.channels,
        duration: m.duration.map(|d| d.as_secs_f64()),
    };
    let channels = options.channels.clamp(1, 16);
    let in_point = options.in_point.max(0.0);
    let out_point = region_len
        .filter(|l| *l > 0.0)
        .map(|l| in_point + l)
        .or(options.out_point.filter(|o| *o > in_point));
    let n = plays(options.loops);
    let cfg = idn::AudioReaderConfig {
        sample_rate: AUDIO_RATE,
        channels: channels as u16,
        in_point: secs(in_point),
        out_point: out_point.map(secs),
        loops: n,
    };
    let reader = idn::AudioReader::open(path, cfg.clone())
        .map_err(|e| anyhow!("cannot read the sound of {}: {e}", path.display()))?;
    let total = reader.length().map(|l| l as i64);
    // Samples per pass: the timeline's length over its plays; for endless loops, one play's.
    let len = match (n, total) {
        (Some(n), Some(l)) => Some(l / n as i64),
        (None, _) => idn::AudioReader::open(
            path,
            idn::AudioReaderConfig {
                loops: Some(1),
                ..cfg
            },
        )
        .ok()
        .and_then(|r| r.length())
        .map(|l| l as i64),
        _ => None,
    };
    let start = (options.start.max(0.0) * AUDIO_RATE as f64).round() as i64;
    let ring = Arc::new(Ring::new(channels));
    let ctl = Arc::new(Ctl {
        seek_gen: AtomicU64::new(0),
        seek_target: AtomicI64::new(start),
        stop: AtomicBool::new(false),
        end: AtomicI64::new(i64::MAX),
        len: AtomicI64::new(len.unwrap_or(-1)),
        decoded: AtomicU64::new(0),
        thread: OnceLock::new(),
        error: Mutex::new(None),
    });
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let d = Decode {
        ring: ring.clone(),
        ctl: ctl.clone(),
        reader,
        channels,
        total,
        one_play: n == Some(1),
        pos: 0,
        generation: u64::MAX,
        ended: false,
        contiguous: false,
        name,
    };
    let thread = std::thread::Builder::new()
        .name("audio decode".into())
        .spawn(move || d.run())?;
    let _ = ctl.thread.set(thread.thread().clone());
    let tap = AudioTap {
        ring,
        ctl: ctl.clone(),
        generation: 0,
        channels,
    };
    Ok(Some((
        AudioDecoder {
            ctl,
            thread: Some(thread),
            info,
        },
        tap,
    )))
}

/// The read-ahead thread's state.
struct Decode {
    ring: Arc<Ring>,
    ctl: Arc<Ctl>,
    reader: idn::AudioReader,
    channels: usize,
    /// The timeline's length, when the reader knows it.
    total: Option<i64>,
    /// A single play (its length is the pass length).
    one_play: bool,
    /// Timeline sample of the next read.
    pos: u64,
    generation: u64,
    /// Read to the end of the timeline (or failed): idle until a seek.
    ended: bool,
    /// `pos` is where the previous read ended (not a seek target).
    contiguous: bool,
    name: String,
}

impl Decode {
    fn run(mut self) {
        loop {
            if self.ctl.stop.load(Ordering::Acquire) {
                return;
            }
            let g = self.ctl.seek_gen.load(Ordering::Acquire);
            if g != self.generation {
                self.generation = g;
                self.pos = self.ctl.seek_target.load(Ordering::Acquire).max(0) as u64;
                self.ctl.end.store(i64::MAX, Ordering::Release);
                self.ended = false;
                self.contiguous = false;
                continue;
            }
            if self.ended || self.ring.free() == 0 {
                std::thread::park_timeout(Duration::from_millis(10));
                continue;
            }
            let ch = self.channels;
            let tail = self.ring.tail.load(Ordering::Relaxed);
            // SAFETY: the slot at `tail` is not published (free() > 0): producer side.
            let b = unsafe { &mut *self.ring.slots[tail % SLOTS].get() };
            match self.reader.read(self.pos, &mut b.data[..BLOCK * ch]) {
                Ok(n) => {
                    if n > 0 {
                        b.generation = self.generation;
                        b.start = self.pos as i64;
                        b.frames = n;
                        self.ring.tail.store(tail + 1, Ordering::Release);
                        self.ctl.decoded.fetch_add(n as u64, Ordering::Relaxed);
                        self.pos += n as u64;
                        self.contiguous = true;
                    }
                    if n < BLOCK {
                        // The timeline's end, published once everything before it is queued.
                        // A seek past an end not known yet reads nothing: the play ends where
                        // it was sent (that is no length, though).
                        self.ended = true;
                        let known = self.contiguous || self.total.is_some();
                        let e = match self.total {
                            Some(t) => t,
                            None => self.pos as i64,
                        };
                        self.ctl.publish_end(e, known && self.one_play);
                    }
                }
                Err(e) => {
                    // Nothing more comes: the play ends after what was read, so it stops (and
                    // a playlist goes on) instead of playing silence forever.
                    self.error(format!("read: {e}"));
                    self.ended = true;
                    self.ctl.publish_end(self.pos as i64, false);
                }
            }
        }
    }

    fn error(&self, msg: String) {
        let mut e = self.ctl.error.lock().unwrap_or_else(|e| e.into_inner());
        if e.as_deref() != Some(msg.as_str()) {
            log::warn!("media: {} (audio): {msg}", self.name);
        }
        *e = Some(msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl() -> Ctl {
        Ctl {
            seek_gen: AtomicU64::new(0),
            seek_target: AtomicI64::new(0),
            stop: AtomicBool::new(false),
            end: AtomicI64::new(i64::MAX),
            len: AtomicI64::new(-1),
            decoded: AtomicU64::new(0),
            thread: OnceLock::new(),
            error: Mutex::new(None),
        }
    }

    #[test]
    fn an_end_is_published_once() {
        // A03-2: a read error publishes where the play stops, without claiming a length.
        let c = ctl();
        c.publish_end(48_000, false);
        assert_eq!(c.end.load(Ordering::Acquire), 48_000);
        assert_eq!(c.len.load(Ordering::Acquire), -1);
        // The first end stands.
        c.publish_end(96_000, true);
        assert_eq!(c.end.load(Ordering::Acquire), 48_000);
        // The end of a single play is its length too.
        let c = ctl();
        c.publish_end(24_000, true);
        assert_eq!(c.len.load(Ordering::Acquire), 24_000);
    }
}
