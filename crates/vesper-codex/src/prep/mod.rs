//! Media preparation (`docs/MEDIA-PREP-PLAN.md`): background jobs on the Codex box that make
//! uploaded videos and sounds easy to play live.
//!
//! After an upload (or a relink) is probed, the media worker queues the asset here (when
//! automatic preparation is on: the server's [`super::CodexOptions::media_prep`] and the
//! setting [`MediaPrepSettings::enabled`]); Oriel's "Prepare" queues one by hand. A job:
//!
//! 1. **Analyses** the file at the lowest CPU priority: its facts (IDN's probe: codec, colour,
//!    the average and base frame rates, the field order, the bit rate), its **loudness** (EBU
//!    R128, through IDN's audio reader) and the **gain** to the house target (stored with the
//!    asset, applied at playback, never baked in), and its **poster frame** (the first of a few
//!    frames that isn't black; the stored thumbnail is made from it too). Saved at once.
//! 2. **Converts** a video that is hard to play live ([`plan::plan_with`]: a codec or colour
//!    format the GPU doesn't decode, interlaced, variable frame rate, a huge bit rate, larger
//!    than 4K or the size limit, above 60 fps, sound that isn't at 48 kHz) to the house format:
//!    H.264 8-bit 4:2:0 (or HEVC, a setting), constant frame rate, AAC 48 kHz stereo, MP4, at
//!    the quality, size and encoder the settings say. IDN's converter does it (`media::convert`,
//!    IDN's `media::convert`, on the machine's FFmpeg libraries) in a **child process**
//!    ([`child`]: `vesper media-convert`, so a crash inside FFmpeg fails only the job) at the
//!    job's priority; no `ffmpeg` tool is needed. The copy goes into the media library next to
//!    the original; the asset then plays the copy and keeps the original (`prep.original`, still
//!    in the library and marked in use) to go back to.
//!
//! **While live** ([`PrepWhileLive`]): by default a conversion goes on at the lowest priority
//! while something is live on this box (a running one is lowered), so media added during a
//! service still get ready; the setting `wait` stops a running one instead and starts it again
//! once nothing is live. Analysis always runs (decoding only, seconds, lowest priority).
//!
//! Each file is prepared once: `prep.analysed` holds the SHA-256 of the file analysed.
//!
//! Jobs not finished are kept in the Codex's `media-prep-queue.json`, so a restart (an update,
//! a shutdown before the box was ever idle) picks them up again.

pub mod child;
pub mod plan;

