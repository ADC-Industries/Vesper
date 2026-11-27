//! Open Media Transport (OMT, openmediatransport.org): sources and receivers through the
//! IDN's `idn-omt` crate (OMT written in Rust, its VMX codec compiled in from the
//! vendored libvmx; MIT / Apache-2.0). Nothing to install or bundle.
//!
//! - [`OmtSink`] is a [`FrameSink`]: one OMT source per sink ("HOST (name)" on the network,
//!   announced on mDNS as `_omt._tcp`, a TCP port from 6400). Frames are converted with the
//!   shared [`crate::convert`] routines (BT.709 limited range, and marked BT.709 at every size with
//!   `Sender::set_color_space`): opaque screens as UYVY, screens
//!   with alpha as UYVY with an alpha plane after it (UYVA): straight colour for a `Straight`
//!   screen, the colour premultiplied (in R'G'B', as rendered) for a `Premultiplied` one, the
//!   source marked premultiplied then (`Sender::set_premultiplied`). With no receiver connected
//!   nothing is converted. `send` codes the picture with VMX on the caller's thread: the server
//!   calls it from the output's own worker thread, which drops frames while it is busy.
//! - [`OmtAudioSender`] sends program audio on the sink's source (`send_audio`, planar float),
//!   stamped with the timecode it was given: the program clock the video is stamped with too.
//! - [`OmtInput`] receives one source with the same [`LiveEvent`]s as an IDN
//!   [`LiveInput`](crate::LiveInput): pictures packed as NV12 (alpha plane kept, straight or
//!   premultiplied as the sender marks it), sound as planar float. The receiver reconnects by
//!   itself (finding the source by name). A thumbnail proxy asks for previews (an eighth of
//!   each side); a source that sends none goes on sending whole pictures, which are used as
//!   they come.
//! - [`sources`] lists the OMT sources on the network (a one-shot mDNS lookup).
//!
//! **Source keys.** Where Vesper names a live source by one string (the video-input layer, a
//! live element fill, the source registry), an OMT source is `omt:` followed by what the
//! receiver resolves ("HOST (NAME)", `host:port`), or an `omt://host:port` address as is
//! ([`source_of`], [`key`]). Strings without the prefix are IDN sources.
//!
//! Colour of received pictures: the matrix the frame header gives (`Picture::color_space`:
//! BT.601 or BT.709; a sender saying nothing is resolved by the library as VMX codes it, BT.709
//! from 720 lines and BT.601 below), always 8-bit limited range.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use idn_omt::vmx::uyvy_to_planes;
use idn_omt::{Event, Receiver, Sender, SenderConfig};

use crate::convert::{premul_rgba_to_uyva, premul_rgba_to_uyvy};
use crate::idn_live::pack_chroma;
use crate::{
    AlphaMode, AudioSink, Frame, FrameSink, LiveEvent, LiveInputConfig, LiveMatrix, LivePicture,
    LiveSound,
};

/// Human-readable name of the transport.
pub const LABEL: &str = "OMT (Open Media Transport)";

/// The OMT release in use (IDN's release of `idn-omt`).
pub const LIBRARY_VERSION: &str = idn_open_protocol::VERSION;

/// The prefix of OMT source keys.
pub const PREFIX: &str = "omt:";

