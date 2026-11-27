//! Machine translation of second-language slides (`docs/DUAL-LANGUAGE-PLAN.md` §5): this box's
//! translation model store and its translation previews.
//!
//! The engine, the store and the catalogue are in the `vesper-translate` crate. Here: the
//! store at `<data>/cache/translate` (the data folder's cache, not in backups), previews as
//! jobs on their own threads (one translating at a time; the model fetched first when it is
//! not on the box), and the loaded model dropped after [`vesper_translate::marian::IDLE`].
//! A preview's answer is never saved: the operator keeps it with `setSlideTranslation`.

use protocol::{
    TranslateJob, TranslateJobStatus, TranslatedSlide, TranslationModel, TranslationModelStatus,
    TranslationModels,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use vesper_translate::NOTICE;
use vesper_translate::catalogue;
pub use vesper_translate::catalogue::Pair;
use vesper_translate::marian::Marian;
use vesper_translate::store::{State, Store};
// What tests (and other engines) need to stand in for the real ones.
pub use vesper_translate::Translator;
pub use vesper_translate::catalogue::{ModelFile, Target};
pub use vesper_translate::store::{Source, sha256};

/// How long an ended preview is kept for its client to read.
const KEEP_ENDED: Duration = Duration::from_secs(10 * 60);
/// The most previews kept (running and ended). Only open ones (still running) count against
/// it: when it is full, the oldest ended ones make room.
pub const MAX_JOBS: usize = 32;
/// The most slides one preview translates.
pub const MAX_SLIDES: usize = 500;

/// Replacements for tests (all `None`: OPUS-MT from Hugging Face).
#[derive(Clone, Default)]
pub struct TranslateOptions {
    /// The engine (tests pass a fake one).
    pub engine: Option<Arc<dyn Translator>>,
    /// The pairs offered (tests pass made-up ones).
    pub pairs: Option<Vec<Pair>>,
    /// Where model files come from (tests serve them from memory).
    pub source: Option<Arc<dyn Source>>,
}

impl std::fmt::Debug for TranslateOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TranslateOptions")
            .field("engine", &self.engine.is_some())
            .field("pairs", &self.pairs.as_ref().map(Vec::len))
            .field("source", &self.source.is_some())
            .finish()
    }
}

/// Where the models are kept.
pub fn models_dir(root: &Path) -> PathBuf {
    root.join("cache").join("translate")
}

struct Job {
    job: TranslateJob,
    cancel: Arc<AtomicBool>,
    ended: Option<Instant>,
}

/// This box's translation: the model store, the engine and the previews.
pub struct Translation {
    store: Arc<Store>,
    engine: Arc<dyn Translator>,
    marian: Option<Arc<Marian>>,
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    /// Held while a preview translates: one at a time.
    running: Arc<Mutex<()>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What a preview translates: the slides (already read from the presentation).
pub struct PreviewInput {
    pub presentation: model::PresentationId,
    pub from: String,
    pub to: String,
    pub slides: Vec<model::Slide>,
}

/// Why a preview was refused.
pub enum Refused {
    /// No pair translates between these languages.
    NoPair(String),
    /// Too many previews.
    Busy,
}

impl Translation {
    /// The store in the data folder at `root`.
    pub fn new(root: &Path, opts: &TranslateOptions) -> Translation {
        let pairs = opts.pairs.clone().unwrap_or_else(catalogue::catalogue);
        let source = opts
            .source
            .clone()
            .unwrap_or_else(|| Arc::new(vesper_translate::store::Https::default()));
        let store = Arc::new(Store::open(models_dir(root), pairs, source));
        let (engine, marian): (Arc<dyn Translator>, _) = match &opts.engine {
            Some(e) => (e.clone(), None),
            None => match Marian::new(store.clone()) {
                Ok(m) => {
                    let m = Arc::new(m);
                    unload_when_idle(Arc::downgrade(&m));
                    (m.clone(), Some(m))
                }
                Err(e) => {
                    tracing::warn!("translation: no engine: {e:#}");
                    (Arc::new(NoEngine(e.to_string())), None)
                }
            },
        };
        Translation {
            store,
            engine,
            marian,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            running: Arc::new(Mutex::new(())),
        }
    }