use crate::Shared;
use crate::Show;
use model::{
    AssetLocation, ChangeBy, CollectionId, MediaAsset, MediaAssetId, MediaKind, MediaPrep,
    PreparedOriginal,
};
use protocol::{
    MediaPrepJob, MediaPrepSettings, MediaPrepStatus, PrepEncoder, PrepPriority, PrepStep,
    PrepWhileLive,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use vesper_core::lock;

/// Threads started (the setting `maxJobs` uses up to this many).
const THREADS: usize = 4;
/// Finished jobs stay in the list this long.
const KEEP_DONE: Duration = Duration::from_secs(60);

struct Job {
    asset: MediaAssetId,
    collection: CollectionId,
    name: String,
    step: PrepStep,
    progress: f64,
    error: Option<String>,
    again: bool,
    finished: Option<Instant>,
    /// Its conversion, while it waits for nothing to be live (step `WaitingForLive`).
    waiting: Option<Box<Conversion>>,
}

impl Job {
    fn open(&self) -> bool {
        !matches!(self.step, PrepStep::Done | PrepStep::Failed)
    }
}

struct Inner {
    jobs: Mutex<Vec<Job>>,
    wake: Condvar,
    settings: Mutex<MediaPrepSettings>,
    running: Mutex<usize>,
    /// Automatic preparation allowed on this server.
    auto: bool,
    stop: tokio::sync::watch::Receiver<bool>,
    file: Mutex<Option<PathBuf>>,
    /// Where the jobs not finished are kept.
    queue_file: PathBuf,
    /// How conversions run (in a child process of the server's own, normally).
    converter: child::Converter,
    /// Starts the child processes from a thread at the server's own priority (the job threads
    /// run at the lowest, which a child would inherit on Linux and couldn't raise for the
    /// setting `priority: normal`).
    spawner: Option<Mutex<std::sync::mpsc::Sender<Spawn>>>,
}

/// A child process start, run on the spawner thread.
type Spawn = Box<dyn FnOnce() + Send>;

/// A job not finished, as `media-prep-queue.json` keeps it.
#[derive(serde::Serialize, serde::Deserialize)]
struct Pending {
    asset: MediaAssetId,
    collection: CollectionId,
    #[serde(default)]
    again: bool,
}

/// The preparation jobs' handle (cheap to clone).
#[derive(Clone)]
pub struct Prep {
    inner: Arc<Inner>,
}

fn settings_path(sh: &Shared) -> PathBuf {
    sh.codex().layout.codex().join("media-prep.json")
}

fn queue_path(sh: &Shared) -> PathBuf {
    sh.codex().layout.codex().join("media-prep-queue.json")
}

/// Keeps settings in their ranges.
pub fn clamp(mut s: MediaPrepSettings) -> MediaPrepSettings {
    if !s.loudness_target.is_finite() {
        s.loudness_target = MediaPrepSettings::default().loudness_target;
    }
    s.loudness_target = s.loudness_target.clamp(-30.0, -5.0);
    s.max_jobs = s.max_jobs.clamp(1, THREADS as u32);
    s.video_kbps = s.video_kbps.map(|k| k.clamp(1_000, 60_000));
    s
}

impl Prep {
    /// Starts the job threads (`auto`: automatic preparation allowed on this server;
    /// `converter`: how conversions run). Called from a thread at the server's own priority
    /// (the spawner of conversions is started from it).
    pub fn start(
        sh: Arc<Shared>,
        show: Arc<dyn Show>,
        auto: bool,
        converter: child::Converter,
        stop: tokio::sync::watch::Receiver<bool>,
    ) -> Prep {
        let spawner = converter.program.as_ref().and_then(|_| {
            let (tx, rx) = std::sync::mpsc::channel::<Spawn>();
            std::thread::Builder::new()
                .name("media-convert-spawn".into())
                .spawn(move || {
                    for f in rx {
                        f();
                    }
                })
                .map_err(|e| tracing::error!("starting the conversions' spawner: {e}"))
                .ok()
                .map(|_| Mutex::new(tx))
        });
        let inner = Arc::new(Inner {
            converter,
            spawner,
            jobs: Mutex::new(Vec::new()),
            wake: Condvar::new(),
            settings: Mutex::new(MediaPrepSettings::default()),
            running: Mutex::new(0),
            auto,
            stop,
            file: Mutex::new(None),
            queue_file: queue_path(&sh),
        });
        // The jobs a restart cut off.
        let pending: Vec<Pending> = std::fs::read(&inner.queue_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let me = Prep {
            inner: inner.clone(),
        };
        for p in pending {
            if let Some(a) = sh
                .collection_content(p.collection)
                .and_then(|c| c.assets.get(&p.asset).cloned())
            {
                tracing::info!(asset = %a.id.0, "media preparation: picking up a job a restart cut off");
                me.queue(p.collection, &a, p.again);
            }
        }
        me.persist();
        let rt = tokio::runtime::Handle::current();
        for i in 0..THREADS {
            let (inner, sh, show, rt) = (inner.clone(), sh.clone(), show.clone(), rt.clone());
            let spawned = std::thread::Builder::new()
                .name(format!("media-prep-{i}"))
                .spawn(move || run(&inner, &sh, &*show, &rt));
            if let Err(e) = spawned {
                tracing::error!("starting a media preparation thread: {e}");
            }
        }
        Prep { inner }
    }

    /// The settings (read from the Codex's `media-prep.json` the first time).
    pub fn settings(&self, sh: &Shared) -> MediaPrepSettings {
        let path = settings_path(sh);
        let mut f = lock(&self.inner.file);
        if f.as_ref() != Some(&path) {
            let s = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<MediaPrepSettings>(&b).ok())
                .map(clamp)
                .unwrap_or_default();
            *lock(&self.inner.settings) = s;
            *f = Some(path);
        }
        lock(&self.inner.settings).clone()
    }

    /// Saves new settings.
    pub fn set_settings(
        &self,
        sh: &Shared,
        s: MediaPrepSettings,
    ) -> std::io::Result<MediaPrepSettings> {
        let s = clamp(s);
        let path = settings_path(sh);
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let json = serde_json::to_vec_pretty(&s).map_err(std::io::Error::other)?;
        crate::store::write_atomic(&path, &json).map_err(std::io::Error::other)?;
        *lock(&self.inner.settings) = s.clone();
        *lock(&self.inner.file) = Some(path);
        self.inner.wake.notify_all();
        Ok(s)
    }

    /// After a probe: queues the asset when automatic preparation is on and its file wasn't
    /// prepared yet.
    pub fn after_probe(&self, sh: &Shared, collection: CollectionId, a: &MediaAsset) {
        if !self.inner.auto || !self.settings(sh).enabled {
            return;
        }
        if !matches!(a.kind, MediaKind::Video | MediaKind::Audio) {
            return;
        }
        let key = file_key(&a.location);
        if a.prep.as_ref().and_then(|p| p.analysed.as_deref()) == key.as_deref() && key.is_some() {
            return;
        }
        // A file that failed is tried again only by hand.
        if a.prep.as_ref().is_some_and(|p| p.error.is_some()) {
            return;
        }
        self.queue(collection, a, false);
    }

    /// Queues `a` (`again`: prepare it even if it was prepared).
    pub fn queue(&self, collection: CollectionId, a: &MediaAsset, again: bool) {
        let mut jobs = lock(&self.inner.jobs);
        if let Some(j) = jobs.iter_mut().find(|j| j.asset == a.id) {
            if j.open() {
                j.again |= again;
                return;
            }
            *j = Job {
                asset: a.id,
                collection,
                name: a.name.clone(),
                step: PrepStep::Queued,
                progress: 0.0,
                error: None,
                again,
                finished: None,
                waiting: None,
            };
        } else {
            jobs.push(Job {
                asset: a.id,
                collection,
                name: a.name.clone(),
                step: PrepStep::Queued,
                progress: 0.0,
                error: None,
                again,
                finished: None,
                waiting: None,
            });
        }
        self.persist_jobs(&jobs);
        drop(jobs);
        self.inner.wake.notify_all();
    }

    /// Keeps the jobs not finished in `media-prep-queue.json`.
    fn persist(&self) {
        let jobs = lock(&self.inner.jobs);
        self.persist_jobs(&jobs);
    }

    fn persist_jobs(&self, jobs: &[Job]) {
        let pending: Vec<Pending> = jobs
            .iter()
            .filter(|j| j.open())
            .map(|j| Pending {
                asset: j.asset,
                collection: j.collection,
                again: j.again,
            })
            .collect();
        let path = &self.inner.queue_file;
        let r = if pending.is_empty() {
            match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        } else {
            serde_json::to_vec(&pending)
                .map_err(|e| e.to_string())
                .and_then(|b| crate::store::write_atomic(path, &b).map_err(|e| e.to_string()))
        };
        if let Err(e) = r {
            tracing::warn!(
                "keeping the media preparation queue in {}: {e}",
                path.display()
            );
        }
    }

    /// The settings, the jobs and what this box can do.
    pub fn status(&self, sh: &Shared) -> MediaPrepStatus {
        let settings = self.settings(sh);
        let live = !sh.live_items().is_empty();
        let encoder = encoder_for(&settings, live).map(|(_, e)| e);
        let (gpu_encoder, cpu_encoder) = media::convert::encoders(hevc(&settings));
        let mut jobs = lock(&self.inner.jobs);
        jobs.retain(|j| {
            j.finished
                .is_none_or(|t| t.elapsed() < KEEP_DONE || j.step == PrepStep::Failed)
        });
        MediaPrepStatus {
            jobs: jobs
                .iter()
                .map(|j| MediaPrepJob {
                    asset: j.asset,
                    collection: j.collection,
                    name: j.name.clone(),
                    step: j.step,
                    progress: j.progress,
                    error: j.error.clone(),
                })
                .collect(),
            converter: Some(format!("IDN media conversion, {}", media::ffmpeg_version())),
            encoder,
            gpu_encoder,
            cpu_encoder,
            codec_encoders: Some(protocol::CodecEncoders {
                h264: box_encoders(false),
                hevc: box_encoders(true),
            }),
            live,
            settings,
        }
    }

    /// Jobs not finished (tests wait for zero).
    pub fn open_jobs(&self) -> usize {
        lock(&self.inner.jobs).iter().filter(|j| j.open()).count()
    }

    /// Starts a conversion's child process from the spawner thread (at the server's own
    /// priority).
    fn spawn(
        &self,
        program: PathBuf,
        job: child::Job,
        config: PathBuf,
    ) -> Result<child::Running, media::convert::ConvertError> {
        let failed = |e: String| {
            media::convert::ConvertError::Failed(format!("starting the converter: {e}"))
        };
        let Some(spawner) = &self.inner.spawner else {
            return child::Running::spawn(&program, &job, &config)
                .map_err(|e| failed(e.to_string()));
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let start: Spawn = Box::new(move || {
            let _ = tx.send(child::Running::spawn(&program, &job, &config));
        });
        lock(spawner)
            .send(start)
            .map_err(|_| failed("the spawner stopped".into()))?;
        rx.recv()
            .map_err(|_| failed("the spawner stopped".into()))?
            .map_err(|e| failed(e.to_string()))
    }

    fn set(&self, asset: MediaAssetId, f: impl FnOnce(&mut Job)) {
        if let Some(j) = lock(&self.inner.jobs).iter_mut().find(|j| j.asset == asset) {
            f(j);
        }
    }
}

/// What identifies a file's bytes: its SHA-256 (library and old-store files), else where it is.
pub fn file_key(location: &AssetLocation) -> Option<String> {
    match location {
        AssetLocation::Library { sha256, .. } | AssetLocation::Managed { sha256, .. } => {
            Some(sha256.to_ascii_lowercase())
        }
        AssetLocation::Linked { root, path } => Some(format!("{}/{path}", root.0)),
        _ => None,
    }
}

fn run(inner: &Arc<Inner>, sh: &Arc<Shared>, show: &dyn Show, rt: &tokio::runtime::Handle) {
    super::disk::lower_thread_priority();
    let me = Prep {
        inner: inner.clone(),
    };
    loop {
        let (collection, asset, again, waiting) = loop {
            if *inner.stop.borrow() {
                return;
            }
            // A waiting conversion goes on once it may (asked before the jobs' lock).
            let resumable = !waits(inner) || sh.live_items().is_empty();
            let mut jobs = lock(&inner.jobs);
            let max = lock(&inner.settings).max_jobs.max(1) as usize;
            let mut running = lock(&inner.running);
            if *running < max
                && let Some(j) = jobs.iter_mut().find(|j| {
                    j.step == PrepStep::Queued || (resumable && j.step == PrepStep::WaitingForLive)
                })
            {
                let waiting = j.waiting.take();
                j.step = if waiting.is_some() {
                    PrepStep::Converting
                } else {
                    PrepStep::Analysing
                };
                j.progress = 0.0;
                *running += 1;
                break (j.collection, j.asset, j.again, waiting);
            }
            drop(running);
            let _ = inner.wake.wait_timeout(jobs, Duration::from_millis(500));
        };
        let r = match waiting {
            Some(c) => convert(&me, sh, show, rt, collection, asset, c),
            None => prepare(&me, sh, show, rt, collection, asset, again),
        };
        {
            let mut running = lock(&inner.running);
            *running = running.saturating_sub(1);
        }
        match r {
            Ok(Prepared::Done) => me.set(asset, |j| {
                j.step = PrepStep::Done;
                j.progress = 1.0;
                j.finished = Some(Instant::now());
            }),
            // Listed as waiting; a thread takes it up again once nothing is live.
            Ok(Prepared::Waits(c)) => me.set(asset, |j| {
                j.step = PrepStep::WaitingForLive;
                j.progress = 0.0;
                j.waiting = Some(c);
            }),
            Err(e) if e == STOPPED => return,
            Err(e) => {
                tracing::info!(asset = %asset.0, "media preparation failed: {e}");
                save_error(sh, show, rt, collection, asset, &e);
                me.set(asset, |j| {
                    j.step = PrepStep::Failed;
                    j.error = Some(e);
                    j.finished = Some(Instant::now());
                });
            }
        }
        me.persist();
        inner.wake.notify_all();
    }
}

const STOPPED: &str = "the server is stopping";

fn by(sh: &Shared) -> ChangeBy {
    ChangeBy {
        client: "media preparation".into(),
        client_id: None,
        machine: sh.auth().identity().id.to_string(),
    }
}

/// Saves `change` into the asset unless its file moved meanwhile (`at`: the location the job
/// started from).
fn save(
    sh: &Arc<Shared>,
    show: &dyn Show,
    rt: &tokio::runtime::Handle,
    collection: CollectionId,
    asset: MediaAssetId,
    at: &AssetLocation,
    change: impl FnOnce(&mut MediaAsset),
) -> Result<bool, String> {
    let mut saved = false;
    rt.block_on(sh.codex_writer().media(
        sh,
        show,
        collection,
        |content| {
            let mut plan = super::writer::MediaPlan::default();
            if let Some(cur) = content.assets.get(&asset)
                && cur.location == *at
            {
                let mut cur = cur.clone();
                change(&mut cur);
                plan.assets.push(cur);
                saved = true;
            }
            Ok(plan)
        },
        None,
        by(sh),
    ))
    .map_err(|e| e.error.message)?;
    Ok(saved)
}

fn save_error(
    sh: &Arc<Shared>,
    show: &dyn Show,
    rt: &tokio::runtime::Handle,
    collection: CollectionId,
    asset: MediaAssetId,
    e: &str,
) {
    let Some(at) = sh
        .collection_content(collection)
        .and_then(|c| c.assets.get(&asset).map(|a| a.location.clone()))
    else {
        return;
    };
    let _ = save(sh, show, rt, collection, asset, &at, |a| {
        let p = a.prep.get_or_insert_with(MediaPrep::default);
        p.error = Some(e.to_string());
        p.at = Some(now());
    });
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// This box's encoders for H.264 or (`hevc`) HEVC, for people.
fn box_encoders(hevc: bool) -> protocol::BoxEncoders {
    let (gpu, cpu) = media::convert::encoders(hevc);
    protocol::BoxEncoders { gpu, cpu }
}

/// Conversions to HEVC (the setting `video`).
fn hevc(s: &MediaPrepSettings) -> bool {
    s.video == protocol::HouseVideoCodec::Hevc
}

/// The encoder a conversion uses with settings `s` (`live`: something is live now): whether
/// it is the GPU's, and its name for people. `None`: none here for the chosen codec and
/// encoder. "Auto" takes the GPU's where there is one, but while something is live the CPU's
/// where there is one for the codec (at the lowest priority it yields to live work; a GPU's
/// encoder has no priority to give).
fn encoder_for(s: &MediaPrepSettings, live: bool) -> Option<(bool, String)> {
    let (gpu, cpu) = media::convert::encoders(hevc(s));
    match s.encoder {
        PrepEncoder::Auto if live && cpu.is_some() => cpu.map(|c| (false, c)),
        PrepEncoder::Auto => gpu.map(|g| (true, g)).or(cpu.map(|c| (false, c))),
        PrepEncoder::Gpu => gpu.map(|g| (true, g)),
        PrepEncoder::Cpu => cpu.map(|c| (false, c)),
    }
}

/// The priority a conversion starts at (`live`: something is live now).
fn level_for(s: &MediaPrepSettings, live: bool) -> child::Level {
    if live {
        child::Level::Lowest
    } else if s.priority == PrepPriority::Normal {
        child::Level::Normal
    } else {
        child::Level::Low
    }
}

/// The facts of a file: IDN's probe.
fn facts(path: &Path, kind: MediaKind) -> Result<plan::Facts, String> {
    let audio = media::probe_audio(path).map_err(|e| format!("{e:#}"))?;
    let mut f = plan::Facts {
        audio_codec: audio.as_ref().map(|a| a.codec.clone()),
        audio_rate: audio.as_ref().map(|a| a.sample_rate),
        audio_channels: audio.as_ref().map(|a| a.channels),
        gpu: media::hardware_available(),
        ..plan::Facts::default()
    };
    if kind == MediaKind::Video {
        let v = media::probe(path).map_err(|e| format!("{e:#}"))?;
        f.video = true;
        f.codec = v.codec;
        f.width = v.width;
        f.height = v.height;
        f.rotation = v.rotation;
        f.fps = if v.avg_fps > 0.0 { v.avg_fps } else { v.fps };
        f.base_fps = Some(v.base_fps).filter(|r| *r > 0.0);
        f.variable_rate = v.variable_rate;
        f.pixel_format = v.pixel_format;
        f.bit_depth = v.bit_depth;
        f.alpha = v.alpha;
        // Known only where the file states its field order.
        f.interlaced = v.field_order.is_some().then_some(v.interlaced);
        f.gpu_decodes = v.hw;
        f.bit_rate = v.file_bit_rate.or(v.bit_rate).or_else(|| {
            let len = std::fs::metadata(path).ok()?.len();
            v.duration
                .filter(|d| *d > 0.5)
                .map(|d| (len as f64 * 8.0 / d) as u64)
        });
    }
    Ok(f)
}

fn prepare(
    me: &Prep,
    sh: &Arc<Shared>,
    show: &dyn Show,
    rt: &tokio::runtime::Handle,
    collection: CollectionId,
    asset: MediaAssetId,
    again: bool,
) -> Result<Prepared, String> {
    let stop = me.inner.stop.clone();
    let stopped = || *stop.borrow();
    let content = sh
        .collection_content(collection)
        .ok_or("the collection isn't there any more")?;
    let a = content
        .assets
        .get(&asset)
        .cloned()
        .ok_or("the media isn't there any more")?;
    if !matches!(a.kind, MediaKind::Video | MediaKind::Audio) {
        return Ok(Prepared::Done);
    }
    let key = file_key(&a.location);
    let prep = a.prep.clone().unwrap_or_default();
    if !again && key.is_some() && prep.analysed == key {
        return Ok(Prepared::Done);
    }
    // Prepared again: from the original.
    let original = prep.original.clone().filter(|_| again);
    let src = match &original {
        Some(o) => resolve(sh, &content, &o.location)?,
        None => content.asset_path(asset)?,
    };
    let settings = me.settings(sh);

    // 1. Analysis.
    let f = facts(&src, a.kind)?;
    let loud = media::measure_loudness(&src, &stopped).map_err(|e| {
        if stopped() {
            STOPPED.to_string()
        } else {
            format!("measuring the loudness: {e:#}")
        }
    })?;
    if stopped() {
        return Err(STOPPED.into());
    }
    let poster = (a.kind == MediaKind::Video)
        .then(|| media::poster_frame(&src, 160).ok().map(|t| t.at))
        .flatten();
    let gain = loud.and_then(|l| media::gain_to_target(&l, settings.loudness_target));
    let p = plan::plan_with(&f, settings.max_size);
    let live = || !sh.live_items().is_empty();
    let encoder = encoder_for(&settings, live());
    let converts = p.converts() && settings.transcode;
    let mut notes = p.notes.clone();
    if p.converts() && !settings.transcode {
        notes.push(format!(
            "Not converted (conversion is off): {}",
            p.reasons.join(", ")
        ));
    } else if converts && p.video.is_some() && encoder.is_none() {
        notes.push(format!(
            "Not converted: this box has no {}{} encoder ({})",
            match settings.encoder {
                PrepEncoder::Auto => "",
                PrepEncoder::Gpu => "GPU ",
                PrepEncoder::Cpu => "CPU ",
            },
            match settings.video {
                protocol::HouseVideoCodec::H264 => "H.264",
                protocol::HouseVideoCodec::Hevc => "HEVC",
            },
            p.reasons.join(", ")
        ));
    }
    let will_convert = converts && (p.video.is_none() || encoder.is_some());
    let analysed_key = if will_convert { None } else { key.clone() };
    save(sh, show, rt, collection, asset, &a.location, |cur| {
        let q = cur.prep.get_or_insert_with(MediaPrep::default);
        q.loudness = loud.and_then(|l| l.integrated).map(round1);
        q.peak = loud.and_then(|l| l.peak).map(round1);
        q.gain_db = gain;
        q.poster_at = poster.map(round1);
        q.notes = notes.clone();
        q.error = None;
        q.at = Some(now());
        if analysed_key.is_some() {
            q.analysed = analysed_key.clone();
        }
    })?;
    if !will_convert {
        return Ok(Prepared::Done);
    }
    convert(
        me,
        sh,
        show,
        rt,
        collection,
        asset,
        Box::new(Conversion {
            a,
            src,
            original,
            f,
            p,
            settings,
        }),
    )
}

/// A conversion its analysis found needed, kept while it waits for nothing to be live.
struct Conversion {
    /// The asset as the job started (its location: the file analysed).
    a: MediaAsset,
    /// The file converted.
    src: PathBuf,
    /// Prepared again: the original (the source).
    original: Option<PreparedOriginal>,
    f: plan::Facts,
    p: plan::Plan,
    settings: MediaPrepSettings,
}

/// How a job's run ended (other than with an error).
enum Prepared {
    Done,
    /// The conversion waits until nothing is live (the setting `wait`): the job is listed as
    /// waiting and holds neither a slot nor a thread meanwhile (H02-3).
    Waits(Box<Conversion>),
}

/// Waits are asked of conversions now (the setting `whileLive: wait`).
fn waits(inner: &Inner) -> bool {
    lock(&inner.settings).while_live == PrepWhileLive::Wait
}

/// Step 2 and 3: converts the file (unless it has to wait) and the asset plays the copy.
fn convert(
    me: &Prep,
    sh: &Arc<Shared>,
    show: &dyn Show,
    rt: &tokio::runtime::Handle,
    collection: CollectionId,
    asset: MediaAssetId,
    c: Box<Conversion>,
) -> Result<Prepared, String> {
    let live = || !sh.live_items().is_empty();
    // Read now: changed to "go on at the lowest priority", a waiting conversion goes on.
    let waits = || {
        me.settings(sh);
        waits(&me.inner)
    };
    if waits() && live() {
        return Ok(Prepared::Waits(c));
    }
    if convert_now(me, sh, show, rt, collection, asset, &c, &waits)? {
        Ok(Prepared::Done)
    } else {
        Ok(Prepared::Waits(c))
    }
}

/// Converts `c` now: `true` when the asset plays the copy; `false`: stopped because something
/// went live (the setting `wait`), to go on later.
#[allow(clippy::too_many_arguments)]
fn convert_now(
    me: &Prep,
    sh: &Arc<Shared>,
    show: &dyn Show,
    rt: &tokio::runtime::Handle,
    collection: CollectionId,
    asset: MediaAssetId,
    c: &Conversion,
    waits: &dyn Fn() -> bool,
) -> Result<bool, String> {
    let Conversion {
        a,
        src,
        original,
        f,
        p,
        settings,
    } = c;
    let stop = me.inner.stop.clone();
    let stopped = || *stop.borrow();
    let live = || !sh.live_items().is_empty();

    // 2. Conversion.
    let work = sh
        .codex()
        .layout
        .codex()
        .join("prep")
        .join(asset.0.to_string());
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    // Removed however the job ends (a failed conversion's partial copy can be gigabytes).
    let _work = RemoveDir(work.clone());
    let file_name = src
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&a.name)
        .to_string();
    let name = plan::prepared_name(&file_name, f.video);
    let out = work.join(&name);
    let mut options = media::convert::ConvertOptions {
        hevc: hevc(settings),
        frame_rate: p.video.as_ref().map(|v| v.rate),
        max_size: p.video.as_ref().and_then(|v| v.max_size),
        deinterlace_all: false,
        video_kbps: p.video.as_ref().map(|v| {
            // By the converted picture's size (upright, fitted to the limit).
            let (w, h) = plan::output_size(f, v);
            plan::video_kbps(
                w,
                h,
                v.rate,
                settings.video,
                settings.quality,
                settings.video_kbps,
            )
        }),
        gpu: true,
        low_priority: true,
    };
    me.set(asset, |j| {
        j.step = PrepStep::Converting;
        j.progress = 0.0;
    });
    // The encoder (for the job's codec) and the priority as things are now.
    let live_now = live();
    if p.video.is_some() {
        match encoder_for(settings, live_now) {
            Some((gpu, _)) => options.gpu = gpu,
            None => return Err("no encoder for the house format on this box now".into()),
        }
    }
    let level = level_for(&me.settings(sh), live_now);
    let progress = |x: f64| me.set(asset, |j| j.progress = x.clamp(0.0, 0.99));
    let r = if let Some(program) = me.inner.converter.program.clone() {
        let mut job = child::Job::new(src, &out, &options, level);
        job.test = me.inner.converter.test.clone();
        me.spawn(program, job, work.join("convert.json"))
            .and_then(|running| running.wait(&|| stopped() || (waits() && live()), &live, progress))
    } else {
        options.low_priority = level != child::Level::Normal;
        convert_while(
            src,
            &out,
            &options,
            &|| stopped() || (waits() && live()),
            progress,
        )
    };
    let made = match r {
        Ok(made) => made,
        Err(media::convert::ConvertError::Cancelled) if stopped() => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(STOPPED.into());
        }
        // Something went live: it waits.
        Err(media::convert::ConvertError::Cancelled) => return Ok(false),
        // The server is stopping (whatever ended the child, a service stop's signal
        // reaching it first): not the file's fault; kept for the next start (R1-1).
        Err(media::convert::ConvertError::Failed(_)) if stopped() => {
            return Err(STOPPED.into());
        }
        Err(media::convert::ConvertError::Failed(e)) => return Err(e),
    };
    tracing::info!(
        asset = %asset.0,
        encoder = %made.encoder,
        frames = made.frames,
        repeated = made.repeated,
        dropped = made.dropped,
        deinterlaced = made.deinterlaced,
        "media preparation: converted in {:.1} s",
        made.took.as_secs_f64()
    );

    // 3. Into the library, and the asset plays the copy.
    let folder = sh
        .codex()
        .collection(collection)
        .map(|c| super::library::folder_for(&c.collection.name, c.collection.id))
        .unwrap_or_default();
    let (item, sha, existed) = sh
        .library()
        .import(&out, &folder)
        .map_err(|e| format!("taking the converted copy into the media library: {e}"))?;
    let _ = std::fs::remove_dir_all(&work);
    sh.library().add_marks(std::slice::from_ref(&item));
    let _ = existed;
    let new_path = sh
        .library()
        .resolve(&item)
        .map_err(|e| format!("the converted copy: {e}"))?;
    let location = AssetLocation::Library {
        id: item,
        sha256: sha.clone(),
    };
    let probe = super::worker::probe_file(&new_path, a.kind)?;
    if let Some(file) = super::blobs::stored_name(&location, &new_path) {
        let _ = super::worker::stored_thumbnail(&sh.codex().layout.codex(), &file, &new_path);
    }
    let format = plan::format_line(
        settings.video,
        f.video
            .then_some(((made.width, made.height), made.frame_rate)),
        made.audio,
    );
    let reasons = p.reasons.clone();
    let made_notes = made.notes.clone();
    let saved = save(sh, show, rt, collection, asset, &a.location, |cur| {
        let orig = match original {
            Some(o) => o.clone(),
            None => PreparedOriginal {
                location: cur.location.clone(),
                probe: cur.probe.clone(),
                reasons: Vec::new(),
                format: String::new(),
            },
        };
        let q = cur.prep.get_or_insert_with(MediaPrep::default);
        q.original = Some(PreparedOriginal {
            reasons: reasons.clone(),
            format: format.clone(),
            ..orig
        });
        q.analysed = Some(sha.clone());
        q.at = Some(now());
        for n in &made_notes {
            if !q.notes.contains(n) {
                q.notes.push(n.clone());
            }
        }
        cur.location = location.clone();
        cur.probe = Some(probe.clone());
    })?;
    if !saved {
        return Err("the media changed while it was converted; prepare it again".into());
    }
    Ok(true)
}

/// Converts `src` into `out` on a thread of its own (IDN's converter blocks for the whole
/// conversion), cancelling it once `stop` says so (asked every quarter second) and giving
/// its progress (0 to 1).
fn convert_while(
    src: &Path,
    out: &Path,
    options: &media::convert::ConvertOptions,
    stop: &dyn Fn() -> bool,
    mut progress: impl FnMut(f64),
) -> Result<media::convert::Converted, media::convert::ConvertError> {
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel::<f64>();
    let (src, out, options, c) = (
        src.to_path_buf(),
        out.to_path_buf(),
        options.clone(),
        cancel.clone(),
    );
    let worker = std::thread::Builder::new()
        .name("media-convert".into())
        .spawn(move || {
            media::convert::convert(
                &src,
                &out,
                &options,
                |p| {
                    if let Some(x) = p.fraction {
                        let _ = tx.send(x);
                    }
                },
                &c,
            )
        })
        .map_err(|e| media::convert::ConvertError::Failed(format!("starting a conversion: {e}")))?;
    while !worker.is_finished() {
        if stop() {
            cancel.store(true, Ordering::Relaxed);
        }
        if let Ok(x) = rx.recv_timeout(Duration::from_millis(250)) {
            progress(x);
        }
        while let Ok(x) = rx.try_recv() {
            progress(x);
        }
    }
    worker.join().unwrap_or_else(|_| {
        Err(media::convert::ConvertError::Failed(
            "the conversion stopped unexpectedly".into(),
        ))
    })
}

/// A folder removed when dropped.
struct RemoveDir(PathBuf);

impl Drop for RemoveDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// The file of a location that isn't the asset's own now (its original).
fn resolve(
    sh: &Shared,
    content: &crate::data::CollectionContent,
    location: &AssetLocation,
) -> Result<PathBuf, String> {
    match location {
        AssetLocation::Library { id, .. } => sh.library().resolve(id),
        AssetLocation::Managed { sha256, extension } => {
            Ok(content.media_dir.join(format!("{sha256}.{extension}")))
        }
        AssetLocation::Linked { root, path } if crate::relink::stays_below(path) => content
            .media_roots
            .get(root)
            .map(|r| r.join(path))
            .ok_or_else(|| "the original's media root isn't bound".to_string()),
        _ => Err("the original can't be found".into()),
    }
}
