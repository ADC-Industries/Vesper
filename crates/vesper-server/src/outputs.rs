//! Output routing: one worker thread per sink, fed with the latest frame of its screen.
//!
//! The screen's clock thread hands each output an `Arc` of the frame through a bounded channel
//! of one slot and never waits: if the worker is still busy with the previous frame (IDN
//! hand-off, scaling, PNG copy, ffmpeg pipe), the new frame is dropped and counted. Each
//! worker owns its sink, so conversion cost is spread over threads.
//!
//! Outputs come from the machine's [`model::OutputBinding`]s (IDN Open Protocol →
//! [`output::IdnSink`]; OMT → [`output::OmtSink`]; display → [`crate::displays`]; SDI → "not
//! supported yet") plus the CLI's development sinks. Each
//! one is an [`OutputRuntime`]; the engine adds, replaces and removes them while everything
//! else keeps running ([`crate::engine::apply_config`]).
//!
//! ## Delivery settings of a binding
//! - **Mode override:** `resolutionOverride` scales the frame on the output's worker thread
//!   (bilinear, premultiplied); `frameRateOverride` gives the output its own clock thread that
//!   samples the screen's latest frame at that rate (frames are repeated or skipped).
//!   Without them the output follows the screen.
//! - **Alpha override:** `opaque` drops the alpha channel (the frame is premultiplied, so
//!   that is "over black"); `straight` / `premultiplied` send alpha.
//! - **Region / tile:** an output shows its `region` of the screen, else its `tile` of a
//!   Grouped / Edge Blend screen ([`model::ScreenArrangement`]): the worker crops it, scales
//!   it to the output's size (its resolution override, else the crop's size) and, in an Edge
//!   Blend screen, multiplies the tile's inner edges by the blend ramps ([`apply_blend`],
//!   gamma-aware), before Fill/Key. A placeholder output (no hardware yet) never runs.
//! - **Target:** `fill` / `key` make the output one half of a Fill/Key pair for a hardware
//!   keyer: after scaling, the worker turns the frame into the fill (the colour over black:
//!   straight, or premultiplied when the output's alpha type is `premultiplied`) or the key
//!   (alpha as grey), both opaque (`docs/DISPLAY-OUTPUTS-PLAN.md`, "Fill/Key on two
//!   connectors"). Display outputs of one screen's fill and key are flipped together on KMS
//!   ([`crate::displays`]).
//! - **Embedded audio:** an IDN or OMT output whose binding has `embedAudio` sends the program
//!   audio from a thread of its own ([`crate::sound::spawn_embedded_audio`]), delayed like its
//!   video.
//! - **Video delay:** a [`DelayLine`] of `videoDelay + 1` frame references per output. Frames
//!   are shared `Arc`s, so the memory cost is one frame per *distinct* frame in the line: at
//!   most `(delay + 1) × width × height × 4` bytes, e.g. 31 × 8.3 MB ≈ 257 MB for a 30-frame
//!   delay of 1080p content that changes every frame (a moving test pattern or video), and
//!   only a few frames for slides, which change rarely.

use crate::data::ShowData;
use model::{
    AlphaMode, BlendCurve, FrameRate, OutputAlpha, OutputBinding, OutputId, OutputTarget,
    OutputTransport, Screen,
};
use output::{FfmpegSink, FrameSink, IdnSenderConfig, IdnSink, PngSink};
use render::RenderedFrame;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// Development sinks requested on the command line.
#[derive(Clone, Debug, Default)]
pub struct DevOutputs {
    /// Write every `png_every`th frame of every screen to `<dir>/<screen>/`.
    pub png_dir: Option<PathBuf>,
    /// N for `png_dir`.
    pub png_every: u64,
    /// Open an ffplay preview window for this screen (name or ID).
    pub preview: Option<String>,
    /// Send every screen as an IDN Open Protocol source `"<prefix> <screen name>"`.
    pub idn_prefix: Option<String>,
    /// Ignore the machine's output bindings.
    pub no_bindings: bool,
}

/// Longest video delay, frames.
pub const MAX_VIDEO_DELAY: u32 = 30;

/// Live counters of one output.
#[derive(Debug, Default)]
pub struct SinkStats {
    /// Frames delivered to the sink.
    pub sent: AtomicU64,
    /// Frames dropped because the worker was busy.
    pub dropped: AtomicU64,
    /// `send` errors.
    pub errors: AtomicU64,
    /// Last error text.
    pub last_error: Mutex<Option<String>>,
    /// The most recent send failed.
    pub failing: AtomicBool,
}

/// A fixed-length frame delay: [`DelayLine::push`] returns the item pushed `delay` pushes
/// ago (the oldest one while the line is still filling).
#[derive(Debug)]
pub struct DelayLine<T> {
    items: VecDeque<T>,
}