    /// The models offered and where each stands.
    pub fn models(&self) -> TranslationModels {
        TranslationModels {
            engine: self.engine.engine(),
            models: self.store.pairs().iter().map(|p| self.model(p)).collect(),
            notice: NOTICE.to_string(),
        }
    }

    fn model(&self, p: &Pair) -> TranslationModel {
        let state = self.store.state(&p.id);
        let (status, downloaded, detail) = match state {
            State::NotDownloaded => (TranslationModelStatus::NotDownloaded, None, None),
            State::Downloading { done, .. } => {
                (TranslationModelStatus::Downloading, Some(done), None)
            }
            State::Ready => (TranslationModelStatus::Ready, None, None),
            State::Failed(why) => (TranslationModelStatus::Failed, None, Some(why)),
        };
        TranslationModel {
            id: p.id.clone(),
            from: p.from.clone(),
            to: p.to.iter().map(|t| t.language.clone()).collect(),
            model: p.model_name().to_string(),
            repo: p.repo.clone(),
            revision: p.revision.clone(),
            card: p.card(),
            licence: p.licence.clone(),
            attribution: p.attribution(),
            size: p.size(),
            status,
            downloaded,
            detail,
        }
    }

    /// The pair `id` and where it stands (`None`: no such pair).
    pub fn model_of(&self, id: &str) -> Option<TranslationModel> {
        self.store.pair(id).map(|p| self.model(p))
    }

    /// Starts fetching pair `id` (blocking briefly: call from a blocking thread).
    pub fn download(&self, id: &str) -> Result<TranslationModel, String> {
        self.store.start(id).map_err(|e| e.to_string())?;
        self.model_of(id)
            .ok_or_else(|| "no such language pair".into())
    }

    /// Stops fetching pair `id` (waits for its thread: call from a blocking thread).
    pub fn stop(&self, id: &str) -> Result<TranslationModel, String> {
        self.store.pair(id).ok_or("no such language pair")?;
        self.store.stop(id);
        self.model_of(id)
            .ok_or_else(|| "no such language pair".into())
    }

    /// Deletes pair `id`'s files (unloading it first; call from a blocking thread).
    pub fn delete(&self, id: &str) -> Result<TranslationModel, String> {
        self.store.pair(id).ok_or("no such language pair")?;
        if let Some(m) = &self.marian {
            m.unload(id);
        }
        self.store.delete(id).map_err(|e| e.to_string())?;
        self.model_of(id)
            .ok_or_else(|| "no such language pair".into())
    }

    /// The preview `id`.
    pub fn job(&self, id: &str) -> Option<TranslateJob> {
        lock(&self.jobs).get(id).map(|j| j.job.clone())
    }

