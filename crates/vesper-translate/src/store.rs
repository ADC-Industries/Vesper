//! Vesper's own small model store for translation (owner, 2026-11-26): the same folder layout
//! and status shape as captions' models, but ours.
//!
//! Each pair lives in `<dir>/<pair>/` (the server passes `<data>/cache/translate`: the data
//! folder's cache, not in backups; on Linux `/var/lib/vesper/cache/translate` for the packaged
//! service, on Windows `%ProgramData%\Vesper\cache\translate`). A pair's files are fetched one
//! by one into `<file>.part`, hashed while they arrive, and renamed only when their size and
//! SHA-256 match the catalogue; `ready.json` (the revision) is written last. A failure writes
//! `failed.txt` (the reason), so the status survives a restart. A network failure keeps the
//! files already checked (a retry goes on from there); a file that fails its size or SHA-256
//! check removes what was fetched. A download stopped (or cut by a restart) leaves the pair
//! "not downloaded".
//!
//! Downloads have no total time limit (a weights file is hundreds of MB, and a church line may
//! be slow): a request whose answer stalls for [`STALL`] (no response headers, or no byte of
//! the body) fails. Stopping never waits on a stalled request: the pair is "not downloaded" at
//! once, and the stalled thread ends on its own without touching the pair's files again.
//!
//! Downloads run on their own thread, one at a time per pair. Models are never bundled in
//! packages: this store is the only way they reach a box.

use crate::catalogue::{ModelFile, Pair};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The marker written once every file is in place.
const READY: &str = "ready.json";
/// Why the last download failed.
const FAILED: &str = "failed.txt";
/// The longest a download waits for a server's answer: its response headers, or any one read
/// of the body. There is no limit on the whole download.
pub const STALL: Duration = Duration::from_secs(60);
/// How long [`Store::stop`] lets the download's thread tidy up (its `.part`) before it returns
/// without it (a thread stuck on a stalled request ends on its own).
const STOP_GRACE: Duration = Duration::from_secs(1);

/// Where model files come from (the publisher over HTTPS; tests pass their own).
pub trait Source: Send + Sync {
    /// A reader of the file at `url`.
    fn open(&self, url: &str) -> anyhow::Result<Box<dyn Read + Send>>;
}

/// HTTPS downloads (rustls with the web's root certificates; redirects followed).
pub struct Https {
    agent: ureq::Agent,
}

impl Https {
    /// Downloads that fail when the server's answer stalls for `stall`: no response headers
    /// within it, or no byte of the body within it of the last. No total limit: a large file
    /// on a slow line takes as long as it takes.
    pub fn new(stall: Duration) -> Https {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_send_request(Some(stall))
            .timeout_recv_response(Some(stall))
            .timeout_per_read(Some(stall))
            .user_agent("vesper-translate")
            .build()
            .into();
        Https { agent }
    }
}

impl Default for Https {
    fn default() -> Self {
        Https::new(STALL)
    }
}

impl Source for Https {
    fn open(&self, url: &str) -> anyhow::Result<Box<dyn Read + Send>> {
        let resp = self.agent.get(url).call()?;
        Ok(Box::new(resp.into_body().into_reader()))
    }
}

/// A pair's state in the store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Not on this box.
    NotDownloaded,
    /// Being fetched: bytes so far, of the total.
    Downloading { done: u64, total: u64 },
    /// On this box, checked.
    Ready,
    /// The last download failed (why).
    Failed(String),
}

