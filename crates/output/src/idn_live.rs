//! Live IDN Open Protocol (compatible with NDI®) sources as layer content: the source list
//! with change events ([`SourceWatcher`]) and a receiver for pictures shown on screens
//! ([`LiveInput`]). Plain values only: no library type leaks to the server.
//!
//! Semantics, as the library documents them (`docs/INTEGRATION.md` §3–4):
//! - the browser keeps a live list and reports `Added` / `Changed` / `Removed`; a source that
//!   vanishes without a goodbye drops off after about 20 s;
//! - an input reconnects by itself (every 2 s) and reports `Connecting` / `Connected` /
//!   `Lost`; pictures are delivered as they arrive, newest wins (late ones are skipped);
//! - planes are 8-bit, limited range, in the source's chroma sampling.
//!
//! Pictures for the renderer are packed as NV12 here ([`pack_nv12`]): the luma plane is
//! moved as is, chroma is averaged down to 4:2:0 and interleaved. An alpha plane is kept
//! ([`LivePicture::a`], with the source's convention). Interlaced sources deliver fields
//! (`field_id` 1 and 2): the two fields of a frame are woven with the library's
//! `input::weave_fields` before packing (a pair that doesn't belong together is skipped).
//! With a decoder ([`LiveInputConfig::decoder`]: the system FFmpeg through idn-ffmpeg),
//! H.264 / HEVC sources arrive decoded like any other, and an NDI|HX camera's AAC sound
//! (`InputEvent::CompressedAudio`: AudioSpecificConfig plus one raw frame) is decoded here
//! with the same adapter's AAC decoder ([`AacSound`]) into ordinary [`LiveSound`] blocks.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use idn_open_protocol::codec::{AudioDecoder, CodecInput, CodecUse, DecoderFactory};
use idn_open_protocol::color::{AlphaMode, Matrix};
use idn_open_protocol::discovery::{Browser, BrowserConfig, SourceEvent};
use idn_open_protocol::input::{ConnectionState, Input, InputConfig, InputEvent, weave_fields};
use idn_open_protocol::shq::Picture;

/// A source on the network, as listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedSource {
    /// `"HOST (name)"`: what [`LiveInput::open`] takes.
    pub name: String,
    /// `ip:port`, once known.
    pub address: Option<String>,
    /// Groups it announced.
    pub groups: Vec<String>,
    /// It announced itself as an IDN device (otherwise another compatible sender).
    pub idn: bool,
}

/// A change of the source list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceChange {
    /// Appeared or changed (address, groups).
    Up(ListedSource),
    /// Went away.
    Gone(String),
}

fn listed(s: idn_open_protocol::discovery::Source) -> ListedSource {
    ListedSource {
        idn: s.is_idn(),
        address: s.socket_addr().map(|a| a.to_string()),
        groups: s.groups.clone(),
        name: s.name,
    }
}

/// Keeps the list of sources on the network (mDNS, or a site's discovery server).
pub struct SourceWatcher {
    browser: Browser,
}

impl SourceWatcher {
    /// Starts browsing: with mDNS on every interface, or with the discovery server at
    /// `discovery_server` (`host` or `host:port`) instead.
    pub fn start(discovery_server: Option<String>) -> Result<Self> {
        let browser = Browser::start(BrowserConfig {
            discovery_server,
            ..Default::default()
        })
        .context("IDN Open Protocol: cannot start the source browser")?;
        Ok(SourceWatcher { browser })
    }

    /// The sources known now.
    pub fn sources(&self) -> Vec<ListedSource> {
        self.browser.sources().into_iter().map(listed).collect()
    }

    /// The next change, waiting up to `timeout` (`None`: nothing changed).
    pub fn next(&self, timeout: Duration) -> Option<SourceChange> {
        Some(match self.browser.recv(timeout)? {
            SourceEvent::Added(s) | SourceEvent::Changed(s) => SourceChange::Up(listed(s)),
            SourceEvent::Removed(s) => SourceChange::Gone(s.name),
        })
    }
}

/// Y'CbCr → RGB matrix of a received picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LiveMatrix {
    /// BT.601.
    Bt601,
    /// BT.709 (the default).
    Bt709,
    /// BT.2020.
    Bt2020,
}

