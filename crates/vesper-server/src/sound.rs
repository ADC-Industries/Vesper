//! Sound: the server's side of the audio bus ([`audio`] crate).
//!
//! - **Sources.** Each play with sound gets a [`PlaySource`] in the audio engine: the
//!   background video's soundtrack (group [`GROUP_MEDIA`]), a foreground video's and the live
//!   slide's video elements' ([`GROUP_SLIDE`], see [`crate::element_video`]) and the audio
//!   layer ([`GROUP_AUDIO`]: audio bins and audio playlists, [`LiveAudio`]). The live source on
//!   the video-input layer plays through its own lane ([`GROUP_LIVE`], see
//!   [`crate::live_sound`]). A play's level and routing come from its trigger's `AudioSettings`; each
//!   layer has its own level and mute (`setVolume` / `setMute` with `target: "layer"`).
//! - **A/V sync: the sound is the clock.** A play keeps a [`MediaClock`] (what the operator
//!   commanded: start, pause, seek) and, with sound, an [`AvSync`]. Its audio source aligns
//!   itself to the clock whenever the clock is changed (it reads the timeline sample due when
//!   its next block will be *heard*: device latency included), and from then on plays freely,
//!   publishing an anchor (this media time is heard at that instant). While the anchor is
//!   current, the play's time — and so the video frame chosen for every output tick — follows
//!   the sound actually played, so a device clock that runs slightly fast or slow drags the
//!   picture along instead of drifting away. Transport commands re-base the clock on the
//!   audio-derived time first, so pausing, resuming and seeking never jump. Plays without
//!   sound (no audio stream, a rate other than 1) keep the plain media clock. After an output
//!   stall (the output's latency steps up at once) a play's sound goes on where it was, every
//!   sample heard, so its picture steps back by the stall; a video element's sound skips the
//!   gap instead, so its picture never steps back ([`PlaySource::skipping_stalls`]).
//! - **Embedded audio.** Every IDN output whose binding has `embedAudio` gets a
//!   worker thread with its own tap of the master bus ([`spawn_embedded_audio`]): blocks are
//!   held until they are due (plus the output's video delay), scaled by the output's level and
//!   sent with the same timecode scale as the video ([`audio::timecode`]), so receivers line
//!   them up. The video worker never waits for it. On IDN the blocks go out on their own
//!   with these timecodes and video frames carry their tick's timecode, one timeline for
//!   both (see `output::idn`).

use audio::{AudioEngine, BlockCtx, EngineConfig, Group, Route, Source, SourceParams};
use media::{AUDIO_RATE, AudioDecoder, AudioTap, MediaClock};
use model::{AudioDeviceConfig, AudioSettings, Layer, MediaAssetId, OutputId, Playback, Repeat};
use protocol::{AudioLayerVolumes, AudioOutputState, PlaybackState, PlaylistContext, VolumeState};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Group of the audio layer.
pub const GROUP_AUDIO: Group = 1;
/// Group of the background video's soundtrack.
pub const GROUP_MEDIA: Group = 2;
/// Group of a foreground video's soundtrack.
pub const GROUP_SLIDE: Group = 3;
/// Group of the test tone.
pub const GROUP_TONE: Group = 4;
/// Group of the video-input layer's live source.
pub const GROUP_LIVE: Group = 5;

/// Source ID of the test tone (play IDs start at 1).
const TONE_ID: u64 = u64::MAX - 1;

/// Levels are clamped to this range, dB.
pub const MIN_DB: f64 = -96.0;
/// Highest level, dB.
pub const MAX_DB: f64 = 12.0;

/// The mixing group of a layer.
pub fn group_of(layer: Layer) -> Group {
    match layer {
        Layer::Audio => GROUP_AUDIO,
        Layer::Slide => GROUP_SLIDE,
        Layer::VideoInput => GROUP_LIVE,
        _ => GROUP_MEDIA,
    }
}