struct Live {
    done: u64,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Why a store action was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum StoreError {
    /// No such pair in the catalogue.
    Unknown,
    /// The files could not be removed (why).
    Io(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Unknown => write!(f, "no such language pair"),
            StoreError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// The store.
pub struct Store {
    dir: PathBuf,
    pairs: Vec<Pair>,
    source: Arc<dyn Source>,
    live: Arc<Mutex<HashMap<String, Live>>>,
    /// Numbers each download, so a stopped one still ending never shares a `.part` with the
    /// next.
    runs: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Store {
    /// The store in `dir` offering `pairs`, fetching from `source`. Leftovers of an
    /// interrupted download (`*.part`) are removed.
    pub fn open(dir: impl Into<PathBuf>, pairs: Vec<Pair>, source: Arc<dyn Source>) -> Store {
        let dir = dir.into();
        for p in &pairs {
            if let Ok(rd) = std::fs::read_dir(dir.join(&p.id)) {
                for e in rd.flatten() {
                    if e.path().extension().is_some_and(|x| x == "part") {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
        }
        Store {
            dir,
            pairs,
            source,
            live: Arc::new(Mutex::new(HashMap::new())),
            runs: AtomicU64::new(0),
        }
    }

    /// The pairs offered.
    pub fn pairs(&self) -> &[Pair] {
        &self.pairs
    }

    /// The pair `id`.
    pub fn pair(&self, id: &str) -> Option<&Pair> {
        self.pairs.iter().find(|p| p.id == id)
    }

    /// The folder holding pair `id`'s files.
    pub fn path(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    /// Pair `id`'s state.
    pub fn state(&self, id: &str) -> State {
        let Some(p) = self.pair(id) else {
            return State::NotDownloaded;
        };
        if let Some(l) = lock(&self.live).get(id) {
            return State::Downloading {
                done: l.done,
                total: p.size(),
            };
        }
        let dir = self.path(id);
        if is_ready(&dir, p) {
            return State::Ready;
        }
        match std::fs::read_to_string(dir.join(FAILED)) {
            Ok(why) => State::Failed(why.trim().to_string()),
            Err(_) => State::NotDownloaded,
        }
    }

    /// Starts fetching pair `id` (nothing when it is ready or already being fetched).
    pub fn start(&self, id: &str) -> Result<(), StoreError> {
        let pair = self.pair(id).ok_or(StoreError::Unknown)?.clone();
        let mut live = lock(&self.live);
        if live.contains_key(id) || is_ready(&self.path(id), &pair) {
            return Ok(());
        }
        let stop = Arc::new(AtomicBool::new(false));
        let dir = self.path(id);
        let _ = std::fs::remove_file(dir.join(FAILED));
        let run = self.runs.fetch_add(1, Ordering::Relaxed);
        let (lives, source, s) = (self.live.clone(), self.source.clone(), stop.clone());
        let key = id.to_string();
        let thread = std::thread::Builder::new()
            .name(format!("translate-fetch-{id}"))
            .spawn(move || {
                // Only this download's own entry: after a stop, a newer one may hold the key.
                let mine = |l: &Live| Arc::ptr_eq(&l.stop, &s);
                let progress = |done: u64| {
                    if let Some(l) = lock(&lives).get_mut(&key)
                        && mine(l)
                    {
                        l.done = done;
                    }
                };
                let r = fetch(&pair, &dir, source.as_ref(), &s, run, &progress);
                // Under the lock: `stop` sets the flag under it too, so a stopped download
                // never writes `failed.txt` or removes files after its stop returned.
                let mut lives = lock(&lives);
                if s.load(Ordering::Relaxed) {
                    tracing::info!(pair = %pair.id, "translation model download stopped");
                } else {
                    match r {
                        Ok(()) => tracing::info!(pair = %pair.id, "translation model ready"),
                        Err(Fetch::Stopped) => {
                            tracing::info!(pair = %pair.id, "translation model download stopped")
                        }
                        Err(Fetch::Failed { why, keep }) => {
                            tracing::warn!(
                                pair = %pair.id,
                                "translation model download failed: {why}"
                            );
                            if !keep {
                                remove_files(&dir);
                            }
                            let _ = std::fs::create_dir_all(&dir);
                            let _ = std::fs::write(dir.join(FAILED), &why);
                        }
                    }
                }
                if lives.get(&key).is_some_and(mine) {
                    lives.remove(&key);
                }
            })
            .map_err(|e| StoreError::Io(e.to_string()))?;
        live.insert(
            id.to_string(),
            Live {
                done: 0,
                stop,
                thread: Some(thread),
            },
        );
        Ok(())
    }

    /// Stops fetching pair `id`; true when a download was running. The pair is "not
    /// downloaded" when this returns. It gives the download a moment to end and tidy up, but
    /// never waits on a stalled request: such a thread ends on its own later, leaving the
    /// pair's files alone.
    pub fn stop(&self, id: &str) -> bool {
        let thread = {
            let mut live = lock(&self.live);
            match live.remove(id) {
                Some(mut l) => {
                    l.stop.store(true, Ordering::Relaxed);
                    l.thread.take()
                }
                None => return false,
            }
        };
        if let Some(t) = thread {
            let until = Instant::now() + STOP_GRACE;
            while !t.is_finished() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(5));
            }
            if t.is_finished() {
                let _ = t.join();
            }
        }
        true
    }

    /// Removes pair `id`'s files (stopping its download first).
    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        self.pair(id).ok_or(StoreError::Unknown)?;
        self.stop(id);
        let dir = self.path(id);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError::Io(format!("could not remove the model: {e}"))),
        }
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        for l in lock(&self.live).values() {
            l.stop.store(true, Ordering::Relaxed);
        }
    }
}

fn is_ready(dir: &Path, p: &Pair) -> bool {
    let Ok(marker) = std::fs::read_to_string(dir.join(READY)) else {
        return false;
    };
    marker.contains(&p.revision)
        && p.files.iter().all(|f| {
            std::fs::metadata(dir.join(&f.name)).is_ok_and(|m| m.is_file() && m.len() == f.size)
        })
}

/// Removes everything a pair's folder holds.
fn remove_files(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

enum Fetch {
    Stopped,
    /// Why; `keep`: the files already checked stay (a network failure: a retry goes on from
    /// them), else everything fetched is removed (a file that is not what the catalogue says).
    Failed {
        why: String,
        keep: bool,
    },
}

/// A failure that removes what was fetched.
fn bad(why: String) -> Fetch {
    Fetch::Failed { why, keep: false }
}

/// A failure that keeps the files already checked.
fn cut(why: String) -> Fetch {
    Fetch::Failed { why, keep: true }
}

fn fetch(
    p: &Pair,
    dir: &Path,
    source: &dyn Source,
    stop: &AtomicBool,
    run: u64,
    progress: &dyn Fn(u64),
) -> Result<(), Fetch> {
    std::fs::create_dir_all(dir).map_err(|e| cut(format!("could not create {dir:?}: {e}")))?;
    let _ = std::fs::remove_file(dir.join(READY));
    let mut done = 0u64;
    for f in &p.files {
        if stop.load(Ordering::Relaxed) {
            return Err(Fetch::Stopped);
        }
        let target = dir.join(&f.name);
        // A file an earlier download already fetched and checked is kept.
        if std::fs::metadata(&target).is_ok_and(|m| m.len() == f.size)
            && sha256_file(&target).is_some_and(|h| h == f.sha256)
        {
            done += f.size;
            progress(done);
            continue;
        }
        let part = dir.join(format!("{}.{run}.part", f.name));
        let r = fetch_file(p, f, &part, source, stop, &mut |n| {
            progress(done + n);
        });
        if let Err(e) = r {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
        if stop.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(&part);
            return Err(Fetch::Stopped);
        }
        std::fs::rename(&part, &target)
            .map_err(|e| cut(format!("could not store {}: {e}", f.name)))?;
        done += f.size;
        progress(done);
    }
    if stop.load(Ordering::Relaxed) {
        return Err(Fetch::Stopped);
    }
    let marker = serde_json::json!({ "repo": p.repo, "revision": p.revision });
    std::fs::write(dir.join(READY), marker.to_string())
        .map_err(|e| cut(format!("could not mark the model ready: {e}")))?;
    Ok(())
}

fn fetch_file(
    p: &Pair,
    f: &ModelFile,
    part: &Path,
    source: &dyn Source,
    stop: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<(), Fetch> {
    let url = p.url(f);
    let mut r = source
        .open(&url)
        .map_err(|e| cut(format!("could not fetch {}: {e}", f.name)))?;
    let mut out =
        std::fs::File::create(part).map_err(|e| cut(format!("could not write {}: {e}", f.name)))?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut n = 0u64;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(Fetch::Stopped);
        }
        let k = r
            .read(&mut buf)
            .map_err(|e| cut(format!("the download of {} broke off: {e}", f.name)))?;
        if k == 0 {
            break;
        }
        n += k as u64;
        if n > f.size {
            return Err(bad(format!(
                "{} is larger than expected (the model at that revision changed?)",
                f.name
            )));
        }
        hash.update(&buf[..k]);
        out.write_all(&buf[..k])
            .map_err(|e| cut(format!("could not write {}: {e}", f.name)))?;
        progress(n);
    }
    out.flush()
        .map_err(|e| cut(format!("could not write {}: {e}", f.name)))?;
    if n < f.size {
        return Err(cut(format!(
            "the download of {} broke off ({n} of {} bytes)",
            f.name, f.size
        )));
    }
    let got = hex(&hash.finalize());
    if got != f.sha256 {
        return Err(bad(format!(
            "{} failed its checksum (SHA-256 {got}, expected {}): not kept",
            f.name, f.sha256
        )));
    }
    Ok(())
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// SHA-256 of a file, lowercase hex.
pub fn sha256_file(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let k = f.read(&mut buf).ok()?;
        if k == 0 {
            break;
        }
        hash.update(&buf[..k]);
    }
    Some(hex(&hash.finalize()))
}

/// SHA-256 of bytes, lowercase hex (catalogues made in tests).
pub fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::Target;

    /// Files served from memory; `slow` pauses each read (to stop a download midway).
    struct Mem {
        files: HashMap<String, Vec<u8>>,
        slow: Option<Duration>,
    }

    struct SlowReader {
        data: std::io::Cursor<Vec<u8>>,
        slow: Option<Duration>,
    }

    impl Read for SlowReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if let Some(d) = self.slow {
                std::thread::sleep(d);
            }
            let k = buf.len().min(1024);
            self.data.read(&mut buf[..k])
        }
    }

