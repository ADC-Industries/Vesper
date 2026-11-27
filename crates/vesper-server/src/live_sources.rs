//! Live network video sources (G7, `docs/LIVE-SOURCES-PLAN.md`): IDN Open Protocol
//! (compatible with NDI®) and OMT sources shown on the video-input layer.
//!
//! - **OMT** (`docs/OMT-PLAN.md`): a source named `omt:…` ([`output::omt::source_of`]) is
//!   received with [`output::OmtInput`] instead, with the same events, so everything below
//!   applies to it too. OMT sources are listed by an mDNS lookup every [`OMT_LOOKUP_EVERY`]
//!   (the crate has no browser with change events); one missing from two lookups in a row
//!   drops off.
//! - **The registry** ([`LiveSources`]) starts on first use: a thread keeps the source list
//!   (the library's browser, mDNS, or the site's discovery server while this box is enrolled
//!   in a site), closes receivers nobody used for [`IDLE_CLOSE`], and sends
//!   `sourcesChanged` after every change of the list or of a source's status.
//! - **One receiver per source in use** ([`LiveReceiver`]), shared by every screen. The render
//!   thread asks for it by name (a map lookup); it opens and reads on a thread of its own and
//!   keeps the newest picture, packed as NV12, in a one-slot mailbox. The render thread draws
//!   the newest picture on every tick of the screen's clock (it never waits for one): camera
//!   clock drift becomes an occasional repeated or skipped frame.
//! - **Lost sources** reconnect by themselves (the library); the last picture holds for
//!   [`HOLD_LOST`], then the layer draws nothing until pictures come back.
//! - **Stage 3 (video):** H.264 / HEVC sources are decoded through the media crate's FFmpeg
//!   adapter ([`media::network_decoder`]); an alpha plane is kept (Y and A packed as pairs,
//!   [`LiveFrame::alpha`]) and drawn through the renderer's alpha video path; interlaced
//!   sources arrive woven (the output crate weaves the fields).
//! - **Sound** (see [`crate::live_sound`]): a receiver in use also takes the source's sound
//!   and pushes it, with the arrival of each picture, into the source's sound feed. The sound
//!   follows the picture: a receiver no screen draws closes as before, and its sound stops
//!   until a screen draws the source again.
//! - **Thumbnails** of the bin come from the receiver in use, else from a proxy receiver
//!   (the source's low-bandwidth stream, one picture in [`PROXY_EVERY`]) that closes after
//!   [`PROXY_IDLE`] without a request.

use crate::{Shared, lock};
use output::{
    ListedSource, LiveEvent, LiveInput, LiveInputConfig, LiveMatrix, OmtInput, SourceChange,
};
use protocol::{LiveSource, LiveSourceStatus, ServerMessage};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

/// How long the last picture of a lost source stays up.
pub const HOLD_LOST: Duration = Duration::from_secs(3);
/// A receiver nobody drew or asked for this long is closed.
pub const IDLE_CLOSE: Duration = Duration::from_secs(5);
/// A thumbnail proxy nobody asked for this long is closed.
pub const PROXY_IDLE: Duration = Duration::from_secs(10);
/// A proxy takes one picture in this many.
pub const PROXY_EVERY: u32 = 15;
/// How often the OMT sources on the network are looked up.
pub const OMT_LOOKUP_EVERY: Duration = Duration::from_secs(5);
/// How long one OMT lookup waits for answers.
const OMT_LOOKUP_WAIT: Duration = Duration::from_millis(1500);

/// Render stream IDs of live sources (apart from video players' IDs).
static STREAM: AtomicU64 = AtomicU64::new(1 << 62);

/// A received picture, as the renderer's video path draws it (NV12, 8-bit).
pub struct LiveFrame {
    /// New number = new pixels.
    pub seq: u64,
    /// Width, px.
    pub width: u32,
    /// Height, px.
    pub height: u32,
    /// Colour matrix.
    pub matrix: LiveMatrix,
    /// Full range.
    pub full_range: bool,
    /// Y plane (`width` bytes a row).
    pub y: Vec<u8>,
    /// Interleaved CbCr (`ceil(width / 2) * 2` bytes a row).
    pub uv: Vec<u8>,
    /// The source has alpha: `y` holds Y and A as pairs (`2 * width` bytes a row), with
    /// the source's convention.
    pub alpha: Option<render::VideoAlpha>,
}