    /// Cancels the preview `id`; false when there is none.
    pub fn cancel(&self, id: &str) -> bool {
        match lock(&self.jobs).get(id) {
            Some(j) => {
                j.cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// Starts a preview.
    pub fn preview(&self, input: PreviewInput) -> Result<TranslateJob, Refused> {
        let pair = catalogue::find(self.store.pairs(), &input.from, &input.to)
            .cloned()
            .ok_or_else(|| {
                Refused::NoPair(format!(
                    "no translation model translates {} to {}",
                    input.from, input.to
                ))
            })?;
        let slides: Vec<model::Slide> = input
            .slides
            .into_iter()
            .filter(|s| !vesper_translate::slide_lines(s).is_empty())
            .collect();
        let id = uuid::Uuid::new_v4().to_string();
        let job = TranslateJob {
            id: id.clone(),
            presentation: input.presentation,
            from: input.from.clone(),
            to: input.to.clone(),
            model: pair.model_name().to_string(),
            status: TranslateJobStatus::Queued,
            done: 0,
            total: slides.len() as u32,
            downloaded: None,
            download_size: None,
            detail: None,
            slides: Vec::new(),
            seconds: None,
            notice: NOTICE.to_string(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut jobs = lock(&self.jobs);
            jobs.retain(|_, j| j.ended.is_none_or(|t| t.elapsed() < KEEP_ENDED));
            if jobs.len() >= MAX_JOBS {
                // Ended previews make room, oldest first; only open ones refuse a new one.
                let mut ended: Vec<(Instant, String)> = jobs
                    .iter()
                    .filter_map(|(id, j)| j.ended.map(|t| (t, id.clone())))
                    .collect();
                ended.sort();
                let over = jobs.len() + 1 - MAX_JOBS;
                for (_, id) in ended.into_iter().take(over) {
                    jobs.remove(&id);
                }
            }
            if jobs.len() >= MAX_JOBS {
                return Err(Refused::Busy);
            }
            jobs.insert(
                id.clone(),
                Job {
                    job: job.clone(),
                    cancel: cancel.clone(),
                    ended: None,
                },
            );
        }
        let run = Run {
            id: id.clone(),
            jobs: self.jobs.clone(),
            running: self.running.clone(),
            store: self.store.clone(),
            engine: self.engine.clone(),
            pair,
            from: input.from,
            to: input.to,
            slides,
            cancel,
        };
        let spawned = std::thread::Builder::new()
            .name("translate-preview".into())
            .spawn(move || run.run());
        if let Err(e) = spawned {
            let mut jobs = lock(&self.jobs);
            if let Some(j) = jobs.get_mut(&id) {
                j.job.status = TranslateJobStatus::Failed;
                j.job.detail = Some(e.to_string());
                j.ended = Some(Instant::now());
            }
        }
        Ok(self.job(&id).unwrap_or(job))
    }
}

/// An engine that could not start (its error, on every call).
struct NoEngine(String);

impl Translator for NoEngine {
    fn translate(&self, _: &str, _: &str, _: &[String]) -> anyhow::Result<Vec<String>> {
        anyhow::bail!("the translation engine could not start: {}", self.0)
    }
}

/// Drops the engine's model once it has been idle long enough.
fn unload_when_idle(m: std::sync::Weak<Marian>) {
    let _ = std::thread::Builder::new()
        .name("translate-idle".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(30));
                let Some(m) = m.upgrade() else { return };
                if m.unload_idle(vesper_translate::marian::IDLE) {
                    tracing::info!("translation model unloaded (idle)");
                }
            }
        });
}

struct Run {
    id: String,
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    running: Arc<Mutex<()>>,
    store: Arc<Store>,
    engine: Arc<dyn Translator>,
    pair: Pair,
    from: String,
    to: String,
    slides: Vec<model::Slide>,
    cancel: Arc<AtomicBool>,
}

impl Run {
    fn update(&self, f: impl FnOnce(&mut TranslateJob)) {
        if let Some(j) = lock(&self.jobs).get_mut(&self.id) {
            f(&mut j.job);
        }
    }

