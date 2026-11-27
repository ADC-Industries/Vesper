//! Live captions (`docs/CAPTIONS-PLAN.md`): speech from a sound source turned into caption
//! lines by IDN's `idn-captions` (whisper.cpp on this machine, on the CPU) and drawn on the
//! captions layer.
//!
//! - **Whisper's process.** IDN runs whisper in a worker process below normal priority (IDN
//!   v0.3.8-Beta18: Windows' below-normal priority class, nice 10 elsewhere), so captions
//!   never take CPU from the screens' frames, whatever whisper's own threads do. The worker
//!   is the `vesper` binary itself (`vesper --idn-captions-worker …`, served by
//!   `idn_captions::worker::serve_if_asked` first thing in `main`; [`CaptionsOptions`]). The
//!   engine is started again when it can't load its model there ([`RETRY`]); a step that
//!   fails meanwhile (a pass too slow, a worker that ended and is being started again by IDN)
//!   shows as the status's error until a step works again. Neither stops the server.
//!
//! - **Hub** ([`Hub`], in [`Shared`]): the box's [`CaptionsConfig`], the engine's status and
//!   the lines. Lines come from the engine's worker or from `POST /captions/lines` (another
//!   captioner); both go through [`Hub::line`]. Each change wakes the show actor
//!   ([`Hub::changed`]), which copies [`Hub::shown`] into the show state: screens re-render
//!   and clients get the state.
//! - **Lines.** A draft is replaced by the next draft or made final; final lines are kept,
//!   the last `maxLines` of them (with the draft) are shown. After [`HOLD`] without a new
//!   line the screens are cleared (the lines stay in the state for Oriel's preview).
//! - **Worker** (one thread while captions are on): fetches the model into
//!   `<data>/cache/whisper/` ([`idn_captions::ensure_model_stop`], pinned size and SHA-256;
//!   `auto` measures the box once with [`idn_captions::auto_choose_stop`], downloading the
//!   models it tries), reporting each download's bytes and size, loads the model, and feeds the
//!   engine from the source about twice a second. A reader thread drains the source (the
//!   program bus's tap, whose queue holds only half a second, or a live source's sound feed)
//!   into a buffer while a whisper pass runs. A new worker (captions back on, or another
//!   source, model, language or words) first waits for the one before it to end, so two never
//!   fetch or measure at once. That is short: the worker's stop flag goes to IDN's fetch and
//!   measurement (IDN v0.3.4-Beta11), which end within about a second when it is set (a
//!   download, its connection, the hash check or a measuring pass under way), with
//!   [`idn_captions::STOPPED`]: a normal stop, not a failure. A stopped download leaves no
//!   file under the model's name (the next fetch starts it again). Loading the model and a
//!   whisper pass of the running engine aren't stopped; they take a few seconds at most.
//! - **Network.** With `network` on, each line is sent on an IDN text-only source called
//!   "Captions" (`text/1`).

use crate::{Shared, lock};
use model::{CaptionPosition, CaptionSource, CaptionsConfig};
use protocol::{CaptionLineState, CaptionsState, CaptionsStatus};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// Lines leave the screens after this long without a new one.
pub const HOLD: Duration = Duration::from_secs(8);
/// Final lines kept (Oriel's preview and the screens show the last `maxLines`).
const KEPT: usize = 8;
/// How much sound the worker gathers before a whisper step.
const STEP: Duration = Duration::from_millis(500);
/// The most sound buffered while a pass runs (older sound is dropped).
const BUFFERED: Duration = Duration::from_secs(20);
/// The name of the IDN text source.
pub const NETWORK_NAME: &str = "Captions";
/// The canvas height the style's sizes are given for.
const STYLE_HEIGHT: f64 = 1080.0;
/// How long the engine waits before loading its model again after a failed load: these, then
/// the last one each time (a worker that ended twice or was too slow on a busy box, or too
/// little memory for the model, can pass).
pub const RETRY: [Duration; 4] = [
    Duration::from_secs(5),
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(300),
];

/// How the speech engine runs (the server's [`crate::ServerConfig::captions`]).
#[derive(Clone, Debug, Default)]
pub struct CaptionsOptions {
    /// The program whisper runs in (IDN's captions worker: one that calls
    /// `idn_captions::worker::serve_if_asked` first thing in its `main`). The `vesper` binary
    /// passes itself ([`CaptionsOptions::this_program`]). `None`: IDN looks for one itself
    /// (the switch `IDN_CAPTIONS_WORKER`, else IDN's `idn` next to this program or on `PATH`),
    /// and whisper runs in this process when none starts.
    pub worker: Option<PathBuf>,
    /// Tests: this file is the model, used as it is (neither fetched nor checked against its
    /// pin).
    #[doc(hidden)]
    pub model_file: Option<PathBuf>,
}

