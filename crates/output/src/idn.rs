//! IDN Open Protocol output and verification receiver: the server's network video transport.
//!
//! IDN Open Protocol (crate `idn-open-protocol`, a git dependency on
//! `github.com/ADC-Industries/idn-open-protocol`) is a separate, open-source (MIT / Apache-2.0) Rust
//! implementation of a network video protocol **compatible with NDI®**: its sources show up
//! in compatible receivers, and its receivers take compatible sources. It is compiled in;
//! there is no runtime to install. NDI® is a registered trademark of Vizrt NDI AB; neither
//! this project nor the library is affiliated with or endorsed by Vizrt.
//!
//! - [`IdnSink`] is a [`FrameSink`]: one named source per sink. Our premultiplied sRGB RGBA
//!   goes to the library as is ([`AlphaMode::Premultiplied`](idn_open_protocol::color::AlphaMode));
//!   the library un-premultiplies and converts to BT.709 limited-range Y'CbCr with straight
//!   alpha (the wire convention). Opaque screens are sent as
//!   opaque pictures (alpha dropped = over black for premultiplied input), screens with alpha
//!   as pictures with an alpha plane.
//! - [`IdnAudioSender`] embeds program audio on the sink's source, sent on its own with
//!   `Output::send_audio` (see *Lip sync* below).
//! - [`IdnFinder`] lists sources (any compatible sender) and [`IdnReceiver`] receives one,
//!   both for tests and tools; the receiver returns plain [`Received`] values.
//!
//! `send` never blocks: the library takes the frame into a one-slot mailbox and converts and
//! encodes it on its own thread (the newest frame wins).
//!
//! # Lip sync: one media timeline for video and audio
//!
//! The library's rule is that **the same media moment gets the same timecode on video and
//! audio**: a receiver then lines them up exactly, by construction. Its API lets the caller
//! bring its own media timeline: `VideoFrame { timecode: Some(t), .. }` on video and
//! `Output::send_audio(block, Some(t))` for audio, "both on the same timeline" (library
//! README / `docs/INTEGRATION.md`, *Lip sync*). That is what this adapter does:
//!
//! - **Video**: every frame goes out with `timecode = Some(`[`Frame::timecode`]`)`.
//! - **Audio**: every block the [`IdnAudioSender`] gets goes out at once with
//!   `send_audio(block, Some(timecode))`, the timecode it was given. It is independent of
//!   video: no queue, no attaching to frames, nothing waits for the other.
//!
//! Both timecodes come from the server's program clock (`audio::timecode`, 100 ns units
//! since one process-wide instant, the library's unit too): a video frame is stamped with
//! the tick instant it is presented at, a program audio block with the instant its first
//! sample is heard. An output's video delay of N frames is applied to both: the delayed
//! frame is stamped with the tick it actually goes out on, and the audio worker holds each
//! block N frame periods and adds the same N periods to its timecode. So equal timecodes
//! mean "presented together", which is exactly what the library asks for.
//!
//! (Earlier versions queued audio and attached one frame's worth to the *next* outgoing
//! frame; the library stamps attached audio's first sample with that frame's timecode, so
//! audio came out about a frame late.)

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use idn_open_protocol::api::{
    AudioBlock, Output, OutputConfig, OutputStats, Publisher, PublisherConfig, SendError,
    VideoFrame,
};
use idn_open_protocol::color::AlphaMode as WireAlpha;
use idn_open_protocol::discovery::{Browser, BrowserConfig};
use idn_open_protocol::input::{ConnectionState, Input, InputConfig, InputEvent, InputStats};

use crate::recv::{FOURCC_RGBA, FOURCC_RGBX, Received, ReceivedAudio, ReceivedFrame, source_part};
use crate::{AudioSink, Frame, FrameRate, FrameSink};

/// The library's version (`idn_open_protocol::VERSION`).
pub const LIBRARY_VERSION: &str = idn_open_protocol::VERSION;

/// Human-readable name of the backend, with the required trademark wording.
pub const LABEL: &str = "IDN Open Protocol (compatible with NDI®)";

/// One publisher per group list, shared by every sink of the process (the library announces
/// all outputs of a publisher in the same groups).
static PUBLISHERS: Mutex<Vec<(Vec<String>, Publisher)>> = Mutex::new(Vec::new());