impl LiveFrame {
    /// Packs a received picture (Y and A as pairs when it has alpha).
    fn new(seq: u64, p: output::LivePicture) -> LiveFrame {
        let (y, alpha) = match &p.a {
            Some(a) => (
                p.y.iter().zip(a).flat_map(|(y, a)| [*y, *a]).collect(),
                Some(if p.premultiplied {
                    render::VideoAlpha::Premultiplied
                } else {
                    render::VideoAlpha::Straight
                }),
            ),
            None => (p.y, None),
        };
        LiveFrame {
            seq,
            width: p.width,
            height: p.height,
            matrix: p.matrix,
            full_range: p.full_range,
            y,
            uv: p.uv,
            alpha,
        }
    }

    /// The frame as the renderer takes it, its alpha (if any) as the source says.
    pub fn video_ref(&self, stream: u64) -> render::VideoFrameRef<'_> {
        self.video_ref_as(stream, model::SourceAlpha::Auto)
    }

    /// The frame as the renderer takes it, its alpha read as `alpha` says (`auto`: as the
    /// source says; `opaque`: ignored).
    pub fn video_ref_as(
        &self,
        stream: u64,
        alpha: model::SourceAlpha,
    ) -> render::VideoFrameRef<'_> {
        use model::SourceAlpha as S;
        let planes = match self.alpha {
            None => render::VideoPlanes::Nv12,
            Some(own) => render::VideoPlanes::Nv12Alpha(match alpha {
                S::Opaque => render::VideoAlpha::Opaque,
                S::Straight => render::VideoAlpha::Straight,
                S::Premultiplied => render::VideoAlpha::Premultiplied,
                S::Auto | S::Unknown => own,
            }),
        };
        render::VideoFrameRef {
            stream,
            seq: self.seq,
            width: self.width,
            height: self.height,
            planes,
            y: &self.y,
            y_stride: self.y_stride(),
            uv: &self.uv,
            uv_stride: self.width.div_ceil(2) as usize * 2,
            matrix: match self.matrix {
                LiveMatrix::Bt601 => render::VideoMatrix::Bt601,
                LiveMatrix::Bt709 => render::VideoMatrix::Bt709,
                LiveMatrix::Bt2020 => render::VideoMatrix::Bt2020,
            },
            full_range: self.full_range,
            rotation: 0,
        }
    }

    /// Bytes per row of `y` (two a pixel with alpha).
    fn y_stride(&self) -> usize {
        self.width as usize * if self.alpha.is_some() { 2 } else { 1 }
    }

    /// The picture scaled to `width` (aspect kept), as RGB (nearest sample, for thumbnails;
    /// alpha is ignored).
    pub fn to_rgb(&self, width: u32) -> image::RgbImage {
        let (w, h) = (self.width.max(1), self.height.max(1));
        let tw = width.clamp(1, w);
        let th = ((h as u64 * tw as u64) / w as u64).max(1) as u32;
        let (kr, kb) = match self.matrix {
            LiveMatrix::Bt601 => (0.299f32, 0.114f32),
            LiveMatrix::Bt709 => (0.2126, 0.0722),
            LiveMatrix::Bt2020 => (0.2627, 0.0593),
        };
        let kg = 1.0 - kr - kb;
        let (ys, yo, cs) = if self.full_range {
            (1.0 / 255.0, 0.0, 1.0 / 255.0)
        } else {
            (1.0 / 219.0, 16.0, 1.0 / 224.0)
        };
        let cw = w.div_ceil(2) as usize;
        image::RgbImage::from_fn(tw, th, |x, y| {
            let sx = (x as u64 * w as u64 / tw as u64) as usize;
            let sy = (y as u64 * h as u64 / th as u64) as usize;
            let yi = sy * self.y_stride() + sx * if self.alpha.is_some() { 2 } else { 1 };
            let yy = (*self.y.get(yi).unwrap_or(&16) as f32 - yo) * ys;
            let ci = (sy / 2) * cw * 2 + (sx / 2) * 2;
            let cb = (*self.uv.get(ci).unwrap_or(&128) as f32 - 128.0) * cs;
            let cr = (*self.uv.get(ci + 1).unwrap_or(&128) as f32 - 128.0) * cs;
            let r = yy + 2.0 * (1.0 - kr) * cr;
            let b = yy + 2.0 * (1.0 - kb) * cb;
            let g = (yy - kr * r - kb * b) / kg;
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            image::Rgb([q(r), q(g), q(b)])
        })
    }
}

