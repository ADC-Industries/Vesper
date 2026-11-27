//! Live sources' sound (G7 stage 3 item 3, `docs/LIVE-SOURCES-PLAN.md` §5): the sound of the
//! source on the video-input layer plays on the program bus, in step with its picture.
//!
//! - **Feed.** Each source's receive thread pushes its sound blocks and the arrival of each
//!   picture into the source's [`LiveSoundFeed`] (one per source name, kept across reconnects
//!   and receiver restarts). The library delivers sound and pictures separately and has no
//!   pairing helper; a sender gives the sound block attached to a picture that picture's
//!   timecode, so both are paired here by timecode.
//! - **Lane.** The show actor adds one [`LiveSoundSource`] to the audio engine for the source
//!   on the video-input layer (group [`crate::sound::GROUP_LIVE`]: the layer's level and mute;
//!   the play's `AudioSettings` for level, mute and routing). It keeps the source's samples
//!   by timecode and, for every block of the bus, plays the source time due when that block is
//!   heard. One source has one lane (its engine ID is the feed's, [`LiveSoundFeed::lane`]):
//!   a source put back on the layer while its lane still fades out gets that lane back (it
//!   fades in again from where it was, still locked on), never a second lane competing with
//!   it for the feed's sound.
//! - **Which source time is due.** The sender's timecodes and this box's clock are related by
//!   the smallest transit seen over the last [`WINDOW`] (a running minimum of "arrived −
//!   timecode", for sound blocks and for pictures; it follows the sender's clock drift). A
//!   picture is shown the newest one at the screen's rate, about [`VIDEO_LEAD`] after it
//!   arrived (one screen frame rendered ahead, half a source frame waiting), so the sound with
//!   the same timecode is heard that long after the picture's arrival — or later, when the
//!   sound itself needs more time: its own transit, [`MARGIN`] of jitter, and the time from
//!   mixing a block to hearing it plus the block's length (the samples must be here when the
//!   block is mixed). On the null output the sound is heard within a frame of its picture
//!   (the server test measures about 10 ms early); a sound card's latency makes it that much
//!   later, within what viewers accept for lip sync (sound late by up to ~100 ms).
//! - **Drift.** The source is resampled to the bus rate (48 kHz) by linear interpolation at a
//!   rate corrected by up to ±[`MAX_CORRECTION`]: a PI loop steers the read position towards
//!   the due position with a time constant of [`STEER`] seconds (its integral learns the
//!   sender's clock rate), so the sender's clock, the network and the sound card's clock
//!   never drift apart audibly. Off by more than [`RELOCK`], it jumps (a
//!   short fade hides it), forward or back: [`HISTORY`] of samples is kept behind the read
//!   position (the due time moves back when pictures start arriving later than the sound,
//!   say after a decoder's first keyframe); due before that, it is silent until the due time
//!   reaches what it has. Without samples for the due time (a gap, a lost source) it is
//!   silent and locks on again when sound comes back.
//! - **Timecodes.** A sender may stamp its sound with a clock other than its sample clock (its
//!   pictures' or the system's): block after block, the samples' place on the timeline is
//!   nudged towards the blocks' timecodes (a small, smoothed step), so that difference becomes
//!   a rate difference for the drift loop instead of growing into a jump. Only a jump of more
//!   than [`JUMP`] (a gap, a restarted sender) starts over.

use media::AUDIO_RATE;
use model::{AudioSettings, Layer};
use output::LiveSound;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long the running minimum of transit times remembers.
pub const WINDOW: Duration = Duration::from_secs(2);
/// How long after its arrival a picture is shown (see the [module docs](self)).
pub const VIDEO_LEAD: Duration = Duration::from_millis(40);
/// Sound jitter allowed above its smallest transit.
pub const MARGIN: Duration = Duration::from_millis(30);
/// Largest rate correction (0.5 %).
pub const MAX_CORRECTION: f64 = 0.005;
/// Time constant of the drift correction, s.
pub const STEER: f64 = 2.0;
/// An error larger than this re-locks (jumps) instead of steering.
pub const RELOCK: Duration = Duration::from_millis(60);
/// Samples kept behind the read position (a re-lock can jump back this far).
pub const HISTORY: Duration = Duration::from_secs(1);
/// A block whose timecode is off by more than this from where the samples before it end
/// starts over (a gap, a restarted sender); smaller differences are followed.
pub const JUMP: Duration = Duration::from_millis(20);
/// Of a block's timecode difference (below [`JUMP`]), the part the samples' timeline follows.
const FOLLOW: f64 = 0.05;
/// Sound waiting in a feed nobody plays is dropped beyond this.
const INBOX_LIMIT: Duration = Duration::from_secs(1);
/// Samples kept beyond the due position at most.
const BUFFER_LIMIT: Duration = Duration::from_secs(3);
/// 100 ns units a second (timecodes).
const TC: f64 = 10_000_000.0;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