fn publisher(groups: &[String]) -> Result<Publisher> {
    let mut key: Vec<String> = groups
        .iter()
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty())
        .collect();
    if key.is_empty() {
        key.push("public".into());
    }
    key.sort();
    key.dedup();
    let mut all = PUBLISHERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, p)) = all.iter().find(|(k, _)| *k == key) {
        return Ok(p.clone());
    }
    let p = Publisher::new(PublisherConfig {
        groups: key.clone(),
        ..Default::default()
    })
    .context("IDN Open Protocol: cannot start announcing sources")?;
    all.push((key, p.clone()));
    Ok(p)
}

/// A text-only source called `name` (IDN `text/1`: caption lines or data rows, no picture or
/// sound), announced by the process's public publisher. Gone when dropped.
pub fn text_source(name: &str) -> Result<idn_open_protocol::text::TextSource> {
    idn_open_protocol::text::TextSource::new(&publisher(&[])?, name)
        .with_context(|| format!("IDN Open Protocol: cannot open the text source {name:?}"))
}

/// The full name a source called `name` gets on the network (`"HOST (name)"`).
pub fn full_name(name: &str) -> Result<String> {
    Ok(publisher(&[])?.full_name(name))
}

/// Settings for one IDN source.
#[derive(Clone, Debug)]
pub struct IdnSenderConfig {
    /// Source name; receivers list `"HOST (name)"`.
    pub name: String,
    /// Groups (empty = public).
    pub groups: Vec<String>,
    /// Whether frames carry alpha. Only the initial setting: the sink restarts the source
    /// if frames arrive with a different alpha intent.
    pub alpha: bool,
    /// Nominal frame rate (labels the stream); also used to size the audio per frame.
    pub frame_rate: FrameRate,
    /// Picture quality 1–99 (`None`: the library's default).
    pub quality: Option<u8>,
}

impl IdnSenderConfig {
    /// Defaults for a source called `name`: public, opaque, 30 fps.
    pub fn new(name: impl Into<String>) -> Self {
        IdnSenderConfig {
            name: name.into(),
            groups: Vec::new(),
            alpha: false,
            frame_rate: FrameRate::FPS30,
            quality: None,
        }
    }
}

/// The output an [`IdnAudioSender`] sends on: the sink's current [`Output`] (replaced when
/// the sink restarts it). A weak reference, so dropping the sink still withdraws the source
/// at once even while a sender is alive.
#[derive(Default)]
struct AudioRoute {
    out: Mutex<Weak<Output>>,
    /// Samples (per channel) dropped because no output was running.
    dropped: AtomicU64,
}

/// Sends program audio on an [`IdnSink`]'s source from another thread (see the
/// [module docs](self) for the timing).
///
/// Each block goes out at once with [`Output::send_audio`], stamped with the timecode it was
/// given: the server's program clock, the timeline the sink stamps video with. It never
/// blocks (the library queues and sends on its own threads) and never waits for video.
pub struct IdnAudioSender {
    route: Arc<AudioRoute>,
    sent: u64,
}

impl AudioSink for IdnAudioSender {
    fn send(
        &mut self,
        interleaved: &[f32],
        channels: usize,
        sample_rate: u32,
        timecode: i64,
    ) -> Result<()> {
        anyhow::ensure!(channels > 0, "no channels");
        let n = interleaved.len() / channels;
        if n == 0 {
            return Ok(());
        }
        let out = self
            .route
            .out
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .upgrade();
        let Some(out) = out else {
            // The sink is between restarts or gone: nothing to send on.
            self.route.dropped.fetch_add(n as u64, Ordering::Relaxed);
            return Ok(());
        };
        let mut planes = vec![Vec::with_capacity(n); channels];
        for frame in interleaved.chunks_exact(channels) {
            for (p, v) in planes.iter_mut().zip(frame) {
                p.push(*v);
            }
        }
        let block = AudioBlock {
            sample_rate,
            channels: planes,
        };
        match out.send_audio(block, Some(timecode)) {
            Ok(()) => {
                self.sent += n as u64;
                Ok(())
            }
            Err(SendError::Closed) => {
                self.route.dropped.fetch_add(n as u64, Ordering::Relaxed);
                Ok(())
            }
            Err(e) => Err(anyhow!("IDN source {:?}: audio: {e}", out.name())),
        }
    }

    fn samples_sent(&self) -> u64 {
        self.sent
    }
}

