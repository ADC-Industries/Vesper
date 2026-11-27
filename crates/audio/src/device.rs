//! Output devices: a sound card through the IDN Open Protocol's `idn-audio` (WASAPI on
//! Windows, ALSA on Linux) and the null output, plus device listing.
//!
//! `idn-audio` is built without its `asio` feature, so no ASIO host is in Vesper: not on
//! Windows, and not the cwASIO drivers IDN Beta15 lists on Linux (`/etc/cwASIO/`). IDN's own
//! ASIO driver (`libidn_asio.so`, its "IDN" instances) is for other programs to send sound into
//! IDN; IDN's host leaves it out of its lists anyway. An interface that has an ASIO driver plays
//! here through WASAPI or ALSA (`IDN_AUDIO_HOST=asio` has no host to pick).
//!
//! Both are paced by the wall clock: the engine writes one period, then waits until the next
//! one is due. A sound card's clock runs a little off the machine's; `idn-audio`'s lane
//! corrects for that (a slow resampling servo), so the bus keeps the machine's time and the
//! card plays it at a steady latency.

use crate::SAMPLE_RATE;
use anyhow::anyhow;
use idn_audio::{
    DeviceProblem, DeviceState, DeviceStatus, Direction, Lane, Playback, PlaybackConfig,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A change of a device's health ([`Output::health`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Health {
    /// Playing, no recent errors.
    Running,
    /// Still playing, with recent errors (the sound may break up): why.
    Failing(String),
    /// Gone (unplugged, taken by another program, the driver stopped): why.
    Lost(String),
}

/// Where the engine's mixed audio goes.
pub trait Output: Send {
    /// Device name (`hw:CARD=Generic,DEV=0`; `null`).
    fn name(&self) -> &str;
    /// Channels the device plays.
    fn channels(&self) -> usize;
    /// How long until the first frame of the next [`Output::write`] is heard.
    fn latency(&mut self) -> Duration;
    /// Plays one period of the bus (`bus`: interleaved, `channels` channels; bus channel `c`
    /// plays on device channel `map[c]`, or `c` where the map has no entry), then waits until
    /// the next period is due. Returns the underruns since the last call.
    fn write(&mut self, bus: &[f32], channels: usize, map: &[u32]) -> anyhow::Result<u32>;
    /// The device's health when it changed since the last call.
    fn health(&mut self) -> Option<Health> {
        None
    }
    /// Frames the device has played and its rate (`None`: no device), for the real-time
    /// check.
    fn played(&mut self) -> Option<(u64, u32)> {
        None
    }
}

/// Wall-clock pacing: a period is due `lead` frames before its time.
struct Pacer {
    start: Option<Instant>,
    written: i64,
    lead: i64,
}

impl Pacer {
    fn new(lead: usize) -> Pacer {
        Pacer {
            start: None,
            written: 0,
            lead: lead as i64,
        }
    }

    fn played(&self, now: Instant) -> i64 {
        match self.start {
            Some(s) => (now.saturating_duration_since(s).as_secs_f64() * SAMPLE_RATE as f64) as i64,
            None => 0,
        }
    }

    /// Frames written but not due yet (the lead before the first write).
    fn queued(&self) -> i64 {
        if self.start.is_none() {
            return self.lead;
        }
        (self.written - self.played(Instant::now())).max(0)
    }

    /// Counts `frames` written and waits until the next period is due. Returns 1 when the
    /// thread had starved (did not run for over 200 ms): the timeline starts over from now.
    fn wrote(&mut self, frames: usize) -> u32 {
        let now = Instant::now();
        let start = *self.start.get_or_insert(now);
        let mut xruns = 0;
        if self.played(now) > self.written + SAMPLE_RATE as i64 / 5 {
            xruns = 1;
            let back = Duration::from_secs_f64(self.written as f64 / SAMPLE_RATE as f64);
            self.start = Some(now.checked_sub(back).unwrap_or(start));
        }
        self.written += frames as i64;
        let due = self.written - self.lead;
        if let Some(s) = self.start {
            let at = s + Duration::from_secs_f64(due.max(0) as f64 / SAMPLE_RATE as f64);
            let now = Instant::now();
            if at > now {
                std::thread::sleep(at - now);
            }
        }
        xruns
    }
}

/// Keeps time without a device: a period is "played" when its time has come. Stays `lead`
/// frames ahead of the wall clock (the size of a device's queue), so its sources see the same
/// timing as with real hardware.
pub struct NullOutput {
    channels: usize,
    pace: Pacer,
}