#[derive(Default)]
struct RxStatus {
    status: LiveSourceStatus,
    width: Option<u32>,
    height: Option<u32>,
    frame_rate: Option<f64>,
    message: Option<String>,
    lost_at: Option<Instant>,
    address: Option<String>,
}

/// One received source (see the [module docs](self)).
pub struct LiveReceiver {
    /// `HOST (name)` or `ip:port`.
    pub source: String,
    /// Render stream ID (uploads are cached per stream).
    pub stream: u64,
    proxy: bool,
    frame: Mutex<Option<Arc<LiveFrame>>>,
    status: Mutex<RxStatus>,
    used: Mutex<Instant>,
    stop: AtomicBool,
    seq: AtomicU64,
    /// Set when the status changed (the registry sends `sourcesChanged`).
    dirty: Arc<AtomicBool>,
    /// Where its sound goes (the source's feed; a proxy's is unused).
    sound: Arc<crate::live_sound::LiveSoundFeed>,
}

impl LiveReceiver {
    /// The picture to draw now: the newest, held for [`HOLD_LOST`] after the source was
    /// lost (`None`: nothing to draw, or the receiver was closed). Marks the receiver as in
    /// use.
    pub fn frame(&self) -> Option<Arc<LiveFrame>> {
        self.touch();
        if self.is_stopped() {
            return None;
        }
        let lost = lock(&self.status)
            .lost_at
            .is_some_and(|t| t.elapsed() > HOLD_LOST);
        if lost {
            return None;
        }
        lock(&self.frame).clone()
    }

    /// Closed (nobody drew it for [`IDLE_CLOSE`], or the server stopped): it receives no
    /// more pictures. Whoever still holds it asks [`LiveSources::receiver`] again.
    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    fn touch(&self) {
        *lock(&self.used) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        lock(&self.used).elapsed()
    }

    fn set(&self, f: impl FnOnce(&mut RxStatus)) {
        f(&mut lock(&self.status));
        self.dirty.store(true, Ordering::Relaxed);
    }

    fn info(&self, out: &mut LiveSource) {
        let s = lock(&self.status);
        out.status = s.status;
        out.width = s.width;
        out.height = s.height;
        out.frame_rate = s.frame_rate;
        out.message = s.message.clone();
        if out.address.is_none() {
            out.address = s.address.clone();
        }
    }