/// First wait before a source that could not start is started again; it doubles with each
/// failure up to [`RETRY_MAX`].
const RETRY_MIN: Duration = Duration::from_millis(250);
/// Longest wait between start attempts.
const RETRY_MAX: Duration = Duration::from_secs(5);

/// Sends frames as an IDN Open Protocol source (see the [module docs](self)).
pub struct IdnSink {
    config: IdnSenderConfig,
    publisher: Publisher,
    out: Option<Arc<Output>>,
    label: String,
    /// Pixel buffers handed to the library; reused once it lets go of them.
    pool: Vec<Arc<[u8]>>,
    /// The buffer of the previous frame (resent as the same `Arc` when nothing changed, so
    /// the library's repeat detection skips the comparison and the encode).
    last: Option<Arc<[u8]>>,
    audio: Arc<AudioRoute>,
    frames_sent: u64,
    restarts: u64,
    /// Stats of outputs replaced by restarts, so counters run for the sink's life.
    stats_base: OutputStats,
    /// The source is not running (a start failed): the next attempt is not before this.
    retry_at: Option<Instant>,
    /// The wait after the next failed start.
    retry_delay: Duration,
    /// Tests: this many starts fail (as a failed `Output::new` would).
    #[cfg(test)]
    fail_starts: u32,
}

impl IdnSink {
    /// Creates and announces the source.
    pub fn new(config: IdnSenderConfig) -> Result<Self> {
        let publisher = publisher(&config.groups)?;
        let mut sink = IdnSink {
            label: format!("IDN: {}", config.name),
            publisher,
            out: None,
            pool: Vec::new(),
            last: None,
            audio: Arc::new(AudioRoute::default()),
            frames_sent: 0,
            restarts: 0,
            stats_base: OutputStats::default(),
            retry_at: None,
            retry_delay: RETRY_MIN,
            #[cfg(test)]
            fail_starts: 0,
            config,
        };
        let (alpha, rate) = (sink.config.alpha, sink.config.frame_rate);
        sink.start(alpha, rate)?;
        Ok(sink)
    }

    /// Starts the source for `alpha` and `frame_rate`; the configuration takes them only once
    /// it runs, so a failed start is tried again (see [`FrameSink::send`]).
    fn start(&mut self, alpha: bool, frame_rate: FrameRate) -> Result<()> {
        #[cfg(test)]
        if self.fail_starts > 0 {
            self.fail_starts -= 1;
            bail!("IDN source {:?} could not start (test)", self.config.name);
        }
        let mut cfg = OutputConfig::new(&self.config.name);
        cfg.alpha = alpha;
        cfg.frame_rate = (frame_rate.num, frame_rate.den.max(1));
        if let Some(q) = self.config.quality {
            cfg.quality = q.clamp(1, 99);
        }
        let out = Output::new(&self.publisher, cfg)
            .with_context(|| format!("IDN source {:?} could not start", self.config.name))?;
        log::info!(
            "IDN source {:?} started (IDN Open Protocol {LIBRARY_VERSION}, port {})",
            out.name(),
            out.port()
        );
        let out = Arc::new(out);
        *self.audio.out.lock().unwrap_or_else(|e| e.into_inner()) = Arc::downgrade(&out);
        self.out = Some(out);
        self.last = None;
        self.config.alpha = alpha;
        self.config.frame_rate = frame_rate;
        self.retry_at = None;
        self.retry_delay = RETRY_MIN;
        Ok(())
    }

    /// Stops the running source (its counters carry over). The audio route lets go of it
    /// first, and an audio send still holding it is waited for (briefly), so the old source
    /// is gone before a new one with the same name starts.
    fn stop(&mut self) {
        let Some(old) = self.out.take() else { return };
        *self.audio.out.lock().unwrap_or_else(|e| e.into_inner()) = Weak::new();
        let deadline = Instant::now() + Duration::from_millis(100);
        while Arc::strong_count(&old) > 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let s = old.stats();
        self.stats_base.submitted += s.submitted;
        self.stats_base.replaced += s.replaced;
        self.stats_base.repeated += s.repeated;
    }

    /// The full name receivers see, `"HOST (name)"`.
    pub fn full_name(&self) -> &str {
        self.out.as_ref().map_or("", |o| o.name())
    }

    /// The TCP port receivers connect to.
    pub fn port(&self) -> u16 {
        self.out.as_ref().map_or(0, |o| o.port())
    }

    /// Receivers connected now.
    pub fn connections(&self) -> usize {
        self.out.as_ref().map_or(0, |o| o.receivers())
    }