impl<T: Clone> Default for DelayLine<T> {
    fn default() -> Self {
        DelayLine {
            items: VecDeque::new(),
        }
    }
}

impl<T: Clone> DelayLine<T> {
    /// Adds the newest item and returns the one to deliver now.
    pub fn push(&mut self, item: T, delay: u32) -> T {
        self.items.push_back(item);
        while self.items.len() > delay as usize + 1 {
            self.items.pop_front();
        }
        self.items.front().cloned().expect("just pushed")
    }

    /// Items held.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Nothing held.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// How an output delivers frames (from its binding's overrides).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Delivery {
    /// Alpha override (`None` = the screen's).
    pub alpha: Option<AlphaMode>,
    /// Scale to this size (`None` = the screen's size).
    pub size: Option<(u32, u32)>,
    /// Own clock at this rate (`None` = the screen's clock).
    pub rate: Option<FrameRate>,
    /// The whole picture, or the fill or key of a Fill/Key pair.
    pub target: OutputTarget,
    /// The part of the screen sent (left, top, width, height in canvas px; `None` = all of
    /// it): the output's region, else its tile of a Grouped / Edge Blend screen.
    pub crop: Option<(u32, u32, u32, u32)>,
    /// Edge Blend: the ramp widths of the crop's left, right, top and bottom edges (canvas
    /// px) and the ramp's shape.
    pub blend: Option<([u32; 4], BlendCurve)>,
}

impl Delivery {
    /// The delivery settings of a binding on its screen: [`Delivery::of`] plus the crop and
    /// blend of its region or tile.
    pub fn for_screen(b: &OutputBinding, screen: &Screen) -> Delivery {
        let mut d = Delivery::of(b);
        let (cw, ch) = (
            screen.size.width.round().max(1.0) as u32,
            screen.size.height.round().max(1.0) as u32,
        );
        let tile = b
            .tile
            .and_then(|t| screen.arrangement.tile_layout(screen.size, t));
        let crop = match b.region {
            Some(r) => {
                let x0 = r.x.round().clamp(0.0, cw as f64) as u32;
                let y0 = r.y.round().clamp(0.0, ch as f64) as u32;
                let x1 = (r.x + r.width).round().clamp(x0 as f64, cw as f64) as u32;
                let y1 = (r.y + r.height).round().clamp(y0 as f64, ch as f64) as u32;
                Some((x0, y0, x1 - x0, y1 - y0))
            }
            None => tile.map(|t| t.crop),
        };
        d.crop = crop.filter(|&(x, y, w, h)| (x, y, w, h) != (0, 0, cw, ch) && w > 0 && h > 0);
        d.blend = tile
            .filter(|t| t.blend != [0; 4])
            .map(|t| (t.blend, screen.arrangement.blend));
        d
    }

    /// Why a binding can't run on its screen's arrangement (a tile that doesn't exist).
    pub fn problem(b: &OutputBinding, screen: &Screen) -> Option<String> {
        let a = &screen.arrangement;
        let t = b.tile?;
        if !a.is_tiled() || b.region.is_some() || b.transport == OutputTransport::Placeholder {
            return None;
        }
        if t >= a.cells() {
            return Some(format!(
                "tile {} is outside the {}×{} arrangement",
                t + 1,
                a.columns.max(1),
                a.rows.max(1)
            ));
        }
        a.tile_layout(screen.size, t)
            .is_none()
            .then(|| "the arrangement leaves no room for its tiles".to_string())
    }

    /// The delivery settings of a binding (without its screen: no crop or blend).
    pub fn of(b: &OutputBinding) -> Delivery {
        Delivery {
            alpha: match &b.alpha_override {
                Some(OutputAlpha::Opaque) => Some(AlphaMode::Opaque),
                Some(OutputAlpha::Straight) | Some(OutputAlpha::KeyFill { .. }) => {
                    Some(AlphaMode::Straight)
                }
                Some(OutputAlpha::Premultiplied) => Some(AlphaMode::Premultiplied),
                _ => None,
            },
            size: b.resolution_override.map(|r| (r.width, r.height)),
            rate: b.frame_rate_override,
            target: b.target,
            crop: None,
            blend: None,
        }
    }

    /// The size frames go out at for a screen of `screen` px: the resolution override, else
    /// the crop's size, else the screen's.
    pub fn out_size(&self, screen: (u32, u32)) -> (u32, u32) {
        self.size
            .or(self.crop.map(|c| (c.2, c.3)))
            .unwrap_or(screen)
    }
}