/// The mixer parameters of a play.
pub fn params_of(layer: Layer, a: &AudioSettings) -> SourceParams {
    SourceParams {
        group: group_of(layer),
        gain_db: a.volume_db.clamp(MIN_DB, MAX_DB),
        muted: a.muted,
        routes: a
            .channels
            .iter()
            .map(|r| Route {
                source: r.source,
                dest: r.dest,
                gain_db: r.gain_db,
                muted: r.muted,
            })
            .collect(),
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ---- A/V sync ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct Transport {
    clock: MediaClock,
    generation: u64,
}

/// "Media time `media` is heard at `heard`", published by the audio source.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    generation: u64,
    heard: Instant,
    media: f64,
    playing: bool,
}

/// What a play's control side and its audio source share (see the module docs). The engine
/// thread only `try_lock`s these.
pub struct AvSync {
    transport: Mutex<Transport>,
    generation: AtomicU64,
    anchor: Mutex<Option<Anchor>>,
}

impl AvSync {
    fn new(clock: MediaClock) -> Arc<AvSync> {
        Arc::new(AvSync {
            transport: Mutex::new(Transport {
                clock,
                generation: 0,
            }),
            generation: AtomicU64::new(0),
            anchor: Mutex::new(None),
        })
    }

    /// Hands a changed clock to the audio source (it re-aligns to it).
    fn publish(&self, clock: MediaClock) {
        let g = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *lock(&self.transport) = Transport {
            clock,
            generation: g,
        };
    }

    /// The audio-derived time at `at`, while the source plays in step with the current clock.
    fn time_at(&self, at: Instant) -> Option<f64> {
        let a = (*lock(&self.anchor))?;
        (a.playing && a.generation == self.generation.load(Ordering::Acquire))
            .then(|| a.media + audio::secs_between(a.heard, at))
    }
}

/// A play's clock: the commanded [`MediaClock`], followed by the sound when there is one.
pub struct SyncedClock {
    clock: Mutex<MediaClock>,
    sync: Option<Arc<AvSync>>,
}

impl SyncedClock {
    /// A paused clock at 0; `sound`: the play has an audio source.
    pub fn new(rate: f64, sound: bool) -> SyncedClock {
        let c = MediaClock::paused_at(0.0, rate);
        SyncedClock {
            clock: Mutex::new(c),
            sync: sound.then(|| AvSync::new(c)),
        }
    }

    /// The sync handle for the play's audio source.
    pub fn sync(&self) -> Option<Arc<AvSync>> {
        self.sync.clone()
    }

    /// A copy of the commanded clock.
    pub fn snapshot(&self) -> MediaClock {
        *lock(&self.clock)
    }

    /// Timeline time at `at` (the sound's, when it is in step).
    pub fn time_at(&self, at: Instant) -> f64 {
        let c = self.snapshot();
        if c.is_paused() {
            return c.time_at(at);
        }
        self.sync
            .as_ref()
            .and_then(|s| s.time_at(at))
            .unwrap_or_else(|| c.time_at(at))
    }

    /// Starts at timeline 0 at `at`.
    pub fn start(&self, at: Instant, rate: f64) {
        self.start_at(at, 0.0, rate);
    }

    /// Starts at timeline time `t` at `at` (a play that continues another one).
    pub fn start_at(&self, at: Instant, t: f64, rate: f64) {
        let c = MediaClock::start(at, t.max(0.0), rate);
        *lock(&self.clock) = c;
        if let Some(s) = &self.sync {
            s.publish(c);
        }
    }

    /// Changes the clock after re-basing it on the current (audio-derived) time.
    pub fn modify(&self, at: Instant, f: impl FnOnce(&mut MediaClock)) {
        let t = self.time_at(at);
        let mut c = lock(&self.clock);
        if !c.is_paused() {
            *c = MediaClock::start(at, t, c.rate());
        }
        f(&mut c);
        let new = *c;
        drop(c);
        if let Some(s) = &self.sync {
            s.publish(new);
        }
    }
}

/// The audio-engine side of a play: reads its decoder's ring at the timeline position the
/// clock asks for (see the module docs).
pub struct PlaySource {
    tap: AudioTap,
    sync: Arc<AvSync>,
    transport: Option<Transport>,
    generation: u64,
    pos: i64,
    playing: bool,
    /// Fade-in length, s (from the play's start).
    fade_in: f64,
    /// Fade-out before the end: (end, length), s.
    fade_out: Option<(f64, f64)>,
    /// Steps over an output stall (see [`PlaySource::skipping_stalls`]).
    skip_stalls: bool,
    /// When the block after the last one is heard (where the next block follows on).
    next_heard: Option<Instant>,
}

/// A block heard this much later than where the last one ended is after an output stall (the
/// engine takes steps of its output latency of 20 ms or more at once and smooths smaller
/// ones, under half a millisecond per block).
const STALL: f64 = 0.015;

impl PlaySource {
    /// A source reading `tap`, following `sync`; fades in over `fade_in` s from the start and
    /// out over `fade_out.1` s before `fade_out.0`.
    pub fn new(
        tap: AudioTap,
        sync: Arc<AvSync>,
        fade_in: f64,
        fade_out: Option<(f64, f64)>,
    ) -> PlaySource {
        PlaySource {
            tap,
            sync,
            transport: None,
            generation: u64::MAX,
            pos: 0,
            playing: false,
            fade_in,
            fade_out,
            skip_stalls: false,
            next_heard: None,
        }
    }

    /// A source that steps over an output stall (M-5, owner's decision for video elements):
    /// when a block is heard later than where the last one ended (the output stalled, its
    /// latency stepped up), the sound re-aligns to the clock at once, skipping the gap (to the
    /// commanded clock when that is further on), so the time the picture follows never steps
    /// back. Without it the sound plays on from where it was, every sample heard, and the
    /// picture that follows it steps back by the stall.
    pub fn skipping_stalls(mut self) -> PlaySource {
        self.skip_stalls = true;
        self
    }

    /// Moves the read position to timeline sample `target`: small steps forward are skipped in
    /// the queue; anything else (backwards beyond 5 ms, far forward) restarts the decoder
    /// there.
    fn align(&mut self, target: i64) {
        let d = target - self.pos;
        if !(-240..=24_000).contains(&d) {
            self.tap.seek(target);
            self.pos = target;
        } else if d > 0 {
            self.pos = target;
        }
    }

    /// A source whose tap was opened at timeline time `t` (its decoding starts there): the
    /// first alignment to the clock steps forward from `t` instead of seeking the decoder.
    pub fn starting_at(mut self, t: f64) -> PlaySource {
        self.pos = (t.max(0.0) * AUDIO_RATE as f64).round() as i64;
        self
    }

    fn envelope(&self, t: f64) -> f32 {
        let mut g = 1.0f32;
        if self.fade_in > 0.0 {
            g = g.min(audio::dsp::fade_in(t / self.fade_in));
        }
        if let Some((end, len)) = self.fade_out
            && len > 0.0
        {
            g = g.min(audio::dsp::fade_in((end - t) / len));
        }
        g
    }
}

impl Source for PlaySource {
    fn channels(&self) -> usize {
        self.tap.channels()
    }

    fn render(&mut self, out: &mut [f32], ctx: &BlockCtx) -> bool {
        self.tap.discard_stale();
        if let Ok(t) = self.sync.transport.try_lock() {
            self.transport = Some(*t);
        }
        let Some(tr) = self.transport else {
            return true;
        };
        let rate = AUDIO_RATE as f64;
        let n = ctx.frames;
        let follows = self
            .next_heard
            .replace(audio::offset(ctx.heard_at, n as f64 / rate));
        let mut aligned = false;
        if tr.generation != self.generation {
            self.generation = tr.generation;
            if !tr.clock.is_paused() {
                self.align((tr.clock.time_at(ctx.heard_at) * rate).round() as i64);
                aligned = true;
            }
        }
        let was = self.playing;
        self.playing = !tr.clock.is_paused();
        // After an output stall: over the gap, to the clock (see `skipping_stalls`).
        if self.skip_stalls
            && !aligned
            && was
            && self.playing
            && let Some(e) = follows
        {
            let gap = audio::secs_between(e, ctx.heard_at);
            if gap > STALL {
                let clock = (tr.clock.time_at(ctx.heard_at) * rate).round() as i64;
                self.align(clock.max(self.pos + (gap * rate).round() as i64));
            }
        }
        let ch = self.tap.channels();
        if was || self.playing {
            self.tap.read(self.pos, out, n);
            let (t0, t1) = (self.pos as f64 / rate, (self.pos + n as i64) as f64 / rate);
            let (g0, g1) = (self.envelope(t0), self.envelope(t1));
            for f in 0..n {
                let mut g = g0 + (g1 - g0) * f as f32 / n as f32;
                // Anti-click ramps on pause and resume.
                if was && !self.playing {
                    g *= 1.0 - f as f32 / n as f32;
                } else if !was && self.playing {
                    g *= f as f32 / n as f32;
                }
                if g != 1.0 {
                    for v in &mut out[f * ch..(f + 1) * ch] {
                        *v *= g;
                    }
                }
            }
        }
        if let Ok(mut a) = self.sync.anchor.try_lock() {
            *a = Some(Anchor {
                generation: self.generation,
                heard: ctx.heard_at,
                media: self.pos as f64 / rate,
                playing: self.playing,
            });
        }
        if was || self.playing {
            self.pos += n as i64;
        }
        true
    }
}

// ---- the audio layer ----------------------------------------------------------------------

/// One play of an audio file on the audio layer.
pub struct LiveAudio {
    /// Play ID (also the audio engine source ID).
    pub play: u64,
    /// The asset.
    pub asset: MediaAssetId,
    /// Display name.
    pub name: String,
    /// Playback settings of the trigger.
    pub playback: Playback,
    /// Audio settings of the trigger.
    pub audio: AudioSettings,
    decoder: AudioDecoder,
    clock: SyncedClock,
    /// Length of the file's audio, s.
    file_len: Option<f64>,
}

impl LiveAudio {
    /// Opens the file (blocking) and returns the play with the reading end of its decoder.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        play: u64,
        asset: MediaAssetId,
        name: String,
        path: &Path,
        playback: &Playback,
        audio: &AudioSettings,
        channels: usize,
        start: f64,
    ) -> anyhow::Result<(LiveAudio, AudioTap)> {
        let loops = match playback.repeat {
            Repeat::Loop | Repeat::LoopFor { .. } => media::Loops::Forever,
            Repeat::LoopCount { count } => media::Loops::Count(count.max(1)),
            _ => media::Loops::Once,
        };
        let (decoder, tap) = media::open_audio(
            path,
            media::AudioOptions {
                channels,
                in_point: playback.in_point.max(0.0),
                out_point: playback.out_point,
                loops,
                start: start.max(0.0),
            },
            None,
        )?
        .ok_or_else(|| anyhow::anyhow!("{} has no audio", path.display()))?;
        let file_len = decoder.info().duration;
        tracing::info!(
            play,
            file = %path.display(),
            codec = %decoder.info().codec,
            rate = decoder.info().sample_rate,
            channels = decoder.info().channels,
            "audio opened"
        );
        Ok((
            LiveAudio {
                play,
                asset,
                name,
                playback: playback.clone(),
                audio: audio.clone(),
                decoder,
                clock: SyncedClock::new(1.0, true),
                file_len,
            },
            tap,
        ))
    }

    /// The sync handle for its source.
    pub fn sync(&self) -> Arc<AvSync> {
        self.clock.sync().expect("audio plays always sync")
    }

    /// Starts the clock at timeline 0 at `at`.
    pub fn start(&self, at: Instant) {
        self.clock.start(at, 1.0);
    }

    /// Starts the clock at timeline time `t` at `at` (opened with `start` = `t`).
    pub fn start_at(&self, at: Instant, t: f64) {
        self.clock.start_at(at, t, 1.0);
    }

    /// Timeline time at `at`.
    pub fn time_at(&self, at: Instant) -> f64 {
        self.clock.time_at(at)
    }

    /// Length of one pass, s.
    pub fn region_len(&self) -> Option<f64> {
        self.decoder
            .region_len()
            .map(|s| s as f64 / AUDIO_RATE as f64)
            .or_else(|| {
                let end = self.playback.out_point.or(self.file_len)?;
                Some((end - self.playback.in_point.max(0.0)).max(0.0))
            })
    }

    /// Timeline time at which the play ends (`None` = loops forever).
    pub fn end(&self) -> Option<f64> {
        let exact = self.decoder.end().map(|s| s as f64 / AUDIO_RATE as f64);
        match self.playback.repeat {
            Repeat::Loop => None,
            Repeat::LoopFor { seconds } => Some(seconds.max(0.0)),
            Repeat::LoopCount { count } => {
                exact.or_else(|| self.region_len().map(|l| l * count.max(1) as f64))
            }
            _ => exact.or_else(|| self.region_len()),
        }
    }

    /// Whether the play loops.
    pub fn looping(&self) -> bool {
        match self.playback.repeat {
            Repeat::Loop | Repeat::LoopFor { .. } => true,
            Repeat::LoopCount { count } => count > 1,
            _ => false,
        }
    }

    fn ended_at_time(&self, t: f64) -> bool {
        match self.playback.repeat {
            Repeat::Loop => false,
            Repeat::LoopFor { seconds } => t >= seconds,
            _ => self
                .decoder
                .end()
                .is_some_and(|e| t >= e as f64 / AUDIO_RATE as f64),
        }
    }

    /// Transport state at `at`.
    pub fn state_at(&self, at: Instant) -> PlaybackState {
        let c = self.clock.snapshot();
        if self.ended_at_time(self.time_at(at)) {
            PlaybackState::Stopped
        } else if c.is_paused() {
            PlaybackState::Paused
        } else {
            PlaybackState::Playing
        }
    }

    /// Position within the current pass, s.
    pub fn position_at(&self, at: Instant) -> f64 {
        let t = self.time_at(at).max(0.0);
        let t = self.end().map_or(t, |e| t.min(e));
        match self.region_len() {
            Some(len) if len > 0.0 => {
                let k = (t / len).floor();
                let off = t - k * len;
                if off < 1e-6 && t > 0.0 && self.end().is_some_and(|e| t >= e - 1e-6) {
                    len
                } else {
                    off
                }
            }
            _ => t,
        }
    }

    /// Resumes (a play that ended starts again).
    pub fn play(&self, at: Instant) {
        if self.ended_at_time(self.time_at(at)) {
            self.seek_timeline(0.0, at);
        }
        self.clock.modify(at, |c| c.resume(at));
    }

    /// Pauses.
    pub fn pause(&self, at: Instant) {
        self.clock.modify(at, |c| c.pause(at));
    }

    /// Whether it is playing at `at`.
    pub fn playing(&self, at: Instant) -> bool {
        self.state_at(at) == PlaybackState::Playing
    }

    /// Jumps to timeline time `t`.
    pub fn seek_timeline(&self, t: f64, at: Instant) {
        let mut t = t.max(0.0);
        if let Some(e) = self.end() {
            t = t.min((e - 0.01).max(0.0));
        }
        self.clock.modify(at, |c| c.seek(t, at));
    }

    /// Jumps to `seconds` from the in point, within the current pass.
    pub fn seek_position(&self, seconds: f64, at: Instant) {
        let t = self.time_at(at);
        let (base, off) = match self.region_len() {
            Some(len) if len > 0.0 => ((t / len).floor().max(0.0) * len, seconds.clamp(0.0, len)),
            _ => (0.0, seconds.max(0.0)),
        };
        self.seek_timeline(base + off, at);
    }

    /// Jumps by `seconds`.
    pub fn skip(&self, seconds: f64, at: Instant) {
        let t = self.time_at(at);
        self.seek_timeline(t + seconds, at);
    }

    /// Back to the in point, playing.
    pub fn restart(&self, at: Instant) {
        self.seek_timeline(0.0, at);
        self.clock.modify(at, |c| c.resume(at));
    }

    /// The commanded clock's rate.
    pub fn rate(&self) -> f64 {
        1.0
    }
}