/// The OMT source a live-source key names (`omt:NAME` → `NAME`, `omt://host:port` as is);
/// `None` for other keys (IDN sources) and an empty name.
pub fn source_of(key: &str) -> Option<&str> {
    let key = key.trim();
    if key.starts_with("omt://") {
        return Some(key);
    }
    key.strip_prefix(PREFIX)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The live-source key of the OMT source `name` ("HOST (NAME)" or an address).
pub fn key(name: &str) -> String {
    let name = name.trim();
    if name.starts_with("omt://") {
        name.to_string()
    } else {
        format!("{PREFIX}{name}")
    }
}

/// The OMT sources on the network now: their full names ("HOST (NAME)"), sorted. Waits up to
/// `timeout` for answers (mDNS).
pub fn sources(timeout: Duration) -> Vec<String> {
    idn_omt::receiver::sources(timeout)
}

/// Settings for one OMT source.
#[derive(Clone, Debug)]
pub struct OmtSenderConfig {
    /// Source name; receivers list `"HOST (name)"`.
    pub name: String,
    /// Announce it on mDNS (off in tests that connect by address).
    pub announce: bool,
}

impl OmtSenderConfig {
    /// An announced source called `name`.
    pub fn new(name: impl Into<String>) -> Self {
        OmtSenderConfig {
            name: name.into(),
            announce: true,
        }
    }
}

/// Sends frames as an OMT source (see the [module docs](self)).
pub struct OmtSink {
    sender: Arc<Sender>,
    label: String,
    /// The converted picture (UYVY, then the alpha plane), reused.
    buf: Vec<u8>,
    frames_sent: u64,
    /// What the source was last told about its alpha (straight when it starts).
    premultiplied: bool,
}

impl OmtSink {
    /// Starts the source (listening and, unless turned off, announced).
    pub fn new(config: OmtSenderConfig) -> Result<Self> {
        let name = config.name.trim().to_string();
        ensure!(!name.is_empty(), "an OMT source needs a name");
        let sender = Sender::start(SenderConfig {
            announce: config.announce,
            info: Some((
                "Vesper".into(),
                "Vesper".into(),
                env!("CARGO_PKG_VERSION").into(),
            )),
            ..SenderConfig::new(&name)
        })
        .with_context(|| format!("OMT source {name:?} could not start"))?;
        // Every size is converted with BT.709 (`crate::convert`), so every picture says so; left
        // alone, pictures under 720 lines would be marked BT.601 (VMX's rule by height).
        sender.set_color_space(Some(709));
        log::info!(
            "OMT source {name:?} started (idn-omt {LIBRARY_VERSION}, port {})",
            sender.port()
        );
        Ok(OmtSink {
            sender: Arc::new(sender),
            label: format!("OMT: {name}"),
            buf: Vec::new(),
            frames_sent: 0,
            premultiplied: false,
        })
    }

    /// The TCP port receivers connect to.
    pub fn port(&self) -> u16 {
        self.sender.port()
    }

    /// Connections from receivers now (each receiver makes one or two).
    pub fn connections(&self) -> usize {
        self.sender.connections()
    }

    /// Frames handed to the source (with at least one receiver connected).
    pub fn frames_sent(&self) -> u64 {
        self.frames_sent
    }

    /// A sender for program audio on this source (usable from another thread).
    pub fn audio_sender(&self) -> OmtAudioSender {
        OmtAudioSender {
            sender: Arc::downgrade(&self.sender),
            sent: 0,
        }
    }
}

impl FrameSink for OmtSink {
    fn name(&self) -> &str {
        &self.label
    }

    fn send(&mut self, frame: &Frame<'_>) -> Result<()> {
        frame.validate()?;
        if self.sender.connections() == 0 {
            return Ok(());
        }
        // UYVY takes pixel pairs: an odd width loses its last column.
        let (fw, w, h) = (
            frame.width as usize,
            frame.width as usize & !1,
            frame.height as usize,
        );
        ensure!(w >= 2, "an OMT picture needs at least two columns");
        let alpha = frame.has_alpha();
        // A premultiplied screen goes out premultiplied, and the source says so (the flag is
        // sender-wide, so it follows the screen's alpha type).
        let premultiplied = alpha && frame.alpha == AlphaMode::Premultiplied;
        if alpha && premultiplied != self.premultiplied {
            self.sender.set_premultiplied(premultiplied);
            self.premultiplied = premultiplied;
        }
        let plane = w * h * 2;
        self.buf.resize(plane + if alpha { w * h } else { 0 }, 0);
        let (uyvy, a) = self.buf.split_at_mut(plane);
        for row in 0..h {
            let src = &frame.pixels[row * fw * 4..][..w * 4];
            let dst = &mut uyvy[row * w * 2..][..w * 2];
            if premultiplied {
                // The colour as rendered (over black), the alpha plane beside it.
                premul_rgba_to_uyvy(src, dst);
                for (a, px) in a[row * w..][..w].iter_mut().zip(src.as_chunks::<4>().0) {
                    *a = px[3];
                }
            } else if alpha {
                premul_rgba_to_uyva(src, dst, &mut a[row * w..][..w]);
            } else {
                premul_rgba_to_uyvy(src, dst);
            }
        }
        let rate = (frame.frame_rate.num, frame.frame_rate.den.max(1));
        self.sender
            .send_video(&mut self.buf, w, h, rate, alpha, false, frame.timecode)
            .with_context(|| format!("{}: sending a picture", self.label))?;
        self.frames_sent += 1;
        Ok(())
    }
}

/// Sends program audio on an [`OmtSink`]'s source from another thread. It never waits for
/// video; once the sink is gone it drops what it gets.
pub struct OmtAudioSender {
    sender: Weak<Sender>,
    sent: u64,
}

impl AudioSink for OmtAudioSender {
    fn send(
        &mut self,
        interleaved: &[f32],
        channels: usize,
        sample_rate: u32,
        timecode: i64,
    ) -> Result<()> {
        ensure!(channels > 0, "no channels");
        let n = interleaved.len() / channels;
        if n == 0 {
            return Ok(());
        }
        let Some(sender) = self.sender.upgrade() else {
            return Ok(());
        };
        let mut planes = vec![Vec::with_capacity(n); channels];
        for frame in interleaved.chunks_exact(channels) {
            for (p, v) in planes.iter_mut().zip(frame) {
                p.push(*v);
            }
        }
        sender.send_audio(&planes, sample_rate, timecode);
        self.sent += n as u64;
        Ok(())
    }

    fn samples_sent(&self) -> u64 {
        self.sent
    }
}

/// Receives one OMT source for showing on screens (see the [module docs](self)); opened on
/// the caller's thread, read from a thread of its own.
pub struct OmtInput {
    rx: Receiver,
    source: String,
    sound: bool,
    /// A thumbnail proxy: previews were asked for.
    preview: bool,
    /// Logged once: the source sends whole pictures to a proxy (it has no previews).
    whole_logged: AtomicBool,
    /// Keep one picture in this many (thumbnail proxies), counting in `seen`.
    every: u32,
    seen: AtomicU32,
}

impl OmtInput {
    /// Opens the receiver for `cfg.source` (the OMT name or address, without the `omt:`
    /// prefix); it connects, and reconnects, in the background. `max_size` has no OMT
    /// equivalent and is ignored; a proxy asks for previews (an eighth of each side), takes
    /// no sound and keeps one picture in `picture_every`.
    pub fn open(cfg: &LiveInputConfig) -> Result<Self> {
        let source = cfg.source.trim();
        let sound = cfg.sound && !cfg.proxy;
        let rx =
            Receiver::start(source, sound).with_context(|| format!("OMT input for {source:?}"))?;
        if cfg.proxy {
            rx.set_preview(true);
        }
        Ok(OmtInput {
            rx,
            source: source.to_string(),
            sound,
            preview: cfg.proxy,
            whole_logged: AtomicBool::new(false),
            every: if cfg.proxy {
                cfg.picture_every.unwrap_or(1).max(1)
            } else {
                1
            },
            seen: AtomicU32::new(0),
        })
    }

    /// Tells the source whether it is on program (its tally light).
    pub fn set_tally(&self, program: bool, preview: bool) {
        self.rx.set_tally(program, preview);
    }

    /// The next event, waiting up to `timeout` in all (`None`: nothing to report arrived).
    /// Metadata, sound unless asked for, and pictures that can't be packed are skipped, and
    /// skipped events don't extend the wait.
    pub fn next(&self, timeout: Duration) -> Option<LiveEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            return Some(match self.rx.recv(left)? {
                Event::Connected => LiveEvent::Connected(String::new()),
                Event::Disconnected(why) => LiveEvent::Lost(why),
                Event::Video(p) => {
                    if !self
                        .seen
                        .fetch_add(1, Ordering::Relaxed)
                        .is_multiple_of(self.every)
                    {
                        continue;
                    }
                    if self.preview
                        && !p.preview
                        && !self.whole_logged.swap(true, Ordering::Relaxed)
                    {
                        log::info!(
                            "OMT source {:?} sends no previews: its thumbnails come from whole \
                             {}x{} pictures",
                            self.source,
                            p.width,
                            p.height
                        );
                    }
                    let Some(p) = pack(p) else { continue };
                    LiveEvent::Picture(Box::new(p))
                }
                Event::Audio {
                    sample_rate,
                    channels,
                    timestamp,
                } if self.sound => LiveEvent::Sound(Box::new(LiveSound {
                    sample_rate,
                    timecode: timestamp,
                    channels,
                })),
                _ => continue,
            });
        }
    }
}