impl NullOutput {
    /// A null output of `channels` channels that keeps `lead` frames queued.
    pub fn new(channels: usize, lead: usize) -> Self {
        NullOutput {
            channels,
            pace: Pacer::new(lead),
        }
    }
}

impl Output for NullOutput {
    fn name(&self) -> &str {
        "null"
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn latency(&mut self) -> Duration {
        Duration::from_secs_f64(self.pace.queued() as f64 / SAMPLE_RATE as f64)
    }

    fn write(&mut self, bus: &[f32], channels: usize, _map: &[u32]) -> anyhow::Result<u32> {
        Ok(self.pace.wrote(bus.len() / channels.max(1)))
    }
}

/// Whether this process lacks access to the sound devices (`/dev/snd/*` belong to the
/// `audio` group). ALSA then reports hardware devices as missing rather than forbidden.
pub fn lacks_permission() -> bool {
    let Ok(rd) = std::fs::read_dir("/dev/snd") else {
        return false;
    };
    rd.filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("control"))
        .any(|e| {
            matches!(
                std::fs::OpenOptions::new().read(true).write(true).open(e.path()),
                Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied
            )
        })
}

/// The first line of the audio system's words (a missing device's message goes on to list
/// every device the machine has).
fn first_line(text: &str) -> &str {
    let t = text.lines().next().unwrap_or("").trim();
    t.split_once("; have:").map_or(t, |(a, _)| a).trim()
}

/// Why a device does not open, for people.
fn problem_text(problem: DeviceProblem, text: &str) -> String {
    let t = first_line(text);
    match problem {
        DeviceProblem::NoPermission => {
            format!("{t} (permission denied: the server user must be in the `audio` group)")
        }
        DeviceProblem::Missing if lacks_permission() => {
            format!(
                "{t} (no such device, or no access: the server user must be in the `audio` group)"
            )
        }
        DeviceProblem::Missing => format!("{t} (no such device)"),
        DeviceProblem::Busy => format!("{t} (the device is in use by another program)"),
        DeviceProblem::Other => t.to_string(),
    }
}

/// The card IDs of `/proc/asound/cards` by index (` 1 [Generic_1      ]: HDA-Intel - …`).
pub fn card_ids(cards: &str) -> Vec<(u32, String)> {
    cards
        .lines()
        .filter_map(|line| {
            let (idx, rest) = line.trim_start().split_once(' ')?;
            let i = idx.parse::<u32>().ok()?;
            let id = rest
                .trim_start()
                .strip_prefix('[')?
                .split(']')
                .next()?
                .trim();
            (!id.is_empty()).then(|| (i, id.to_string()))
        })
        .collect()
}

/// A stored device name as `idn-audio` finds it (`None`: the system's default device).
///
/// Names stored before the port were ALSA PCMs by card index: `hw:1,0` and `plughw:1,0`
/// (opened as such) and `default`. `idn-audio` matches the audio system's identifiers,
/// which name cards by ID (`hw:CARD=Generic_1,DEV=0`), so an index is turned into the
/// card's ID from `cards` (`/proc/asound/cards`), or into `hw:CARD=1,DEV=0` when the card
/// is not there. `default` is the system default. Anything else is passed on as is (a
/// device's name or identifier, or a part of one).
pub fn resolve(name: &str, cards: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name == "default" {
        return None;
    }
    let (plug, rest) = match name.strip_prefix("plughw:") {
        Some(r) => ("plug", r),
        None => match name.strip_prefix("hw:") {
            Some(r) => ("", r),
            None => return Some(name.to_string()),
        },
    };
    let (c, d) = match rest.split_once(',') {
        Some((c, d)) => (c.trim(), d.trim()),
        None => (rest.trim(), "0"),
    };
    let (Ok(c), Ok(d)) = (c.parse::<u32>(), d.parse::<u32>()) else {
        return Some(name.to_string());
    };
    let card = card_ids(cards)
        .into_iter()
        .find(|(i, _)| *i == c)
        .map_or_else(|| c.to_string(), |(_, id)| id);
    Some(format!("{plug}hw:CARD={card},DEV={d}"))
}

fn proc_cards() -> String {
    std::fs::read_to_string("/proc/asound/cards").unwrap_or_default()
}

/// The sound cards this process has open: (name, description, channels). An open hardware
/// device is missing from the audio system's list (it cannot be opened again while it is
/// held), so [`list`] adds these.
static OPEN: Mutex<Vec<(String, String, usize)>> = Mutex::new(Vec::new());

fn open_cards() -> std::sync::MutexGuard<'static, Vec<(String, String, usize)>> {
    OPEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a device thread hands back: the lane to push into and the device's channels.