    /// A sender for program audio on this source (usable from another thread).
    pub fn audio_sender(&self) -> IdnAudioSender {
        IdnAudioSender {
            route: self.audio.clone(),
            sent: 0,
        }
    }

    /// Frames handed to the library.
    pub fn frames_sent(&self) -> u64 {
        self.frames_sent
    }

    /// Times the source was restarted because the alpha intent or frame rate changed.
    pub fn restarts(&self) -> u64 {
        self.restarts
    }

    /// Audio samples (per channel) dropped because the source was not running (restarting
    /// or stopped).
    pub fn audio_dropped(&self) -> u64 {
        self.audio.dropped.load(Ordering::Relaxed)
    }

    /// The library's counters. Frames submitted, replaced in its mailbox and repeated are
    /// summed over the sink's life (across source restarts); the other counters are the
    /// current source's.
    pub fn stats(&self) -> OutputStats {
        // `OutputStats` is `#[non_exhaustive]`: start from the library's value and adjust.
        let mut s = self.out.as_ref().map(|o| o.stats()).unwrap_or_default();
        s.submitted += self.stats_base.submitted;
        s.replaced += self.stats_base.replaced;
        s.repeated += self.stats_base.repeated;
        s
    }

    /// A pixel buffer holding `pixels`: the previous frame's if identical, else a pooled
    /// buffer the library no longer holds, else a new one.
    fn buffer(&mut self, pixels: &[u8]) -> Arc<[u8]> {
        if let Some(last) = &self.last
            && **last == *pixels
        {
            return last.clone();
        }
        self.pool.retain(|b| b.len() == pixels.len());
        let reusable = self.pool.iter_mut().find_map(|b| {
            if Arc::strong_count(b) == 1 && self.last.as_ref().is_none_or(|l| !Arc::ptr_eq(l, b)) {
                Arc::get_mut(b).map(|m| {
                    m.copy_from_slice(pixels);
                })?;
                Some(b.clone())
            } else {
                None
            }
        });
        let buf = match reusable {
            Some(b) => b,
            None => {
                let b: Arc<[u8]> = Arc::from(pixels);
                if self.pool.len() < 4 {
                    self.pool.push(b.clone());
                }
                b
            }
        };
        self.last = Some(buf.clone());
        buf
    }
}

impl FrameSink for IdnSink {
    fn name(&self) -> &str {
        &self.label
    }

    fn send(&mut self, frame: &Frame<'_>) -> Result<()> {
        frame.validate()?;
        let alpha = frame.has_alpha();
        if self.out.is_some()
            && (alpha != self.config.alpha || frame.frame_rate != self.config.frame_rate)
        {
            // The library fixes alpha and frame rate per source: restart it.
            log::info!(
                "IDN source {:?}: restarting for alpha {alpha}, {}/{} fps",
                self.config.name,
                frame.frame_rate.num,
                frame.frame_rate.den
            );
            self.stop();
            self.restarts += 1;
            self.retry_at = None;
        }
        if self.out.is_none() {
            // Not running (a start failed): try again, backing off while it keeps failing.
            let now = Instant::now();
            if self.retry_at.is_some_and(|t| now < t) {
                bail!("IDN source {:?} stopped (starting again)", self.config.name);
            }
            if let Err(e) = self.start(alpha, frame.frame_rate) {
                self.retry_at = Some(now + self.retry_delay);
                self.retry_delay = (self.retry_delay * 2).min(RETRY_MAX);
                return Err(e);
            }
        }
        let pixels = self.buffer(frame.pixels);
        let mut f = VideoFrame::rgba(
            frame.width as usize,
            frame.height as usize,
            pixels,
            if alpha {
                WireAlpha::Premultiplied
            } else {
                WireAlpha::Opaque
            },
        );
        // The program clock's timecode, the timeline the audio sender stamps audio with.
        f.timecode = Some(frame.timecode);
        let out = self
            .out
            .as_ref()
            .ok_or_else(|| anyhow!("IDN source stopped"))?;
        match out.send(f) {
            Ok(()) => {
                self.frames_sent += 1;
                Ok(())
            }
            Err(SendError::Closed) => bail!("IDN source {:?} closed", self.config.name),
            Err(e) => Err(anyhow!("IDN source {:?}: {e}", self.config.name)),
        }
    }
}