// ---- the hub ------------------------------------------------------------------------------

/// Level of an output's embedded audio.
#[derive(Debug, Default)]
pub struct OutputAudio {
    db: AtomicU64,
    muted: AtomicBool,
}

impl OutputAudio {
    /// Gain (dB) and mute.
    pub fn get(&self) -> (f64, bool) {
        (
            f64::from_bits(self.db.load(Ordering::Relaxed)),
            self.muted.load(Ordering::Relaxed),
        )
    }

    fn gain(&self) -> f32 {
        let (db, m) = self.get();
        if m { 0.0 } else { audio::dsp::db_to_gain(db) }
    }
}

/// The server's audio: the engine, layer and output levels, the device configuration.
pub struct Hub {
    /// The engine.
    pub engine: AudioEngine,
    layers: Mutex<AudioLayerVolumes>,
    outputs: Mutex<HashMap<OutputId, Arc<OutputAudio>>>,
    config: Mutex<AudioDeviceConfig>,
    device_override: Option<String>,
}

impl Hub {
    /// Starts the engine for `config` (`device_override` replaces its device).
    pub fn start(config: &AudioDeviceConfig, device_override: Option<String>) -> Hub {
        let device = device_override
            .clone()
            .unwrap_or_else(|| config.device.clone());
        let engine = AudioEngine::start(EngineConfig {
            device: device.clone(),
            channel_map: config.channel_map.clone(),
            master_db: config.master_gain_db.clamp(MIN_DB, MAX_DB),
            ..EngineConfig::default()
        });
        let st = engine.status();
        match st.state {
            audio::OutputState::Ok => {
                tracing::info!(device = %st.open, latency_ms = st.latency_ms, "audio output")
            }
            audio::OutputState::Null => {
                tracing::info!("no audio device configured: null audio output (keeps time)")
            }
            audio::OutputState::Error => tracing::warn!(
                "audio device {device:?} unavailable ({}): null audio output",
                st.detail.as_deref().unwrap_or("?")
            ),
        }
        Hub {
            engine,
            layers: Mutex::new(AudioLayerVolumes::default()),
            outputs: Mutex::new(HashMap::new()),
            config: Mutex::new(config.clone()),
            device_override,
        }
    }