/// Copies the `(x, y, w, h)` part of a `sw` × `sh` RGBA8 frame (clamped to the frame).
pub fn crop_rgba(
    src: &[u8],
    sw: u32,
    sh: u32,
    (x, y, w, h): (u32, u32, u32, u32),
) -> (u32, u32, Vec<u8>) {
    let x = x.min(sw.saturating_sub(1));
    let y = y.min(sh.saturating_sub(1));
    let w = w.min(sw - x).max(1);
    let h = h.min(sh - y).max(1);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let a = ((row * sw + x) * 4) as usize;
        out.extend_from_slice(&src[a..a + (w * 4) as usize]);
    }
    (w, h, out)
}

/// Multiplies the edges of an RGBA8 frame by Edge Blend ramps, in place. `edges` are the
/// ramp widths of the left, right, top and bottom edges in this frame's pixels; at `t`
/// (0 at the edge, 1 at the ramp's inner end, sampled at pixel centres) the colour is
/// multiplied by [`BlendCurve::factor`] (gamma-aware); where two ramps meet (a corner) the
/// factors multiply. Alpha is untouched.
pub fn apply_blend(px: &mut [u8], w: u32, h: u32, edges: [f64; 4], curve: &BlendCurve) {
    // Factors in 16.16 fixed point (65536 = 1).
    let table = |n: u32, lo: f64, hi: f64| -> Vec<u32> {
        (0..n)
            .map(|i| {
                let c = i as f64 + 0.5;
                let mut f = 1.0;
                if lo > 0.0 && c < lo {
                    f *= curve.factor(c / lo);
                }
                let d = n as f64 - c;
                if hi > 0.0 && d < hi {
                    f *= curve.factor(d / hi);
                }
                (f * 65536.0).round().clamp(0.0, 65536.0) as u32
            })
            .collect()
    };
    let fx = table(w, edges[0], edges[1]);
    let fy = table(h, edges[2], edges[3]);
    let ramp_cols: Vec<usize> = (0..w as usize).filter(|&x| fx[x] < 65536).collect();
    for y in 0..h as usize {
        let row = &mut px[y * w as usize * 4..(y + 1) * w as usize * 4];
        let mul = |p: &mut [u8], f: u32| {
            for c in &mut p[..3] {
                *c = ((*c as u32 * f + 32768) >> 16) as u8;
            }
        };
        if fy[y] < 65536 {
            for x in 0..w as usize {
                let f = ((fx[x] as u64 * fy[y] as u64) >> 16) as u32;
                mul(&mut row[x * 4..x * 4 + 4], f);
            }
        } else {
            for &x in &ramp_cols {
                mul(&mut row[x * 4..x * 4 + 4], fx[x]);
            }
        }
    }
}

/// The fill or key of a premultiplied RGBA8 frame (`dst` is resized): see
/// [`OutputTarget`]. `alpha` is the output's alpha type: `premultiplied` gives a shaped fill
/// (premultiplied colour over black), anything else an unshaped one (straight colour).
pub fn fill_key(target: OutputTarget, alpha: AlphaMode, src: &[u8], dst: &mut Vec<u8>) {
    dst.resize(src.len(), 0);
    match (target, alpha) {
        (OutputTarget::Key, _) => output::convert::premul_rgba_to_key(src, dst),
        (_, AlphaMode::Premultiplied) => output::convert::premul_rgba_to_rgbx(src, dst),
        _ => output::convert::premul_rgba_to_straight_rgbx(src, dst),
    }
}

struct Job {
    frame: Arc<RenderedFrame>,
    timecode: i64,
    frame_rate: FrameRate,
}

/// A sink running on its own thread.
pub struct SinkWorker {
    tx: Option<SyncSender<Job>>,
    handle: Option<JoinHandle<()>>,
    stats: Arc<SinkStats>,
}

/// Bilinear scale of a premultiplied RGBA8 frame (premultiplied colour interpolates
/// correctly). Integer arithmetic: a few ms for 1080p → 720p.
pub fn scale_rgba(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (
        sw as usize,
        sh as usize,
        dw.max(1) as usize,
        dh.max(1) as usize,
    );
    let mut out = vec![0u8; dw * dh * 4];
    if sw == 0 || sh == 0 {
        return out;
    }
    // 16.16 fixed point source coordinates of each destination column, centre-aligned.
    let map = |d: usize, sn: usize, dn: usize| -> (usize, usize, u32) {
        let f = ((d as f64 + 0.5) * sn as f64 / dn as f64 - 0.5).max(0.0);
        let i = (f.floor() as usize).min(sn - 1);
        let j = (i + 1).min(sn - 1);
        (i, j, ((f - i as f64) * 256.0) as u32)
    };
    let cols: Vec<(usize, usize, u32)> = (0..dw).map(|x| map(x, sw, dw)).collect();
    for y in 0..dh {
        let (y0, y1, fy) = map(y, sh, dh);
        let r0 = &src[y0 * sw * 4..(y0 + 1) * sw * 4];
        let r1 = &src[y1 * sw * 4..(y1 + 1) * sw * 4];
        let o = &mut out[y * dw * 4..(y + 1) * dw * 4];
        for (x, &(x0, x1, fx)) in cols.iter().enumerate() {
            for c in 0..4 {
                let a = r0[x0 * 4 + c] as u32;
                let b = r0[x1 * 4 + c] as u32;
                let cc = r1[x0 * 4 + c] as u32;
                let d = r1[x1 * 4 + c] as u32;
                let top = a * (256 - fx) + b * fx;
                let bot = cc * (256 - fx) + d * fx;
                o[x * 4 + c] = ((top * (256 - fy) + bot * fy + 32768) >> 16) as u8;
            }
        }
    }
    out
}