/// Lists sources on the network (IDN and other compatible senders) with the library's browser.
pub struct IdnFinder {
    browser: Browser,
}

impl IdnFinder {
    /// Starts browsing (mDNS, all interfaces).
    pub fn new() -> Result<Self> {
        Ok(IdnFinder {
            browser: Browser::start(BrowserConfig::default())
                .context("IDN Open Protocol: cannot start the source browser")?,
        })
    }

    /// Sources currently known, after waiting `wait` for the list to settle.
    pub fn sources(&self, wait: Duration) -> Vec<IdnSourceInfo> {
        let end = Instant::now() + wait;
        while let Some(left) = end.checked_duration_since(Instant::now()) {
            if left.is_zero() {
                break;
            }
            let _ = self.browser.recv(left.min(Duration::from_millis(100)));
        }
        self.browser
            .sources()
            .into_iter()
            .map(|s| IdnSourceInfo {
                is_idn: s.is_idn(),
                address: s.socket_addr().map(|a| a.to_string()),
                groups: s.groups.clone(),
                name: s.name,
            })
            .collect()
    }

    /// Waits up to `timeout` for a source whose full name or source part equals `name`.
    pub fn find_by_name(&self, name: &str, timeout: Duration) -> Option<IdnSourceInfo> {
        let deadline = Instant::now() + timeout;
        loop {
            let found = self
                .sources(Duration::ZERO)
                .into_iter()
                .find(|s| s.name == name || source_part(&s.name) == name);
            if found.is_some() || Instant::now() >= deadline {
                return found;
            }
            let _ = self.browser.recv(Duration::from_millis(100));
        }
    }
}

/// A source listed by an [`IdnFinder`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdnSourceInfo {
    /// `"HOST (name)"`.
    pub name: String,
    /// `ip:port`, once known.
    pub address: Option<String>,
    /// Groups the source announced.
    pub groups: Vec<String>,
    /// The source announced itself as an IDN device (otherwise another compatible sender).
    pub is_idn: bool,
}

/// A minimal receiver on the library's input (verification and tools; not an input layer
/// yet). Pictures come back as straight-alpha RGBA (`RGBX` without an alpha plane), audio
/// as planar float, as [`Received`] values.
pub struct IdnReceiver {
    input: Input,
    connected: Option<String>,
    events: VecDeque<InputEvent>,
}

impl IdnReceiver {
    /// Starts receiving `source` (`"HOST (name)"` or `ip:port`); connecting happens in the
    /// background.
    pub fn connect(source: &str) -> Result<Self> {
        let mut cfg = InputConfig::new(source);
        cfg.receiver_name = "output crate receiver".into();
        cfg.rgba = Some(WireAlpha::Straight);
        cfg.queue = 64;
        let input = Input::open(cfg).with_context(|| format!("IDN input for {source:?}"))?;
        Ok(IdnReceiver {
            input,
            connected: None,
            events: VecDeque::new(),
        })
    }

    /// The address it is connected to, once connected.
    pub fn connected_to(&self) -> Option<&str> {
        self.connected.as_deref()
    }

    /// The library's input counters.
    pub fn stats(&self) -> InputStats {
        self.input.stats()
    }