    impl Source for Mem {
        fn open(&self, url: &str) -> anyhow::Result<Box<dyn Read + Send>> {
            let name = url.rsplit('/').next().unwrap_or_default();
            let data = self
                .files
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("404"))?;
            Ok(Box::new(SlowReader {
                data: std::io::Cursor::new(data),
                slow: self.slow,
            }))
        }
    }

    fn pair(files: &[(&str, &[u8])]) -> Pair {
        Pair {
            id: "xx-yy".into(),
            from: "xx".into(),
            to: vec![Target {
                language: "yy".into(),
                token: None,
            }],
            repo: "Test/opus-mt-xx-yy".into(),
            revision: "0123456789abcdef0123456789abcdef01234567".into(),
            licence: "Apache-2.0".into(),
            base_url: "http://192.0.2.1".into(),
            files: files
                .iter()
                .map(|(n, b)| ModelFile {
                    name: (*n).into(),
                    size: b.len() as u64,
                    sha256: sha256(b),
                })
                .collect(),
        }
    }

    fn wait_done(s: &Store, id: &str) -> State {
        for _ in 0..500 {
            match s.state(id) {
                State::Downloading { .. } => std::thread::sleep(Duration::from_millis(10)),
                other => return other,
            }
        }
        panic!("the download never ended");
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vesper-translate-{name}-{}", uuid_like()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn uuid_like() -> String {
        format!(
            "{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    /// The `.part` files in the pair's folder.
    fn parts(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir.join("xx-yy"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "part"))
                    .collect()
            })
            .unwrap_or_default()
    }

    const A: &[u8] = b"config of a made-up model";
    const B: &[u8] = &[7u8; 5000];

    #[test]
    fn a_download_is_checked_and_survives_a_restart() {
        let dir = tmp("ok");
        let p = pair(&[("config.json", A), ("weights.bin", B)]);
        let src = Arc::new(Mem {
            files: [
                ("config.json".into(), A.to_vec()),
                ("weights.bin".into(), B.to_vec()),
            ]
            .into(),
            slow: None,
        });
        let s = Store::open(&dir, vec![p.clone()], src.clone());
        assert_eq!(s.state("xx-yy"), State::NotDownloaded);
        s.start("xx-yy").unwrap();
        assert_eq!(wait_done(&s, "xx-yy"), State::Ready);
        assert_eq!(std::fs::read(dir.join("xx-yy/weights.bin")).unwrap(), B);
        drop(s);
        // After a restart: still ready.
        let s = Store::open(&dir, vec![p], src);
        assert_eq!(s.state("xx-yy"), State::Ready);
        s.delete("xx-yy").unwrap();
        assert_eq!(s.state("xx-yy"), State::NotDownloaded);
        assert!(!dir.join("xx-yy").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_checksum_mismatch_fails_and_keeps_nothing() {
        let dir = tmp("sha");
        let p = pair(&[("config.json", A), ("weights.bin", B)]);
        let mut bad = B.to_vec();
        bad[10] ^= 1;
        let src = Arc::new(Mem {
            files: [
                ("config.json".into(), A.to_vec()),
                ("weights.bin".into(), bad),
            ]
            .into(),
            slow: None,
        });
        let s = Store::open(&dir, vec![p.clone()], src.clone());
        s.start("xx-yy").unwrap();
        let State::Failed(why) = wait_done(&s, "xx-yy") else {
            panic!("not failed");
        };
        assert!(why.contains("weights.bin failed its checksum"), "{why}");
        assert!(!dir.join("xx-yy/weights.bin").exists());
        assert!(parts(&dir).is_empty());
        assert!(!dir.join("xx-yy/config.json").exists());
        drop(s);
        // The failure is remembered over a restart, and a new start clears it.
        let s = Store::open(&dir, vec![p], src);
        assert!(matches!(s.state("xx-yy"), State::Failed(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stopped_download_is_not_downloaded_and_leaves_no_part() {
        let dir = tmp("stop");
        let big = vec![3u8; 200 * 1024];
        let p = pair(&[("config.json", A), ("weights.bin", &big)]);
        let src = Arc::new(Mem {
            files: [
                ("config.json".into(), A.to_vec()),
                ("weights.bin".into(), big.clone()),
            ]
            .into(),
            slow: Some(Duration::from_millis(2)),
        });
        let s = Store::open(&dir, vec![p.clone()], src);
        s.start("xx-yy").unwrap();
        // Progress is reported while it runs.
        let mut seen = false;
        for _ in 0..500 {
            if let State::Downloading { done, total } = s.state("xx-yy")
                && done > A.len() as u64
            {
                assert_eq!(total, (A.len() + big.len()) as u64);
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(seen, "no progress seen");
        assert!(s.stop("xx-yy"));
        assert_eq!(s.state("xx-yy"), State::NotDownloaded);
        assert!(parts(&dir).is_empty());
        assert!(!s.stop("xx-yy"));
        // A leftover part from a cut-off run is cleared on open.
        std::fs::write(dir.join("xx-yy/weights.bin.part"), b"half").unwrap();
        drop(s);
        let src = Arc::new(Mem {
            files: HashMap::new(),
            slow: None,
        });
        let s = Store::open(&dir, vec![p], src);
        assert!(parts(&dir).is_empty());
        assert_eq!(s.state("xx-yy"), State::NotDownloaded);
        assert_eq!(s.start("zz"), Err(StoreError::Unknown));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Fails the first open of `weights.bin` midway (the connection drops), counting opens.
    struct Flaky {
        files: HashMap<String, Vec<u8>>,
        opens: Mutex<Vec<String>>,
    }

    struct Drops(std::io::Cursor<Vec<u8>>);

    impl Read for Drops {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.0.position() >= 1000 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "connection reset",
                ));
            }
            let k = buf.len().min(500);
            self.0.read(&mut buf[..k])
        }
    }

    impl Source for Flaky {
        fn open(&self, url: &str) -> anyhow::Result<Box<dyn Read + Send>> {
            let name = url.rsplit('/').next().unwrap_or_default().to_string();
            let first = !self.opens.lock().unwrap().contains(&name);
            self.opens.lock().unwrap().push(name.clone());
            let data = self.files.get(&name).cloned().unwrap();
            if first && name == "weights.bin" {
                return Ok(Box::new(Drops(std::io::Cursor::new(data))));
            }
            Ok(Box::new(std::io::Cursor::new(data)))
        }
    }

    #[test]
    fn a_network_failure_keeps_the_checked_files_and_a_retry_resumes() {
        let dir = tmp("cut");
        let p = pair(&[("config.json", A), ("weights.bin", B)]);
        let src = Arc::new(Flaky {
            files: [
                ("config.json".into(), A.to_vec()),
                ("weights.bin".into(), B.to_vec()),
            ]
            .into(),
            opens: Mutex::new(Vec::new()),
        });
        let s = Store::open(&dir, vec![p], src.clone());
        s.start("xx-yy").unwrap();
        let State::Failed(why) = wait_done(&s, "xx-yy") else {
            panic!("not failed");
        };
        assert!(why.contains("weights.bin broke off"), "{why}");
        // The checked file stays; the cut one leaves no part.
        assert_eq!(std::fs::read(dir.join("xx-yy/config.json")).unwrap(), A);
        assert!(parts(&dir).is_empty());
        // A retry fetches only what is missing.
        s.start("xx-yy").unwrap();
        assert_eq!(wait_done(&s, "xx-yy"), State::Ready);
        assert_eq!(
            *src.opens.lock().unwrap(),
            vec!["config.json", "weights.bin", "weights.bin"]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The first open of `weights.bin` hangs until released (a request whose answer stalls).
    struct Stalls {
        files: HashMap<String, Vec<u8>>,
        stalled: Mutex<bool>,
        release: std::sync::Condvar,
        waiting: AtomicBool,
        hung: AtomicBool,
    }

    impl Source for Stalls {
        fn open(&self, url: &str) -> anyhow::Result<Box<dyn Read + Send>> {
            let name = url.rsplit('/').next().unwrap_or_default().to_string();
            if name == "weights.bin" && !self.hung.swap(true, Ordering::SeqCst) {
                self.waiting.store(true, Ordering::SeqCst);
                let mut g = self.stalled.lock().unwrap();
                while *g {
                    g = self.release.wait(g).unwrap();
                }
            }
            Ok(Box::new(std::io::Cursor::new(
                self.files.get(&name).cloned().unwrap(),
            )))
        }
    }

    #[test]
    fn stop_never_waits_on_a_stalled_request() {
        let dir = tmp("stall");
        let p = pair(&[("config.json", A), ("weights.bin", B)]);
        let src = Arc::new(Stalls {
            files: [
                ("config.json".into(), A.to_vec()),
                ("weights.bin".into(), B.to_vec()),
            ]
            .into(),
            stalled: Mutex::new(true),
            release: std::sync::Condvar::new(),
            waiting: AtomicBool::new(false),
            hung: AtomicBool::new(false),
        });
        let s = Store::open(&dir, vec![p], src.clone());
        s.start("xx-yy").unwrap();
        for _ in 0..500 {
            if src.waiting.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            src.waiting.load(Ordering::SeqCst),
            "the request never started"
        );
        let t = Instant::now();
        assert!(s.stop("xx-yy"));
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "stop took {:?}",
            t.elapsed()
        );
        assert_eq!(s.state("xx-yy"), State::NotDownloaded);
        // A new download runs while the stalled one still hangs.
        s.start("xx-yy").unwrap();
        assert_eq!(wait_done(&s, "xx-yy"), State::Ready);
        // The stalled request finally answers: its thread ends without touching the files.
        *src.stalled.lock().unwrap() = false;
        src.release.notify_all();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(s.state("xx-yy"), State::Ready);
        assert!(parts(&dir).is_empty());
        assert!(!dir.join("xx-yy").join(FAILED).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A one-request HTTP server on 127.0.0.1: answers with `body` in `chunks` pieces, `gap`
    /// apart (or never answers when `chunks` is 0). Its address.
    fn serve(body: Vec<u8>, chunks: usize, gap: Duration) -> std::net::SocketAddr {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut c, _) = l.accept().unwrap();
            let mut req = [0u8; 4096];
            let _ = c.read(&mut req);
            if chunks == 0 {
                std::thread::sleep(Duration::from_secs(5));
                return;
            }
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            c.write_all(head.as_bytes()).unwrap();
            for piece in body.chunks(body.len().div_ceil(chunks)) {
                std::thread::sleep(gap);
                c.write_all(piece).unwrap();
                c.flush().unwrap();
            }
        });
        addr
    }

    #[test]
    fn https_has_no_total_limit_only_a_stall_limit() {
        let stall = Duration::from_millis(400);
        let h = Https::new(stall);
        // A body arriving slowly, over far longer than the stall limit, but steadily.
        let body = vec![5u8; 64 * 1024];
        let addr = serve(body.clone(), 8, Duration::from_millis(150));
        let mut got = Vec::new();
        h.open(&format!("http://{addr}/weights.bin"))
            .unwrap()
            .read_to_end(&mut got)
            .unwrap();
        assert_eq!(got, body);
        // A server that never answers: the request fails after about the stall limit.
        let addr = serve(Vec::new(), 0, Duration::ZERO);
        let t = Instant::now();
        assert!(h.open(&format!("http://{addr}/weights.bin")).is_err());
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    }
}