    /// Bus channels.
    pub fn channels(&self) -> usize {
        self.engine.channels()
    }

    /// The stored device configuration in use.
    pub fn config(&self) -> AudioDeviceConfig {
        lock(&self.config).clone()
    }

    /// Applies a (new) device configuration: reopens the device when it changed, sets the
    /// master gain.
    pub fn apply_config(&self, cfg: &AudioDeviceConfig) {
        let old = std::mem::replace(&mut *lock(&self.config), cfg.clone());
        if self.device_override.is_none()
            && (old.device != cfg.device || old.channel_map != cfg.channel_map)
        {
            self.engine.set_device(&cfg.device, cfg.channel_map.clone());
        }
        if old.master_gain_db != cfg.master_gain_db {
            self.engine
                .set_master(cfg.master_gain_db.clamp(MIN_DB, MAX_DB));
        }
    }

    /// The device in use (the override, else the configured one).
    pub fn device(&self) -> String {
        self.device_override
            .clone()
            .unwrap_or_else(|| lock(&self.config).device.clone())
    }

    /// Output latency estimate (the time from mixing to hearing).
    pub fn latency(&self) -> Duration {
        Duration::from_secs_f64(self.engine.status().latency_ms.max(0.0) / 1000.0)
    }

    /// Sets a layer's level and / or mute.
    pub fn set_layer(&self, layer: Layer, db: Option<f64>, muted: Option<bool>) -> bool {
        let mut l = lock(&self.layers);
        let v = match layer {
            Layer::Audio => &mut l.audio,
            Layer::Media => &mut l.media,
            Layer::Slide => &mut l.slide,
            Layer::VideoInput => &mut l.video_input,
            _ => return false,
        };
        if let Some(d) = db {
            v.db = d.clamp(MIN_DB, MAX_DB);
        }
        if let Some(m) = muted {
            v.muted = m;
        }
        let VolumeState { db, muted } = *v;
        drop(l);
        self.engine.set_group(group_of(layer), db, muted);
        true
    }