impl CaptionsOptions {
    /// The running program as the worker: on Linux `/proc/self/exe`, which still starts the
    /// same program (and so the same worker protocol) after a package update replaced its file.
    pub fn this_program() -> CaptionsOptions {
        #[cfg(target_os = "linux")]
        let worker = Some(PathBuf::from("/proc/self/exe"));
        #[cfg(not(target_os = "linux"))]
        let worker = std::env::current_exe().ok();
        CaptionsOptions {
            worker,
            model_file: None,
        }
    }
}

/// What the screens draw on the captions layer (part of the show state).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shown {
    /// On audience screens (per look).
    pub audience: bool,
    /// On stage screens.
    pub stage: bool,
    /// The box's style.
    pub style: model::CaptionStyle,
    /// The lines, oldest first (empty: nothing drawn).
    pub lines: Vec<String>,
}

#[derive(Default)]
struct Lines {
    /// Final lines (number, text), oldest first.
    finals: VecDeque<(u64, String)>,
    /// The line being corrected.
    draft: Option<String>,
    /// The next line's number.
    next: u64,
    /// When the last line came.
    at: Option<Instant>,
}

#[derive(Default)]
struct Status {
    status: CaptionsStatus,
    detail: Option<String>,
    progress: Option<u32>,
    /// Bytes downloaded and the size (when known) while `Downloading`.
    downloaded: Option<(u64, Option<u64>)>,
    model: Option<String>,
    device: Option<String>,
    load: Option<f64>,
}

struct Worker {
    stop: Arc<AtomicBool>,
    /// What it was started with (source, model, language, words, music).
    key: String,
    /// Its thread (`None` in tests).
    thread: Option<std::thread::JoinHandle<()>>,
}

/// The captions hub (see the [module docs](self)).
pub struct Hub {
    root: PathBuf,
    options: CaptionsOptions,
    config: Mutex<CaptionsConfig>,
    lines: Mutex<Lines>,
    status: Mutex<Status>,
    worker: Mutex<Option<Worker>>,
    /// The thread of the last worker stopped: the next worker waits for it to end.
    previous: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Held across a save's write and its apply: two saves never cross.
    saving: Mutex<()>,
    network: Mutex<Option<idn_open_protocol::text::TextSource>>,
    shared: std::sync::OnceLock<Weak<Shared>>,
    /// Wakes the show actor when the lines, the status or the configuration changed.
    pub changed: tokio::sync::Notify,
}

/// The stored configuration's file.
pub fn config_path(root: &Path) -> PathBuf {
    crate::store::doc_path(
        root,
        "machine",
        model::DocumentKind::CaptionsConfig,
        uuid::Uuid::nil(),
    )
}

/// Where the models are kept: the data folder's cache (not in backups).
pub fn models_dir(root: &Path) -> PathBuf {
    root.join("cache").join("whisper")
}

impl Hub {
    /// The hub of the data folder at `root`, with its stored configuration (captions start
    /// once [`Hub::bind`] ties it to the server).
    pub fn new(root: &Path) -> Hub {
        Hub::with_options(root, CaptionsOptions::default())
    }

    /// [`Hub::new`] with how the engine runs.
    pub fn with_options(root: &Path, options: CaptionsOptions) -> Hub {
        let config = std::fs::read(config_path(root))
            .ok()
            .and_then(|b| match serde_json::from_slice::<CaptionsConfig>(&b) {
                Ok(c) => Some(c),
                Err(e) => {
                    tracing::warn!("captions: the stored configuration is not valid: {e}");
                    None
                }
            })
            .unwrap_or_default();
        Hub {
            root: root.to_path_buf(),
            options,
            config: Mutex::new(config),
            lines: Mutex::default(),
            status: Mutex::default(),
            worker: Mutex::new(None),
            previous: Mutex::new(None),
            saving: Mutex::new(()),
            network: Mutex::new(None),
            shared: std::sync::OnceLock::new(),
            changed: tokio::sync::Notify::new(),
        }
    }

    /// Ties the hub to its server and starts captions when they are on.
    pub fn bind(&self, shared: Weak<Shared>) {
        let _ = self.shared.set(shared);
        let cfg = self.config();
        self.apply(&cfg);
    }

    /// The configuration.
    pub fn config(&self) -> CaptionsConfig {
        lock(&self.config).clone()
    }