struct Arrived {
    at: Instant,
    sound: LiveSound,
}

#[derive(Default)]
struct Inbox {
    blocks: VecDeque<Arrived>,
    /// Pictures that arrived: (timecode, when).
    pictures: Vec<(i64, Instant)>,
    /// Queued samples (per channel).
    queued: usize,
}

/// A block of a source's sound mixed to mono: (sample rate, samples).
pub type MonoBlock = (u32, Vec<f32>);

/// One source's sound on its way from the receive thread to the audio engine (see the
/// [module docs](self)).
pub struct LiveSoundFeed {
    inbox: Mutex<Inbox>,
    /// The engine ID of the source's lane (one lane per source).
    pub lane: u64,
    /// The lane's counters.
    stats: Arc<Mutex<LiveSoundStats>>,
    /// Others hearing the source's sound as it arrives (captions): (rate, mono samples).
    listeners: Mutex<Vec<std::sync::mpsc::SyncSender<MonoBlock>>>,
}

impl Default for LiveSoundFeed {
    fn default() -> Self {
        LiveSoundFeed {
            inbox: Mutex::default(),
            lane: LANE_ID.fetch_add(1, Ordering::Relaxed),
            stats: Arc::default(),
            listeners: Mutex::default(),
        }
    }
}

impl LiveSoundFeed {
    /// The source's sound as it arrives, mixed to mono: (rate, samples) per block. Blocks a
    /// slow listener has no room for (about 2 s queued) are dropped; dropping the receiver
    /// ends it.
    pub fn listen(&self) -> std::sync::mpsc::Receiver<MonoBlock> {
        let (tx, rx) = std::sync::mpsc::sync_channel(100);
        lock(&self.listeners).push(tx);
        rx
    }

    /// A block of sound arrived (the receive thread).
    pub fn sound(&self, sound: LiveSound, at: Instant) {
        if sound.sample_rate == 0 || sound.channels.is_empty() {
            return;
        }
        {
            let mut listeners = lock(&self.listeners);
            if !listeners.is_empty() {
                let n = sound.channels.len() as f32;
                let mono: Vec<f32> = (0..sound.channels[0].len())
                    .map(|i| {
                        sound
                            .channels
                            .iter()
                            .map(|c| c.get(i).copied().unwrap_or(0.0))
                            .sum::<f32>()
                            / n
                    })
                    .collect();
                listeners.retain(|tx| {
                    !matches!(
                        tx.try_send((sound.sample_rate, mono.clone())),
                        Err(std::sync::mpsc::TrySendError::Disconnected(_))
                    )
                });
            }
        }
        let mut inbox = lock(&self.inbox);
        inbox.queued += sound.channels[0].len();
        let rate = sound.sample_rate as usize;
        inbox.blocks.push_back(Arrived { at, sound });
        // Nobody plays it: keep the newest second.
        let limit = INBOX_LIMIT.as_secs_f64() as usize * rate;
        while inbox.queued > limit {
            match inbox.blocks.pop_front() {
                Some(b) => inbox.queued -= b.sound.channels[0].len(),
                None => break,
            }
        }
    }

    /// A picture with `timecode` arrived (the receive thread).
    pub fn picture(&self, timecode: i64, at: Instant) {
        let mut inbox = lock(&self.inbox);
        if inbox.pictures.len() >= 64 {
            inbox.pictures.remove(0);
        }
        inbox.pictures.push((timecode, at));
    }
}