    /// Layer levels.
    pub fn layers(&self) -> AudioLayerVolumes {
        *lock(&self.layers)
    }

    /// The embedded-audio level of an output (created at 0 dB).
    pub fn output(&self, id: OutputId) -> Arc<OutputAudio> {
        lock(&self.outputs)
            .entry(id)
            .or_insert_with(|| Arc::new(OutputAudio::default()))
            .clone()
    }

    /// Sets an output's embedded-audio level and / or mute.
    pub fn set_output(&self, id: OutputId, db: Option<f64>, muted: Option<bool>) {
        let o = self.output(id);
        if let Some(d) = db {
            o.db.store(d.clamp(MIN_DB, MAX_DB).to_bits(), Ordering::Relaxed);
        }
        if let Some(m) = muted {
            o.muted.store(m, Ordering::Relaxed);
        }
    }

    /// Plays a test tone on the master bus.
    pub fn test_tone(&self, freq: f64, seconds: f64) {
        let ch = self.channels();
        self.engine.add_source(
            TONE_ID,
            Box::new(audio::tone::Tone::new(freq, -18.0, seconds, ch)),
            SourceParams {
                group: GROUP_TONE,
                ..SourceParams::default()
            },
            0.0,
        );
    }

    /// The protocol view.
    pub fn state(
        &self,
        playlist: Option<PlaylistContext>,
        shuffle: bool,
        repeat: bool,
    ) -> protocol::AudioState {
        let st = self.engine.status();
        let (master_db, muted) = self.engine.master();
        protocol::AudioState {
            device: self.device(),
            output_state: match st.state {
                audio::OutputState::Ok => AudioOutputState::Ok,
                audio::OutputState::Null => AudioOutputState::Null,
                audio::OutputState::Error => AudioOutputState::Error,
            },
            detail: st.detail,
            master_db: (master_db * 10.0).round() / 10.0,
            muted,
            levels: self.engine.levels(),
            xruns: self.engine.xruns(),
            layers: self.layers(),
            shuffle,
            repeat,
            playlist,
        }
    }
}