    /// Checks a configuration from the API (`Err`: why it is refused).
    pub fn validate(cfg: &CaptionsConfig) -> Result<(), String> {
        if !(1..=4).contains(&cfg.max_lines) {
            return Err("maxLines must be between 1 and 4".into());
        }
        if cfg.model != "auto" && idn_captions::model_file(&cfg.model).is_none() {
            return Err(format!(
                "model must be auto or one of {}",
                idn_captions::MODEL_LIST.map(|m| m.0).join(", ")
            ));
        }
        // As it is stored (see `save`).
        let lang = cfg.language.trim().to_lowercase();
        if lang.is_empty() || lang.len() > 8 || !lang.chars().all(|c| c.is_ascii_alphabetic()) {
            return Err("language must be a language code such as en or es".into());
        }
        if !idn_captions::model_speaks(&cfg.model, &lang) {
            return Err(format!(
                "{} understands English only: choose auto or large-v3-turbo for {lang}",
                cfg.model
            ));
        }
        if cfg.words.len() > 1000 {
            return Err("words must be at most 1000 characters".into());
        }
        if let CaptionSource::LiveSource { name } = &cfg.source
            && (name.trim().is_empty() || name.len() > 256)
        {
            return Err("a live source needs its name".into());
        }
        let s = &cfg.style;
        let finite = [s.size, s.margin].iter().all(|v| v.is_finite());
        if !finite || !(12.0..=200.0).contains(&s.size) || !(0.0..=400.0).contains(&s.margin) {
            return Err("style: size must be 12–200 and margin 0–400".into());
        }
        if s.font.len() > 200 {
            return Err("style: font name too long".into());
        }
        Ok(())
    }

    /// Stores `cfg` (validated) and applies it: the worker restarts only when what it
    /// listens with changed.
    pub fn save(&self, mut cfg: CaptionsConfig) -> Result<CaptionsConfig, String> {
        Self::validate(&cfg)?;
        cfg.schema_version = model::SCHEMA_VERSION;
        cfg.language = cfg.language.trim().to_lowercase();
        let write = crate::store::DocWrite::put(&cfg).map_err(|e| e.to_string())?;
        // The file and the running configuration always end on the same save.
        let _saving = lock(&self.saving);
        crate::store::apply(&self.root, &[write]).map_err(|e| format!("saving failed: {e}"))?;
        self.apply(&cfg);
        Ok(cfg)
    }

    fn apply(&self, cfg: &CaptionsConfig) {
        *lock(&self.config) = cfg.clone();
        let key = serde_json::json!([
            cfg.source,
            cfg.model,
            cfg.language,
            cfg.words,
            cfg.keep_music
        ])
        .to_string();
        {
            let engine = cfg.enabled && cfg.source != CaptionSource::External;
            let mut worker = lock(&self.worker);
            let keep = engine && worker.as_ref().is_some_and(|w| w.key == key);
            if !keep && let Some(w) = worker.take() {
                w.stop.store(true, Ordering::Relaxed);
                // The next worker waits for it (it waits for any worker before it): a download
                // or a measurement under way ends within about a second of the stop.
                if let Some(t) = w.thread {
                    *lock(&self.previous) = Some(t);
                }
            }
            if cfg.enabled && !engine {
                *lock(&self.status) = Status {
                    status: CaptionsStatus::Running,
                    detail: Some("lines come from another captioner".into()),
                    ..Status::default()
                };
            } else if engine && worker.is_none() {
                // A new engine: nothing of the one before it (model, device, load) shows.
                *lock(&self.status) = Status {
                    status: CaptionsStatus::Starting,
                    ..Status::default()
                };
                if let Some(shared) = self.shared.get().cloned() {
                    let stop = Arc::new(AtomicBool::new(false));
                    let before = lock(&self.previous).take();
                    if before.as_ref().is_some_and(|t| !t.is_finished()) {
                        lock(&self.status).detail =
                            Some("waiting for the previous engine to finish".into());
                    }
                    let run = (
                        shared,
                        cfg.clone(),
                        stop.clone(),
                        models_dir(&self.root),
                        self.options.clone(),
                    );
                    let spawned =
                        std::thread::Builder::new()
                            .name("captions".into())
                            .spawn(move || {
                                if let Some(t) = before {
                                    let _ = t.join();
                                }
                                worker_main(run.0, run.1, run.2, run.3, run.4)
                            });
                    match spawned {
                        Ok(t) => {
                            *worker = Some(Worker {
                                stop,
                                key,
                                thread: Some(t),
                            })
                        }
                        Err(e) => self.fail(format!("no thread: {e}")),
                    }
                }
            } else if !cfg.enabled {
                *lock(&self.status) = Status::default();
                *lock(&self.lines) = Lines::default();
            }
        }
        let mut network = lock(&self.network);
        if cfg.enabled && cfg.network {
            if network.is_none() {
                match output::idn::text_source(NETWORK_NAME) {
                    Ok(s) => *network = Some(s),
                    Err(e) => tracing::warn!("captions: not sent on the network: {e:#}"),
                }
            }
        } else {
            *network = None;
        }
        drop(network);
        self.changed.notify_one();
    }

    /// Stops the worker (server shutdown).
    pub fn stop(&self) {
        if let Some(w) = lock(&self.worker).take() {
            w.stop.store(true, Ordering::Relaxed);
        }
        *lock(&self.network) = None;
    }

    fn set_status(&self, status: CaptionsStatus, detail: Option<String>) {
        let mut s = lock(&self.status);
        s.status = status;
        s.detail = detail;
        if status != CaptionsStatus::Downloading {
            s.progress = None;
            s.downloaded = None;
        }
        drop(s);
        self.changed.notify_one();
    }