    fn end(&self, status: TranslateJobStatus, detail: Option<String>) {
        if let Some(j) = lock(&self.jobs).get_mut(&self.id) {
            j.job.status = status;
            j.job.detail = detail;
            j.ended = Some(Instant::now());
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn run(self) {
        let _one = lock(&self.running);
        if self.cancelled() {
            return self.end(TranslateJobStatus::Cancelled, None);
        }
        // The model first, when it is not on the box. Once this job has seen the download
        // run (its own, or one the admin started), its end is final: a stop is never undone
        // by starting it again.
        let mut started = false;
        let mut seen = false;
        loop {
            match self.store.state(&self.pair.id) {
                State::Ready => break,
                State::Downloading { done, total } => {
                    seen = true;
                    self.update(|j| {
                        j.status = TranslateJobStatus::Downloading;
                        j.downloaded = Some(done);
                        j.download_size = Some(total);
                    })
                }
                State::Failed(why) if seen => {
                    return self.end(
                        TranslateJobStatus::Failed,
                        Some(format!(
                            "the {} model could not be fetched: {why}",
                            self.pair.id
                        )),
                    );
                }
                State::NotDownloaded if seen => {
                    return self.end(
                        TranslateJobStatus::Failed,
                        Some(format!("the {} model download was stopped", self.pair.id)),
                    );
                }
                State::Failed(_) | State::NotDownloaded => {
                    if let Err(e) = self.store.start(&self.pair.id) {
                        return self.end(TranslateJobStatus::Failed, Some(e.to_string()));
                    }
                    started = true;
                    seen = true;
                    self.update(|j| {
                        j.status = TranslateJobStatus::Downloading;
                        j.download_size = Some(self.pair.size());
                    });
                }
            }
            if self.cancelled() {
                if started {
                    self.store.stop(&self.pair.id);
                }
                return self.end(TranslateJobStatus::Cancelled, None);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        self.update(|j| {
            j.status = TranslateJobStatus::Translating;
            j.downloaded = None;
            j.download_size = None;
        });
        let t0 = Instant::now();
        let model = self.pair.model_name().to_string();
        for s in &self.slides {
            if self.cancelled() {
                return self.end(TranslateJobStatus::Cancelled, None);
            }
            match vesper_translate::propose(self.engine.as_ref(), s, &self.from, &self.to, &model) {
                Ok(Some(translation)) => self.update(|j| {
                    j.slides.push(TranslatedSlide {
                        slide: s.id,
                        translation,
                    });
                    j.done += 1;
                }),
                Ok(None) => self.update(|j| j.done += 1),
                Err(e) => {
                    return self.end(TranslateJobStatus::Failed, Some(format!("{e:#}")));
                }
            }
        }
        let secs = t0.elapsed().as_secs_f64();
        tracing::info!(
            pair = %self.pair.id,
            slides = self.slides.len(),
            "translation preview in {secs:.2} s"
        );
        self.update(|j| j.seconds = Some((secs * 100.0).round() / 100.0));
        self.end(TranslateJobStatus::Done, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::Condvar;

    /// Upper-cases each line; while `gate` is shut, waits for it to open.
    struct Fake {
        gate: Mutex<bool>,
        open: Condvar,
    }

    impl Fake {
        fn new(shut: bool) -> Arc<Fake> {
            Arc::new(Fake {
                gate: Mutex::new(shut),
                open: Condvar::new(),
            })
        }
        fn open_gate(&self) {
            *lock(&self.gate) = false;
            self.open.notify_all();
        }
    }

    impl Translator for Fake {
        fn translate(&self, _: &str, _: &str, lines: &[String]) -> anyhow::Result<Vec<String>> {
            let mut g = lock(&self.gate);
            while *g {
                g = self.open.wait(g).unwrap_or_else(|e| e.into_inner());
            }
            Ok(lines.iter().map(|l| l.to_uppercase()).collect())
        }
    }

    /// Model files from memory, each read pausing `slow`.
    struct Mem {
        files: HashMap<String, Vec<u8>>,
        slow: Duration,
    }

    struct Slow(std::io::Cursor<Vec<u8>>, Duration);

    impl Read for Slow {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            std::thread::sleep(self.1);
            let k = buf.len().min(1024);
            self.0.read(&mut buf[..k])
        }
    }

    impl Source for Mem {
        fn open(&self, url: &str) -> anyhow::Result<Box<dyn Read + Send>> {
            let name = url.rsplit('/').next().unwrap_or_default();
            let b = self.files.get(name).cloned().unwrap_or_default();
            Ok(Box::new(Slow(std::io::Cursor::new(b), self.slow)))
        }
    }

    fn translation(engine: Arc<Fake>, weights: usize, slow: Duration) -> (Translation, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "vesper-translate-jobs-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let w = vec![4u8; weights];
        let pair = Pair {
            id: "xx-yy".into(),
            from: "xx".into(),
            to: vec![Target {
                language: "yy".into(),
                token: None,
            }],
            repo: "Example/opus-mt-xx-yy".into(),
            revision: "0123456789abcdef0123456789abcdef01234567".into(),
            licence: "Apache-2.0".into(),
            base_url: "http://192.0.2.1".into(),
            files: vec![ModelFile {
                name: "weights.bin".into(),
                size: w.len() as u64,
                sha256: sha256(&w),
            }],
        };
        let opts = TranslateOptions {
            engine: Some(engine),
            pairs: Some(vec![pair]),
            source: Some(Arc::new(Mem {
                files: [("weights.bin".to_string(), w)].into(),
                slow,
            })),
        };
        (Translation::new(&root, &opts), root)
    }

    fn input() -> PreviewInput {
        let mut s = model::Slide::new();
        s.elements.push(model::Element::text(
            model::Rect::new(0.0, 0.0, 10.0, 10.0),
            model::TextBox::plain("a made-up line"),
        ));
        PreviewInput {
            presentation: model::PresentationId::new(),
            from: "xx".into(),
            to: "yy".into(),
            slides: vec![s],
        }
    }

    fn wait(t: &Translation, id: &str, what: impl Fn(&TranslateJob) -> bool) -> TranslateJob {
        for _ in 0..1000 {
            let j = t.job(id).expect("the job");
            if what(&j) {
                return j;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the job never got there: {:?}", t.job(id));
    }

    fn ended(j: &TranslateJob) -> bool {
        matches!(
            j.status,
            TranslateJobStatus::Done | TranslateJobStatus::Failed | TranslateJobStatus::Cancelled
        )
    }

    #[test]
    fn only_open_previews_count_against_the_limit() {
        let fake = Fake::new(false);
        let (t, root) = translation(fake.clone(), 64, Duration::ZERO);
        // Slide by slide: each preview ends before the next; far more than MAX_JOBS of them.
        for _ in 0..MAX_JOBS + 8 {
            let Ok(j) = t.preview(input()) else {
                panic!("refused with nothing running");
            };
            assert_eq!(wait(&t, &j.id, ended).status, TranslateJobStatus::Done);
        }
        assert!(lock(&t.jobs).len() <= MAX_JOBS);
        // Open previews (waiting for the engine) still have a limit.
        *lock(&fake.gate) = true;
        let open: Vec<String> = (0..MAX_JOBS)
            .map(|_| t.preview(input()).ok().expect("room").id)
            .collect();
        assert!(matches!(t.preview(input()), Err(Refused::Busy)));
        fake.open_gate();
        for id in &open {
            wait(&t, id, ended);
        }
        assert!(t.preview(input()).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_admins_stop_is_not_undone_by_a_waiting_preview() {
        let (t, root) = translation(Fake::new(false), 400 * 1024, Duration::from_millis(2));
        // The admin starts the download; a preview waits on it.
        t.download("xx-yy").unwrap();
        let j = t.preview(input()).ok().expect("room");
        wait(&t, &j.id, |j| j.status == TranslateJobStatus::Downloading);
        // The admin stops it: the preview ends, and the download stays stopped.
        t.stop("xx-yy").unwrap();
        let j = wait(&t, &j.id, ended);
        assert_eq!(j.status, TranslateJobStatus::Failed);
        assert!(j.detail.unwrap_or_default().contains("stopped"));
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            t.model_of("xx-yy").unwrap().status,
            TranslationModelStatus::NotDownloaded
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