/// A received picture packed as NV12 (8-bit, Y plane then interleaved CbCr at half size).
#[derive(Clone, Debug)]
pub struct LivePicture {
    /// Width, px.
    pub width: u32,
    /// Height, px (a field's height for interlaced sources).
    pub height: u32,
    /// Frame rate (numerator, denominator) the source states.
    pub frame_rate: (u32, u32),
    /// The source's timecode, 100 ns units.
    pub timecode: i64,
    /// 0 progressive, 1 / 2 a field.
    pub field: u32,
    /// Two fields woven into this frame (an interlaced source).
    pub interlaced: bool,
    /// The source has an alpha plane (in [`a`](Self::a)).
    pub alpha: bool,
    /// The alpha plane, `width` bytes a row (sources with alpha).
    pub a: Option<Vec<u8>>,
    /// The colour is premultiplied by alpha on the wire (else straight, NDI's convention).
    pub premultiplied: bool,
    /// Colour matrix.
    pub matrix: LiveMatrix,
    /// Full range (otherwise limited).
    pub full_range: bool,
    /// Y plane, `width` bytes a row.
    pub y: Vec<u8>,
    /// CbCr pairs, `ceil(width / 2) * 2` bytes a row, `ceil(height / 2)` rows.
    pub uv: Vec<u8>,
}

/// What a [`LiveInput`] reports.
#[derive(Clone, Debug)]
pub enum LiveEvent {
    /// Looking up the source or connecting.
    Connecting,
    /// Connected (to this address).
    Connected(String),
    /// Unreachable or gone, with the reason (it keeps trying).
    Lost(String),
    /// A picture.
    Picture(Box<LivePicture>),
    /// Compressed video (H.264 / HEVC) that this receiver does not decode (no decoder, or
    /// none for its codec).
    Compressed,
    /// A block of sound (only with [`LiveInputConfig::sound`]).
    Sound(Box<LiveSound>),
}

/// A block of a source's sound, as received (the library delivers sound and pictures
/// separately; pair them by timecode).
#[derive(Clone, Debug)]
pub struct LiveSound {
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// The source's timecode of the first sample, 100 ns units (a sender gives the block
    /// attached to a picture that picture's timecode).
    pub timecode: i64,
    /// Planar samples, one `Vec` per channel, nominally −1 … 1.
    pub channels: Vec<Vec<f32>>,
}

/// A decoder of compressed network video (H.264 / HEVC / AV1): the media crate's FFmpeg
/// adapter (idn-ffmpeg) for [`LiveInputConfig::decoder`].
#[derive(Clone)]
pub struct LiveDecoder(pub Arc<dyn DecoderFactory>);

impl std::fmt::Debug for LiveDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("LiveDecoder")
            .field(&self.0.codecs())
            .finish()
    }
}

/// Settings of a [`LiveInput`].
#[derive(Clone, Debug, Default)]
pub struct LiveInputConfig {
    /// `"HOST (name)"` or `ip:port`.
    pub source: String,
    /// Ask for pictures no larger than this (`None`: full size).
    pub max_size: Option<(u32, u32)>,
    /// A thumbnail proxy: the low-bandwidth stream, one picture in `picture_every`.
    pub proxy: bool,
    /// For a proxy: one picture in this many.
    pub picture_every: Option<u32>,
    /// Decode H.264 / HEVC sources (NDI|HX cameras; an IDN source above 1080p with a GPU
    /// encoder) with this; `None`: they report [`LiveEvent::Compressed`].
    pub decoder: Option<LiveDecoder>,
    /// Report the source's sound ([`LiveEvent::Sound`]); otherwise it is skipped.
    pub sound: bool,
}

/// Receives one source for showing on screens (opened on the caller's thread; read it from
/// a thread of its own).
pub struct LiveInput {
    input: Input,
    /// Interlaced sources: the first field of a frame, waiting for the second.
    fields: Mutex<Fields<Picture>>,
    sound: bool,
    /// NDI|HX AAC sound, decoded with [`LiveInputConfig::decoder`] (`None`: no decoder).
    aac: Option<Mutex<AacSound>>,
}

/// Decodes a source's AAC frames (NDI|HX cameras) into [`LiveSound`] blocks with a
/// [`LiveDecoder`]'s AAC decoder, opened on the first frame and again when the
/// AudioSpecificConfig changes. A frame that fails to decode is skipped.
pub struct AacSound {
    factory: Arc<dyn DecoderFactory>,
    open: Option<(Vec<u8>, Box<dyn AudioDecoder>)>,
    /// The decoder could not be opened for this config (not tried again until it changes).
    failed: Option<Vec<u8>>,
    warned: bool,
}