/// A received OMT picture (UYVY, its alpha plane after it) as NV12 with the alpha plane
/// kept (premultiplied when the sender says so), in the matrix the sender gives; `None` for
/// an odd width or a buffer shorter than the picture.
fn pack(p: idn_omt::Picture) -> Option<LivePicture> {
    let (w, h) = (p.width, p.height);
    if w < 2 || h == 0 || !w.is_multiple_of(2) {
        return None;
    }
    let need = w * h * 2 + if p.alpha { w * h } else { 0 };
    if p.uyvy.len() < need {
        return None;
    }
    let (y, cb, cr, a) = uyvy_to_planes(&p.uyvy, w, h, p.alpha);
    let uv = pack_chroma(w, h, &cb, &cr, w / 2, h)?;
    Some(LivePicture {
        width: w as u32,
        height: h as u32,
        frame_rate: p.rate,
        timecode: p.timestamp,
        field: 0,
        interlaced: p.interlaced,
        alpha: a.is_some(),
        premultiplied: a.is_some() && p.premultiplied,
        a,
        matrix: match p.color_space {
            709 => LiveMatrix::Bt709,
            601 => LiveMatrix::Bt601,
            // The library resolves "not given" by height; anything else, the same way.
            _ if h >= 720 => LiveMatrix::Bt709,
            _ => LiveMatrix::Bt601,
        },
        full_range: false,
        y,
        uv,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AlphaMode, test_util};

    #[test]
    fn source_keys() {
        assert_eq!(source_of("omt:STUDIO (Program)"), Some("STUDIO (Program)"));
        assert_eq!(source_of(" omt: 192.0.2.10:6400 "), Some("192.0.2.10:6400"));
        assert_eq!(
            source_of("omt://192.0.2.10:6400"),
            Some("omt://192.0.2.10:6400")
        );
        assert_eq!(source_of("omt:"), None);
        assert_eq!(source_of("STUDIO (Program)"), None);
        assert_eq!(source_of("192.0.2.10:5961"), None);
        assert_eq!(key(" STUDIO (Program) "), "omt:STUDIO (Program)");
        assert_eq!(key("omt://192.0.2.10:6400"), "omt://192.0.2.10:6400");
        assert_eq!(source_of(&key("A (B)")), Some("A (B)"));
    }

    #[test]
    fn needs_a_name() {
        assert!(OmtSink::new(OmtSenderConfig::new("  ")).is_err());
    }

    /// Sends frames (and, with `sound`, audio) to a receiver (a thumbnail proxy with
    /// `proxy`) until `until` says it got what it wanted, or 10 s pass.
    fn loopback(
        alpha: AlphaMode,
        (w, h): (u32, u32),
        px: Vec<u8>,
        (sound, proxy): (bool, bool),
        mut until: impl FnMut(LiveEvent) -> bool + Send + 'static,
    ) -> bool {
        let name = format!(
            "vesper-omt-test-{}-{w}-{sound}-{proxy}-{alpha:?}",
            std::process::id()
        );
        let mut sink = OmtSink::new(OmtSenderConfig {
            announce: false,
            ..OmtSenderConfig::new(name)
        })
        .expect("OMT source");
        let input = OmtInput::open(&LiveInputConfig {
            source: format!("127.0.0.1:{}", sink.port()),
            sound,
            proxy,
            ..LiveInputConfig::default()
        })
        .expect("OMT input");
        let reader = std::thread::spawn(move || {
            let start = Instant::now();
            while start.elapsed() < Duration::from_secs(10) {
                if let Some(ev) = input.next(Duration::from_millis(50))
                    && until(ev)
                {
                    return true;
                }
            }
            false
        });
        let mut audio = sink.audio_sender();
        let block = [0.25f32, -0.25].repeat(1600);
        let mut tc = 0i64;
        while !reader.is_finished() {
            let f = Frame {
                timecode: tc,
                ..test_util::frame(w, h, &px, alpha)
            };
            sink.send(&f).expect("send");
            if sound {
                audio.send(&block, 2, 48_000, tc).unwrap();
            }
            tc += 333_333;
            std::thread::sleep(Duration::from_millis(33));
        }
        reader.join().unwrap()
    }

    /// A flat colour comes back as about the same Y'CbCr (VMX is lossy), marked BT.709 like it
    /// was converted even below 720 lines (VMX alone would mark it BT.601), no alpha from an
    /// opaque screen, at the rate it was sent.
    #[test]
    fn opaque_pictures_loop_back() {
        let (w, h) = (320u32, 180u32);
        let px = [180u8, 40, 40, 255].repeat((w * h) as usize);
        let got = loopback(AlphaMode::Opaque, (w, h), px, (false, false), move |ev| {
            let LiveEvent::Picture(p) = ev else {
                return false;
            };
            assert_eq!((p.width, p.height), (w, h));
            assert_eq!(p.frame_rate, (30, 1));
            assert!(!p.alpha && p.a.is_none());
            assert_eq!(p.matrix, LiveMatrix::Bt709);
            assert!(!p.full_range);
            assert_eq!(p.uv.len(), w as usize * (h as usize / 2));
            // The sender converts with BT.709 limited coefficients: Y ≈ 16 + 219 × luma.
            let luma = (0.2126 * 180.0 + 0.7152 * 40.0 + 0.0722 * 40.0) / 255.0;
            let want = 16.0 + 219.0 * luma;
            let y = p.y[(h as usize / 2) * w as usize + w as usize / 2] as f64;
            assert!((y - want).abs() < 6.0, "Y {y}, want about {want}");
            let cr = p.uv[(h as usize / 4) * w as usize + w as usize / 2 + 1] as f64;
            assert!(cr > 160.0, "a red picture has a high Cr ({cr})");
            true
        });
        assert!(got, "no picture came back");
    }

    /// A screen with alpha arrives with a straight alpha plane.
    #[test]
    fn alpha_loops_back() {
        let (w, h) = (320u32, 180u32);
        // Left half transparent, right half opaque white.
        let mut px = Vec::new();
        for _ in 0..h {
            for x in 0..w {
                px.extend_from_slice(if x < w / 2 {
                    &[0, 0, 0, 0]
                } else {
                    &[255, 255, 255, 255]
                });
            }
        }
        let got = loopback(AlphaMode::Straight, (w, h), px, (false, false), move |ev| {
            let LiveEvent::Picture(p) = ev else {
                return false;
            };
            assert!(p.alpha && !p.premultiplied);
            let a = p.a.as_ref().expect("alpha plane");
            assert_eq!(a.len(), (w * h) as usize);
            let row = (h as usize / 2) * w as usize;
            assert!(a[row + 10] < 16, "left is clear ({})", a[row + 10]);
            assert!(a[row + w as usize - 10] > 240, "right is opaque");
            true
        });
        assert!(got, "no picture with alpha came back");
    }

    /// A premultiplied screen goes out marked premultiplied, its colour as rendered: the
    /// receiver keeps it premultiplied (not divided again).
    #[test]
    fn premultiplied_alpha_loops_back() {
        let (w, h) = (320u32, 180u32);
        // White at half alpha, premultiplied: (128, 128, 128, 128).
        let px = [128u8, 128, 128, 128].repeat((w * h) as usize);
        let got = loopback(
            AlphaMode::Premultiplied,
            (w, h),
            px,
            (false, false),
            move |ev| {
                let LiveEvent::Picture(p) = ev else {
                    return false;
                };
                assert!(p.alpha && p.premultiplied, "marked premultiplied");
                let a = p.a.as_ref().expect("alpha plane");
                let mid = (h as usize / 2) * w as usize + w as usize / 2;
                assert!((a[mid] as i32 - 128).abs() < 8, "alpha {}", a[mid]);
                // Premultiplied grey 128 is Y ≈ 16 + 219 × 128/255 ≈ 126; straight white is 235.
                let y = p.y[mid] as i32;
                assert!((y - 126).abs() < 8, "Y {y}: the colour stays premultiplied");
                true
            },
        );
        assert!(got, "no premultiplied picture came back");
    }

    /// A straight screen after a premultiplied one goes out straight again (the flag is
    /// the sender's, so it follows the frames).
    #[test]
    fn straight_alpha_is_marked_straight() {
        let (w, h) = (64u32, 36u32);
        let px = [128u8, 128, 128, 128].repeat((w * h) as usize);
        let mut sink = OmtSink::new(OmtSenderConfig {
            announce: false,
            ..OmtSenderConfig::new(format!("vesper-omt-flag-{}", std::process::id()))
        })
        .unwrap();
        let input = OmtInput::open(&LiveInputConfig {
            source: format!("127.0.0.1:{}", sink.port()),
            ..LiveInputConfig::default()
        })
        .unwrap();
        let start = Instant::now();
        let mut seen = (false, false);
        while start.elapsed() < Duration::from_secs(10) && seen != (true, true) {
            let mode = if seen.0 {
                AlphaMode::Straight
            } else {
                AlphaMode::Premultiplied
            };
            sink.send(&test_util::frame(w, h, &px, mode)).unwrap();
            while let Some(ev) = input.next(Duration::from_millis(30)) {
                if let LiveEvent::Picture(p) = ev {
                    if p.premultiplied {
                        seen.0 = true;
                    } else if seen.0 && p.alpha {
                        seen.1 = true;
                    }
                }
            }
        }
        assert_eq!(seen, (true, true), "premultiplied, then straight");
    }

    /// A thumbnail proxy gets previews: an eighth of each side, packed as usual, in the
    /// whole picture's matrix (BT.709 for 720 lines, though the preview has 90).
    #[test]
    fn proxies_get_previews() {
        let (w, h) = (1280u32, 720u32);
        let px = [40u8, 160, 40, 255].repeat((w * h) as usize);
        let got = loopback(AlphaMode::Opaque, (w, h), px, (false, true), move |ev| {
            let LiveEvent::Picture(p) = ev else {
                return false;
            };
            assert_eq!((p.width, p.height), (w / 8, h / 8), "a preview");
            assert_eq!(p.y.len(), (w / 8 * h / 8) as usize);
            assert_eq!(
                p.matrix,
                LiveMatrix::Bt709,
                "the source's matrix, not the preview's"
            );
            true
        });
        assert!(got, "no preview came back");
    }

    /// Program audio sent on the source arrives as planar float with its rate.
    #[test]
    fn sound_loops_back() {
        let (w, h) = (64u32, 36u32);
        let px = [0u8, 0, 0, 255].repeat((w * h) as usize);
        let got = loopback(AlphaMode::Opaque, (w, h), px, (true, false), |ev| {
            let LiveEvent::Sound(s) = ev else {
                return false;
            };
            assert_eq!(s.sample_rate, 48_000);
            assert_eq!(s.channels.len(), 2);
            assert!(!s.channels[0].is_empty());
            assert!((s.channels[0][0] - 0.25).abs() < 1e-3);
            assert!((s.channels[1][0] + 0.25).abs() < 1e-3);
            true
        });
        assert!(got, "no sound came back");
    }

    /// Without a receiver the sink converts and sends nothing.
    #[test]
    fn nothing_is_sent_without_receivers() {
        let mut sink = OmtSink::new(OmtSenderConfig {
            announce: false,
            ..OmtSenderConfig::new(format!("vesper-omt-idle-{}", std::process::id()))
        })
        .unwrap();
        let px = test_util::pattern(64, 36, 1);
        sink.send(&test_util::frame(64, 36, &px, AlphaMode::Straight))
            .unwrap();
        assert_eq!(sink.frames_sent(), 0);
        assert_eq!(sink.connections(), 0);
    }
}