/// A running minimum over [`WINDOW`] (values in timecode units, keyed by local time).
#[derive(Default)]
struct MinWindow {
    q: VecDeque<(Instant, f64)>,
}

impl MinWindow {
    fn push(&mut self, at: Instant, v: f64) {
        while self.q.back().is_some_and(|(_, b)| *b >= v) {
            self.q.pop_back();
        }
        self.q.push_back((at, v));
        while self
            .q
            .front()
            .is_some_and(|(t, _)| at.saturating_duration_since(*t) > WINDOW)
        {
            self.q.pop_front();
        }
    }

    fn min(&self) -> Option<f64> {
        self.q.front().map(|(_, v)| *v)
    }
}

/// Counters of a [`LiveSoundSource`] (tests, logs).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LiveSoundStats {
    /// Times it locked on (the first time included).
    pub locks: u64,
    /// Bus blocks it had no samples for while locked (it lost the lock).
    pub underruns: u64,
    /// The last rate correction applied.
    pub correction: f64,
    /// The last error between the read and the due position, s (+ = behind).
    pub error: f64,
    /// The source timecode due for the last block's first frame.
    pub due: f64,
}

/// The audio-engine lane of one live source (see the [module docs](self)).
pub struct LiveSoundSource {
    feed: Arc<LiveSoundFeed>,
    channels: usize,
    rate: u32,
    /// Samples by channel, at the source rate.
    buf: Vec<VecDeque<f32>>,
    /// Source timecode of `buf[..][0]`.
    buf_tc: f64,
    /// Read position in `buf`, samples.
    pos: f64,
    locked: bool,
    /// The drift loop's integral (the learnt rate difference).
    integral: f64,
    /// Ramp in after a (re)lock: frames left.
    ramp: usize,
    audio_transit: MinWindow,
    video_transit: MinWindow,
    /// Negated "heard − mixed + block length" (its running minimum is the largest).
    ahead: MinWindow,
    stats: Arc<Mutex<LiveSoundStats>>,
    /// Not drained yet: what is queued then is from before it went live.
    fresh: bool,
}

impl LiveSoundSource {
    /// A lane playing `feed` on a bus of `channels` channels (a source's channel N plays on
    /// bus channel N; a mono source on every channel). Nothing is taken from the feed before
    /// it first plays (a lane the engine drops in favour of the source's lane still fading
    /// out never touches it).
    pub fn new(feed: Arc<LiveSoundFeed>, channels: usize) -> LiveSoundSource {
        LiveSoundSource {
            stats: feed.stats.clone(),
            fresh: true,
            feed,
            channels: channels.max(1),
            rate: 0,
            buf: Vec::new(),
            buf_tc: 0.0,
            pos: 0.0,
            locked: false,
            integral: 0.0,
            ramp: 0,
            audio_transit: MinWindow::default(),
            video_transit: MinWindow::default(),
            ahead: MinWindow::default(),
        }
    }

    /// Its counters (readable while the engine owns it).
    pub fn stats(&self) -> Arc<Mutex<LiveSoundStats>> {
        self.stats.clone()
    }

    fn buffered(&self) -> usize {
        self.buf.first().map_or(0, VecDeque::len)
    }

    fn reset(&mut self, rate: u32, channels: usize, tc: f64) {
        self.rate = rate;
        self.buf = vec![VecDeque::new(); channels];
        self.buf_tc = tc;
        self.pos = 0.0;
        self.locked = false;
    }