impl AacSound {
    /// A decoder of AAC frames with `decoder`'s AAC support.
    pub fn new(decoder: &LiveDecoder) -> Self {
        AacSound {
            factory: decoder.0.clone(),
            open: None,
            failed: None,
            warned: false,
        }
    }

    /// Decodes one raw AAC frame (`config`: its AudioSpecificConfig) whose first sample has
    /// the source timecode `timecode` (100 ns units). `None`: nothing to hear yet (the
    /// decoder holds the frame back), or the frame could not be decoded.
    pub fn decode(
        &mut self,
        config: &[u8],
        frame: &[u8],
        sample_rate: u32,
        timecode: i64,
    ) -> Option<LiveSound> {
        if self.open.as_ref().is_none_or(|(c, _)| c != config) {
            self.open = None;
            if self.failed.as_deref() == Some(config) {
                return None;
            }
            match self.factory.open_aac(config) {
                Ok(d) => {
                    self.failed = None;
                    self.open = Some((config.to_vec(), d));
                }
                Err(e) => {
                    log::warn!("IDN input: cannot decode its AAC sound: {e}");
                    self.failed = Some(config.to_vec());
                    return None;
                }
            }
        }
        let (_, d) = self.open.as_mut()?;
        match d.decode(frame) {
            Ok(channels) if channels.first().is_some_and(|c| !c.is_empty()) => Some(LiveSound {
                sample_rate,
                timecode,
                channels,
            }),
            Ok(_) => None,
            Err(e) => {
                if !self.warned {
                    self.warned = true;
                    log::warn!("IDN input: an AAC frame failed to decode (skipped): {e}");
                }
                None
            }
        }
    }
}

impl LiveInput {
    /// Opens the input; connecting happens in the background.
    pub fn open(cfg: &LiveInputConfig) -> Result<Self> {
        let mut c = InputConfig::new(&cfg.source);
        c.receiver_name = "Vesper".into();
        c.max_size = cfg.max_size;
        if cfg.proxy {
            c.low_bandwidth = true;
            c.picture_every = cfg.picture_every;
        }
        // Compressed video only where SpeedHQ isn't offered (HX cameras) or above 1080p: on
        // a LAN SpeedHQ is cheaper to decode.
        c.codec = cfg
            .decoder
            .as_ref()
            .map(|d| CodecInput::new(d.0.clone(), CodecUse::Allow));
        let input = Input::open(c).with_context(|| format!("IDN input for {:?}", cfg.source))?;
        Ok(LiveInput {
            input,
            fields: Mutex::new(Fields::default()),
            sound: cfg.sound,
            aac: cfg
                .decoder
                .as_ref()
                .filter(|_| cfg.sound)
                .map(|d| Mutex::new(AacSound::new(d))),
        })
    }

    /// Asks for pictures no larger than `size` from now on.
    pub fn set_max_size(&self, size: Option<(u32, u32)>) {
        self.input.set_max_size(size);
    }