    /// Next event, handling state changes; `Ok(None)` on timeout.
    fn next(&mut self, timeout: Duration) -> Result<Option<Received>> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let ev = match self.events.pop_front() {
                Some(e) => Some(e),
                None => self.input.recv(left),
            };
            match ev {
                Some(InputEvent::State(ConnectionState::Connected(a))) => {
                    self.connected = Some(a.to_string());
                }
                Some(InputEvent::State(ConnectionState::Lost(why))) => {
                    self.connected = None;
                    log::debug!("IDN input lost: {why}");
                }
                Some(InputEvent::Video(v)) => {
                    let Some(data) = v.rgba else {
                        bail!("IDN input delivered no RGBA");
                    };
                    let has_alpha = v.picture.planes.len() > 3;
                    return Ok(Some(Received::Video(ReceivedFrame {
                        width: v.picture.width as u32,
                        height: v.picture.height as u32,
                        fourcc: if has_alpha { FOURCC_RGBA } else { FOURCC_RGBX },
                        frame_rate: (v.header.frame_rate_n as i32, v.header.frame_rate_d as i32),
                        timecode: v.header.timecode,
                        data,
                    })));
                }
                Some(InputEvent::Audio(a)) => {
                    let samples = a.channels.first().map_or(0, Vec::len);
                    let channels = a.channels.len();
                    return Ok(Some(Received::Audio(ReceivedAudio {
                        sample_rate: a.header.sample_rate,
                        channels,
                        samples,
                        timecode: a.header.timecode,
                        data: a.channels.into_iter().flatten().collect(),
                    })));
                }
                Some(InputEvent::Error(e)) => log::debug!("IDN input: {e}"),
                Some(_) => {}
                None => return Ok(None),
            }
            if left.is_zero() {
                return Ok(None);
            }
        }
    }

    /// Waits up to `timeout` for the next video or audio frame. `Ok(None)` on timeout.
    pub fn capture(&mut self, timeout: Duration) -> Result<Option<Received>> {
        self.next(timeout)
    }

    /// Waits up to `timeout` for the next video frame (audio is skipped).
    pub fn capture_video(&mut self, timeout: Duration) -> Result<Option<ReceivedFrame>> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.next(left)? {
                Some(Received::Video(f)) => return Ok(Some(f)),
                Some(Received::Audio(_)) if !left.is_zero() => {}
                _ => return Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_known() {
        assert!(!LIBRARY_VERSION.is_empty());
        assert_ne!(LIBRARY_VERSION, "unknown");
    }

    /// A03-3: a restart whose start fails is tried again (with a back-off) on later frames,
    /// and the configuration only follows once the source runs.
    #[test]
    fn a_failed_restart_is_tried_again() {
        let name = format!("vesper-retry-{}", std::process::id());
        let mut sink = IdnSink::new(IdnSenderConfig::new(name)).unwrap();
        let px = vec![0u8; 16 * 9 * 4];
        let frame = |alpha| Frame {
            width: 16,
            height: 9,
            pixels: &px,
            alpha,
            frame_rate: FrameRate { num: 60, den: 1 },
            timecode: 0,
        };
        let opaque = frame(crate::AlphaMode::Opaque);
        // The restart for 60 fps fails twice.
        sink.fail_starts = 2;
        assert!(sink.send(&opaque).is_err());
        assert_eq!(sink.restarts(), 1);
        assert_eq!(
            sink.config.frame_rate,
            FrameRate::FPS30,
            "not running at 60"
        );
        assert!(sink.out.is_none());
        // Backing off: no attempt yet.
        assert!(sink.send(&opaque).is_err());
        assert_eq!(sink.fail_starts, 1);
        std::thread::sleep(RETRY_MIN + Duration::from_millis(50));
        assert!(sink.send(&opaque).is_err(), "second failure");
        assert_eq!(sink.fail_starts, 0);
        std::thread::sleep(RETRY_MIN * 2 + Duration::from_millis(50));
        sink.send(&opaque).expect("started again");
        assert!(sink.out.is_some());
        assert_eq!(sink.config.frame_rate, FrameRate { num: 60, den: 1 });
        assert_eq!(sink.restarts(), 1);
        assert_eq!(sink.frames_sent(), 1);
        // A change of alpha intent restarts it again, at once.
        sink.send(&frame(crate::AlphaMode::Premultiplied)).unwrap();
        assert!(sink.config.alpha);
        assert_eq!(sink.restarts(), 2);
    }

    /// Audio at a rate the library refuses (Beta8: outside 8–384 kHz, `SendError::BadAudio`)
    /// is an error for that block only: the sender goes on with the next one.
    #[test]
    fn audio_at_a_refused_rate_fails_that_block_only() {
        let name = format!("vesper-audio-rate-{}", std::process::id());
        let mut sink = IdnSink::new(IdnSenderConfig::new(name)).unwrap();
        let px = vec![0u8; 16 * 9 * 4];
        sink.send(&Frame {
            width: 16,
            height: 9,
            pixels: &px,
            alpha: crate::AlphaMode::Opaque,
            frame_rate: FrameRate::FPS30,
            timecode: 0,
        })
        .unwrap();
        let mut audio = sink.audio_sender();
        let block = vec![0.0f32; 2 * 480];
        for rate in [0, 4_000, 500_000] {
            let e = audio.send(&block, 2, rate, 0).unwrap_err();
            assert!(format!("{e:#}").contains("bad audio"), "{rate}: {e:#}");
        }
        assert_eq!(audio.samples_sent(), 0);
        audio.send(&block, 2, 48_000, 0).expect("48 kHz goes out");
        assert_eq!(audio.samples_sent(), 480);
    }
}