/// The crop, scale and blend of one frame for an output of `out` px.
fn reshape_frame(f: &RenderedFrame, d: &Delivery, (w, h): (u32, u32)) -> Vec<u8> {
    let (cw, ch, cropped) = match d.crop {
        Some(c) => {
            let (cw, ch, px) = crop_rgba(&f.data, f.width, f.height, c);
            (cw, ch, std::borrow::Cow::Owned(px))
        }
        None => (f.width, f.height, std::borrow::Cow::Borrowed(&f.data[..])),
    };
    let mut px = if (w, h) != (cw, ch) {
        scale_rgba(&cropped, cw, ch, w, h)
    } else {
        cropped.into_owned()
    };
    if let Some((edges, curve)) = &d.blend {
        let (sx, sy) = (w as f64 / cw as f64, h as f64 / ch as f64);
        let e = [
            edges[0] as f64 * sx,
            edges[1] as f64 * sx,
            edges[2] as f64 * sy,
            edges[3] as f64 * sy,
        ];
        apply_blend(&mut px, w, h, e, curve);
    }
    px
}

impl SinkWorker {
    fn spawn(
        mut sink: Box<dyn FrameSink>,
        delivery: Delivery,
        stats: Arc<SinkStats>,
    ) -> anyhow::Result<SinkWorker> {
        let (tx, rx) = sync_channel::<Job>(1);
        let st = stats.clone();
        let label = sink.name().to_string();
        let handle = std::thread::Builder::new()
            .name(format!("out {label}").chars().take(15).collect())
            .spawn(move || {
                let mut scaled: Option<(u64, Vec<u8>)> = None;
                // The fill or key of the last frame, and which frame it was made from.
                let mut keyed: (Option<u64>, Vec<u8>) = (None, Vec::new());
                for job in rx {
                    let f = &job.frame;
                    let key = Arc::as_ptr(f) as u64 ^ f.sequence.rotate_left(32);
                    let (w, h) = delivery.out_size((f.width, f.height));
                    let reshape = delivery.crop.is_some()
                        || delivery.blend.is_some()
                        || (w, h) != (f.width, f.height);
                    let pixels: &[u8] = if reshape {
                        if scaled.as_ref().map(|s| s.0) != Some(key) {
                            scaled = Some((key, reshape_frame(f, &delivery, (w, h))));
                        }
                        &scaled.as_ref().expect("scaled").1
                    } else {
                        &f.data
                    };
                    let mut alpha = delivery.alpha.unwrap_or(f.alpha);
                    let pixels: &[u8] = match delivery.target {
                        OutputTarget::Fill | OutputTarget::Key => {
                            if keyed.0 != Some(key) {
                                fill_key(delivery.target, alpha, pixels, &mut keyed.1);
                                keyed.0 = Some(key);
                            }
                            alpha = AlphaMode::Opaque;
                            &keyed.1
                        }
                        _ => pixels,
                    };
                    let frame = output::Frame {
                        width: w,
                        height: h,
                        pixels,
                        alpha,
                        frame_rate: job.frame_rate,
                        timecode: job.timecode,
                    };
                    match sink.send(&frame) {
                        Ok(()) => {
                            st.sent.fetch_add(1, Ordering::Relaxed);
                            st.failing.store(false, Ordering::Relaxed);
                        }
                        Err(e) => {
                            let n = st.errors.fetch_add(1, Ordering::Relaxed);
                            let msg = format!("{e:#}");
                            if n == 0 || n.is_power_of_two() {
                                tracing::warn!(output = %label, errors = n + 1, "send failed: {msg}");
                            }
                            *st.last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg);
                            st.failing.store(true, Ordering::Relaxed);
                        }
                    }
                }
                tracing::debug!(output = %label, "output worker stopped");
                // Dropping the sink here flushes it (an IDN source stops announcing itself and
                // closes its connections, ffmpeg finishes).
            })?;
        Ok(SinkWorker {
            tx: Some(tx),
            handle: Some(handle),
            stats,
        })
    }

    fn offer(&self, frame: &Arc<RenderedFrame>, timecode: i64, frame_rate: FrameRate) {
        let Some(tx) = &self.tx else { return };
        match tx.try_send(Job {
            frame: frame.clone(),
            timecode,
            frame_rate,
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl Drop for SinkWorker {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// One running (or unavailable) output of a screen.
pub struct OutputRuntime {
    /// Display name ("IDN: Program Feed", "PNG every 30: /tmp/x").
    pub name: String,
    /// `idn`, `png`, `preview`, `omt`, `display`, `sdi`.
    pub kind: &'static str,
    /// Where it came from: `binding` or `cli`.
    pub origin: &'static str,
    /// The binding it was built from (`None` for CLI outputs).
    pub binding: Option<OutputBinding>,
    /// Running (false: unavailable, see `detail`).
    pub active: bool,
    /// Why it is not running.
    pub detail: Option<String>,
    /// Counters (zero for inactive outputs).
    pub stats: Arc<SinkStats>,
    /// Delivery settings.
    pub delivery: Delivery,
    /// Level of the embedded audio (`Some` when the output embeds audio).
    pub audio: Option<Arc<crate::sound::OutputAudio>>,
    delay: Arc<AtomicU32>,
    /// The output's frame period, ns (its frame rate override, else its screen's): the
    /// embedded audio is held `delay` of these, so it follows a change of the screen's rate.
    frame_period: Arc<AtomicU64>,
    /// The embedded-audio worker: stop flag and handle.
    audio_worker: Mutex<Option<(Arc<AtomicBool>, JoinHandle<()>)>>,
    line: Mutex<DelayLine<Arc<RenderedFrame>>>,
    worker: Mutex<Option<SinkWorker>>,
    /// Own clock thread (frame rate override): stop flag and handle.
    pub(crate) own_clock: Mutex<Option<(Arc<AtomicBool>, JoinHandle<()>)>>,
    /// A display output: what its display looked like when it was built
    /// ([`crate::displays::Displays::signature`]); a change rebuilds it.
    pub display_signature: Option<String>,
    /// Why the binding can't run on its screen's arrangement ([`Delivery::problem`]); a
    /// change rebuilds it.
    pub problem: Option<String>,
}

impl OutputRuntime {
    fn new(
        name: String,
        kind: &'static str,
        origin: &'static str,
        binding: Option<&OutputBinding>,
        sink: anyhow::Result<Box<dyn FrameSink>>,
        screen: &Screen,
    ) -> OutputRuntime {
        let delivery = binding
            .map(|b| Delivery::for_screen(b, screen))
            .unwrap_or_default();
        let stats = Arc::new(SinkStats::default());
        let (worker, active, detail) = match sink
            .and_then(|s| SinkWorker::spawn(s, delivery.clone(), stats.clone()))
        {
            Ok(w) => {
                tracing::info!(screen = %screen.name, output = %name, "output started");
                (Some(w), true, None)
            }
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::warn!(screen = %screen.name, output = %name, "output unavailable: {msg}");
                (
                    None,
                    false,
                    Some(msg.lines().next().unwrap_or_default().to_string()),
                )
            }
        };
        OutputRuntime {
            name,
            kind,
            origin,
            binding: binding.cloned(),
            active,
            detail,
            stats,
            delay: Arc::new(AtomicU32::new(
                binding
                    .map(|b| b.video_delay.min(MAX_VIDEO_DELAY))
                    .unwrap_or(0),
            )),
            frame_period: Arc::new(AtomicU64::new(period_ns(
                binding
                    .and_then(|b| b.frame_rate_override)
                    .unwrap_or(screen.frame_rate),
            ))),
            audio: None,
            audio_worker: Mutex::new(None),
            line: Mutex::new(DelayLine::default()),
            worker: Mutex::new(worker),
            delivery,
            own_clock: Mutex::new(None),
            display_signature: None,
            problem: None,
        }
    }

    /// The binding's ID (`None` for CLI outputs).
    pub fn binding_id(&self) -> Option<OutputId> {
        self.binding.as_ref().map(|b| b.id)
    }

    /// Whether this output runs on its own clock (frame rate override).
    pub fn has_own_clock(&self) -> bool {
        self.delivery.rate.is_some()
    }

    /// Video delay, frames.
    pub fn delay(&self) -> u32 {
        self.delay.load(Ordering::Relaxed)
    }

    /// Whether `b` (a new version of this output's binding) can be applied without
    /// rebuilding the sink: only the operator name or the video delay changed.
    pub fn can_update_in_place(&self, b: &OutputBinding) -> bool {
        let Some(cur) = &self.binding else {
            return false;
        };
        let strip = |x: &OutputBinding| OutputBinding {
            name: String::new(),
            video_delay: 0,
            schema_version: 0,
            ..x.clone()
        };
        strip(cur) == strip(b)
    }

    /// Changes the video delay (takes effect on the next frame).
    pub fn set_delay(&self, frames: u32) {
        self.delay
            .store(frames.min(MAX_VIDEO_DELAY), Ordering::Relaxed);
    }

    /// The frame rate the output runs at now (its override, else its screen's): the delay
    /// of its embedded audio follows it.
    pub fn set_frame_rate(&self, rate: FrameRate) {
        self.frame_period.store(period_ns(rate), Ordering::Relaxed);
    }

    /// The output's frame period now.
    pub fn frame_period(&self) -> std::time::Duration {
        std::time::Duration::from_nanos(self.frame_period.load(Ordering::Relaxed))
    }

    /// Hands the screen's current frame to the output (through its delay line); never waits.
    pub fn offer(&self, frame: &Arc<RenderedFrame>, timecode: i64, frame_rate: FrameRate) {
        let w = crate::lock(&self.worker);
        let Some(worker) = w.as_ref() else { return };
        let out = crate::lock(&self.line).push(frame.clone(), self.delay());
        worker.offer(&out, timecode, frame_rate);
    }

    /// Frames currently held by the delay line.
    pub fn delayed_frames(&self) -> usize {
        crate::lock(&self.line).len()
    }

    /// Stops the output: its own clock (if any), then its worker (the sink is flushed and
    /// closed, e.g. the IDN source is withdrawn). Blocks until both threads ended.
    pub fn shutdown(&self) {
        if let Some((stop, h)) = crate::lock(&self.own_clock).take() {
            stop.store(true, Ordering::Release);
            let _ = h.join();
        }
        if let Some((stop, h)) = crate::lock(&self.audio_worker).take() {
            stop.store(true, Ordering::Release);
            let _ = h.join();
        }
        let w = crate::lock(&self.worker).take();
        drop(w);
        crate::lock(&self.line).items.clear();
    }

    /// Snapshot for the show state.
    pub fn state(&self, screen: &Screen) -> protocol::OutputState {
        let failing = self.stats.failing.load(Ordering::Relaxed);
        let last_error = self
            .stats
            .last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let (status, detail) = if !self.active {
            (protocol::OutputStatus::Unavailable, self.detail.clone())
        } else if failing {
            (protocol::OutputStatus::Error, last_error)
        } else {
            (protocol::OutputStatus::Ok, None)
        };
        let (sw, sh) = (
            screen.size.width.round().max(1.0) as u32,
            screen.size.height.round().max(1.0) as u32,
        );
        let (width, height) = self.delivery.out_size((sw, sh));
        protocol::OutputState {
            name: self.name.clone(),
            kind: self.kind.to_string(),
            origin: self.origin.to_string(),
            binding: self.binding_id(),
            width,
            height,
            frame_rate: self.delivery.rate.unwrap_or(screen.frame_rate),
            delay_frames: self.delay(),
            status,
            detail,
            frames_sent: self.stats.sent.load(Ordering::Relaxed),
            frames_dropped: self.stats.dropped.load(Ordering::Relaxed),
            errors: self.stats.errors.load(Ordering::Relaxed),
            embed_audio: self.audio.is_some(),
            audio_db: self.audio.as_ref().map(|a| a.get().0).unwrap_or(0.0),
            audio_muted: self.audio.as_ref().is_some_and(|a| a.get().1),
        }
    }
}

fn safe_file_name(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if t.is_empty() { "screen".into() } else { t }
}

/// Whether `query` names `screen` (ID, or name case-insensitively).
pub fn screen_matches(screen: &Screen, query: &str) -> bool {
    screen.id.0.to_string() == query || screen.name.eq_ignore_ascii_case(query)
}

/// The display name of a binding's output ("IDN: Main", "Display: HDMI-A-2 (key)").
pub fn binding_label(b: &OutputBinding) -> (String, &'static str) {
    let (name, kind) = transport_label(b);
    match b.target {
        OutputTarget::Fill => (format!("{name} (fill)"), kind),
        OutputTarget::Key => (format!("{name} (key)"), kind),
        _ => (name, kind),
    }
}

fn transport_label(b: &OutputBinding) -> (String, &'static str) {
    match &b.transport {
        OutputTransport::Idn { source_name, .. } => (format!("IDN: {source_name}"), "idn"),
        OutputTransport::Omt { name } => (format!("OMT: {name}"), "omt"),
        OutputTransport::Display { selector, .. } => (
            format!("Display: {}", crate::displays::selector_label(selector)),
            "display",
        ),
        OutputTransport::Sdi { device, .. } => (format!("SDI: {device}"), "sdi"),
        OutputTransport::Placeholder => (format!("Placeholder: {}", b.name), "placeholder"),
        OutputTransport::Unknown => (format!("Output: {}", b.name), "unknown"),
    }
}

/// One frame at `rate`, ns.
fn period_ns(rate: FrameRate) -> u64 {
    1_000_000_000 * rate.den.max(1) as u64 / rate.num.max(1) as u64
}

/// Builds the output of one binding. Unavailable outputs (displays, SDI, a source that
/// fails to start) are logged and kept as inactive entries; nothing here is fatal. With
/// `sound`, an IDN or OMT binding that has `embedAudio` sends the program audio.
pub fn build_binding(
    screen: &Screen,
    b: &OutputBinding,
    sound: Option<&crate::sound::Hub>,
    displays: Option<&crate::displays::Displays>,
) -> OutputRuntime {
    let (name, kind) = binding_label(b);
    let mut audio_sender: Option<Box<dyn output::AudioSink>> = None;
    let problem = Delivery::problem(b, screen);
    let sink: anyhow::Result<Box<dyn FrameSink>> = match &b.transport {
        // Nothing to open: the slot waits for hardware.
        OutputTransport::Placeholder => Err(anyhow::anyhow!("No hardware yet")),
        _ if let Some(p) = &problem => Err(anyhow::anyhow!(p.clone())),
        OutputTransport::Idn {
            source_name,
            groups,
        } => idn_sink(screen, b, source_name, groups, &mut audio_sender),
        OutputTransport::Omt { name } => omt_sink(b, name, &mut audio_sender),
        OutputTransport::Display { .. } => match displays {
            Some(d) => d.open(b),
            None => Err(anyhow::anyhow!("display outputs are not available here")),
        },
        OutputTransport::Sdi { .. } => Err(anyhow::anyhow!("SDI outputs are not supported yet")),
        OutputTransport::Unknown => Err(anyhow::anyhow!("no transport configured")),
    };
    let mut rt = OutputRuntime::new(name, kind, "binding", Some(b), sink, screen);
    rt.problem = problem;
    rt.display_signature = displays.and_then(|d| d.signature(b));
    if let (Some(sender), Some(hub), true) = (audio_sender, sound, rt.active) {
        let level = hub.output(b.id);
        let stop = Arc::new(AtomicBool::new(false));
        match crate::sound::spawn_embedded_audio(
            &rt.name,
            hub,
            sender,
            level.clone(),
            rt.delay.clone(),
            rt.frame_period.clone(),
            stop.clone(),
        ) {
            Ok(h) => {
                *crate::lock(&rt.audio_worker) = Some((stop, h));
                rt.audio = Some(level);
                tracing::info!(output = %rt.name, "embedding program audio");
            }
            Err(e) => tracing::error!(output = %rt.name, "cannot start the audio worker: {e}"),
        }
    }
    rt
}

/// An IDN Open Protocol sink for a binding.
fn idn_sink(
    screen: &Screen,
    b: &OutputBinding,
    source_name: &str,
    groups: &[String],
    audio_sender: &mut Option<Box<dyn output::AudioSink>>,
) -> anyhow::Result<Box<dyn FrameSink>> {
    let delivery = Delivery::of(b);
    let alpha = delivery.alpha.unwrap_or(screen.alpha);
    let mut cfg = IdnSenderConfig::new(source_name);
    cfg.groups = groups.to_vec();
    // The halves of a Fill/Key pair are opaque pictures.
    let half = matches!(delivery.target, OutputTarget::Fill | OutputTarget::Key);
    cfg.alpha = !half && matches!(alpha, AlphaMode::Straight | AlphaMode::Premultiplied);
    cfg.frame_rate = delivery.rate.unwrap_or(screen.frame_rate);
    let s = IdnSink::new(cfg)?;
    if b.embed_audio {
        *audio_sender = Some(Box::new(s.audio_sender()));
    }
    Ok(Box::new(s))
}

/// An OMT sink for a binding (the halves of a Fill/Key pair are opaque frames already: the
/// worker flattens them before the sink sees them).
fn omt_sink(
    b: &OutputBinding,
    name: &str,
    audio_sender: &mut Option<Box<dyn output::AudioSink>>,
) -> anyhow::Result<Box<dyn FrameSink>> {
    let s = output::OmtSink::new(output::OmtSenderConfig::new(name))?;
    if b.embed_audio {
        *audio_sender = Some(Box::new(s.audio_sender()));
    }
    Ok(Box::new(s))
}

/// The command-line development outputs of one screen.
pub fn build_cli(screen: &Screen, dev: &DevOutputs) -> Vec<OutputRuntime> {
    let mut out = Vec::new();
    if let Some(prefix) = &dev.idn_prefix {
        let name = format!("{prefix} {}", screen.name);
        let mut cfg = IdnSenderConfig::new(name.clone());
        cfg.alpha = matches!(screen.alpha, AlphaMode::Straight | AlphaMode::Premultiplied);
        cfg.frame_rate = screen.frame_rate;
        out.push(OutputRuntime::new(
            format!("IDN: {name}"),
            "idn",
            "cli",
            None,
            IdnSink::new(cfg).map(|s| Box::new(s) as Box<dyn FrameSink>),
            screen,
        ));
    }
    if let Some(dir) = &dev.png_dir {
        let d = dir.join(safe_file_name(&screen.name));
        out.push(OutputRuntime::new(
            format!("PNG every {}: {}", dev.png_every, d.display()),
            "png",
            "cli",
            None,
            PngSink::every_nth(d, dev.png_every.max(1)).map(|s| Box::new(s) as Box<dyn FrameSink>),
            screen,
        ));
    }
    if let Some(q) = &dev.preview
        && screen_matches(screen, q)
    {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            tracing::warn!(
                "--preview: no DISPLAY / WAYLAND_DISPLAY set; ffplay will probably fail"
            );
        }
        out.push(OutputRuntime::new(
            format!("Preview: {}", screen.name),
            "preview",
            "cli",
            None,
            Ok(Box::new(FfmpegSink::preview(&screen.name))),
            screen,
        ));
    }
    out
}

/// The enabled bindings of `screen`, in data order (none with `--no-bindings`).
pub fn bindings_of<'a>(
    data: &'a ShowData,
    screen: &Screen,
    dev: &DevOutputs,
) -> Vec<&'a OutputBinding> {
    if dev.no_bindings {
        return Vec::new();
    }
    data.outputs
        .iter()
        .filter(|b| b.screen == screen.id && b.enabled)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_line_delivers_in_order_after_n_frames() {
        let mut d = DelayLine::default();
        // Delay 3: the first pushes repeat the oldest frame, then frame n-3 comes out.
        let out: Vec<u32> = (0..8).map(|i| d.push(i, 3)).collect();
        assert_eq!(out, vec![0, 0, 0, 0, 1, 2, 3, 4]);
        assert_eq!(d.len(), 4);
        // Delay 0 passes frames straight through.
        let mut z = DelayLine::default();
        assert_eq!(
            (0..3).map(|i| z.push(i, 0)).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        // Shortening the delay drops the oldest frames at once.
        assert_eq!(d.push(8, 1), 7);
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn blend_ramps_meet_in_light_and_leave_alpha() {
        let curve = BlendCurve::default();
        // Two 8 px wide white tiles whose 4 px edges overlap.
        let mut left = [255u8, 255, 255, 255].repeat(8 * 2);
        let mut right = left.clone();
        apply_blend(&mut left, 8, 2, [0.0, 4.0, 0.0, 0.0], &curve);
        apply_blend(&mut right, 8, 2, [4.0, 0.0, 0.0, 0.0], &curve);
        let light = |v: u8| (v as f64 / 255.0).powf(curve.gamma);
        for k in 0..4 {
            let (a, b) = (&left[(4 + k) * 4..], &right[k * 4..]);
            assert!((light(a[0]) + light(b[0]) - 1.0).abs() < 0.02, "column {k}");
            assert_eq!((a[3], b[3]), (255, 255));
        }
        assert_eq!(
            &left[0..4],
            &[255, 255, 255, 255],
            "the outer edge is untouched"
        );
        assert_eq!(&left[32..36], &left[0..4], "the second row matches");
    }

    #[test]
    fn crop_takes_the_rectangle() {
        let src: Vec<u8> = (0..4 * 3).flat_map(|i| [i as u8, 0, 0, 255]).collect();
        let (w, h, out) = crop_rgba(&src, 4, 3, (1, 1, 2, 2));
        assert_eq!((w, h), (2, 2));
        let reds: Vec<u8> = out.chunks(4).map(|p| p[0]).collect();
        assert_eq!(reds, vec![5, 6, 9, 10]);
    }

    #[test]
    fn scaling_keeps_flat_colours_and_halves() {
        let src: Vec<u8> = [10u8, 200, 30, 255].repeat(8 * 4);
        let out = scale_rgba(&src, 8, 4, 4, 2);
        assert_eq!(out.len(), 4 * 2 * 4);
        assert!(out.chunks(4).all(|p| p == [10, 200, 30, 255]));
        // A left/right split stays split.
        let mut src = Vec::new();
        for _ in 0..4 {
            for x in 0..8 {
                src.extend_from_slice(if x < 4 {
                    &[0, 0, 0, 255]
                } else {
                    &[255, 255, 255, 255]
                });
            }
        }
        let out = scale_rgba(&src, 8, 4, 4, 2);
        assert_eq!(&out[0..4], &[0, 0, 0, 255]);
        assert_eq!(&out[12..16], &[255, 255, 255, 255]);
    }
}
