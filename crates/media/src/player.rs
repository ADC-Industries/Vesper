//! One player per active video: an IDN `VideoReader` decoding ahead on its own thread, and
//! nearest-frame selection, frame numbering and counters for the renderer.

use crate::DecodeMode;
use crate::frame::{Matrix, PlaneFormat, VideoFrame};
use crate::probe::{VideoInfo, probe_file};
use anyhow::anyhow;
use idn_ffmpeg::media as idn;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// Frames decoded ahead of the newest requested media time.
pub const DEFAULT_QUEUE: usize = 8;

/// How many times the in → out range plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loops {
    /// Once.
    Once,
    /// `n` passes in total.
    Count(u32),
    /// Until stopped (also "loop for N seconds": the caller ends it).
    Forever,
}

/// How a player plays its file.
#[derive(Clone, Debug)]
pub struct PlayerOptions {
    /// Start of the played range, seconds from file start.
    pub in_point: f64,
    /// End of the played range (`None` = end of file).
    pub out_point: Option<f64>,
    /// Passes over the range.
    pub loops: Loops,
    /// Frames decoded ahead of the last time asked for (clamped to 2–32).
    pub queue: usize,
    /// Timeline time to start decoding at (0 = the in point).
    pub start: f64,
    /// Decode mode for this player (`None` = the process-wide [`crate::decode_mode`]).
    pub mode: Option<DecodeMode>,
    /// Also decode the soundtrack, to this many channels (see [`crate::audio`]); take its
    /// reading end with [`Player::take_audio`]. Without an out point, a video with sound
    /// plays to the file's duration (picture and sound both use that length per pass).
    pub audio: Option<usize>,
}

impl Default for PlayerOptions {
    fn default() -> Self {
        PlayerOptions {
            in_point: 0.0,
            out_point: None,
            loops: Loops::Once,
            queue: DEFAULT_QUEUE,
            start: 0.0,
            mode: None,
            audio: None,
        }
    }
}

/// Live numbers of a player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerStats {
    /// Frames handed out or passed over (the reader does not count its own decoding).
    pub decoded: u64,
    /// Distinct frames handed out by [`Player::frame_for`].
    pub shown: u64,
    /// Frames passed over between two frames handed out (60 fps media on a 30 fps screen
    /// skips every other frame by design; a decoder that fell behind skips late ones).
    pub skipped: u64,
    /// Requests for a time the decoder had not reached yet (the previous frame was shown
    /// again instead of waiting).
    pub late: u64,
    /// Frames queued ahead of the newest request (not reported by the reader: 0).
    pub queued: usize,
    /// GPU decoding expected (see [`VideoInfo::hw`]).
    pub hw: bool,
    /// Average decode time per frame, ms (not reported by the reader: 0).
    pub decode_ms: f64,
    /// 1 once decoding failed ([`Player::error`]).
    pub errors: u64,
    /// Why GPU decoding is not expected, when it was wanted.
    pub fallback: Option<String>,
}

struct State {
    /// The frame handed out last.
    last: Option<Arc<VideoFrame>>,
    seq: u64,
    generation: u64,
    stats: PlayerStats,
    info: VideoInfo,
    /// Pass length learned from a picture of a later pass (loops without a known length).
    learned_len: Option<f64>,
    /// The first picture set the stream's colour facts.
    colour_known: bool,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A video player (see the crate docs). Dropping it stops its reader's thread.
pub struct Player {
    reader: idn::VideoReader,
    st: Mutex<State>,
    audio: Mutex<Option<crate::audio::AudioTap>>,
    audio_dec: Option<crate::audio::AudioDecoder>,
    id: u64,
    path: PathBuf,
    options: PlayerOptions,
    frame_dur: f64,
    in_point: f64,
    out_point: Option<f64>,
}

impl std::fmt::Debug for Player {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Player")
            .field("id", &self.id)
            .field("path", &self.path)
            .finish()
    }
}

/// IDN's number of plays for Vesper's loops (`None`: forever).
pub(crate) fn plays(loops: Loops) -> Option<u32> {
    match loops {
        Loops::Once => Some(1),
        Loops::Count(n) => Some(n.max(1)),
        Loops::Forever => None,
    }
}

fn secs(t: f64) -> Duration {
    Duration::from_secs_f64(if t.is_finite() { t.max(0.0) } else { 0.0 })
}