/// Starts the embedded-audio worker of a network output (IDN): its own tap of the master bus, blocks held until due (`delay` added: the output's video delay), scaled by the output's level,
/// sent with their timecodes. Stops when `stop` is set.
pub fn spawn_embedded_audio(
    name: &str,
    hub: &Hub,
    mut sender: Box<dyn output::AudioSink>,
    level: Arc<OutputAudio>,
    delay: Arc<std::sync::atomic::AtomicU32>,
    frame_period: Arc<std::sync::atomic::AtomicU64>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    let tap = hub.engine.tap();
    let label = name.to_string();
    std::thread::Builder::new()
        .name("out audio".into())
        .spawn(move || {
            let mut held: VecDeque<audio::TapBlock> = VecDeque::new();
            let mut errors = 0u64;
            while !stop.load(Ordering::Acquire) {
                if let Some(b) = tap.recv_timeout(Duration::from_millis(50)) {
                    held.push_back(b);
                }
                while let Some(b) = tap.recv_timeout(Duration::ZERO) {
                    held.push_back(b);
                }
                // Both read each pass: a new delay or screen frame rate applies at once.
                let shift = Duration::from_nanos(frame_period.load(Ordering::Relaxed))
                    * delay.load(Ordering::Relaxed);
                let now = Instant::now();
                while held.front().is_some_and(|b| b.heard_at + shift <= now) {
                    let mut b = held.pop_front().expect("front");
                    let g = level.gain();
                    if g != 1.0 {
                        b.data.iter_mut().for_each(|v| *v *= g);
                    }
                    let tc = audio::timecode(b.heard_at + shift);
                    if let Err(e) = sender.send(&b.data, b.channels, AUDIO_RATE, tc) {
                        errors += 1;
                        if errors.is_power_of_two() {
                            tracing::warn!(output = %label, "embedded audio send failed: {e:#}");
                        }
                    }
                    tap.recycle(b);
                }
                // Sleep until the next block is due (short sleeps keep stopping prompt).
                if let Some(b) = held.front() {
                    let due = b.heard_at + shift;
                    let now = Instant::now();
                    if due > now {
                        std::thread::sleep((due - now).min(Duration::from_millis(20)));
                    }
                }
            }
            if tap.dropped() > 0 {
                tracing::debug!(output = %label, dropped = tap.dropped(), "embedded audio tap overflowed");
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mono 16-bit WAV of `secs` of silence at the bus rate.
    fn wav(path: &Path, secs: f64) {
        let n = (AUDIO_RATE as f64 * secs) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + n * 2).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&AUDIO_RATE.to_le_bytes());
        b.extend_from_slice(&(AUDIO_RATE * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(n * 2).to_le_bytes());
        b.resize(b.len() + n as usize * 2, 0);
        std::fs::write(path, b).unwrap();
    }

    /// The play's time, as the picture follows it, at the present instants of 10 ms blocks
    /// whose sound is heard 50 ms later, until the output stalls (block 30) and is heard
    /// 250 ms later from then on.
    fn times_through_a_stall(skip: bool) -> Vec<f64> {
        let path =
            std::env::temp_dir().join(format!("vesper-stall-{}-{skip}.wav", std::process::id()));
        wav(&path, 3.0);
        let (decoder, tap) = media::open_audio(&path, media::AudioOptions::default(), None)
            .unwrap()
            .unwrap();
        let clock = SyncedClock::new(1.0, true);
        let mut src = PlaySource::new(tap, clock.sync().unwrap(), 0.0, None);
        if skip {
            src = src.skipping_stalls();
        }
        let t0 = Instant::now();
        clock.start_at(t0, 0.0, 1.0);
        let n = 480;
        let mut out = vec![0.0; n * 2];
        let mut times = Vec::new();
        for i in 0..60 {
            let present = t0 + Duration::from_millis(10 * i as u64);
            let latency = if i < 30 { 50 } else { 250 };
            let ctx = BlockCtx {
                sample: (i * n) as i64,
                heard_at: present + Duration::from_millis(latency),
                frames: n,
            };
            out.fill(0.0);
            src.render(&mut out, &ctx);
            times.push(clock.time_at(present));
        }
        drop(decoder);
        let _ = std::fs::remove_file(&path);
        times
    }

    /// M-5: after an output stall a video element's sound (a source that skips stalls)
    /// re-aligns to its clock over the gap: the time its picture follows goes on evenly and
    /// never steps back. A play that does not skip keeps every sample: its time steps back by
    /// the stall.
    #[test]
    fn a_stall_never_steps_the_picture_back() {
        let times = times_through_a_stall(true);
        for (i, w) in times.windows(2).enumerate() {
            let step = w[1] - w[0];
            assert!(
                (step - 0.010).abs() < 0.001,
                "block {}: the time moved {step:.4} s ({times:?})",
                i + 1
            );
        }
        let kept = times_through_a_stall(false);
        let back = kept[29] - kept[30];
        assert!(
            (back - 0.190).abs() < 0.001,
            "a play that keeps every sample steps back by the stall: {back:.4} s"
        );
    }
}
