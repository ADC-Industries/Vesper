//! The audio engine: one thread that owns the output device and the [`Mixer`].

use crate::device::{Health, NullOutput, Output, SoundCard};
use crate::mixer::{BlockCtx, Group, Mixer, Source, SourceParams};
use crate::{MAX_CHANNELS, SAMPLE_RATE, dsp};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How the engine runs.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineConfig {
    /// Output device (`""` = none: null output).
    pub device: String,
    /// Bus channels.
    pub channels: usize,
    /// Bus channel → device channel (empty = identity).
    pub channel_map: Vec<u32>,
    /// Master gain, dB.
    pub master_db: f64,
    /// Frames per period (mixing block). 480 = 10 ms.
    pub period: usize,
    /// Output buffering in periods: the sound held before the device is `periods − 1`
    /// periods (20 ms; the device's own buffer comes on top, about a quarter of that plus
    /// its output latency); the null output keeps as much queued.
    pub periods: usize,
    /// How often a device that failed is tried again.
    pub retry: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            device: String::new(),
            channels: crate::DEFAULT_CHANNELS,
            channel_map: Vec::new(),
            master_db: 0.0,
            period: 480,
            periods: 3,
            retry: Duration::from_secs(10),
        }
    }
}

/// State of the output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputState {
    /// Playing on the configured device.
    Ok,
    /// No device configured: null output.
    #[default]
    Null,
    /// The configured device cannot be used: null output.
    Error,
}

/// What the engine reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineStatus {
    /// The configured device (`""` = none).
    pub device: String,
    /// The device actually open (`null`, `hw:1,0`, `plughw:1,0`).
    pub open: String,
    /// Output state.
    pub state: OutputState,
    /// Why the configured device is not used.
    pub detail: Option<String>,
    /// Output latency (from mixing a block to hearing it), ms: on a sound card what its lane
    /// holds plus the device's output delay, as `idn-audio` measures them (updated every
    /// second).
    pub latency_ms: f64,
    /// Device channels.
    pub device_channels: usize,
}

/// A block of program audio for a tap (interleaved bus channels).
#[derive(Debug)]
pub struct TapBlock {
    /// When its first frame is heard at the output.
    pub heard_at: Instant,
    /// [`crate::timecode`] of `heard_at`.
    pub timecode: i64,
    /// Bus sample index of its first frame.
    pub sample: i64,
    /// Channels.
    pub channels: usize,
    /// Frames.
    pub frames: usize,
    /// `frames × channels` samples.
    pub data: Vec<f32>,
}

struct TapSender {
    tx: SyncSender<TapBlock>,
    free: Receiver<Vec<f32>>,
    dropped: Arc<AtomicU64>,
}

/// The receiving end of a program-audio tap ([`AudioEngine::tap`]). The engine never waits for
/// it: when its queue (about 0.5 s) is full, blocks are dropped and counted.
pub struct TapReceiver {
    rx: Receiver<TapBlock>,
    free: SyncSender<Vec<f32>>,
    dropped: Arc<AtomicU64>,
}

impl TapReceiver {
    /// The next block, waiting up to `timeout`.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<TapBlock> {
        self.rx.recv_timeout(timeout).ok()
    }

    /// Hands a block's buffer back for reuse (saves an allocation per block).
    pub fn recycle(&self, block: TapBlock) {
        let _ = self.free.try_send(block.data);
    }

    /// Blocks dropped because this tap's queue was full.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

enum Cmd {
    Add {
        id: u64,
        src: Box<dyn Source>,
        params: SourceParams,
        fade: f64,
    },
    Remove {
        id: u64,
        fade: f64,
    },
    Resume {
        id: u64,
        src: Box<dyn Source>,
        params: SourceParams,
        fade: f64,
    },
    Params {
        id: u64,
        params: SourceParams,
    },
    Group {
        group: Group,
        db: f64,
        muted: bool,
    },
    Master(f64),
    Mute(bool),
    Device {
        name: String,
        map: Vec<u32>,
    },
    Tap(TapSender),
    Stop,
}