    /// A model's download: `n` bytes of `total` so far.
    fn downloading(&self, model: &str, n: u64, total: Option<u64>) {
        let total = total.filter(|t| *t > 0);
        let mut s = lock(&self.status);
        s.status = CaptionsStatus::Downloading;
        s.detail = Some(format!("downloading the {model} model"));
        s.progress = total.map(|t| (n.saturating_mul(100) / t).min(100) as u32);
        s.downloaded = Some((n, total));
        drop(s);
        self.changed.notify_one();
    }

    fn fail(&self, why: String) {
        tracing::warn!("captions: {why}");
        self.set_status(CaptionsStatus::Error, Some(why));
    }

    /// A caption line: from the engine or another captioner. A draft replaces the draft
    /// before it (an empty one takes it down); a final line ends it.
    pub fn line(&self, text: &str, is_final: bool) {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut l = lock(&self.lines);
        let number = l.next;
        if is_final {
            l.draft = None;
            if !text.is_empty() {
                l.finals.push_back((number, text.clone()));
                while l.finals.len() > KEPT {
                    l.finals.pop_front();
                }
                l.next += 1;
            }
        } else {
            l.draft = (!text.is_empty()).then(|| text.clone());
        }
        l.at = Some(Instant::now());
        drop(l);
        if let Some(net) = lock(&self.network).as_ref() {
            let lang = lock(&self.config).language.clone();
            net.send_caption(&idn_open_protocol::text::CaptionLine {
                line: number,
                text,
                is_final,
                lang,
                at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64),
                ..Default::default()
            });
        }
        self.changed.notify_one();
    }

    /// Takes the lines off the screens (`clearLayer captions`); captions keep running.
    pub fn clear(&self) {
        let mut l = lock(&self.lines);
        l.finals.clear();
        l.draft = None;
        drop(l);
        self.changed.notify_one();
    }

    /// What the screens draw now.
    pub fn shown(&self, now: Instant) -> Shown {
        let cfg = lock(&self.config);
        if !cfg.enabled {
            return Shown::default();
        }
        let l = lock(&self.lines);
        let fresh = l.at.is_some_and(|at| now.duration_since(at) < HOLD);
        let mut lines: Vec<String> = Vec::new();
        if fresh {
            lines.extend(l.finals.iter().map(|(_, t)| t.clone()));
            lines.extend(l.draft.clone());
            let keep = cfg.max_lines.clamp(1, 4) as usize;
            if lines.len() > keep {
                lines.drain(..lines.len() - keep);
            }
        }
        Shown {
            audience: cfg.audience,
            stage: cfg.stage,
            style: cfg.style.clone(),
            lines,
        }
    }

    /// The captions part of the operator state.
    pub fn state(&self) -> CaptionsState {
        let enabled = lock(&self.config).enabled;
        let max = lock(&self.config).max_lines.clamp(1, 4) as usize;
        let s = lock(&self.status);
        let l = lock(&self.lines);
        let mut lines: Vec<CaptionLineState> = l
            .finals
            .iter()
            .map(|(n, t)| CaptionLineState {
                line: *n,
                text: t.clone(),
                is_final: true,
            })
            .collect();
        if let Some(d) = &l.draft {
            lines.push(CaptionLineState {
                line: l.next,
                text: d.clone(),
                is_final: false,
            });
        }
        if lines.len() > max {
            lines.drain(..lines.len() - max);
        }
        CaptionsState {
            enabled,
            status: if enabled {
                s.status
            } else {
                CaptionsStatus::Off
            },
            detail: s.detail.clone(),
            progress: s.progress,
            downloaded: s.downloaded.map(|d| d.0),
            download_size: s.downloaded.and_then(|d| d.1),
            model: s.model.clone(),
            device: s.device.clone(),
            load: s.load,
            lines,
        }
    }

    /// The models offered, and which are on this box.
    pub fn models(&self) -> Vec<protocol::CaptionModel> {
        let dir = models_dir(&self.root);
        idn_captions::MODEL_LIST
            .iter()
            .map(|(name, mb, note)| protocol::CaptionModel {
                name: name.to_string(),
                size_mb: *mb,
                note: note.to_string(),
                downloaded: idn_captions::model_file(name).is_some_and(|f| dir.join(f).is_file()),
            })
            .collect()
    }
}