impl Player {
    /// Opens `path` (synchronously: stream facts, reader) and starts decoding at
    /// `options.start`.
    pub fn open(path: &Path, options: PlayerOptions) -> anyhow::Result<Player> {
        let m = probe_file(path)?;
        let v = m
            .video
            .as_ref()
            .ok_or_else(|| anyhow!("{} has no video stream", path.display()))?;
        let mut info = VideoInfo::of(&m, v);
        let mode = options.mode.unwrap_or_else(crate::decode_mode);
        let want_hw = mode == DecodeMode::Auto;
        info.hw = want_hw && v.hardware_decodable && crate::hardware_available();
        let mut stats = PlayerStats {
            hw: info.hw,
            ..Default::default()
        };
        if want_hw && !info.hw {
            stats.fallback = Some(match crate::hardware_decoder() {
                None => "no GPU decoder".into(),
                Some(hw) => format!("{hw} does not decode {}", info.codec),
            });
            log::info!(
                "media: {}: software decode ({})",
                path.display(),
                stats.fallback.as_deref().unwrap_or("")
            );
        }
        let frame_dur = 1.0 / info.fps.max(1.0);
        let in_point = options.in_point.max(0.0);
        let has_audio = options.audio.is_some() && m.audio.is_some();
        // A video with sound: picture and sound both play to the out point, else the file's
        // duration, so pass k of the sound starts where pass k of the picture does.
        let out_point = options
            .out_point
            .or(if has_audio { info.duration } else { None })
            .filter(|o| *o > in_point + frame_dur * 0.5);
        let reader = idn::VideoReader::open(
            path,
            crate::ffmpeg(),
            idn::VideoReaderConfig {
                in_point: secs(in_point),
                out_point: out_point.map(secs),
                loops: plays(options.loops),
                format: None,
                size: None,
                ahead: options.queue.clamp(2, 32),
                decoder: mode.choice(),
            },
        )
        .map_err(|e| anyhow!("cannot open {}: {e}", path.display()))?;
        let start = options.start.max(0.0);
        if start > 0.0 {
            // Asking moves the decoding there.
            let _ = reader.frame_at(secs(start));
        }
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (audio_dec, audio) = match options.audio.filter(|_| has_audio) {
            Some(ch) => match crate::audio::open_audio(
                path,
                crate::audio::AudioOptions {
                    channels: ch,
                    in_point,
                    out_point,
                    loops: options.loops,
                    start,
                },
                out_point.map(|o| o - in_point),
            ) {
                Ok(Some((d, t))) => (Some(d), Some(t)),
                Ok(None) => (None, None),
                Err(e) => {
                    log::warn!("media: {}: no sound: {e:#}", path.display());
                    (None, None)
                }
            },
            None => (None, None),
        };
        Ok(Player {
            reader,
            st: Mutex::new(State {
                last: None,
                seq: 0,
                generation: 0,
                stats,
                info,
                learned_len: None,
                colour_known: false,
            }),
            audio: Mutex::new(audio),
            audio_dec,
            id,
            path: path.to_path_buf(),
            options,
            frame_dur,
            in_point,
            out_point,
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The reading end of the soundtrack (once; `None` without sound or when taken).
    pub fn take_audio(&self) -> Option<crate::audio::AudioTap> {
        self.audio.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    /// The soundtrack's decoder (`None`: no sound requested, or the file has none).
    pub fn audio(&self) -> Option<&crate::audio::AudioDecoder> {
        self.audio_dec.as_ref()
    }

    /// Process-unique player ID (renderers key uploaded textures by it).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The options it was opened with.
    pub fn options(&self) -> &PlayerOptions {
        &self.options
    }

    /// Stream facts (colour from the first picture handed out).
    pub fn info(&self) -> VideoInfo {
        self.lock().info.clone()
    }

    /// Nominal frame duration, seconds.
    pub fn frame_duration(&self) -> f64 {
        self.frame_dur
    }

    /// Counters.
    pub fn stats(&self) -> PlayerStats {
        let mut s = self.lock().stats.clone();
        s.errors = self.reader.error().is_some() as u64;
        s
    }

    /// Length of one pass (out − in), once known: an out point (or a video with sound), a
    /// counted play of a file whose length is known, or a later pass reached.
    pub fn region_len(&self) -> Option<f64> {
        if let Some(o) = self.out_point {
            return Some(o - self.in_point);
        }
        if let (Some(n), Some(l)) = (plays(self.options.loops), self.reader.length()) {
            return Some(l.as_secs_f64() / n as f64);
        }
        self.lock().learned_len
    }

    /// Timeline time at which playback ends: the timeline's length when the reader knows it,
    /// else the end of the last picture once decoded there (`None` = not known yet, or loops
    /// forever).
    pub fn end_time(&self) -> Option<f64> {
        plays(self.options.loops)?;
        if let Some(l) = self.reader.length() {
            return Some(l.as_secs_f64());
        }
        if !self.reader.ended() {
            return None;
        }
        // Past the end the reader holds its last picture.
        let last = self.reader.frame_at(Duration::from_secs(1 << 30))?;
        Some((last.time + last.duration).as_secs_f64())
    }

    /// Why decoding stopped, if it failed.
    pub fn error(&self) -> Option<String> {
        self.reader.error()
    }

    /// Restarts decoding at timeline time `t` (exact: from the keyframe before it). Until the
    /// new position is decoded, [`Player::frame_for`] keeps returning the last frame it
    /// returned.
    pub fn seek(&self, t: f64) {
        self.lock().generation += 1;
        // Asking for a time before the pictures held (or well past them) seeks.
        let _ = self.reader.frame_at(secs(t));
    }

    /// The reader's query time for the frame nearest to `t` (ties: the earlier one): the
    /// picture starting at or before `t` + half a frame.
    fn nearest(&self, t: f64) -> Duration {
        secs(t + self.frame_dur * 0.5 - 1e-6)
    }

    /// The frame whose timeline time is nearest to `t` (ties: the earlier one), without
    /// waiting. `None` before the first frame is decoded.
    pub fn frame_for(&self, t: f64) -> Option<Arc<VideoFrame>> {
        let pic = self.reader.frame_at(self.nearest(t));
        self.hand_out(pic, t, true)
    }

    /// [`Player::frame_for`], but waits (up to `timeout`) until the reader has decoded the
    /// frame for `t` (or the end), so the choice is exact. For tests, tools and the first
    /// frame of a play; never for live output.
    pub fn wait_frame_for(&self, t: f64, timeout: Duration) -> Option<Arc<VideoFrame>> {
        let pic = self.reader.frame_at_wait(self.nearest(t), timeout);
        self.hand_out(pic, t, false)
    }

    /// Whether playback has ended at timeline time `t` (the last frame of the last pass has
    /// been on screen for its duration).
    pub fn finished_at(&self, t: f64) -> bool {
        self.end_time().is_some_and(|e| t >= e - 1e-9)
    }

    /// Wraps the reader's picture (once per picture: the same one keeps its `seq`), counts.
    fn hand_out(
        &self,
        pic: Option<Arc<idn::VideoFrame>>,
        t: f64,
        count_late: bool,
    ) -> Option<Arc<VideoFrame>> {
        let mut st = self.lock();
        let Some(pic) = pic else {
            if count_late && st.last.is_some() {
                st.stats.late += 1;
            }
            return st.last.clone();
        };
        let dur = self.frame_dur;
        let time = pic.time.as_secs_f64();
        // The reader has not reached `t` yet: the picture before it is shown again.
        if count_late && !self.reader.ended() && time + pic.duration.as_secs_f64() < t - dur * 0.5 {
            st.stats.late += 1;
        }
        if let Some(last) = &st.last
            && Arc::ptr_eq(&last.pic, &pic)
        {
            return Some(last.clone());
        }
        let Some(format) = PlaneFormat::of(pic.format) else {
            log::warn!(
                "media: {}: unexpected picture layout {:?}",
                self.path.display(),
                pic.format
            );
            return st.last.clone();
        };
        let (w, h) = (pic.width as u32, pic.height as u32);
        let matrix = Matrix::of(pic.matrix.as_deref(), w, h);
        if !st.colour_known {
            st.colour_known = true;
            st.info.matrix = matrix;
            st.info.full_range = pic.full_range;
        }
        if pic.play > 0 && st.learned_len.is_none() {
            let off = pic.file_time.as_secs_f64() - self.in_point;
            st.learned_len = Some((time - off) / pic.play as f64);
        }
        if let Some(prev) = &st.last
            && prev.generation == st.generation
            && time > prev.t
        {
            let gap = ((time - prev.t) / dur).round() as i64 - 1;
            if gap > 0 && gap < 1000 {
                st.stats.skipped += gap as u64;
            }
        }
        st.seq += 1;
        st.stats.shown += 1;
        st.stats.decoded = st.stats.shown + st.stats.skipped;
        let f = Arc::new(VideoFrame {
            t: time,
            pts: pic.file_time.as_secs_f64(),
            duration: pic.duration.as_secs_f64(),
            seq: st.seq,
            generation: st.generation,
            loop_index: pic.play,
            width: w,
            height: h,
            format,
            bit_depth: pic.bit_depth,
            matrix,
            full_range: pic.full_range,
            transfer: pic.transfer.clone(),
            primaries: pic.primaries.clone(),
            rotation: pic.rotation,
            // What the reader actually does (it can fall back to software once).
            hw: self.reader.decoding().gpu.is_some(),
            pic,
        });
        st.last = Some(f.clone());
        Some(f)
    }
}