    /// Takes what arrived (never blocks: skipped when the receive thread holds the lock).
    fn drain(&mut self) {
        let Ok(mut inbox) = self.feed.inbox.try_lock() else {
            return;
        };
        if std::mem::take(&mut self.fresh) {
            // Sound queued before it was added is from before it went live; the counters
            // start over with the new lane.
            *inbox = Inbox::default();
            *lock(&self.stats) = LiveSoundStats::default();
            return;
        }
        let blocks = std::mem::take(&mut inbox.blocks);
        let pictures = std::mem::take(&mut inbox.pictures);
        inbox.queued = 0;
        drop(inbox);
        for (tc, at) in pictures {
            self.video_transit
                .push(at, audio::timecode(at) as f64 - tc as f64);
        }
        for Arrived { at, sound } in blocks {
            let rate = sound.sample_rate;
            let n = sound.channels[0].len();
            let tc = sound.timecode as f64;
            let len = n as f64 / rate as f64 * TC;
            // Its last sample was due at `tc + len` (the sender's clock) and is here at `at`.
            self.audio_transit
                .push(at, audio::timecode(at) as f64 - (tc + len));
            let end = self.buf_tc + self.buffered() as f64 / self.rate.max(1) as f64 * TC;
            // A new format, or a jump in the timecodes (a gap, a restarted sender): start over.
            if rate != self.rate
                || sound.channels.len() != self.buf.len()
                || (tc - end).abs() > JUMP.as_secs_f64() * TC
            {
                self.reset(rate, sound.channels.len(), tc);
            } else {
                // Timecodes from a clock other than the samples': follow them (E02-2).
                self.buf_tc += (tc - end) * FOLLOW;
            }
            for (b, c) in self.buf.iter_mut().zip(&sound.channels) {
                b.extend(c.iter().take(n));
            }
        }
    }

    /// The source timecode due when a block is heard at `heard` (`None`: nothing arrived yet).
    fn due(&self, heard: Instant) -> Option<f64> {
        let ahead = -self.ahead.min().unwrap_or(0.0);
        let a = self.audio_transit.min()? + MARGIN.as_secs_f64() * TC + ahead;
        let delay = match self.video_transit.min() {
            Some(v) => a.max(v + VIDEO_LEAD.as_secs_f64() * TC),
            None => a,
        };
        Some(audio::timecode(heard) as f64 - delay)
    }

    /// The source timecode at the read position.
    pub fn read_tc(&self) -> f64 {
        self.buf_tc + self.pos / self.rate.max(1) as f64 * TC
    }

    /// Drops samples more than [`HISTORY`] before the read position (and beyond
    /// [`BUFFER_LIMIT`] of backlog after it).
    fn trim(&mut self) {
        let rate = self.rate.max(1) as f64;
        let history = (HISTORY.as_secs_f64() * rate) as isize;
        let mut drop_n = (self.pos.floor() as isize - 1 - history).max(0) as usize;
        let read = self.pos.floor().max(0.0) as usize;
        let backlog = self.buffered().saturating_sub(read);
        let limit = (BUFFER_LIMIT.as_secs_f64() * rate) as usize;
        if backlog > limit {
            drop_n = drop_n.max(self.buffered() - limit);
            self.locked = false;
        }
        let drop_n = drop_n.min(self.buffered());
        if drop_n == 0 {
            return;
        }
        for b in &mut self.buf {
            b.drain(..drop_n);
        }
        self.pos -= drop_n as f64;
        self.buf_tc += drop_n as f64 / rate * TC;
    }
}

impl audio::Source for LiveSoundSource {
    fn channels(&self) -> usize {
        self.channels
    }

    fn render(&mut self, out: &mut [f32], ctx: &audio::BlockCtx) -> bool {
        self.render_at(out, ctx, Instant::now())
    }
}