/// The captions layer's slide for `shown` on a screen of `size` (its own canvas, so the
/// band keeps its proportions on any screen).
pub fn slide(shown: &Shown, size: model::Size) -> model::Slide {
    use model::{
        CharStyle, Element, FitMode, Paragraph, ParagraphStyle, Rect, TextAlign, TextBox,
        VerticalAlign,
    };
    let (w, h) = (size.width.max(1.0), size.height.max(1.0));
    let k = h / STYLE_HEIGHT;
    let st = &shown.style;
    let size_px = st.size * k;
    let margin = st.margin * k;
    let pad = size_px * 0.3;
    let rows = shown.lines.len().max(1) as f64;
    let band_h = (rows * size_px * 1.25 + 2.0 * pad).min(h);
    let y = match st.position {
        CaptionPosition::Bottom => (h - margin - band_h).max(0.0),
        CaptionPosition::Top => margin.min(h - band_h).max(0.0),
    };
    let band_r = Rect::new(margin, y, (w - 2.0 * margin).max(1.0), band_h);
    let mut s = model::Slide::new();
    if st.background.a > 0.0 {
        let mut band = Element::new(band_r);
        band.name = "Captions Band".into();
        band.fill = model::Fill::color(st.background);
        s.elements.push(band);
    }
    let text_r = Rect::new(
        band_r.x + pad,
        band_r.y + pad * 0.5,
        (band_r.width - 2.0 * pad).max(1.0),
        (band_r.height - pad).max(1.0),
    );
    let mut t = Element::text(
        text_r,
        TextBox {
            paragraphs: Paragraph::from_lines(&shown.lines.join("\n")),
            default_style: CharStyle {
                font_family: Some(if st.font.trim().is_empty() {
                    "DejaVu Sans".to_string()
                } else {
                    st.font.trim().to_string()
                }),
                font_weight: Some(600),
                size: Some(size_px),
                color: Some(st.color),
                ..CharStyle::default()
            },
            default_paragraph: ParagraphStyle {
                alignment: Some(TextAlign::Center),
                ..ParagraphStyle::default()
            },
            vertical_align: VerticalAlign::Middle,
            fit: FitMode::ShrinkToFit,
            ..TextBox::default()
        },
    );
    t.name = "Captions".into();
    s.elements.push(t);
    s
}

// ---- the worker ----------------------------------------------------------------------------

/// Sound gathered by the reader thread (mono, at `rate`).
#[derive(Default)]
struct Gathered {
    rate: u32,
    samples: Vec<f32>,
}

/// Which download callbacks are worth a state change: a new model, a new whole percent, or
/// (size unknown) each further megabyte.
#[derive(Default)]
struct Download {
    last: Option<(String, u64)>,
}

impl Download {
    const UNKNOWN_STEP: u64 = 1 << 20;

    fn step(&mut self, model: &str, n: u64, total: Option<u64>) -> bool {
        let mark = match total.filter(|t| *t > 0) {
            Some(t) => (n.saturating_mul(100) / t).min(100),
            None => n / Self::UNKNOWN_STEP,
        };
        let new = self
            .last
            .as_ref()
            .is_none_or(|(m, at)| m != model || *at != mark);
        if new {
            self.last = Some((model.to_string(), mark));
        }
        new
    }
}

fn hub(shared: &Weak<Shared>) -> Option<Arc<Shared>> {
    shared.upgrade()
}

/// Sleeps `d` unless `stop` is set meanwhile: whether it slept to the end.
fn wait_unless_stopped(stop: &AtomicBool, d: Duration) -> bool {
    let end = Instant::now() + d;
    while !stop.load(Ordering::Relaxed) {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(Duration::from_millis(200)));
    }
    false
}