    /// The next event, waiting up to `timeout` in all (`None`: nothing to report arrived).
    /// Metadata, diagnostics, sound unless asked for (AAC sound without a decoder, or that
    /// doesn't decode), and pictures that can't be packed are skipped; skipped
    /// events don't extend the wait (an audio-only source sends sound all the time, and the
    /// caller must get control back to see that it should stop).
    pub fn next(&self, timeout: Duration) -> Option<LiveEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let ev = self.input.recv(left)?;
            return Some(match ev {
                InputEvent::State(ConnectionState::Connecting) => LiveEvent::Connecting,
                InputEvent::State(ConnectionState::Connected(a)) => {
                    LiveEvent::Connected(a.to_string())
                }
                InputEvent::State(ConnectionState::Routed { addr, .. }) => {
                    LiveEvent::Connected(addr.to_string())
                }
                InputEvent::State(ConnectionState::Lost(why)) => LiveEvent::Lost(why),
                InputEvent::Video(mut v) => {
                    let pushed = self.fields.lock().unwrap_or_else(|e| e.into_inner()).push(
                        v.header.field_id,
                        v.picture,
                        weave_fields,
                    );
                    let Some((picture, interlaced)) = pushed else {
                        continue;
                    };
                    v.picture = picture;
                    let a = alpha_plane(&mut v.picture);
                    let Some((y, uv)) = pack_nv12(&mut v.picture) else {
                        continue;
                    };
                    LiveEvent::Picture(Box::new(LivePicture {
                        width: v.picture.width as u32,
                        height: v.picture.height as u32,
                        frame_rate: (v.header.frame_rate_n, v.header.frame_rate_d),
                        timecode: v.header.timecode,
                        field: v.header.field_id,
                        interlaced,
                        alpha: a.is_some(),
                        a,
                        premultiplied: v.rgb.wire_alpha == AlphaMode::Premultiplied,
                        matrix: match v.rgb.matrix {
                            Matrix::Bt601 => LiveMatrix::Bt601,
                            Matrix::Bt709 => LiveMatrix::Bt709,
                            Matrix::Bt2020 => LiveMatrix::Bt2020,
                        },
                        full_range: !v.rgb.limited,
                        y,
                        uv,
                    }))
                }
                InputEvent::Compressed(_) => LiveEvent::Compressed,
                InputEvent::Audio(a) if self.sound => LiveEvent::Sound(Box::new(LiveSound {
                    sample_rate: a.header.sample_rate,
                    timecode: a.header.timecode,
                    channels: a.channels,
                })),
                InputEvent::CompressedAudio(a) if self.sound => {
                    let Some(aac) = &self.aac else {
                        continue;
                    };
                    let decoded = aac.lock().unwrap_or_else(|e| e.into_inner()).decode(
                        &a.config,
                        &a.data,
                        a.header.sample_rate,
                        a.header.timecode,
                    );
                    match decoded {
                        Some(s) => LiveEvent::Sound(Box::new(s)),
                        None => continue,
                    }
                }
                _ => continue,
            });
        }
    }
}

/// Pairs the fields of an interlaced source into frames (see [`Fields::push`]).
#[derive(Debug)]
struct Fields<T> {
    first: Option<T>,
}

impl<T> Default for Fields<T> {
    fn default() -> Self {
        Fields { first: None }
    }
}

impl<T> Fields<T> {
    /// Takes a picture with its `field_id`: a whole frame (0) comes back as it is; a first
    /// field (1) is kept; a second field (2) comes back woven with the kept first field
    /// (`weave`), as `(frame, true)`. `None`: nothing to show yet, or a second field without
    /// its first, or two fields that don't belong together (`weave` gives `None`: the
    /// library's rule is to skip that frame).
    fn push(
        &mut self,
        field_id: u32,
        p: T,
        weave: impl FnOnce(&T, &T) -> Option<T>,
    ) -> Option<(T, bool)> {
        match field_id {
            0 => {
                self.first = None;
                Some((p, false))
            }
            1 => {
                self.first = Some(p);
                None
            }
            _ => {
                let first = self.first.take()?;
                weave(&first, &p).map(|f| (f, true))
            }
        }
    }
}

/// The alpha plane of a picture (its fourth plane), `width` bytes a row; `None` without one
/// or when its size doesn't match the picture.
fn alpha_plane(p: &mut Picture) -> Option<Vec<u8>> {
    let a = p.planes.get_mut(3)?;
    if a.width != p.width || a.height != p.height || a.data.len() < p.width * p.height {
        return None;
    }
    let mut d = std::mem::take(&mut a.data);
    d.truncate(p.width * p.height);
    Some(d)
}

/// Packs a picture's planes as NV12: the Y plane is taken as is, Cb and Cr are averaged
/// over the block each 4:2:0 sample covers and interleaved. `None` for pictures without
/// three planes of consistent sizes (e.g. coefficients only).
pub fn pack_nv12(p: &mut Picture) -> Option<(Vec<u8>, Vec<u8>)> {
    if p.planes.len() < 3 || p.planes[0].width != p.width || p.planes[0].height != p.height {
        return None;
    }
    let (cb, cr) = (&p.planes[1], &p.planes[2]);
    if cb.width != cr.width || cb.height != cr.height {
        return None;
    }
    let uv = pack_chroma(p.width, p.height, &cb.data, &cr.data, cb.width, cb.height)?;
    let mut y = std::mem::take(&mut p.planes[0].data);
    if y.len() < p.width * p.height {
        return None;
    }
    y.truncate(p.width * p.height);
    Some((y, uv))
}