impl LiveSoundSource {
    /// [`audio::Source::render`] mixed at `now`.
    fn render_at(&mut self, out: &mut [f32], ctx: &audio::BlockCtx, now: Instant) -> bool {
        self.drain();
        let n = ctx.frames;
        let block = n as f64 / AUDIO_RATE as f64;
        self.ahead.push(
            now,
            -(audio::secs_between(now, ctx.heard_at).max(0.0) + block) * TC,
        );
        let (Some(due), true) = (self.due(ctx.heard_at), self.rate > 0) else {
            return true;
        };
        let rate = self.rate as f64;
        let due_pos = (due - self.buf_tc) / TC * rate;
        let len = self.buffered() as f64;
        let counters = self.stats.clone();
        let mut stats = counters.try_lock().ok();
        if !self.locked {
            if due_pos < 0.0 || due_pos >= len - 1.0 {
                // Not here yet (or long gone): silence; old samples go.
                if due_pos >= len - 1.0 {
                    self.pos = len;
                }
                self.trim();
                return true;
            }
            self.pos = due_pos;
            self.locked = true;
            self.ramp = n;
            if let Some(s) = stats.as_deref_mut() {
                s.locks += 1;
            }
        }
        let mut err = (due_pos - self.pos) / rate;
        if err.abs() > RELOCK.as_secs_f64() {
            if due_pos >= 0.0 && due_pos < len - 1.0 {
                // Jump (forward, or back into the history kept) and steer from there.
                self.pos = due_pos;
                self.ramp = n;
                err = 0.0;
                if let Some(s) = stats.as_deref_mut() {
                    s.locks += 1;
                }
            } else if due_pos < 0.0 {
                // Due before anything kept: silent until the due time gets here.
                self.locked = false;
                self.trim();
                return true;
            }
        }
        // PI, critically damped at 1 / STEER rad/s.
        let dt = n as f64 / AUDIO_RATE as f64;
        self.integral =
            (self.integral + err * dt / (STEER * STEER)).clamp(-MAX_CORRECTION, MAX_CORRECTION);
        let correction = (err * 2.0 / STEER + self.integral).clamp(-MAX_CORRECTION, MAX_CORRECTION);
        let step = rate / AUDIO_RATE as f64 * (1.0 + correction);
        if let Some(s) = stats.as_deref_mut() {
            s.correction = correction;
            s.error = err;
            s.due = due;
        }
        let src_ch = self.buf.len();
        let ch = self.channels;
        let ramp = self.ramp;
        for f in 0..n {
            let p = self.pos + f as f64 * step;
            let i = p.floor() as usize;
            if i + 1 >= self.buf[0].len() {
                // Ran dry: silent until it locks on again.
                self.locked = false;
                if let Some(s) = stats.as_deref_mut() {
                    s.underruns += 1;
                }
                break;
            }
            let frac = (p - i as f64) as f32;
            let g = if f < ramp {
                f as f32 / ramp as f32
            } else {
                1.0
            };
            for c in 0..ch {
                let sc = if src_ch == 1 {
                    0
                } else if c < src_ch {
                    c
                } else {
                    continue;
                };
                let b = &self.buf[sc];
                out[f * ch + c] = (b[i] + (b[i + 1] - b[i]) * frac) * g;
            }
        }
        self.ramp = 0;
        self.pos += n as f64 * step;
        self.trim();
        true
    }
}

// ---- the video-input layer's lane -----------------------------------------------------

/// Audio-engine source IDs of live sound lanes, one per source's feed (apart from play IDs
/// and element plays).
static LANE_ID: AtomicU64 = AtomicU64::new(1 << 60);

/// The live source whose sound plays (see [`crate::Shared::live_sound`]).
pub struct Playing {
    /// The source.
    pub source: String,
    /// Its settings.
    pub audio: AudioSettings,
    /// Its lane's source ID in the audio engine.
    pub id: u64,
    /// The lane's counters.
    pub stats: Arc<Mutex<LiveSoundStats>>,
}