struct Shared {
    status: Mutex<EngineStatus>,
    xruns: AtomicU64,
    peaks: [AtomicU32; MAX_CHANNELS],
    blocks: AtomicU64,
    sources: AtomicU32,
    channels: usize,
    master: Mutex<(f64, bool)>,
    stopped: AtomicBool,
}

/// The audio engine (see the crate docs). Cheap to share (`Arc`); dropping the last handle
/// stops the thread.
pub struct AudioEngine {
    tx: Mutex<Sender<Cmd>>,
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for AudioEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioEngine")
            .field("status", &self.status())
            .finish()
    }
}

impl AudioEngine {
    /// Starts the engine thread (opens the device; a device that cannot be opened is
    /// reported in [`AudioEngine::status`] and the null output runs instead).
    pub fn start(config: EngineConfig) -> AudioEngine {
        let channels = config.channels.clamp(1, MAX_CHANNELS);
        let shared = Arc::new(Shared {
            status: Mutex::new(EngineStatus::default()),
            xruns: AtomicU64::new(0),
            peaks: std::array::from_fn(|_| AtomicU32::new(0)),
            blocks: AtomicU64::new(0),
            sources: AtomicU32::new(0),
            channels,
            master: Mutex::new((config.master_db, false)),
            stopped: AtomicBool::new(false),
        });
        let (tx, rx) = std::sync::mpsc::channel();
        let sh = shared.clone();
        let thread = std::thread::Builder::new()
            .name("audio engine".into())
            .spawn(move || run(config, sh, rx))
            .expect("spawn the audio engine thread");
        let e = AudioEngine {
            tx: Mutex::new(tx),
            shared,
            thread: Mutex::new(Some(thread)),
        };
        // The first status is there before `start` returns.
        let t0 = Instant::now();
        while e.shared.blocks.load(Ordering::Acquire) == 0
            && t0.elapsed() < Duration::from_secs(5)
            && !e.shared.stopped.load(Ordering::Acquire)
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        e
    }

    fn send(&self, c: Cmd) {
        let _ = self.tx.lock().unwrap_or_else(|e| e.into_inner()).send(c);
    }

    /// Bus channels.
    pub fn channels(&self) -> usize {
        self.shared.channels
    }

    /// Adds a source (replaces one with the same `id`), fading in over `fade` seconds.
    pub fn add_source(&self, id: u64, src: Box<dyn Source>, params: SourceParams, fade: f64) {
        self.send(Cmd::Add {
            id,
            src,
            params,
            fade,
        });
    }

    /// Brings back source `id` while it fades out (it fades in again from its current
    /// level over `fade` seconds, with `params`; `src` is dropped), or adds `src` as `id`
    /// when it is not in the mix (see [`crate::Mixer::resume`]).
    pub fn resume_source(&self, id: u64, src: Box<dyn Source>, params: SourceParams, fade: f64) {
        self.send(Cmd::Resume {
            id,
            src,
            params,
            fade,
        });
    }

    /// Fades a source out over `fade` seconds and removes it.
    pub fn remove_source(&self, id: u64, fade: f64) {
        self.send(Cmd::Remove { id, fade });
    }

    /// Changes a source's level and routing.
    pub fn set_params(&self, id: u64, params: SourceParams) {
        self.send(Cmd::Params { id, params });
    }

    /// Sets a group's (layer's) level.
    pub fn set_group(&self, group: Group, db: f64, muted: bool) {
        self.send(Cmd::Group { group, db, muted });
    }

    /// Sets the master gain.
    pub fn set_master(&self, db: f64) {
        self.shared
            .master
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .0 = db;
        self.send(Cmd::Master(db));
    }

    /// Mutes the master.
    pub fn set_mute(&self, muted: bool) {
        self.shared
            .master
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .1 = muted;
        self.send(Cmd::Mute(muted));
    }