type Opened = Result<(Lane, usize), String>;

/// A sound card through `idn-audio`: a [`Playback`] (kept on a thread of its own, where it
/// was started, until this is dropped) with one lane, pushed one period at a time and paced
/// by the wall clock. When a block is heard comes from the lane ([`Lane::latency`]: what it
/// holds plus the device's output delay, from the device's callback timestamps).
pub struct SoundCard {
    name: String,
    channels: usize,
    lane: Option<Lane>,
    status: Arc<Mutex<Option<DeviceStatus>>>,
    changed: Arc<AtomicBool>,
    /// Dropped (with the lane) to close the device.
    _stop: SyncSender<()>,
    pace: Pacer,
    planar: Vec<Vec<f32>>,
    routes: Vec<(usize, u16)>,
    routes_for: (usize, Vec<u32>),
    underruns: u64,
    writes: u64,
}

impl SoundCard {
    /// Opens `name` (see [`resolve`]; `""` / `default`: the system default) holding
    /// `latency_ms` of sound before the device (which is asked for a buffer of about a quarter
    /// of that). Blocks until the device has opened or failed to.
    pub fn open(name: &str, latency_ms: u32) -> anyhow::Result<SoundCard> {
        let device = resolve(name, &proc_cards());
        let shown = device.clone().unwrap_or_else(|| "default".into());
        // Errors name the device as stored, and as looked for when that differs.
        let asked = if shown == name.trim() || name.trim().is_empty() {
            shown.clone()
        } else {
            format!("{} ({shown})", name.trim())
        };
        let status: Arc<Mutex<Option<DeviceStatus>>> = Arc::new(Mutex::new(None));
        let changed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::sync_channel::<Opened>(1);
        let (stop, stopped) = std::sync::mpsc::sync_channel::<()>(1);
        let (st, ch) = (status.clone(), changed.clone());
        let cfg = PlaybackConfig {
            device: device.clone(),
            latency_ms,
            ..PlaybackConfig::default()
        };
        std::thread::Builder::new()
            .name("audio device".into())
            .spawn(move || device_thread(cfg, tx, stopped, st, ch))
            .map_err(|e| anyhow!("audio device thread: {e}"))?;
        let (lane, channels) = match rx.recv() {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => {
                // Why, as the device's probe says it.
                let p = idn_audio::probe_device(device.as_deref(), Direction::Output);
                let why = match p.problem {
                    Some(k) => problem_text(k, &p.text),
                    None => first_line(&e).to_string(),
                };
                return Err(anyhow!("cannot open {asked}: {why}"));
            }
            Err(_) => return Err(anyhow!("cannot open {asked}: the device thread ended")),
        };
        let description = lane.stats().device;
        log::info!(
            "audio: opened {shown} ({description}): {channels} ch, holding {latency_ms} ms \
             before the device"
        );
        open_cards().push((shown.clone(), description, channels));
        Ok(SoundCard {
            name: shown,
            channels,
            lane: Some(lane),
            status,
            changed,
            _stop: stop,
            pace: Pacer::new(0),
            planar: Vec::new(),
            routes: Vec::new(),
            routes_for: (0, Vec::new()),
            underruns: 0,
            writes: 0,
        })
    }

    fn lane(&self) -> &Lane {
        self.lane.as_ref().expect("lane until dropped")
    }
}

fn device_thread(
    cfg: PlaybackConfig,
    tx: SyncSender<Opened>,
    stop: Receiver<()>,
    status: Arc<Mutex<Option<DeviceStatus>>>,
    changed: Arc<AtomicBool>,
) {
    let playback = match Playback::start(&cfg) {
        Ok(p) => p,
        Err(e) => {
            let _ = tx.send(Err(e.to_string()));
            return;
        }
    };
    playback.on_status(move |s| {
        *status.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
        changed.store(true, Ordering::Release);
    });
    let lane = playback.lane();
    let channels = playback.device_channels() as usize;
    if tx.send(Ok((lane, channels))).is_err() {
        return;
    }
    // Until the sound card is dropped.
    let _ = stop.recv();
    drop(playback);
}

impl Drop for SoundCard {
    fn drop(&mut self) {
        // The lane goes first (what it holds fades out), then the device thread closes the
        // device (`_stop` is dropped after this).
        self.lane = None;
        let mut open = open_cards();
        if let Some(i) = open.iter().position(|o| o.0 == self.name) {
            open.remove(i);
        }
    }
}