    /// The receive loop (its own thread): opens the input, then keeps the newest picture.
    fn run(self: Arc<Self>, max_size: Option<(u32, u32)>) {
        // H.264 / HEVC sources decode through the system FFmpeg (not for thumbnail proxies:
        // their low-bandwidth stream is SpeedHQ).
        let decoder = (!self.proxy).then(|| {
            let d: Arc<dyn idn_open_protocol::codec::DecoderFactory> = media::network_decoder();
            output::LiveDecoder(d)
        });
        let cfg = LiveInputConfig {
            decoder,
            source: self.source.clone(),
            max_size: if self.proxy {
                Some((320, 180))
            } else {
                max_size
            },
            proxy: self.proxy,
            picture_every: self.proxy.then_some(PROXY_EVERY),
            sound: !self.proxy,
        };
        let omt = output::omt::source_of(&self.source).map(str::to_string);
        let input = loop {
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            let opened = match &omt {
                Some(name) => OmtInput::open(&LiveInputConfig {
                    source: name.clone(),
                    decoder: None,
                    ..cfg.clone()
                })
                .map(Input::Omt),
                None => LiveInput::open(&cfg).map(|i| Input::Idn(Box::new(i))),
            };
            match opened {
                Ok(i) => break i,
                Err(e) => {
                    tracing::warn!("live source {:?}: {e:#}", self.source);
                    self.set(|s| {
                        s.status = LiveSourceStatus::Lost;
                        s.message = Some(format!("{e:#}"));
                        s.lost_at.get_or_insert_with(Instant::now);
                    });
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        };
        tracing::info!(
            "live source {:?}: receiving{}",
            self.source,
            if self.proxy { " (thumbnail proxy)" } else { "" }
        );
        while !self.stop.load(Ordering::Relaxed) {
            let Some(ev) = input.next(Duration::from_millis(100)) else {
                continue;
            };
            match ev {
                LiveEvent::Connecting => self.set(|s| {
                    if s.status != LiveSourceStatus::Lost {
                        s.status = LiveSourceStatus::Connecting;
                    }
                }),
                LiveEvent::Connected(a) if a.is_empty() => {}
                LiveEvent::Connected(a) => self.set(|s| s.address = Some(a)),
                LiveEvent::Lost(why) => {
                    tracing::info!("live source {:?} lost: {why}", self.source);
                    self.set(|s| {
                        s.status = LiveSourceStatus::Lost;
                        s.message = Some(why);
                        s.lost_at.get_or_insert_with(Instant::now);
                    });
                }
                LiveEvent::Compressed => {
                    let new = lock(&self.status).status != LiveSourceStatus::Unsupported;
                    if new {
                        self.set(|s| {
                            s.status = LiveSourceStatus::Unsupported;
                            s.message = Some(
                                "H.264 / HEVC video this machine doesn't decode (see the \
                                 server log for FFmpeg's decoders)"
                                    .into(),
                            );
                        });
                    }
                }
                LiveEvent::Sound(s) => self.sound.sound(*s, Instant::now()),
                LiveEvent::Picture(p) => {
                    self.sound.picture(p.timecode, Instant::now());
                    let fps =
                        (p.frame_rate.1 > 0).then(|| p.frame_rate.0 as f64 / p.frame_rate.1 as f64);
                    let changed = {
                        let s = lock(&self.status);
                        s.status != LiveSourceStatus::Live
                            || s.width != Some(p.width)
                            || s.height != Some(p.height)
                            || s.frame_rate != fps
                    };
                    let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
                    let (width, height, alpha, interlaced) =
                        (p.width, p.height, p.alpha, p.interlaced);
                    *lock(&self.frame) = Some(Arc::new(LiveFrame::new(seq, *p)));
                    if changed {
                        // A new format shows up in the picture's header: there is no
                        // separate event (library semantics).
                        tracing::info!(
                            "live source {:?}: {}x{} at {:.2} fps{}{}",
                            self.source,
                            width,
                            height,
                            fps.unwrap_or(0.0),
                            if interlaced {
                                ", interlaced (woven)"
                            } else {
                                ""
                            },
                            if alpha { ", with alpha" } else { "" }
                        );
                        self.set(|s| {
                            s.status = LiveSourceStatus::Live;
                            s.width = Some(width);
                            s.height = Some(height);
                            s.frame_rate = fps;
                            s.message = None;
                            s.lost_at = None;
                        });
                    }
                }
            }
        }
        tracing::info!("live source {:?}: closed", self.source);
    }
}

/// The input of a receiver: IDN or OMT, with the same events.
enum Input {
    Idn(Box<LiveInput>),
    Omt(OmtInput),
}

impl Input {
    fn next(&self, timeout: Duration) -> Option<LiveEvent> {
        match self {
            Input::Idn(i) => i.next(timeout),
            Input::Omt(i) => i.next(timeout),
        }
    }
}

#[derive(Default)]
struct Inner {
    listed: BTreeMap<String, ListedSource>,
    /// OMT sources found by the last lookups, by key (`omt:HOST (name)`), with the lookups
    /// in a row they were missing from.
    omt: BTreeMap<String, u32>,
    receivers: HashMap<String, Arc<LiveReceiver>>,
    proxies: HashMap<String, Arc<LiveReceiver>>,
}

/// The source registry and the receivers in use (see the [module docs](self)).
#[derive(Default)]
pub struct LiveSources {
    inner: Mutex<Inner>,
    /// Sound feeds by source name (kept across receiver restarts).
    feeds: Mutex<HashMap<String, Arc<crate::live_sound::LiveSoundFeed>>>,
    dirty: Arc<AtomicBool>,
    shared: OnceLock<Weak<Shared>>,
    started: AtomicBool,
    stopped: AtomicBool,
}

impl LiveSources {
    /// Ties the registry to its server (once, when the server starts).
    pub fn bind(&self, shared: Weak<Shared>) {
        let _ = self.shared.set(shared);
    }

    /// The sound feed of `source` (its receivers push into it; the live sound lane plays it).
    pub fn sound_feed(&self, source: &str) -> Arc<crate::live_sound::LiveSoundFeed> {
        lock(&self.feeds)
            .entry(source.to_string())
            .or_default()
            .clone()
    }

    /// The receiver of `source`, opened (on its own thread) if it is not running. Never
    /// blocks on the network.
    pub fn receiver(&self, source: &str, max_size: Option<(u32, u32)>) -> Arc<LiveReceiver> {
        self.ensure_started();
        let mut inner = lock(&self.inner);
        if let Some(r) = inner.receivers.get(source) {
            r.touch();
            return r.clone();
        }
        let r = self.open(source, false, max_size);
        if !r.is_stopped() {
            inner.receivers.insert(source.to_string(), r.clone());
        }
        r
    }

    fn open(&self, source: &str, proxy: bool, max_size: Option<(u32, u32)>) -> Arc<LiveReceiver> {
        let r = Arc::new(LiveReceiver {
            source: source.to_string(),
            stream: STREAM.fetch_add(1, Ordering::Relaxed),
            proxy,
            frame: Mutex::new(None),
            status: Mutex::new(RxStatus {
                status: LiveSourceStatus::Connecting,
                ..RxStatus::default()
            }),
            used: Mutex::new(Instant::now()),
            stop: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            dirty: self.dirty.clone(),
            sound: if proxy {
                Arc::default()
            } else {
                self.sound_feed(source)
            },
        });
        // A stopped server opens nothing (a screen still planning hears of a closed one).
        if self.stopped.load(Ordering::Relaxed) {
            r.stop.store(true, Ordering::Relaxed);
            return r;
        }
        let run = r.clone();
        let spawned = std::thread::Builder::new()
            .name("live-source".into())
            .spawn(move || run.run(max_size));
        if let Err(e) = spawned {
            tracing::warn!("live source {source:?}: no thread: {e}");
        }
        self.dirty.store(true, Ordering::Relaxed);
        r
    }

    /// The newest picture of `source` for a thumbnail: from its receiver in use, else from a
    /// thumbnail proxy (opened now; `None` until its first picture).
    pub fn thumbnail(&self, source: &str) -> Option<Arc<LiveFrame>> {
        self.ensure_started();
        let mut inner = lock(&self.inner);
        if let Some(r) = inner.receivers.get(source)
            && let Some(f) = lock(&r.frame).clone()
        {
            return Some(f);
        }
        let r = match inner.proxies.get(source) {
            Some(r) => r.clone(),
            None => {
                let r = self.open(source, true, None);
                if !r.is_stopped() {
                    inner.proxies.insert(source.to_string(), r.clone());
                }
                r
            }
        };
        drop(inner);
        r.touch();
        lock(&r.frame).clone()
    }

    /// Every listed source and every source received, by name.
    pub fn list(&self) -> Vec<LiveSource> {
        self.ensure_started();
        let inner = lock(&self.inner);
        let mut out: BTreeMap<String, LiveSource> = inner
            .listed
            .values()
            .map(|s| {
                (
                    s.name.clone(),
                    LiveSource {
                        name: s.name.clone(),
                        address: s.address.clone(),
                        idn: s.idn,
                        listed: true,
                        ..LiveSource::default()
                    },
                )
            })
            .collect();
        for key in inner.omt.keys() {
            out.insert(
                key.clone(),
                LiveSource {
                    name: key.clone(),
                    omt: true,
                    listed: true,
                    ..LiveSource::default()
                },
            );
        }
        for (name, r) in &inner.receivers {
            let e = out.entry(name.clone()).or_insert_with(|| LiveSource {
                name: name.clone(),
                omt: output::omt::source_of(name).is_some(),
                ..LiveSource::default()
            });
            r.info(e);
        }
        out.into_values().collect()
    }

    /// Starts the registry thread (once).
    fn ensure_started(&self) {
        if self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(weak) = self.shared.get().cloned() else {
            return;
        };
        let omt = weak.clone();
        let spawned = std::thread::Builder::new()
            .name("live-sources".into())
            .spawn(move || registry_loop(weak));
        if let Err(e) = spawned {
            tracing::warn!("live sources: no registry thread: {e}");
        }
        let spawned = std::thread::Builder::new()
            .name("omt-sources".into())
            .spawn(move || omt_loop(omt));
        if let Err(e) = spawned {
            tracing::warn!("live sources: no OMT lookup thread: {e}");
        }
    }

    /// Takes the result of an OMT lookup (full names): new ones are listed, and one missing
    /// from two lookups in a row drops off. Returns whether the list changed.
    fn omt_found(&self, found: &[String]) -> bool {
        let mut inner = lock(&self.inner);
        let keys: std::collections::BTreeSet<String> =
            found.iter().map(|n| output::omt::key(n)).collect();
        let mut changed = false;
        inner.omt.retain(|k, missed| {
            if keys.contains(k) {
                *missed = 0;
                return true;
            }
            *missed += 1;
            let keep = *missed < 2;
            changed |= !keep;
            keep
        });
        for k in keys {
            if let std::collections::btree_map::Entry::Vacant(e) = inner.omt.entry(k) {
                e.insert(0);
                changed = true;
            }
        }
        changed
    }

    /// Closes receivers and proxies nobody used lately; returns whether any closed.
    fn reap(&self) -> bool {
        let mut guard = lock(&self.inner);
        let inner = &mut *guard;
        let mut closed = false;
        for (map, idle) in [
            (&mut inner.receivers, IDLE_CLOSE),
            (&mut inner.proxies, PROXY_IDLE),
        ] {
            map.retain(|_, r| {
                let keep = r.idle_for() < idle;
                if !keep {
                    r.stop.store(true, Ordering::Relaxed);
                    closed = true;
                }
                keep
            });
        }
        closed
    }

    /// Stops every receiver (the server is stopping).
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        let mut inner = lock(&self.inner);
        for r in inner.receivers.values().chain(inner.proxies.values()) {
            r.stop.store(true, Ordering::Relaxed);
        }
        inner.receivers.clear();
        inner.proxies.clear();
    }
}

/// The discovery server to browse with: the site's while this box is enrolled in another
/// server's site, else none (mDNS).
fn discovery_server(sh: &Shared) -> Option<String> {
    let m = sh.site.membership();
    (m.state == protocol::MembershipState::Enrolled)
        .then_some(m.server)
        .flatten()
}

fn registry_loop(weak: Weak<Shared>) {
    let mut watcher: Option<(Option<String>, output::SourceWatcher)> = None;
    let mut checked = Instant::now() - Duration::from_secs(60);
    loop {
        let Some(sh) = weak.upgrade() else { return };
        let reg = &sh.live_sources;
        if reg.stopped.load(Ordering::Relaxed) {
            return;
        }
        // (Re)start the browser when the discovery server to use changes.
        if checked.elapsed() > Duration::from_secs(3) {
            checked = Instant::now();
            let want = discovery_server(&sh);
            if watcher.as_ref().is_none_or(|(d, _)| *d != want) {
                match output::SourceWatcher::start(want.clone()) {
                    Ok(w) => {
                        let mut inner = lock(&reg.inner);
                        inner.listed = w
                            .sources()
                            .into_iter()
                            .map(|s| (s.name.clone(), s))
                            .collect();
                        drop(inner);
                        tracing::info!(
                            "live sources: browsing {}",
                            want.as_deref().unwrap_or("with mDNS")
                        );
                        watcher = Some((want, w));
                        reg.dirty.store(true, Ordering::Relaxed);
                    }
                    Err(e) => tracing::warn!("live sources: {e:#}"),
                }
            }
        }
        let change = watcher
            .as_ref()
            .and_then(|(_, w)| w.next(Duration::from_millis(250)));
        if watcher.is_none() {
            std::thread::sleep(Duration::from_millis(250));
        }
        if let Some(c) = change {
            let mut inner = lock(&reg.inner);
            match c {
                SourceChange::Up(s) => {
                    inner.listed.insert(s.name.clone(), s);
                }
                SourceChange::Gone(n) => {
                    inner.listed.remove(&n);
                }
            }
            reg.dirty.store(true, Ordering::Relaxed);
        }
        if reg.reap() {
            reg.dirty.store(true, Ordering::Relaxed);
        }
        if reg.dirty.swap(false, Ordering::AcqRel) {
            let sources = reg.list();
            let _ = sh
                .events
                .send(Arc::new(ServerMessage::SourcesChanged { sources }));
        }
        drop(sh);
    }
}

/// Looks up the OMT sources every [`OMT_LOOKUP_EVERY`] until the server stops (mDNS only:
/// a site's discovery server lists IDN sources).
fn omt_loop(weak: Weak<Shared>) {
    loop {
        let found = output::omt::sources(OMT_LOOKUP_WAIT);
        let Some(sh) = weak.upgrade() else { return };
        let reg = &sh.live_sources;
        if reg.stopped.load(Ordering::Relaxed) {
            return;
        }
        if reg.omt_found(&found) {
            reg.dirty.store(true, Ordering::Relaxed);
        }
        drop(sh);
        let until = Instant::now() + OMT_LOOKUP_EVERY;
        while Instant::now() < until {
            std::thread::sleep(Duration::from_millis(250));
            let gone = weak
                .upgrade()
                .is_none_or(|sh| sh.live_sources.stopped.load(Ordering::Relaxed));
            if gone {
                return;
            }
        }
    }
}

/// A slide element whose fill is a live source (stage 3: a camera box on a slide).
#[derive(Clone, Debug, PartialEq)]
pub struct LiveFill {
    /// The element.
    pub element: model::ElementId,
    /// The source to receive (`HOST (name)` or `ip:port`).
    pub source: String,
    /// How its alpha is read.
    pub alpha: model::SourceAlpha,
}

/// Whether `slide` has a visible element with a live-source fill.
pub fn has_live_fills(slide: &model::Slide) -> bool {
    slide.elements.iter().any(|e| {
        !e.hidden && e.fill.enabled && matches!(e.fill.kind, model::FillKind::LiveSource { .. })
    })
}

/// The live-source fills of `slide` with a source to receive: a fill's logical input
/// resolves through this machine's bindings (`data`), else its `source` is used; fills with
/// neither are left out (they draw nothing).
pub fn live_fills(slide: &model::Slide, data: &crate::data::ShowData) -> Vec<LiveFill> {
    slide
        .elements
        .iter()
        .filter(|e| !e.hidden && e.fill.enabled)
        .filter_map(|e| {
            let model::FillKind::LiveSource {
                source,
                input,
                display,
            } = &e.fill.kind
            else {
                return None;
            };
            let bound = input.and_then(|i| data.input_source(i).ok());
            let input_alpha = input
                .and_then(|i| data.video_inputs.iter().find(|v| v.id == i))
                .map(|v| v.alpha);
            let source = bound.or_else(|| {
                let s = source.trim();
                (!s.is_empty()).then(|| s.to_string())
            })?;
            Some(LiveFill {
                element: e.id,
                source,
                alpha: match (display.alpha, input_alpha) {
                    (model::SourceAlpha::Auto, Some(a)) => a,
                    (a, _) => a,
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OMT lookups: new names are listed as `omt:` keys; one missing from a single lookup
    /// stays, missing from two in a row drops off.
    #[test]
    fn omt_lookups_list_and_drop_sources() {
        let reg = LiveSources::default();
        let a = "STUDIO (Program)".to_string();
        let b = "STUDIO (Slides)".to_string();
        assert!(reg.omt_found(&[a.clone(), b.clone()]));
        assert!(!reg.omt_found(&[a.clone(), b.clone()]), "nothing new");
        let names = |r: &LiveSources| lock(&r.inner).omt.keys().cloned().collect::<Vec<_>>();
        assert_eq!(names(&reg), ["omt:STUDIO (Program)", "omt:STUDIO (Slides)"]);
        assert!(
            !reg.omt_found(std::slice::from_ref(&a)),
            "missed once: kept"
        );
        assert_eq!(names(&reg).len(), 2);
        assert!(
            reg.omt_found(std::slice::from_ref(&a)),
            "missed twice: gone"
        );
        assert_eq!(names(&reg), ["omt:STUDIO (Program)"]);
        let listed = reg.list();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].omt && listed[0].listed && !listed[0].idn);
    }
}