    /// Master gain (dB) and mute.
    pub fn master(&self) -> (f64, bool) {
        *self.shared.master.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Switches the output device (`""` = null output). Sources keep playing.
    pub fn set_device(&self, name: &str, channel_map: Vec<u32>) {
        self.send(Cmd::Device {
            name: name.to_string(),
            map: channel_map,
        });
    }

    /// A program-audio tap: every mixed block (after the master and soft clip, before the
    /// device channel map), stamped with the time it is heard.
    pub fn tap(&self) -> TapReceiver {
        let (tx, rx) = std::sync::mpsc::sync_channel(48);
        let (ftx, frx) = std::sync::mpsc::sync_channel(48);
        let dropped = Arc::new(AtomicU64::new(0));
        self.send(Cmd::Tap(TapSender {
            tx,
            free: frx,
            dropped: dropped.clone(),
        }));
        TapReceiver {
            rx,
            free: ftx,
            dropped,
        }
    }

    /// Output status.
    pub fn status(&self) -> EngineStatus {
        self.shared
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Underruns since the start.
    pub fn xruns(&self) -> u64 {
        self.shared.xruns.load(Ordering::Relaxed)
    }

    /// Blocks mixed since the start.
    pub fn blocks(&self) -> u64 {
        self.shared.blocks.load(Ordering::Relaxed)
    }

    /// Sources in the mix.
    pub fn sources(&self) -> usize {
        self.shared.sources.load(Ordering::Relaxed) as usize
    }

    /// Peak level per bus channel over the last 100 ms, dBFS rounded (see [`dsp::meter_db`]).
    pub fn levels(&self) -> Vec<f64> {
        self.peaks().into_iter().map(dsp::meter_db).collect()
    }

    /// Peak amplitude per bus channel over the last 100 ms.
    pub fn peaks(&self) -> Vec<f32> {
        (0..self.shared.channels)
            .map(|c| f32::from_bits(self.shared.peaks[c].load(Ordering::Relaxed)))
            .collect()
    }

    /// Stops the thread (the device is closed).
    pub fn stop(&self) {
        self.send(Cmd::Stop);
        if let Some(h) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = h.join();
        }
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Sound held before the device, ms.
fn hold_ms(cfg: &EngineConfig) -> u32 {
    ((cfg.period * (cfg.periods.max(2) - 1)) as f64 * 1000.0 / SAMPLE_RATE as f64).round() as u32
}

fn null_output(channels: usize, cfg: &EngineConfig) -> Box<dyn Output> {
    Box::new(NullOutput::new(
        channels,
        cfg.period * (cfg.periods.max(2) - 1),
    ))
}

fn open_output(
    device: &str,
    channels: usize,
    cfg: &EngineConfig,
) -> (Box<dyn Output>, OutputState, Option<String>) {
    if device.is_empty() || device == "null" {
        return (null_output(channels, cfg), OutputState::Null, None);
    }
    match open_card(device, hold_ms(cfg)) {
        Ok(o) => (Box::new(o), OutputState::Ok, None),
        Err(e) => {
            let msg = format!("{e:#}");
            log::warn!("audio: {msg}; using the null output");
            (null_output(channels, cfg), OutputState::Error, Some(msg))
        }
    }
}

/// Opens a sound card. In tests, a device named `test-slow:<ms>` takes that long and then
/// fails, like a card that is slow to open and not there.
fn open_card(device: &str, hold_ms: u32) -> anyhow::Result<SoundCard> {
    #[cfg(test)]
    if let Some(ms) = device.strip_prefix("test-slow:") {
        std::thread::sleep(Duration::from_millis(ms.parse().unwrap_or(300)));
        anyhow::bail!("cannot open {device}: test device");
    }
    SoundCard::open(device, hold_ms)
}

/// Tries the device again on a thread of its own (opening takes a moment: the engine keeps
/// mixing on the null output meanwhile).
fn reopen(device: &str, cfg: &EngineConfig) -> Receiver<anyhow::Result<SoundCard>> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let (device, hold) = (device.to_string(), hold_ms(cfg));
    let spawned = std::thread::Builder::new()
        .name("audio reopen".into())
        .spawn(move || {
            let _ = tx.send(open_card(&device, hold));
        });
    if let Err(e) = spawned {
        log::warn!("audio: reopen thread: {e}");
    }
    rx
}

fn run(cfg: EngineConfig, sh: Arc<Shared>, rx: Receiver<Cmd>) {
    let ch = sh.channels;
    let period = cfg.period.clamp(64, 4096);
    let mut mixer = Mixer::new(ch);
    mixer.set_master(cfg.master_db);
    let mut device = cfg.device.clone();
    let mut map = cfg.channel_map.clone();
    let (mut out, mut state, mut detail) = open_output(&device, ch, &cfg);
    let mut last_try = Instant::now();
    let mut retry = cfg.retry;
    let mut pending: Option<Receiver<anyhow::Result<SoundCard>>> = None;
    // `pending` is the first open of a device just switched to (not a retry).
    let mut opening = false;
    let mut taps: Vec<TapSender> = Vec::new();
    let mut bus = vec![0.0f32; period * ch];
    let mut sample: i64 = 0;
    let mut est: Option<f64> = None;
    // Real-time check of the device: (wall instant, frames played) at the window start.
    let mut pace: Option<(Instant, u64)> = None;
    let epoch = crate::epoch();
    let publish = |out: &mut dyn Output, state, detail: &Option<String>, device: &str| {
        let st = EngineStatus {
            device: device.to_string(),
            open: out.name().to_string(),
            state,
            detail: detail.clone(),
            latency_ms: (out.latency().as_secs_f64() * 10_000.0).round() / 10.0,
            device_channels: out.channels(),
        };
        *sh.status.lock().unwrap_or_else(|e| e.into_inner()) = st;
    };
    publish(out.as_mut(), state, &detail, &device);
    loop {
        // Commands (never waits).
        loop {
            match rx.try_recv() {
                Ok(Cmd::Add {
                    id,
                    src,
                    params,
                    fade,
                }) => mixer.add(id, src, params, fade),
                Ok(Cmd::Remove { id, fade }) => mixer.remove(id, fade),
                Ok(Cmd::Resume {
                    id,
                    src,
                    params,
                    fade,
                }) => mixer.resume(id, src, params, fade),
                Ok(Cmd::Params { id, params }) => mixer.set_params(id, params),
                Ok(Cmd::Group { group, db, muted }) => mixer.set_group(group, db, muted),
                Ok(Cmd::Master(db)) => mixer.set_master(db),
                Ok(Cmd::Mute(m)) => mixer.set_mute(m),
                Ok(Cmd::Tap(t)) => taps.push(t),
                Ok(Cmd::Device { name, map: m }) => {
                    device = name;
                    map = m;
                    pending = None;
                    // The old device closes now. The new one opens on a thread of its own
                    // (a sound card can take a few hundred ms) while the engine keeps mixing
                    // on the null output, so taps and the audio clock never stop; the status
                    // follows once it is open or has failed.
                    out = null_output(ch, &cfg);
                    last_try = Instant::now();
                    retry = cfg.retry;
                    est = None;
                    pace = None;
                    if device.is_empty() || device == "null" {
                        opening = false;
                        state = OutputState::Null;
                        detail = None;
                        publish(out.as_mut(), state, &detail, &device);
                    } else {
                        opening = true;
                        state = OutputState::Error;
                        pending = Some(reopen(&device, &cfg));
                    }
                }
                Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => {
                    sh.stopped.store(true, Ordering::Release);
                    return;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        // A device that failed is tried again now and then, on a thread of its own.
        if state == OutputState::Error && pending.is_none() && last_try.elapsed() >= retry {
            pending = Some(reopen(&device, &cfg));
        }
        if let Some(p) = &pending {
            match p.try_recv() {
                Ok(Ok(card)) => {
                    pending = None;
                    retry = cfg.retry;
                    if !std::mem::take(&mut opening) {
                        log::info!("audio: {device} is back");
                    }
                    out = Box::new(card);
                    state = OutputState::Ok;
                    detail = None;
                    est = None;
                    pace = None;
                    publish(out.as_mut(), state, &detail, &device);
                }
                Ok(Err(e)) => {
                    pending = None;
                    last_try = Instant::now();
                    if std::mem::take(&mut opening) {
                        let msg = format!("{e:#}");
                        log::warn!("audio: {msg}; using the null output");
                        detail = Some(msg);
                        publish(out.as_mut(), state, &detail, &device);
                    } else {
                        // Retries back off to once a minute, as IDN reopens its own lost
                        // cards (`idn_audio::reopen`): a card plugged in again during a
                        // service is back within a minute.
                        retry = (retry * 2).min(idn_audio::reopen::MAX_WAIT.max(cfg.retry));
                        log::debug!("audio: {device} still unavailable: {e:#}");
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    pending = None;
                    if std::mem::take(&mut opening) {
                        detail = Some(format!("cannot open {device}: the open thread ended"));
                        publish(out.as_mut(), state, &detail, &device);
                    }
                }
            }
        }
        // The device's health: failing is shown; lost falls back to the null output (and is
        // tried again later).
        match out.health() {
            Some(Health::Running) if state == OutputState::Ok && detail.is_some() => {
                detail = None;
                publish(out.as_mut(), state, &detail, &device);
            }
            Some(Health::Failing(why)) if state == OutputState::Ok => {
                log::warn!("audio: {}: {why}", out.name());
                detail = Some(why);
                publish(out.as_mut(), state, &detail, &device);
            }
            Some(Health::Lost(why)) => {
                let msg = format!("{}: {why}", out.name());
                log::warn!("audio: output lost: {msg}; using the null output");
                out = null_output(ch, &cfg);
                state = OutputState::Error;
                detail = Some(msg);
                last_try = Instant::now();
                // A card lost now is tried again soon, whatever earlier failures backed off to.
                retry = cfg.retry;
                est = None;
                pace = None;
                publish(out.as_mut(), state, &detail, &device);
            }
            _ => {}
        }
        // When this block will be heard: now + the output's latency (on a sound card what
        // its lane holds plus the device's output delay, as idn-audio measures them; the
        // first block is heard when the next pushed sample would be). Smoothed: the lane's
        // queue moves in device callbacks (a few ms), sampled at this thread's wake-ups; a
        // step of 20 ms or more (the device started, a reopen) is taken at once.
        let now = Instant::now();
        let latency = out.latency().as_secs_f64();
        let measured =
            crate::secs_between(epoch, now) + latency - sample as f64 / SAMPLE_RATE as f64;
        let e = match est {
            Some(e) if (measured - e).abs() < 0.02 => e + 0.02 * (measured - e),
            _ => measured,
        };
        est = Some(e);
        let heard_at = crate::offset(epoch, e + sample as f64 / SAMPLE_RATE as f64);
        let ctx = BlockCtx {
            sample,
            heard_at,
            frames: period,
        };
        mixer.mix(&mut bus, &ctx);
        sh.sources.store(mixer.len() as u32, Ordering::Relaxed);
        // Taps.
        taps.retain(|t| {
            let mut data = t.free.try_recv().unwrap_or_default();
            data.clear();
            data.extend_from_slice(&bus);
            match t.tx.try_send(TapBlock {
                heard_at,
                timecode: crate::timecode(heard_at),
                sample,
                channels: ch,
                frames: period,
                data,
            }) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => {
                    t.dropped.fetch_add(1, Ordering::Relaxed);
                    true
                }
                Err(TrySendError::Disconnected(_)) => false,
            }
        });
        // Out (the device channel map is applied there); waits until the next block is due.
        match out.write(&bus, ch, &map) {
            Ok(x) => {
                if x > 0 {
                    sh.xruns.fetch_add(x as u64, Ordering::Relaxed);
                    log::debug!("audio: {x} underrun(s) on {}", out.name());
                }
            }
            Err(e) => {
                let msg = format!("{e:#}");
                log::warn!("audio: output failed: {msg}; using the null output");
                out = null_output(ch, &cfg);
                state = OutputState::Error;
                detail = Some(msg);
                last_try = Instant::now();
                est = None;
                pace = None;
                publish(out.as_mut(), state, &detail, &device);
            }
        }
        sample += period as i64;
        // A device that does not play in real time (an HDMI output with no monitor plugged
        // in consumes at a fraction of its rate) plays nothing useful: after a 2 s window
        // more than 2 % off, it is dropped for the null output.
        if state == OutputState::Ok {
            let now = Instant::now();
            match pace {
                None => pace = out.played().map(|(p, _)| (now, p)),
                Some((t0, p0)) if now - t0 >= Duration::from_secs(2) => {
                    pace = None;
                    if let Some((p, rate)) = out.played() {
                        let r = p.saturating_sub(p0) as f64 / (now - t0).as_secs_f64();
                        if (r / rate as f64 - 1.0).abs() > 0.02 {
                            let msg = format!(
                                "{} does not play in real time ({:.0} % of {rate} Hz; \
                                 is a monitor or amplifier connected?)",
                                out.name(),
                                r * 100.0 / rate as f64
                            );
                            log::warn!("audio: {msg}; using the null output");
                            out = null_output(ch, &cfg);
                            state = OutputState::Error;
                            detail = Some(msg);
                            last_try = Instant::now();
                            est = None;
                            publish(out.as_mut(), state, &detail, &device);
                        } else {
                            pace = Some((now, p));
                        }
                    }
                }
                _ => {}
            }
        } else {
            pace = None;
        }
        let blocks = sh.blocks.fetch_add(1, Ordering::AcqRel) + 1;
        // Meters every 100 ms.
        if blocks.is_multiple_of(((SAMPLE_RATE as usize / 10) / period).max(1) as u64) {
            for (c, p) in mixer.take_peaks().into_iter().enumerate() {
                sh.peaks[c].store(p.to_bits(), Ordering::Relaxed);
            }
        }
        // The latency shown, every second.
        if blocks.is_multiple_of((SAMPLE_RATE as usize / period).max(1) as u64) {
            let ms = (latency * 10_000.0).round() / 10.0;
            let mut st = sh.status.lock().unwrap_or_else(|e| e.into_inner());
            if (st.latency_ms - ms).abs() >= 0.5 {
                st.latency_ms = ms;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tone::Tone;

    #[test]
    fn null_engine_runs_in_real_time_and_taps_get_stamped_blocks() {
        let e = AudioEngine::start(EngineConfig::default());
        assert_eq!(e.status().state, OutputState::Null);
        let tap = e.tap();
        e.add_source(
            1,
            Box::new(Tone::new(1000.0, -6.0, 10.0, 2)),
            SourceParams::default(),
            0.0,
        );
        let t0 = Instant::now();
        let b0 = e.blocks();
        let mut got = Vec::new();
        while t0.elapsed() < Duration::from_millis(600) {
            if let Some(b) = tap.recv_timeout(Duration::from_millis(100)) {
                got.push((b.sample, b.heard_at, b.timecode));
                tap.recycle(b);
            }
        }
        let el = t0.elapsed().as_secs_f64();
        let blocks = (e.blocks() - b0) as f64;
        // 100 blocks a second.
        assert!((blocks / el - 100.0).abs() < 8.0, "{blocks} blocks in {el}");
        // Consecutive blocks are heard 10 ms apart, after now.
        for w in got.windows(2) {
            assert_eq!(w[1].0 - w[0].0, 480);
            let dt = crate::secs_between(w[0].1, w[1].1);
            assert!((dt - 0.01).abs() < 0.001, "{dt}");
            assert!((w[1].2 - w[0].2 - 100_000).abs() < 10_000);
        }
        let lvl = e.levels();
        assert_eq!(lvl.len(), 2);
        assert!((lvl[0] + 6.0).abs() <= 1.0, "{lvl:?}");
        e.set_mute(true);
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(e.levels(), vec![-60.0, -60.0]);
        assert_eq!(e.master(), (0.0, true));
        e.stop();
    }

    /// On a sound card (the system default; skipped when none opens): the engine runs at
    /// 100 blocks a second, reports the measured latency, and stamps blocks with when they
    /// will be heard.
    #[test]
    fn a_sound_card_plays_and_reports_when_blocks_are_heard() {
        if !crate::device::probe("default").0 {
            eprintln!("no sound card opens: skipped");
            return;
        }
        let e = AudioEngine::start(EngineConfig {
            device: "default".into(),
            ..EngineConfig::default()
        });
        let st = e.status();
        assert_eq!(st.state, OutputState::Ok, "{st:?}");
        assert_eq!(st.open, "default");
        std::thread::sleep(Duration::from_millis(1500));
        let tap = e.tap();
        // Skip what queued while the tap was being set up.
        for _ in 0..10 {
            if let Some(b) = tap.recv_timeout(Duration::from_millis(100)) {
                tap.recycle(b);
            }
        }
        let (t0, b0) = (Instant::now(), e.blocks());
        let mut ahead = Vec::new();
        while t0.elapsed() < Duration::from_millis(1000) {
            if let Some(b) = tap.recv_timeout(Duration::from_millis(100)) {
                ahead.push(crate::secs_between(Instant::now(), b.heard_at));
                tap.recycle(b);
            }
        }
        let rate = (e.blocks() - b0) as f64 / t0.elapsed().as_secs_f64();
        assert!((rate - 100.0).abs() < 8.0, "{rate} blocks/s");
        let lat = e.status().latency_ms;
        assert!((5.0..200.0).contains(&lat), "latency {lat} ms");
        // Blocks reach the tap as they are mixed: heard later, within the latency.
        let late = ahead.iter().filter(|a| !(0.0..0.25).contains(*a)).count();
        assert!(late <= ahead.len() / 20, "{ahead:?}");
        e.stop();
    }

    #[test]
    fn a_device_that_cannot_open_reports_an_error_and_keeps_time() {
        let e = AudioEngine::start(EngineConfig {
            device: "hw:99,0".into(),
            ..EngineConfig::default()
        });
        let st = e.status();
        assert_eq!(st.state, OutputState::Error);
        assert!(st.detail.unwrap().contains("hw:99,0"));
        assert_eq!(st.open, "null");
        let b = e.blocks();
        std::thread::sleep(Duration::from_millis(200));
        assert!(e.blocks() - b >= 15);
        e.set_device("", vec![]);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(e.status().state, OutputState::Null);
    }

    /// Switching the device while playing never stops the mixing: the new device opens on a
    /// thread of its own (a sound card can take a few hundred ms), the taps and the clock
    /// keep running on the null output meanwhile, and a device that cannot open is reported.
    #[test]
    fn switching_the_device_keeps_mixing_while_it_opens() {
        let e = AudioEngine::start(EngineConfig::default());
        let tap = e.tap();
        std::thread::sleep(Duration::from_millis(100));
        while let Some(b) = tap.recv_timeout(Duration::from_millis(1)) {
            tap.recycle(b);
        }
        e.set_device("test-slow:400", vec![]);
        let t0 = Instant::now();
        let mut last = Instant::now();
        let mut max_gap = Duration::ZERO;
        while t0.elapsed() < Duration::from_millis(700) {
            if let Some(b) = tap.recv_timeout(Duration::from_millis(500)) {
                max_gap = max_gap.max(last.elapsed());
                last = Instant::now();
                tap.recycle(b);
            }
        }
        assert!(
            max_gap < Duration::from_millis(150),
            "the taps went {max_gap:?} without a block while the device opened"
        );
        let st = e.status();
        assert_eq!(st.state, OutputState::Error, "{st:?}");
        assert_eq!(st.device, "test-slow:400");
        assert_eq!(st.open, "null");
        assert!(st.detail.unwrap().contains("test-slow:400"));
        e.stop();
    }
}