/// Keeps the live sound in step with the video-input layer (`want`: its source and sound
/// settings; called by the show actor): another source fades the old lane out and the new
/// one in over `fade` s (the layer's transition), new settings apply in place. The source's
/// receiver is opened if no screen has asked for it yet.
pub fn sync(shared: &crate::Shared, want: Option<(String, AudioSettings)>, fade: f64) {
    let mut cur = lock(&shared.live_sound);
    match (&mut *cur, want) {
        (Some(p), Some((source, audio))) if p.source == source => {
            if p.audio != audio {
                shared
                    .sound
                    .engine
                    .set_params(p.id, crate::sound::params_of(Layer::VideoInput, &audio));
                p.audio = audio;
            }
        }
        (_, want) => {
            if let Some(old) = cur.take() {
                shared.sound.engine.remove_source(old.id, fade);
            }
            let Some((source, audio)) = want else { return };
            let data = shared.data();
            let max = data.screens.iter().fold((0u32, 0u32), |m, s| {
                (
                    m.0.max(s.size.width.round() as u32),
                    m.1.max(s.size.height.round() as u32),
                )
            });
            let _ = shared
                .live_sources
                .receiver(&source, (max.0 > 0 && max.1 > 0).then_some(max));
            // The source's one lane: brought back if it still fades out (E02-1), else new.
            let feed = shared.live_sources.sound_feed(&source);
            let id = feed.lane;
            let lane = LiveSoundSource::new(feed, shared.sound.channels());
            let stats = lane.stats();
            shared.sound.engine.resume_source(
                id,
                Box::new(lane),
                crate::sound::params_of(Layer::VideoInput, &audio),
                fade,
            );
            *cur = Some(Playing {
                source,
                audio,
                id,
                stats,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audio::BlockCtx;

    /// A sender whose clock runs `drift` fast, sending `block` samples at `rate` every block
    /// length (its clock), each sample the fraction of a second of its timecode (a saw that
    /// tells which source time is playing), arriving with up to `jitter` of extra delay.
    struct Sim {
        feed: Arc<LiveSoundFeed>,
        src: LiveSoundSource,
        rate: u32,
        drift: f64,
        t0: Instant,
        sent: usize,
        block: usize,
        transit: Duration,
        jitter: Duration,
        seed: u64,
        pictures: bool,
        /// Pictures arrive this much later than their sound.
        picture_delay: Duration,
        /// The sender's timecodes run this much fast against its samples (they come from
        /// another clock).
        tc_drift: f64,
        /// Blocks run so far.
        blocks: usize,
    }

    impl Sim {
        fn new(rate: u32, drift: f64, jitter: Duration, pictures: bool) -> Sim {
            let feed = Arc::new(LiveSoundFeed::default());
            let src = LiveSoundSource::new(feed.clone(), 2);
            Sim {
                feed,
                src,
                rate,
                drift,
                t0: Instant::now(),
                sent: 0,
                block: rate as usize / 100,
                transit: Duration::from_millis(3),
                jitter,
                seed: 7,
                pictures,
                picture_delay: Duration::ZERO,
                tc_drift: 0.0,
                blocks: 0,
            }
        }

        fn rand(&mut self) -> f64 {
            self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (self.seed >> 11) as f64 / (1u64 << 53) as f64
        }

        /// The source timecode of sample `k` (an hour in, so timecodes are large).
        fn tc(&self, k: usize) -> f64 {
            3600.0 * TC + k as f64 / self.rate as f64 * TC
        }

        /// Sends what the sender has produced by local time `now`.
        fn send(&mut self, now: Instant) {
            loop {
                let end = self.sent + self.block;
                // Local instant the sender finished this block (its clock is `drift` fast).
                let done = self.t0
                    + Duration::from_secs_f64(end as f64 / self.rate as f64 / (1.0 + self.drift));
                let jitter = Duration::from_secs_f64(self.jitter.as_secs_f64() * self.rand());
                let at = done + self.transit + jitter;
                if at > now {
                    return;
                }
                let tc = self.tc(self.sent) + (self.tc(self.sent) - self.tc(0)) * self.tc_drift;
                let samples: Vec<f32> = (self.sent..end)
                    .map(|k| ((self.tc(k) / TC).fract() * 0.5) as f32)
                    .collect();
                self.feed.sound(
                    LiveSound {
                        sample_rate: self.rate,
                        timecode: tc as i64,
                        channels: vec![samples.clone(), samples],
                    },
                    at,
                );
                // A picture every 4th block (40 ms), with the same timecode as its sound.
                if self.pictures && (self.sent / self.block).is_multiple_of(4) {
                    self.feed.picture(tc as i64, at + self.picture_delay);
                }
                self.sent = end;
            }
        }

        /// Runs the engine for `secs` more (10 ms blocks heard `latency` after mixing);
        /// returns, for each block after `settle` s (from the start), (due timecode, played
        /// timecode) of its first frame.
        fn run(&mut self, secs: f64, settle: f64) -> Vec<(f64, f64)> {
            let latency = Duration::from_millis(20);
            let frames = 480;
            let mut out = vec![0f32; frames * 2];
            let mut seen = Vec::new();
            let first = self.blocks;
            self.blocks += (secs * 100.0) as usize;
            for b in first..self.blocks {
                let now = self.t0 + Duration::from_millis(10 * b as u64);
                self.send(now);
                out.iter_mut().for_each(|v| *v = 0.0);
                let heard = now + latency;
                let ctx = BlockCtx {
                    sample: (b * frames) as i64,
                    heard_at: heard,
                    frames,
                };
                self.src.render_at(&mut out, &ctx, now);
                let due = self.src.stats.lock().unwrap().due;
                if b as f64 / 100.0 >= settle && due > 0.0 {
                    // The saw's value says which second fraction plays (away from its wrap).
                    let played = out[2] as f64 * 2.0;
                    let want = (due / TC).fract();
                    if (0.002..0.998).contains(&want) {
                        seen.push((want, played));
                    }
                }
            }
            seen
        }
    }

    /// Wrapped difference of two second fractions, s.
    fn diff(a: f64, b: f64) -> f64 {
        let d = a - b;
        d - d.round()
    }

    #[test]
    fn plays_the_due_source_time_without_drift() {
        let mut sim = Sim::new(48_000, 0.0, Duration::from_millis(4), false);
        let seen = sim.run(6.0, 2.0);
        assert!(!seen.is_empty());
        for (want, played) in &seen {
            assert!(diff(*want, *played).abs() < 0.002, "{want} vs {played}");
        }
        let st = *sim.src.stats.lock().unwrap();
        assert_eq!(st.underruns, 0, "{st:?}");
        assert_eq!(st.locks, 1, "{st:?}");
    }

    /// A sender clock 0.1 % fast and one 0.1 % slow, a 44.1 kHz source, jittery arrivals: the
    /// lane stays on the due time (within 2 ms) without running dry or jumping.
    #[test]
    fn follows_a_drifting_sender_at_another_rate() {
        for drift in [0.001, -0.001] {
            let mut sim = Sim::new(44_100, drift, Duration::from_millis(8), false);
            let seen = sim.run(30.0, 5.0);
            let worst = seen
                .iter()
                .map(|(w, p)| diff(*w, *p).abs())
                .fold(0.0, f64::max);
            let st = *sim.src.stats.lock().unwrap();
            assert!(worst < 0.002, "drift {drift}: worst {worst:.4} s, {st:?}");
            assert_eq!(st.underruns, 0, "drift {drift}: {st:?}");
            assert_eq!(st.locks, 1, "drift {drift}: {st:?}");
            assert!(
                (st.correction - drift).abs() < 0.0005,
                "drift {drift}: correction {:.5}",
                st.correction
            );
        }
    }

    /// The sound with a picture's timecode is heard [`VIDEO_LEAD`] after the picture
    /// arrived, unless the sound needs longer (its transit, [`MARGIN`], the time from mixing to
    /// hearing and the block's length).
    #[test]
    fn pairs_with_the_pictures_by_timecode() {
        let block = Duration::from_millis(10);
        let latency = Duration::from_millis(20);
        for (pictures, late) in [(false, 0), (true, 0), (true, 50)] {
            let mut sim = Sim::new(48_000, 0.0, Duration::ZERO, pictures);
            sim.picture_delay = Duration::from_millis(late);
            sim.run(3.0, 1.0);
            let due = sim.src.stats.lock().unwrap().due;
            let heard = sim.t0 + Duration::from_millis(2990) + latency;
            // Sample X is made at `t0 + (X − tc0)`; its block arrives `transit` after the
            // block's end, the picture with the block's (first sample's) timecode `late` after.
            let sound = sim.transit + MARGIN + latency + block;
            let delay = if pictures {
                sound.max(sim.transit + block + Duration::from_millis(late) + VIDEO_LEAD)
            } else {
                sound
            };
            let want = audio::timecode(heard) as f64
                - (audio::timecode(sim.t0) as f64 - sim.tc(0))
                - delay.as_secs_f64() * TC;
            assert!(
                ((due - want) / TC).abs() < 0.0002,
                "pictures {pictures}, {late} ms late: due {:.4} s off",
                (due - want) / TC
            );
        }
    }

    #[test]
    fn a_gap_goes_silent_and_locks_on_again() {
        let mut sim = Sim::new(48_000, 0.0, Duration::from_millis(2), false);
        sim.run(2.0, 2.0);
        // Half a second of the sender's sound is lost on the way.
        let mut out = vec![0f32; 960];
        let mut silent = 0;
        for b in 200..400 {
            let now = sim.t0 + Duration::from_millis(10 * b);
            if !(200..250).contains(&b) {
                sim.send(now);
            } else {
                // Nothing arrives (the sender is away): what it made is lost.
                let lost = sim.rate as usize / 100;
                sim.sent += lost;
            }
            out.iter_mut().for_each(|v| *v = 0.0);
            sim.src.render_at(
                &mut out,
                &BlockCtx {
                    sample: 0,
                    heard_at: now + Duration::from_millis(20),
                    frames: 480,
                },
                now,
            );
            if out.iter().all(|v| *v == 0.0) {
                silent += 1;
            }
        }
        let st = *sim.src.stats.lock().unwrap();
        assert!(silent >= 3, "{silent} silent blocks");
        assert!(st.locks >= 2, "locked on again: {st:?}");
        // Playing again at the end.
        assert!(out.iter().any(|v| *v != 0.0));
    }

    /// E02-2: a sender whose timecodes come from a clock 0.1 % off its sample clock (20 ms
    /// apart after 20 s): the lane follows its timecodes, never starts over, never runs dry.
    #[test]
    fn follows_timecodes_from_another_clock() {
        for drift in [0.001, -0.001] {
            let mut sim = Sim::new(48_000, 0.0, Duration::from_millis(4), true);
            sim.tc_drift = drift;
            sim.run(45.0, 45.0);
            let st = *sim.src.stats.lock().unwrap();
            assert_eq!(st.locks, 1, "drift {drift}: {st:?}");
            assert_eq!(st.underruns, 0, "drift {drift}: {st:?}");
            assert!(st.error.abs() < 0.002, "drift {drift}: {st:?}");
        }
    }

    /// R2-1: the due time moves back (pictures start arriving 200 ms after their sound, as
    /// after a decoder's first keyframe): the lane jumps back to it at once instead of playing
    /// early while the drift loop catches up at 0.5 %.
    #[test]
    fn locks_on_again_when_the_due_time_moves_back() {
        let mut sim = Sim::new(48_000, 0.0, Duration::from_millis(2), false);
        sim.run(2.0, 2.0);
        sim.pictures = true;
        sim.picture_delay = Duration::from_millis(200);
        let seen = sim.run(3.0, 2.3);
        assert!(!seen.is_empty());
        for (want, played) in &seen {
            assert!(diff(*want, *played).abs() < 0.002, "{want} vs {played}");
        }
        let st = *sim.src.stats.lock().unwrap();
        assert_eq!(st.locks, 2, "{st:?}");
        assert_eq!(st.underruns, 0, "{st:?}");
    }

    /// E02-1: the engine may be handed a new lane for a source whose lane is still in the mix
    /// (fading out; [`audio::Mixer::resume`] keeps that one and drops the new one): making
    /// the new lane takes nothing from the feed, so the lane in the mix goes on unbroken.
    #[test]
    fn a_new_lane_takes_nothing_until_it_plays() {
        let mut sim = Sim::new(48_000, 0.0, Duration::from_millis(2), false);
        sim.run(2.0, 2.0);
        for _ in 0..5 {
            sim.send(sim.t0 + Duration::from_millis(10 * sim.blocks as u64));
            drop(LiveSoundSource::new(sim.feed.clone(), 2));
            sim.run(0.5, 0.0);
        }
        let seen = sim.run(1.0, 0.0);
        for (want, played) in &seen {
            assert!(diff(*want, *played).abs() < 0.002, "{want} vs {played}");
        }
        let st = *sim.src.stats.lock().unwrap();
        assert_eq!(st.locks, 1, "{st:?}");
        assert_eq!(st.underruns, 0, "{st:?}");
        assert_eq!(
            sim.feed.lane,
            LiveSoundSource::new(sim.feed.clone(), 2).feed.lane
        );
    }
}