impl Output for SoundCard {
    fn name(&self) -> &str {
        &self.name
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn latency(&mut self) -> Duration {
        self.lane().latency()
    }

    fn write(&mut self, bus: &[f32], channels: usize, map: &[u32]) -> anyhow::Result<u32> {
        let ch = channels.max(1);
        let frames = bus.len() / ch;
        if self.routes_for.0 != ch || self.routes_for.1 != map {
            self.routes = (0..ch)
                .map(|c| (c, map.get(c).copied().unwrap_or(c as u32) as u16))
                .collect();
            self.routes_for = (ch, map.to_vec());
        }
        self.planar.resize_with(ch, Vec::new);
        for (c, p) in self.planar.iter_mut().enumerate() {
            p.clear();
            p.extend(bus.iter().skip(c).step_by(ch).take(frames));
        }
        self.lane.as_ref().expect("lane until dropped").push(
            SAMPLE_RATE,
            &self.planar,
            &self.routes,
        );
        let mut xruns = self.pace.wrote(frames);
        // Underruns, twice a second.
        self.writes += 1;
        if self.writes.is_multiple_of(50) {
            let u = self.lane().stats().underruns;
            xruns += u.saturating_sub(self.underruns) as u32;
            self.underruns = u;
        }
        Ok(xruns)
    }

    fn health(&mut self) -> Option<Health> {
        if !self.changed.swap(false, Ordering::AcqRel) {
            return None;
        }
        let s = self
            .status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()?;
        let text = first_line(&s.text).to_string();
        Some(match s.state {
            DeviceState::Running => Health::Running,
            DeviceState::Failing => Health::Failing(text),
            DeviceState::Lost => Health::Lost(text),
        })
    }

    fn played(&mut self) -> Option<(u64, u32)> {
        let s = self.lane().stats();
        (s.device_rate > 0).then_some((s.frames_out, s.device_rate))
    }
}

/// A playback device the machine has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Name to open (`default`, `hw:CARD=Generic_1,DEV=0`).
    pub name: String,
    /// Description (`HD-Audio Generic, ALC897 Analog`).
    pub description: String,
    /// It can be opened now.
    pub available: bool,
    /// Why not.
    pub problem: Option<String>,
    /// Maximum channels (when known).
    pub channels: Option<u32>,
}

/// Whether a device of `idn_audio::devices` is offered: ALSA lists every PCM of a card
/// (`front:`, `dmix:`, `surround51:`, by card ID and by index, …); of those only the
/// hardware devices by card ID (`hw:CARD=Generic,DEV=0`), as before the port. Other audio
/// systems' devices (WASAPI, a sound server's `pipewire`) are all offered, except ALSA's
/// `null` (no device is the null output).
fn offered(id: &str) -> bool {
    if id == "null" {
        return false;
    }
    if !id.contains("CARD=") {
        return true;
    }
    id.strip_prefix("hw:CARD=")
        .and_then(|r| r.split(',').next())
        .is_some_and(|card| card.parse::<u32>().is_err())
}

fn info(name: String, description: String, probe: idn_audio::DeviceProbe) -> DeviceInfo {
    DeviceInfo {
        name,
        description,
        available: probe.opens,
        problem: (!probe.opens).then(|| match probe.problem {
            Some(k) => problem_text(k, &probe.text),
            None => first_line(&probe.text).to_string(),
        }),
        channels: (probe.max_channels > 0)
            .then_some(probe.max_channels as u32)
            .or_else(|| probe.default.map(|d| d.channels as u32)),
    }
}

/// The playback devices: `default` (the system's), then every device `idn-audio` lists (on
/// ALSA its hardware devices, `hw:CARD=…,DEV=…`) and the sound cards this process has open,
/// each probed (one after the other: probes opening devices at the same time see each other's
/// as busy or missing; about half a second each on ALSA). `in_use` is the device the engine
/// has open (reported available without probing it, which would fail as busy).
///
/// Clients ask for this list again and again (Settings ▸ Audio), so it comes from
/// `idn_audio::devices_known()` (with an ASIO host a driver already asked isn't loaded again
/// for every listing; without one, as Vesper builds it, the same as `devices()`).
pub fn list(in_use: Option<&str>) -> Vec<DeviceInfo> {
    let mut want = vec![("default".to_string(), "System default".to_string())];
    match idn_audio::devices_known() {
        Ok(devs) => want.extend(
            devs.into_iter()
                .filter(|d| d.output.is_some() && offered(&d.id))
                .map(|d| (d.id, d.name)),
        ),
        Err(e) => log::warn!("audio: listing devices: {e}"),
    }
    let open = open_cards().clone();
    for (name, description, _) in &open {
        if !want.iter().any(|w| &w.0 == name) {
            want.insert(1, (name.clone(), description.clone()));
        }
    }
    want.into_iter()
        .map(|(name, description)| {
            if in_use.is_some_and(|u| u == name || u == description) {
                let channels = open.iter().find(|o| o.0 == name).map(|o| o.2 as u32);
                return DeviceInfo {
                    name,
                    description,
                    available: true,
                    problem: None,
                    channels,
                };
            }
            let dev = (name != "default").then_some(name.as_str());
            let p = idn_audio::probe_device(dev, Direction::Output);
            info(name, description, p)
        })
        .collect()
}