/// Interleaved 4:2:0 CbCr for a `w` × `h` picture from Cb and Cr planes of `sw` × `sh`
/// samples (4:2:0, 4:2:2 or 4:4:4), each output sample the average of the block it covers.
pub(crate) fn pack_chroma(
    w: usize,
    h: usize,
    cb: &[u8],
    cr: &[u8],
    sw: usize,
    sh: usize,
) -> Option<Vec<u8>> {
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    if w == 0 || h == 0 || sw < cw || sh < ch || cb.len() < sw * sh || cr.len() < sw * sh {
        return None;
    }
    // Source samples per output sample: 1 or 2 each way.
    let rx = if sw > cw { 2 } else { 1 };
    let ry = if sh > ch { 2 } else { 1 };
    let mut uv = vec![0u8; cw * 2 * ch];
    for oy in 0..ch {
        let y0 = (oy * ry).min(sh - 1);
        let y1 = (y0 + ry - 1).min(sh - 1);
        let row = &mut uv[oy * cw * 2..(oy + 1) * cw * 2];
        for ox in 0..cw {
            let x0 = (ox * rx).min(sw - 1);
            let x1 = (x0 + rx - 1).min(sw - 1);
            let avg = |d: &[u8]| -> u8 {
                let s = d[y0 * sw + x0] as u32
                    + d[y0 * sw + x1] as u32
                    + d[y1 * sw + x0] as u32
                    + d[y1 * sw + x1] as u32;
                s.div_ceil(4).min(255) as u8
            };
            row[ox * 2] = avg(cb);
            row[ox * 2 + 1] = avg(cr);
        }
    }
    Some(uv)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D02-3: an audio-only source sends sound all the time; `next` still comes back within
    /// its timeout (the receive loop must see its stop flag).
    #[test]
    fn next_returns_on_an_audio_only_source() {
        use crate::{AudioSink, IdnSenderConfig, IdnSink};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let name = format!("audio-only-{}", std::process::id());
        let sink = IdnSink::new(IdnSenderConfig::new(name)).expect("IDN source");
        let source = format!("127.0.0.1:{}", sink.port());
        let stop = Arc::new(AtomicBool::new(false));
        let mut audio = sink.audio_sender();
        let sender = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                // 5 ms blocks of a quiet 48 kHz stereo tone, every 5 ms.
                let block = vec![0.01f32; 240 * 2];
                let mut tc = 0i64;
                while !stop.load(Ordering::Relaxed) {
                    let _ = audio.send(&block, 2, 48_000, tc);
                    tc += 50_000;
                    std::thread::sleep(Duration::from_millis(5));
                }
                drop(sink);
            })
        };
        let input = LiveInput::open(&LiveInputConfig {
            source,
            ..LiveInputConfig::default()
        })
        .expect("input");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let start = Instant::now();
            let mut longest = Duration::ZERO;
            let mut connected = false;
            while start.elapsed() < Duration::from_secs(3) {
                let t = Instant::now();
                let ev = input.next(Duration::from_millis(100));
                longest = longest.max(t.elapsed());
                connected |= matches!(ev, Some(LiveEvent::Connected(_)));
            }
            let _ = tx.send((longest, connected));
        });
        let r = rx.recv_timeout(Duration::from_secs(10));
        stop.store(true, Ordering::Relaxed);
        sender.join().unwrap();
        let (longest, connected) = r.expect("next() never came back while sound kept arriving");
        assert!(connected, "never connected to the audio-only source");
        assert!(
            longest < Duration::from_millis(500),
            "next() took {longest:?} with a 100 ms timeout"
        );
    }

    /// Interlaced sources: field 1 waits for field 2, the pair is woven; a lone field 2 or a
    /// pair that doesn't weave is skipped; a progressive frame passes and drops a waiting
    /// field.
    #[test]
    fn pairs_fields_into_frames() {
        let weave = |a: &Vec<u8>, b: &Vec<u8>| {
            (a.len() == b.len()).then(|| a.iter().zip(b).flat_map(|(x, y)| [*x, *y]).collect())
        };
        let mut f = Fields::default();
        assert_eq!(f.push(0, vec![9], weave), Some((vec![9], false)));
        assert_eq!(f.push(2, vec![2], weave), None, "a second field alone");
        assert_eq!(f.push(1, vec![1, 3], weave), None);
        assert_eq!(f.push(2, vec![2, 4], weave), Some((vec![1, 2, 3, 4], true)));
        assert_eq!(f.push(2, vec![2, 4], weave), None, "its first was used");
        assert_eq!(f.push(1, vec![1], weave), None);
        assert_eq!(
            f.push(2, vec![2, 4], weave),
            None,
            "fields that don't belong together"
        );
        assert_eq!(f.push(1, vec![1], weave), None);
        assert_eq!(f.push(0, vec![7], weave), Some((vec![7], false)));
        assert_eq!(
            f.push(2, vec![2], weave),
            None,
            "the progressive frame dropped field 1"
        );
    }

    /// With `sound`, a source's sound arrives as blocks with its rate, channels and the
    /// timecodes the sender gave them.
    #[test]
    fn sound_arrives_with_its_timecodes() {
        use crate::{AudioSink, IdnSenderConfig, IdnSink};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let name = format!("sound-{}", std::process::id());
        let sink = IdnSink::new(IdnSenderConfig::new(name)).expect("IDN source");
        let source = format!("127.0.0.1:{}", sink.port());
        let stop = Arc::new(AtomicBool::new(false));
        let mut audio = sink.audio_sender();
        let sender = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                // 10 ms blocks of 48 kHz stereo: left 0.25, right −0.25.
                let block: Vec<f32> = (0..480).flat_map(|_| [0.25f32, -0.25]).collect();
                let mut tc = 1_000_000_000i64;
                while !stop.load(Ordering::Relaxed) {
                    let _ = audio.send(&block, 2, 48_000, tc);
                    tc += 100_000;
                    std::thread::sleep(Duration::from_millis(10));
                }
                drop(sink);
            })
        };
        let input = LiveInput::open(&LiveInputConfig {
            source,
            sound: true,
            ..LiveInputConfig::default()
        })
        .expect("input");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut blocks = Vec::new();
        while blocks.len() < 5 && Instant::now() < deadline {
            if let Some(LiveEvent::Sound(s)) = input.next(Duration::from_millis(100)) {
                blocks.push(s);
            }
        }
        stop.store(true, Ordering::Relaxed);
        sender.join().unwrap();
        assert!(blocks.len() >= 5, "{} sound blocks", blocks.len());
        for b in &blocks {
            assert_eq!(b.sample_rate, 48_000);
            assert_eq!(b.channels.len(), 2);
            assert!(b.channels[0].iter().all(|v| (v - 0.25).abs() < 1e-3));
            assert!(b.channels[1].iter().all(|v| (v + 0.25).abs() < 1e-3));
            assert_eq!((b.timecode - 1_000_000_000) % 100_000, 0, "{}", b.timecode);
        }
        for w in blocks.windows(2) {
            assert!(w[1].timecode > w[0].timecode);
        }
    }

    fn plane(w: usize, h: usize, f: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| f(x, y))
            .collect()
    }

    #[test]
    fn packs_422_420_and_444_chroma() {
        // 4:2:2 (4 × 4 chroma for 8 × 4): Cb follows x; Cr averages row pairs.
        let cb = plane(4, 4, |x, _| (100 + x) as u8);
        let cr = plane(4, 4, |_, y| (50 + 2 * y) as u8);
        let uv = pack_chroma(8, 4, &cb, &cr, 4, 4).unwrap();
        assert_eq!(uv.len(), 4 * 2 * 2);
        assert_eq!(&uv[0..4], &[100, 51, 101, 51]);
        assert_eq!(uv[8 + 1], 55);
        // 4:2:0 passes through.
        let cb = plane(4, 2, |x, y| (10 * y + x) as u8);
        let uv = pack_chroma(8, 4, &cb, &cb, 4, 2).unwrap();
        assert_eq!(uv[2], 1);
        assert_eq!(uv[8 + 2 * 3], 13);
        // 4:4:4 averages 2 × 2 blocks; odd sizes round up.
        let cb = plane(5, 3, |x, _| (x * 4) as u8);
        let uv = pack_chroma(5, 3, &cb, &cb, 5, 3).unwrap();
        assert_eq!(uv.len(), 3 * 2 * 2);
        assert_eq!(uv[0], 2);
        assert_eq!(uv[4], 16, "the last column stands alone");
        // Too small planes are refused.
        assert!(pack_chroma(8, 4, &cb, &cb, 2, 2).is_none());
    }
}