fn worker_main(
    shared: Weak<Shared>,
    cfg: CaptionsConfig,
    stop: Arc<AtomicBool>,
    dir: PathBuf,
    options: CaptionsOptions,
) {
    let report = |f: &dyn Fn(&Hub)| {
        if !stop.load(Ordering::Relaxed)
            && let Some(sh) = hub(&shared)
        {
            f(&sh.captions);
        }
    };
    // Stopped while it waited for the worker before it.
    if stop.load(Ordering::Relaxed) {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        report(&|h| h.fail(format!("cannot create {}: {e}", dir.display())));
        return;
    }
    // Whisper's passes must never take the CPU from the screens' frames: IDN runs them in its
    // worker process below normal priority (`Priority::Low`, the default), every thread of it
    // (IDN Beta18, H05-1), on IDN's default thread count. This thread stays at the server's
    // priority: it starts the worker, which would otherwise inherit a lowered nice value on
    // Linux and could not set its own (raising a priority again needs privileges).
    let engine_cfg = idn_captions::EngineConfig {
        lang: cfg.language.clone(),
        words: cfg.words.clone(),
        keep_music: cfg.keep_music,
        // CPU only (see `docs/CAPTIONS-PLAN.md`): built without the crate's GPU feature.
        gpu: false,
        worker: options.worker.clone(),
        ..Default::default()
    };
    // The model: chosen (auto), fetched when missing; a test's file as it is.
    let name = if let Some(file) = &options.model_file {
        file.file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_prefix("ggml-"))
            .and_then(|n| n.strip_suffix(".bin"))
            .unwrap_or("test")
            .to_string()
    } else if cfg.model == "auto" && cfg.language != "en" {
        "large-v3-turbo".to_string()
    } else if cfg.model == "auto" {
        report(&|h| {
            h.set_status(
                CaptionsStatus::Starting,
                Some("choosing a model for this box".into()),
            )
        });
        // The models it tries are downloaded as it goes: the same progress as below. While a
        // download runs (a callback in the last 2 s, not finished), its log lines don't
        // replace the progress in the status.
        let busy: std::cell::Cell<Option<Instant>> = std::cell::Cell::new(None);
        let mut log = |m: &str| {
            tracing::info!("captions: {m}");
            if busy
                .get()
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
            {
                return;
            }
            let m = m.to_string();
            report(&|h| h.set_status(CaptionsStatus::Starting, Some(m.clone())));
        };
        let mut fetch = Download::default();
        let mut progress = |model: &str, n: u64, total: Option<u64>| {
            let done = total.is_some_and(|t| n >= t);
            busy.set((!done).then(Instant::now));
            if fetch.step(model, n, total) {
                report(&|h| h.downloading(model, n, total));
            }
        };
        match idn_captions::auto_choose_stop(&dir, &engine_cfg, &mut log, &mut progress, &stop) {
            Ok(m) => m,
            Err(e) if e == idn_captions::STOPPED => {
                tracing::debug!("captions: stopped while choosing a model");
                return;
            }
            Err(e) => {
                report(&|h| h.fail(format!("no model: {e}")));
                return;
            }
        }
    } else {
        cfg.model.clone()
    };
    if stop.load(Ordering::Relaxed) {
        return;
    }
    let mut fetch = Download::default();
    let path = match &options.model_file {
        Some(file) => Ok(file.clone()),
        None => idn_captions::ensure_model_stop(
            &dir,
            &name,
            &mut |n, total| {
                if fetch.step(&name, n, total) {
                    report(&|h| h.downloading(&name, n, total));
                }
            },
            &stop,
        ),
    };
    let path = match path {
        Ok(p) => p,
        Err(e) if e == idn_captions::STOPPED => {
            tracing::debug!(model = %name, "captions: stopped while fetching the model");
            return;
        }
        Err(e) => {
            report(&|h| h.fail(e.clone()));
            return;
        }
    };
    if stop.load(Ordering::Relaxed) {
        return;
    }
    report(&|h| {
        h.set_status(
            CaptionsStatus::Starting,
            Some(format!("loading the {name} model")),
        )
    });
    // IDN refuses an English-only model for another language (Beta11); the settings are
    // checked as they are saved, so only a file from before that check gets here.
    if !idn_captions::model_speaks(&name, &cfg.language) {
        report(&|h| {
            h.fail(format!(
                "{name} understands English only: choose auto or large-v3-turbo for {}",
                cfg.language
            ))
        });
        return;
    }
    // Loaded in the worker; a load that fails (the worker ended twice, was too slow, or
    // refused the model) is tried again after a while, the error shown meanwhile.
    let mut failed = 0;
    let mut engine = loop {
        match idn_captions::Engine::new(&path, engine_cfg.clone()) {
            Ok(e) => break e,
            Err(e) => {
                let wait = RETRY[failed.min(RETRY.len() - 1)];
                failed += 1;
                report(&|h| {
                    h.fail(format!(
                        "loading the {name} model: {e} (trying again in {} s)",
                        wait.as_secs()
                    ))
                });
                if !wait_unless_stopped(&stop, wait) {
                    return;
                }
                report(&|h| {
                    h.set_status(
                        CaptionsStatus::Starting,
                        Some(format!("loading the {name} model again")),
                    )
                });
            }
        }
    };
    let gathered = Arc::new(Mutex::new(Gathered::default()));
    {
        let (shared, stop, gathered, source) = (
            shared.clone(),
            stop.clone(),
            gathered.clone(),
            cfg.source.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("captions-in".into())
            .spawn(move || reader(shared, source, stop, gathered));
        if let Err(e) = spawned {
            report(&|h| h.fail(format!("no thread: {e}")));
            return;
        }
    }
    report(&|h| {
        let mut s = lock(&h.status);
        s.status = CaptionsStatus::Running;
        s.detail = None;
        s.model = Some(name.clone());
        s.device = Some(engine.device.clone());
        s.load = None;
        drop(s);
        h.changed.notify_one();
    });
    tracing::info!(
        model = %name,
        device = %engine.device,
        in_worker = engine.in_worker,
        priority = engine.priority.name(),
        "captions running"
    );
    // The last step's error, while steps fail (shown and logged once).
    let mut failing: Option<String> = None;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(STEP);
        let (rate, samples) = {
            let mut g = lock(&gathered);
            (g.rate, std::mem::take(&mut g.samples))
        };
        if rate == 0 || samples.is_empty() {
            continue;
        }
        engine.push(&samples, rate);
        match engine.step() {
            Ok(lines) => {
                let load = (engine.load * 100.0).round() / 100.0;
                let recovered = failing.take().is_some();
                if recovered {
                    tracing::info!(device = %engine.device, "captions: whisper works again");
                }
                report(&|h| {
                    for l in &lines {
                        h.line(&l.text, l.is_final);
                    }
                    let mut s = lock(&h.status);
                    // A worker started again may run elsewhere (IDN: on the CPU after one
                    // that ended loading the model on the GPU).
                    let changed = recovered
                        || s.load != Some(load)
                        || s.device.as_deref() != Some(engine.device.as_str());
                    if recovered {
                        s.status = CaptionsStatus::Running;
                        s.detail = None;
                    }
                    s.load = Some(load);
                    s.device = Some(engine.device.clone());
                    drop(s);
                    if changed {
                        h.changed.notify_one();
                    }
                });
            }
            // IDN keeps the engine going: a pass too slow is skipped, a worker that ended is
            // started again (each step an error meanwhile). Shown until a step works again.
            Err(e) => {
                if failing.as_deref() != Some(e.as_str()) {
                    report(&|h| h.fail(format!("whisper: {e}")));
                    failing = Some(e);
                }
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
}

/// Drains the source into `gathered` (mono) until `stop`.
fn reader(
    shared: Weak<Shared>,
    source: CaptionSource,
    stop: Arc<AtomicBool>,
    gathered: Arc<Mutex<Gathered>>,
) {
    // At the lowest priority as the worker (a thread of its own: Windows doesn't pass the
    // worker's on).
    vesper_codex::disk::lower_thread_priority();
    let push = |rate: u32, mono: &[f32]| {
        let mut g = lock(&gathered);
        if g.rate != rate {
            g.samples.clear();
            g.rate = rate;
        }
        g.samples.extend_from_slice(mono);
        let most = (BUFFERED.as_secs_f64() * rate as f64) as usize;
        if g.samples.len() > most {
            let cut = g.samples.len() - most;
            g.samples.drain(..cut);
        }
    };
    match source {
        CaptionSource::Program => {
            let Some(sh) = shared.upgrade() else { return };
            let tap = sh.sound.engine.tap();
            drop(sh);
            let mut mono = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                let Some(b) = tap.recv_timeout(Duration::from_millis(200)) else {
                    continue;
                };
                let ch = b.channels.max(1);
                mono.clear();
                mono.extend(
                    b.data
                        .chunks_exact(ch)
                        .map(|f| f.iter().sum::<f32>() / ch as f32),
                );
                push(media::AUDIO_RATE, &mono);
                tap.recycle(b);
            }
        }
        CaptionSource::External => {}
        CaptionSource::LiveSource { name } => {
            let Some(sh) = shared.upgrade() else { return };
            let rx = sh.live_sources.sound_feed(&name).listen();
            drop(sh);
            let mut touched = Instant::now() - Duration::from_secs(10);
            while !stop.load(Ordering::Relaxed) {
                // Keep the source's receiver open while captions listen to it.
                if touched.elapsed() >= Duration::from_secs(2) {
                    let Some(sh) = shared.upgrade() else { return };
                    sh.live_sources.receiver(&name, None);
                    touched = Instant::now();
                }
                if let Ok((rate, mono)) = rx.recv_timeout(Duration::from_millis(200)) {
                    push(rate, &mono);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F04-4: the language is checked as it is stored (trimmed, lower case).
    #[test]
    fn the_language_is_checked_as_stored() {
        let cfg = |language: &str| CaptionsConfig {
            language: language.into(),
            model: "base.en".into(),
            ..CaptionsConfig::default()
        };
        assert_eq!(Hub::validate(&cfg("EN")), Ok(()));
        assert_eq!(Hub::validate(&cfg(" en ")), Ok(()));
        assert!(
            Hub::validate(&cfg("ES"))
                .unwrap_err()
                .contains("English only")
        );
    }

    /// F04-3 / F04-1: a restarted engine starts with a clean status (no model, device or load
    /// of the one before it), and the stopped worker's thread is kept for the next one to
    /// wait for.
    #[test]
    fn a_restarted_engine_shows_nothing_of_the_last_one() {
        let dir = std::env::temp_dir().join(format!("vesper-captions-re-{}", std::process::id()));
        let hub = Hub::new(&dir);
        let old = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn(|| {});
        *lock(&hub.worker) = Some(Worker {
            stop: old.clone(),
            key: "the last one".into(),
            thread: Some(thread),
        });
        *lock(&hub.status) = Status {
            status: CaptionsStatus::Running,
            model: Some("small.en".into()),
            device: Some("CPU".into()),
            load: Some(0.4),
            ..Status::default()
        };
        hub.apply(&CaptionsConfig {
            enabled: true,
            model: "base.en".into(),
            ..CaptionsConfig::default()
        });
        assert!(old.load(Ordering::Relaxed), "the old worker is stopped");
        assert!(lock(&hub.previous).is_some(), "and waited for");
        let s = hub.state();
        assert_eq!(s.status, CaptionsStatus::Starting);
        assert_eq!((s.model, s.device, s.load), (None, None, None));
    }

    /// A stopped worker ends within about a second, inside IDN's model fetch (Beta11: the
    /// hash check of a model file already there sees the stop), and the file stays under its
    /// name (nothing set aside, no part file). Offline: the file is a sparse one of the pinned
    /// size, so IDN hashes it (no download unless the stop were missed).
    #[test]
    fn a_stopped_worker_ends_within_a_second() {
        let name = "large-v3-turbo";
        let (_, size, _) = idn_captions::MODEL_PINS
            .iter()
            .find(|(n, _, _)| *n == name)
            .copied()
            .expect("a pinned model");
        let dir = std::env::temp_dir().join(format!("vesper-captions-stop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(idn_captions::model_file(name).unwrap());
        std::fs::File::create(&file)
            .and_then(|f| f.set_len(size))
            .unwrap();
        let cfg = CaptionsConfig {
            enabled: true,
            model: name.into(),
            language: "en".into(),
            ..CaptionsConfig::default()
        };
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let (stop, dir) = (stop.clone(), dir.clone());
            std::thread::spawn(move || {
                worker_main(Weak::new(), cfg, stop, dir, CaptionsOptions::default())
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        assert!(!worker.is_finished(), "still checking the model");
        let stopped = Instant::now();
        stop.store(true, Ordering::Relaxed);
        while !worker.is_finished() {
            assert!(
                stopped.elapsed() < Duration::from_millis(1500),
                "the worker ends within about a second of its stop"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        worker.join().unwrap();
        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            left,
            [idn_captions::model_file(name).unwrap()],
            "nothing set aside or half-written"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A model file IDN refuses for the language (an English-only one for Spanish) is said
    /// clearly, before IDN is asked to load it: a file saved before the check.
    #[test]
    fn an_english_only_model_for_another_language_is_refused_clearly() {
        assert!(idn_captions::model_speaks("base.en", "en"));
        assert!(!idn_captions::model_speaks("base.en", "es"));
        assert!(idn_captions::model_speaks("large-v3-turbo", "es"));
        let cfg = CaptionsConfig {
            model: "small.en".into(),
            language: "es".into(),
            ..CaptionsConfig::default()
        };
        assert_eq!(
            Hub::validate(&cfg).unwrap_err(),
            "small.en understands English only: choose auto or large-v3-turbo for es"
        );
    }

    /// A wait ends early when the engine is stopped.
    #[test]
    fn a_retry_wait_ends_at_a_stop() {
        let stop = AtomicBool::new(false);
        assert!(super::wait_unless_stopped(&stop, Duration::from_millis(50)));
        stop.store(true, Ordering::Relaxed);
        let t = Instant::now();
        assert!(!super::wait_unless_stopped(&stop, Duration::from_secs(60)));
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    /// Download callbacks change the state at each new percent (or megabyte, size unknown)
    /// and for each new model, not for every chunk.
    #[test]
    fn download_progress_steps() {
        let mut d = Download::default();
        let mb = 1 << 20;
        assert!(d.step("base.en", 0, Some(100 * mb)));
        assert!(!d.step("base.en", mb / 2, Some(100 * mb)));
        assert!(d.step("base.en", mb, Some(100 * mb)));
        assert!(d.step("small.en", mb, Some(100 * mb)), "another model");
        assert!(d.step("tiny.en", 10, None));
        assert!(!d.step("tiny.en", mb - 1, None));
        assert!(d.step("tiny.en", mb, None), "a megabyte more, size unknown");
    }

    /// The state says how far a download is: percent, bytes and size; other statuses clear
    /// them.
    #[test]
    fn download_progress_in_the_state() {
        let dir = std::env::temp_dir().join(format!("vesper-captions-dl-{}", std::process::id()));
        let hub = Hub::new(&dir);
        lock(&hub.config).enabled = true;
        hub.downloading("small.en", 233_000_000, Some(466_000_000));
        let s = hub.state();
        assert_eq!(s.status, CaptionsStatus::Downloading);
        assert_eq!(s.progress, Some(50));
        assert_eq!(s.downloaded, Some(233_000_000));
        assert_eq!(s.download_size, Some(466_000_000));
        assert_eq!(s.detail.as_deref(), Some("downloading the small.en model"));
        hub.downloading("small.en", 5_000_000, None);
        let s = hub.state();
        assert_eq!(
            (s.progress, s.downloaded, s.download_size),
            (None, Some(5_000_000), None)
        );
        hub.set_status(
            CaptionsStatus::Starting,
            Some("loading the small.en model".into()),
        );
        let s = hub.state();
        assert_eq!(
            (s.progress, s.downloaded, s.download_size),
            (None, None, None)
        );
    }
}