/// Whether `name` opens for playback now, why not, and its most channels (opened and closed
/// again, as playback would open it).
pub fn probe(name: &str) -> (bool, Option<String>, Option<u32>) {
    let dev = resolve(name, &proc_cards());
    let d = info(
        name.to_string(),
        String::new(),
        idn_audio::probe_device(dev.as_deref(), Direction::Output),
    );
    (d.available, d.problem, d.channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARDS: &str = " 0 [Loopback       ]: Loopback - Loopback\n                      Loopback 1\n 1 [Generic        ]: HDA-Intel - HD-Audio Generic\n                      HD-Audio Generic at 0xfc888000 irq 84\n 2 [Generic_1      ]: HDA-Intel - HD-Audio Generic\n                      HD-Audio Generic at 0xfc880000 irq 85\n";

    #[test]
    fn proc_asound_cards_are_parsed() {
        assert_eq!(
            card_ids(CARDS),
            vec![
                (0, "Loopback".to_string()),
                (1, "Generic".to_string()),
                (2, "Generic_1".to_string()),
            ]
        );
    }

    #[test]
    fn stored_alsa_names_are_migrated() {
        assert_eq!(resolve("", CARDS), None);
        assert_eq!(resolve("default", CARDS), None);
        assert_eq!(resolve("hw:2,0", CARDS).unwrap(), "hw:CARD=Generic_1,DEV=0");
        assert_eq!(
            resolve("plughw:1,3", CARDS).unwrap(),
            "plughw:CARD=Generic,DEV=3"
        );
        assert_eq!(resolve("hw:1", CARDS).unwrap(), "hw:CARD=Generic,DEV=0");
        // A card that is not there keeps its index.
        assert_eq!(resolve("hw:7,0", CARDS).unwrap(), "hw:CARD=7,DEV=0");
        // Identifiers and names pass.
        assert_eq!(
            resolve("hw:CARD=Generic_1,DEV=0", CARDS).unwrap(),
            "hw:CARD=Generic_1,DEV=0"
        );
        assert_eq!(resolve("Speakers", CARDS).unwrap(), "Speakers");
    }

    #[test]
    fn only_hardware_devices_by_card_id_are_offered() {
        assert!(offered("hw:CARD=Generic_1,DEV=0"));
        assert!(!offered("hw:CARD=2,DEV=0"));
        assert!(!offered("plughw:CARD=Generic_1,DEV=0"));
        assert!(!offered("dmix:CARD=Generic,DEV=3"));
        assert!(!offered("null"));
        assert!(offered("pipewire"));
        assert!(offered("{0.0.0.00000000}.{guid}"));
    }

    #[test]
    fn missing_device_text_is_one_line() {
        let t = "no playback device named \"x\"; have:\n  A [hw:CARD=A,DEV=0]\n";
        assert_eq!(first_line(t), "no playback device named \"x\"");
    }

    #[test]
    fn null_output_keeps_time() {
        let mut o = NullOutput::new(2, 960);
        let t0 = Instant::now();
        let block = vec![0.0f32; 480 * 2];
        // 50 periods of 10 ms = 0.5 s, minus the 20 ms lead.
        for _ in 0..50 {
            assert_eq!(o.write(&block, 2, &[]).unwrap(), 0);
        }
        let el = t0.elapsed().as_secs_f64();
        assert!((el - 0.48).abs() < 0.03, "elapsed {el}");
        let d = o.latency().as_secs_f64() * SAMPLE_RATE as f64;
        assert!((800.0..=1100.0).contains(&d), "delay {d}");
    }

    #[test]
    fn opening_a_missing_device_fails_cleanly() {
        let e = SoundCard::open("hw:99,0", 20).err().expect("no such card");
        let e = format!("{e:#}");
        assert!(e.contains("hw:99,0 (hw:CARD=99,DEV=0)"), "{e}");
        assert!(e.contains("(no such device)"), "{e}");
        assert!(!e.contains('\n'), "{e}");
        let (ok, why, _) = probe("hw:99,0");
        assert!(!ok && why.is_some());
    }
}
