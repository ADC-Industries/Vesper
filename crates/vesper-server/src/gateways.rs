//! Recording and streaming a screen through IDN's gateway (`idn_ffmpeg::gateway`): see
//! `docs/RECORD-STREAM-PLAN.md`.
//!
//! - **Jobs.** The recording and each running stream target is a job with a [`Gateway`] of
//!   its own (one encode each; stopping one closes its gateway and waits for the file's
//!   trailer or the stream's last packets, so nothing else is disturbed).
//! - **Feeds.** Every screen that has jobs has one feed: a thread on the screen's frame grid
//!   (the timecodes its outputs stamp) that converts each new frame of the screen once to
//!   4:2:2 Y'CbCr planes (BT.709 limited range: the output crate's UYVY conversion) and
//!   hands it to every job of the screen, and the program audio from a master-bus tap
//!   ([`crate::sound::spawn_embedded_audio`], blocks sent when heard), on the same scale.
//!   The feed stops when the screen's last job goes.
//! - **Disk.** A recording refuses to start under `minFreeGb` free in its folder; while it
//!   runs, the feed checks every 5 s and stops it (a warning notice) under half of that. The
//!   check runs on a thread of its own, one at a time, without the gateways' lock (a stalled
//!   network folder blocks only that thread); the status shows its last reading. Starting
//!   and splitting the recording make, name and open its files without the gateways' lock
//!   too (one at a time per folder: a second start or split there meanwhile is refused as
//!   busy, "the recordings folder is not answering"), so a stalled folder holds up only that
//!   request and that folder, never the status, the streams, the settings or a start in
//!   another folder.
//! - **Server stop.** [`Gateways::shutdown`] is called by the server's shutdown before the
//!   render thread stops: files are finished, streams closed. Feed threads hold the server's
//!   state only for a moment each tick, so it is never dropped (and the feeds joined) there.
//! - **Secrets.** Stream keys and SRT URLs are kept in `machine/streaming.json` (owner-only
//!   on Unix; not in settings backups) and never logged or sent back: status and logs use
//!   the target's name and the gateway's redacted description.

use crate::{Shared, lock};
use idn_ffmpeg::Ffmpeg;
use idn_ffmpeg::gateway::{Container, Destination, Gateway, State, StreamSettings};
use idn_open_protocol::api::TappedFrame;
use idn_open_protocol::codec::Codec;
use idn_open_protocol::shq::{Fourcc, Plane};
use model::ScreenId;
use protocol::{
    ErrorCode, GatewayState, NoticeLevel, RecordingContainer, RecordingSettings, RecordingStatus,
    StreamCodec, StreamEncoder, StreamKind, StreamStatus, StreamTarget, StreamingSettings,
    StreamingStatus,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Where the settings are kept, under the data folder.
pub const SETTINGS_FILE: &str = "machine/streaming.json";
/// The default recordings folder, under the data folder.
pub const RECORDINGS_DIR: &str = "recordings";
/// Most stream targets.
pub const MAX_TARGETS: usize = 16;

/// A request the gateways refuse: an API error code and message.
pub type GwError = (ErrorCode, String);

fn bad(msg: impl Into<String>) -> GwError {
    (ErrorCode::BadRequest, msg.into())
}

/// What `machine/streaming.json` holds (keys included).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    recording: RecordingSettings,
    targets: Vec<StreamTarget>,
}

/// The recording in progress.
struct Recording {
    screen: ScreenId,
    gw: Arc<Gateway>,
    /// What each part is encoded with (a split's new part too).
    settings: StreamSettings,
    dest: u64,
    folder: PathBuf,
    file: PathBuf,
    part: u32,
    /// Bytes of the parts closed by a split.
    bytes_before: u64,
    started: Instant,
    started_at: u64,
    min_free: u64,
    /// Free bytes in `folder` at the last disk check.
    free: Arc<AtomicU64>,
}

/// A stream target that is running.
struct Stream {
    screen: ScreenId,
    gw: Arc<Gateway>,
    dest: u64,
    name: String,
    started: Instant,
    started_at: u64,
}

/// A screen's feed: its jobs' gateways and the threads that feed them.
struct Feed {
    sinks: Arc<Mutex<Vec<Arc<Gateway>>>>,
    stop: Arc<AtomicBool>,
    video: Option<JoinHandle<()>>,
    audio: Option<JoinHandle<()>>,
}

impl Feed {
    fn shutdown(mut self) {
        self.stop.store(true, Ordering::Release);
        let me = std::thread::current().id();
        for h in [self.video.take(), self.audio.take()].into_iter().flatten() {
            // Never on a feed's own thread (it would join itself); it stops on its own.
            if h.thread().id() != me {
                let _ = h.join();
            }
        }
    }
}

#[derive(Default)]
struct Inner {
    stored: Stored,
    recording: Option<Recording>,
    streams: HashMap<String, Stream>,
    feeds: HashMap<ScreenId, Feed>,
}

/// The box's recording and streams.
pub struct Gateways {
    data_dir: PathBuf,
    inner: Mutex<Inner>,
    /// The folders a start or split is using right now (without `inner`): one at a time per
    /// folder, so a folder that stopped answering holds up only itself (R1-1).
    files: Mutex<Vec<PathBuf>>,
}

/// A recordings folder claimed by a start or split ([`Gateways::claim_folder`]); released
/// when dropped.
struct FolderClaim<'a> {
    claims: &'a Mutex<Vec<PathBuf>>,
    folder: PathBuf,
}

impl Drop for FolderClaim<'_> {
    fn drop(&mut self) {
        let mut c = lock(self.claims);
        if let Some(i) = c.iter().position(|f| *f == self.folder) {
            c.swap_remove(i);
        }
    }
}

/// The answer to a start or split while another one still waits on the same folder.
fn folder_busy() -> GwError {
    (
        ErrorCode::Conflict,
        "the recordings folder is not answering: a start or split there is still waiting on it"
            .to_string(),
    )
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 100 ns units since the Unix epoch of the process's audio epoch (timecode 0).
fn epoch_unix_100ns() -> i64 {
    static BASE: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *BASE.get_or_init(|| {
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64 / 100)
            .unwrap_or(0);
        now_unix - audio::timecode(Instant::now())
    })
}

fn codec_of(c: StreamCodec) -> Codec {
    match c {
        StreamCodec::H264 => Codec::H264,
        StreamCodec::Hevc => Codec::Hevc,
        StreamCodec::Av1 => Codec::Av1,
    }
}

fn stream_codec(c: Codec) -> StreamCodec {
    match c {
        Codec::H264 => StreamCodec::H264,
        Codec::Hevc => StreamCodec::Hevc,
        Codec::Av1 => StreamCodec::Av1,
    }
}

/// The machine's FFmpeg encoding `codec`: its GPU's encoder, else its CPU's; `None` when
/// neither takes it. With the encoder's name for people.
fn encoder_for(codec: Codec) -> Option<(Arc<Ffmpeg>, String)> {
    let ff = media::machine_ffmpeg();
    if let Some((_, b)) = ff.encoders().iter().find(|e| e.0 == codec) {
        return Some((
            Arc::new(ff.with_decoder(ff.decoding())),
            b.label().to_string(),
        ));
    }
    let (_, b) = ff.cpu_encoders().iter().find(|e| e.0 == codec)?;
    let cpu = ff.for_output(
        idn_open_protocol::node::video::Processor::Cpu,
        idn_open_protocol::node::video::Format::Auto,
    );
    Some((Arc::new(cpu), format!("{} (CPU)", b.label())))
}

/// Every codec this box encodes, and with what.
pub fn encoders() -> Vec<StreamEncoder> {
    [Codec::H264, Codec::Hevc, Codec::Av1]
        .into_iter()
        .filter_map(|c| {
            encoder_for(c).map(|(_, label)| StreamEncoder {
                codec: stream_codec(c),
                label,
            })
        })
        .collect()
}

/// An SRT URL with its query values hidden (passphrases, stream IDs).
pub fn redact_srt(url: &str) -> String {
    match url.split_once('?') {
        None => url.to_string(),
        Some((base, q)) => {
            let q: Vec<String> = q
                .split('&')
                .map(|kv| match kv.split_once('=') {
                    Some((k, _)) => format!("{k}={HIDDEN}"),
                    None => kv.to_string(),
                })
                .collect();
            format!("{base}?{}", q.join("&"))
        }
    }
}

/// An SRT URL with a query value that is the hidden marker `***` (see [`redact_srt`]).
fn srt_has_hidden_value(url: &str) -> bool {
    url.split_once('?').is_some_and(|(_, q)| {
        q.split('&')
            .any(|kv| kv.split_once('=').is_some_and(|(_, v)| v == "***"))
    })
}

/// A hidden SRT query value, as clients see it.
const HIDDEN: &str = "***";

/// `url` with its hidden query values (`k=***`) taken from `saved`'s (where `saved` has `k`).
fn unredact_srt(url: &str, saved: &str) -> String {
    let Some((base, q)) = url.split_once('?') else {
        return url.to_string();
    };
    let saved_q = saved.split_once('?').map(|x| x.1).unwrap_or("");
    let value = |k: &str| {
        saved_q
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(sk, _)| *sk == k)
            .map(|(_, v)| v)
    };
    let q: Vec<String> = q
        .split('&')
        .map(|kv| match kv.split_once('=') {
            Some((k, v)) if v == HIDDEN => format!("{k}={}", value(k).unwrap_or(v)),
            _ => kv.to_string(),
        })
        .collect();
    format!("{base}?{}", q.join("&"))
}

/// The server a URL names: its scheme, host and port (lowercase), without user info, path or
/// query. A saved secret is kept only for the same server.
fn server_of(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        host.to_ascii_lowercase()
    )
}

/// A target as clients see it: no key, SRT query values hidden.
fn public_target(t: &StreamTarget) -> StreamTarget {
    StreamTarget {
        key: None,
        has_key: t.key.as_deref().is_some_and(|k| !k.is_empty()),
        url: match t.kind {
            StreamKind::Srt => redact_srt(&t.url),
            StreamKind::Rtmp => t.url.clone(),
        },
        ..t.clone()
    }
}

/// The URL a target sends to (the key appended for RTMP). Secret.
fn send_url(t: &StreamTarget) -> String {
    match (t.kind, t.key.as_deref().filter(|k| !k.is_empty())) {
        (StreamKind::Rtmp, Some(k)) => format!("{}/{}", t.url.trim_end_matches('/'), k.trim()),
        _ => t.url.clone(),
    }
}

fn check_rates(mbps: f64, kbps: u32, max_mbps: f64, max_kbps: u32) -> Result<(), GwError> {
    if !(0.5..=max_mbps).contains(&mbps) {
        return Err(bad(format!("videoMbps must be 0.5–{max_mbps}")));
    }
    if !(64..=max_kbps).contains(&kbps) {
        return Err(bad(format!("audioKbps must be 64–{max_kbps}")));
    }
    Ok(())
}

fn check_recording(r: &RecordingSettings) -> Result<(), GwError> {
    check_rates(r.video_mbps, r.audio_kbps, 100.0, 512)?;
    if r.split_minutes > 24 * 60 {
        return Err(bad("splitMinutes must be at most 1440"));
    }
    if !(0.0..=10_000.0).contains(&r.min_free_gb) {
        return Err(bad("minFreeGb must be 0–10000"));
    }
    if let Some(f) = &r.folder
        && !f.trim().is_empty()
        && !Path::new(f.trim()).is_absolute()
    {
        return Err(bad("folder must be an absolute path"));
    }
    Ok(())
}

fn check_target(t: &StreamTarget) -> Result<(), GwError> {
    let name = t.name.trim();
    if name.is_empty() || name.chars().count() > crate::config::MAX_NAME {
        return Err(bad("name must be 1–64 characters"));
    }
    let url = t.url.trim();
    let ok = match t.kind {
        StreamKind::Rtmp => url.starts_with("rtmp://") || url.starts_with("rtmps://"),
        StreamKind::Srt => url.starts_with("srt://"),
    };
    if !ok || url.len() < 10 || url.chars().any(char::is_whitespace) {
        return Err(bad(match t.kind {
            StreamKind::Rtmp => "url must be an rtmp:// or rtmps:// URL",
            StreamKind::Srt => "url must be an srt://host:port URL",
        }));
    }
    if t.kind == StreamKind::Srt && srt_has_hidden_value(url) {
        // An edited copy of the shown URL (its passphrase and stream ID hidden as `***`): saving
        // it would replace the real values with `***`.
        return Err(bad(
            "the SRT URL still has hidden values (***): type the whole URL again, with its passphrase and stream ID",
        ));
    }
    if t.key
        .as_deref()
        .is_some_and(|k| k.chars().any(char::is_whitespace))
    {
        return Err(bad("the stream key has spaces in it"));
    }
    check_rates(t.video_mbps, t.audio_kbps, 50.0, 320)
}

fn map_state(s: &State) -> (GatewayState, Option<String>) {
    match s {
        State::Connecting => (GatewayState::Connecting, None),
        State::Live => (GatewayState::Live, None),
        State::Retrying(m) => (GatewayState::Retrying, Some(m.clone())),
        State::Failed(m) => (GatewayState::Failed, Some(m.clone())),
        State::Done => (GatewayState::Idle, None),
    }
}

/// A file name part made of the screen's name (letters, digits, spaces, `-`, `_`).
fn file_safe(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim().to_string();
    if s.is_empty() { "Screen".into() } else { s }
}

/// Local date and time for file names (`2026-11-25 14-03-07`), UTC where the offset is
/// unknown.
fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %H-%M-%S").to_string()
}

impl Gateways {
    /// The gateways of a data folder, with its saved settings.
    pub fn load(data_dir: &Path) -> Gateways {
        let path = data_dir.join(SETTINGS_FILE);
        let stored = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                tracing::warn!(
                    "{} could not be read ({e}); recording and streaming settings start empty",
                    path.display()
                );
                Stored::default()
            }),
            Err(_) => Stored::default(),
        };
        Gateways {
            data_dir: data_dir.to_path_buf(),
            inner: Mutex::new(Inner {
                stored,
                ..Inner::default()
            }),
            files: Mutex::new(Vec::new()),
        }
    }

    fn save(&self, stored: &Stored) -> Result<(), GwError> {
        let path = self.data_dir.join(SETTINGS_FILE);
        let io = |e: std::io::Error| (ErrorCode::Internal, format!("saving the settings: {e}"));
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let bytes =
            serde_json::to_vec_pretty(stored).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
        // The stream keys: readable by the server's account only, from the first byte.
        vesper_core::fs::write_atomic_private(&path, &bytes).map_err(io)?;
        Ok(())
    }

    fn folder_of(&self, r: &RecordingSettings) -> PathBuf {
        match r.folder.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
            Some(f) => PathBuf::from(f),
            None => self.data_dir.join(RECORDINGS_DIR),
        }
    }

    /// `GET /streaming`.
    pub fn settings(&self) -> StreamingSettings {
        let g = lock(&self.inner);
        StreamingSettings {
            recording: g.stored.recording.clone(),
            targets: g.stored.targets.iter().map(public_target).collect(),
            recordings_folder: self.folder_of(&g.stored.recording).display().to_string(),
            encoders: encoders(),
        }
    }

    /// `PUT /streaming/recording`: applies to the next recording (a running one keeps its
    /// settings).
    pub fn set_recording(&self, mut r: RecordingSettings) -> Result<StreamingSettings, GwError> {
        check_recording(&r)?;
        r.folder = r
            .folder
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty());
        {
            let mut g = lock(&self.inner);
            let mut next = g.stored.clone();
            next.recording = r;
            self.save(&next)?;
            g.stored = next;
        }
        Ok(self.settings())
    }

    /// `POST /streaming/targets`.
    pub fn add_target(&self, mut t: StreamTarget) -> Result<StreamTarget, GwError> {
        t.id = format!("t{}", uuid::Uuid::new_v4().simple());
        t.name = t.name.trim().to_string();
        t.url = t.url.trim().to_string();
        t.key = t
            .key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
            // An SRT target has no stream key (its secrets are in its URL, R1-2).
            .filter(|_| t.kind != StreamKind::Srt);
        check_target(&t)?;
        let mut g = lock(&self.inner);
        if g.stored.targets.len() >= MAX_TARGETS {
            return Err(bad(format!("at most {MAX_TARGETS} targets")));
        }
        let mut next = g.stored.clone();
        next.targets.push(t.clone());
        self.save(&next)?;
        g.stored = next;
        Ok(public_target(&t))
    }

    /// `PUT /streaming/targets/{id}`: `key` absent keeps the saved key while the server
    /// (scheme, host and port) stays the same, `""` clears it; an SRT URL's query values sent
    /// back hidden (`***`) keep the saved ones on the same server. For another server the
    /// secrets are entered again: an RTMP target with a saved key needs `key` (a new key, or
    /// `""` to save it without one), hidden SRT values are refused (they never leave the box
    /// for a server nobody entered them for). A running stream keeps going with its old
    /// settings until restarted.
    pub fn update_target(&self, id: &str, mut t: StreamTarget) -> Result<StreamTarget, GwError> {
        let mut g = lock(&self.inner);
        let Some(i) = g.stored.targets.iter().position(|x| x.id == id) else {
            return Err((ErrorCode::NotFound, "no such target".into()));
        };
        let old = &g.stored.targets[i];
        t.id = old.id.clone();
        t.name = t.name.trim().to_string();
        t.url = t.url.trim().to_string();
        let same_server = t.kind == old.kind && server_of(&t.url) == server_of(&old.url);
        if t.kind == StreamKind::Srt {
            if same_server {
                t.url = unredact_srt(&t.url, &old.url);
            }
            if t.url.split_once('?').is_some_and(|(_, q)| {
                q.split('&')
                    .any(|kv| kv.split_once('=').is_some_and(|(_, v)| v == HIDDEN))
            }) {
                return Err(bad(
                    "enter the SRT passphrase and stream ID again: they are kept only for the same server",
                ));
            }
        }
        let had_key = old.key.as_deref().is_some_and(|k| !k.is_empty());
        t.key = match t.key {
            None if same_server => old.key.clone(),
            // Never dropped without a word (H05-1): the key is typed again, or cleared.
            None if had_key && t.kind == StreamKind::Rtmp => {
                return Err(bad(
                    "enter the stream key again for the new server (or remove it): it is kept only for the same server",
                ));
            }
            None => None,
            Some(k) => Some(k.trim().to_string()).filter(|k| !k.is_empty()),
        };
        // An SRT target has no stream key (its secrets are in its URL, R1-2).
        if t.kind == StreamKind::Srt {
            t.key = None;
        }
        check_target(&t)?;
        let mut next = g.stored.clone();
        next.targets[i] = t.clone();
        self.save(&next)?;
        g.stored = next;
        Ok(public_target(&t))
    }

    /// `DELETE /streaming/targets/{id}` (a running stream is stopped first).
    pub fn delete_target(&self, id: &str) -> Result<(), GwError> {
        if lock(&self.inner).streams.contains_key(id) {
            self.stop_stream(id)?;
        }
        let mut g = lock(&self.inner);
        let Some(i) = g.stored.targets.iter().position(|x| x.id == id) else {
            return Err((ErrorCode::NotFound, "no such target".into()));
        };
        let mut next = g.stored.clone();
        next.targets.remove(i);
        self.save(&next)?;
        g.stored = next;
        Ok(())
    }

    /// The screen to use: `asked`, else the setting's, else the first enabled screen; it must
    /// be running.
    fn pick_screen(
        shared: &Shared,
        asked: Option<ScreenId>,
        setting: Option<ScreenId>,
    ) -> Result<(ScreenId, String), GwError> {
        let screens = shared.screens();
        let want = asked.or(setting);
        let rt = match want {
            Some(id) => screens.iter().find(|r| r.id == id),
            None => screens.first(),
        };
        match rt {
            Some(rt) => Ok((rt.id, rt.screen().name)),
            None if want.is_some() => Err(bad("that screen isn't running (missing or disabled)")),
            None => Err((
                ErrorCode::Conflict,
                "there is no screen to record or stream".into(),
            )),
        }
    }

    /// Adds `gw` to `screen`'s feed, starting it when it's the first job there.
    fn attach(inner: &mut Inner, shared: &Arc<Shared>, screen: ScreenId, gw: &Arc<Gateway>) {
        if let Some(f) = inner.feeds.get(&screen) {
            lock(&f.sinks).push(gw.clone());
            return;
        }
        let sinks = Arc::new(Mutex::new(vec![gw.clone()]));
        let stop = Arc::new(AtomicBool::new(false));
        let video = {
            let (w, s, k) = (Arc::downgrade(shared), stop.clone(), sinks.clone());
            std::thread::Builder::new()
                .name("gateway feed".into())
                .spawn(move || feed_video(w, screen, k, s))
                .map_err(|e| tracing::error!("cannot start the gateway feed: {e}"))
                .ok()
        };
        let audio = crate::sound::spawn_embedded_audio(
            "gateway",
            &shared.sound,
            Box::new(GatewayAudio {
                sinks: sinks.clone(),
                sent: 0,
            }),
            Arc::new(crate::sound::OutputAudio::default()),
            Arc::new(AtomicU32::new(0)),
            Arc::new(AtomicU64::new(33_333_333)),
            stop.clone(),
        )
        .map_err(|e| tracing::error!("cannot start the gateway's sound: {e}"))
        .ok();
        inner.feeds.insert(
            screen,
            Feed {
                sinks,
                stop,
                video,
                audio,
            },
        );
    }

    /// Takes `gw` out of `screen`'s feed; returns the feed to shut down when it was the last.
    fn detach(inner: &mut Inner, screen: ScreenId, gw: &Arc<Gateway>) -> Option<Feed> {
        let f = inner.feeds.get(&screen)?;
        let mut s = lock(&f.sinks);
        s.retain(|g| !Arc::ptr_eq(g, gw));
        let empty = s.is_empty();
        drop(s);
        if empty {
            inner.feeds.remove(&screen)
        } else {
            None
        }
    }

    fn new_gateway(codec: StreamCodec) -> Result<(Arc<Gateway>, Codec), GwError> {
        let c = codec_of(codec);
        let Some((ff, _)) = encoder_for(c) else {
            return Err((
                ErrorCode::Conflict,
                format!("this box has no {} encoder", c.name()),
            ));
        };
        let gw = Arc::new(Gateway::new(ff));
        gw.set_rate(media::AUDIO_RATE);
        Ok((gw, c))
    }

    /// A free file name in `folder` for a recording of `screen_name`.
    fn file_name(folder: &Path, screen_name: &str, container: RecordingContainer) -> PathBuf {
        let ext = match container {
            RecordingContainer::Mp4 => "mp4",
            RecordingContainer::Mkv => "mkv",
        };
        #[cfg(test)]
        tests::in_folder();
        let base = format!("{} {}", file_safe(screen_name), stamp());
        let mut p = folder.join(format!("{base}.{ext}"));
        let mut n = 2;
        while p.exists() {
            p = folder.join(format!("{base} ({n}).{ext}"));
            n += 1;
        }
        p
    }

    /// `POST /recording/start`.
    pub fn start_recording(
        &self,
        shared: &Arc<Shared>,
        screen: Option<ScreenId>,
    ) -> Result<StreamingStatus, GwError> {
        let busy = || (ErrorCode::Conflict, "already recording".to_string());
        let (r, folder) = {
            let g = lock(&self.inner);
            if g.recording.is_some() {
                return Err(busy());
            }
            let r = g.stored.recording.clone();
            let folder = self.folder_of(&r);
            (r, folder)
        };
        // One start or split at a time per folder (two would pick the same file name); a
        // start in another folder goes ahead, and the last check below keeps it to one
        // recording.
        let _files = self.claim_folder(&folder).ok_or_else(folder_busy)?;
        // The folder is made and measured without the lock (a slow network folder).
        std::fs::create_dir_all(&folder).map_err(|e| {
            (
                ErrorCode::Internal,
                format!("cannot create the recordings folder: {e}"),
            )
        })?;
        let min_free = (r.min_free_gb * 1e9) as u64;
        let measured = vesper_codex::disk::space(&folder).map(|s| s.available).ok();
        let free = measured.unwrap_or(u64::MAX);
        if free < min_free {
            return Err((
                ErrorCode::InsufficientStorage,
                format!(
                    "only {:.1} GB free in the recordings folder (at least {:.1} GB needed)",
                    free as f64 / 1e9,
                    r.min_free_gb
                ),
            ));
        }
        // The file is named and opened without the lock too (H05-2).
        let (screen, screen_name) = Self::pick_screen(shared, screen, r.screen)?;
        let (gw, codec) = Self::new_gateway(r.codec)?;
        let container = match r.container {
            RecordingContainer::Mp4 => Container::Mp4,
            RecordingContainer::Mkv => Container::Mkv,
        };
        let file = Self::file_name(&folder, &screen_name, r.container);
        let settings = StreamSettings {
            codec,
            video_mbps: r.video_mbps,
            audio_kbps: r.audio_kbps,
            ..StreamSettings::default()
        };
        let dest = gw
            .add_with(
                Destination::File(file.clone(), container),
                settings.clone(),
                false,
            )
            .map_err(|e| (ErrorCode::Conflict, format!("cannot record: {e}")))?;
        if r.split_minutes > 0 {
            gw.set_split(dest, u64::from(r.split_minutes) * 60, 0);
        }
        {
            let mut g = lock(&self.inner);
            if g.recording.is_some() {
                drop(g);
                gw.close(Duration::from_secs(1));
                return Err(busy());
            }
            Self::attach(&mut g, shared, screen, &gw);
            tracing::info!(screen = %screen_name, file = %file.display(), "recording started");
            g.recording = Some(Recording {
                screen,
                gw,
                settings,
                dest,
                folder,
                file,
                part: 1,
                bytes_before: 0,
                started: Instant::now(),
                started_at: unix_ms(),
                min_free,
                free: Arc::new(AtomicU64::new(measured.unwrap_or(0))),
            });
        }
        Ok(self.status(shared))
    }

    /// `POST /recording/stop`: the file is finished (trailer written) before this returns.
    pub fn stop_recording(&self) -> Result<(), GwError> {
        self.stop_recording_of(None)
    }

    /// Stops the recording (only the one encoded by `only`, when given).
    fn stop_recording_of(&self, only: Option<&Weak<Gateway>>) -> Result<(), GwError> {
        let (rec, feed) = {
            let mut g = lock(&self.inner);
            let wanted = |r: &Recording| only.is_none_or(|w| std::ptr::eq(w.as_ptr(), &*r.gw));
            let Some(rec) = g.recording.take_if(|r| wanted(r)) else {
                return Err((ErrorCode::Conflict, "not recording".into()));
            };
            let feed = Self::detach(&mut g, rec.screen, &rec.gw);
            (rec, feed)
        };
        if let Some(f) = feed {
            f.shutdown();
        }
        rec.gw.close(Duration::from_secs(5));
        tracing::info!(file = %rec.file.display(), "recording stopped");
        Ok(())
    }

    /// `POST /recording/split`: closes the file and goes on in a new one.
    pub fn split_recording(&self, shared: &Arc<Shared>) -> Result<StreamingStatus, GwError> {
        self.split_with(|screen| {
            shared
                .screens()
                .iter()
                .find(|r| r.id == screen)
                .map(|r| r.screen().name)
                .unwrap_or_default()
        })?;
        Ok(self.status(shared))
    }

    /// Splits the recording (`screen_name`: the name of its screen). The new file is named
    /// and opened, and the old one closed, without the gateways' lock (H05-2: a stalled
    /// network folder holds up only this request).
    fn split_with(&self, screen_name: impl FnOnce(ScreenId) -> String) -> Result<(), GwError> {
        let not_recording = || (ErrorCode::Conflict, "not recording".to_string());
        let (gw, screen, folder, settings, container, split_s) = {
            let g = lock(&self.inner);
            let split_s = u64::from(g.stored.recording.split_minutes) * 60;
            let rec = g.recording.as_ref().ok_or_else(not_recording)?;
            let container = if rec.file.extension().is_some_and(|e| e == "mkv") {
                (Container::Mkv, RecordingContainer::Mkv)
            } else {
                (Container::Mp4, RecordingContainer::Mp4)
            };
            (
                rec.gw.clone(),
                rec.screen,
                rec.folder.clone(),
                rec.settings.clone(),
                container,
                split_s,
            )
        };
        let _files = self.claim_folder(&folder).ok_or_else(folder_busy)?;
        let file = Self::file_name(&folder, &screen_name(screen), container.1);
        let dest = gw
            .add_with(
                Destination::File(file.clone(), container.0),
                settings,
                false,
            )
            .map_err(|e| (ErrorCode::Internal, format!("cannot split: {e}")))?;
        if split_s > 0 {
            gw.set_split(dest, split_s, 0);
        }
        let closed = {
            let mut g = lock(&self.inner);
            match g.recording.as_mut() {
                // Still the same recording (not stopped meanwhile).
                Some(rec) if Arc::ptr_eq(&rec.gw, &gw) => {
                    let old = gw
                        .status()
                        .into_iter()
                        .find(|s| s.id == rec.dest)
                        .map(|s| s.bytes)
                        .unwrap_or(0);
                    let closed = std::mem::replace(&mut rec.dest, dest);
                    rec.file = file;
                    rec.part += 1;
                    rec.bytes_before += old;
                    Some(closed)
                }
                _ => None,
            }
        };
        match closed {
            Some(old) => {
                gw.remove(old);
                Ok(())
            }
            None => {
                gw.remove(dest);
                Err(not_recording())
            }
        }
    }

    /// Claims `folder` for a start or split, `None` while another one still uses it.
    fn claim_folder(&self, folder: &Path) -> Option<FolderClaim<'_>> {
        let mut c = lock(&self.files);
        if c.iter().any(|f| f == folder) {
            return None;
        }
        c.push(folder.to_path_buf());
        Some(FolderClaim {
            claims: &self.files,
            folder: folder.to_path_buf(),
        })
    }

    /// `POST /streaming/targets/{id}/start`.
    pub fn start_stream(
        &self,
        shared: &Arc<Shared>,
        id: &str,
        screen: Option<ScreenId>,
    ) -> Result<StreamingStatus, GwError> {
        {
            let mut g = lock(&self.inner);
            if g.streams.contains_key(id) {
                return Err((
                    ErrorCode::Conflict,
                    "that target is already streaming".into(),
                ));
            }
            let Some(t) = g.stored.targets.iter().find(|t| t.id == id).cloned() else {
                return Err((ErrorCode::NotFound, "no such target".into()));
            };
            if t.kind == StreamKind::Rtmp && !t.key.as_deref().is_some_and(|k| !k.is_empty()) {
                return Err(bad("the target has no stream key"));
            }
            let (screen, screen_name) = Self::pick_screen(shared, screen, t.screen)?;
            let (gw, codec) = Self::new_gateway(t.codec)?;
            let dest = match t.kind {
                StreamKind::Rtmp => Destination::Rtmp(send_url(&t)),
                StreamKind::Srt => Destination::Srt(send_url(&t)),
            };
            let shown = dest.describe();
            let settings = StreamSettings {
                codec,
                video_mbps: t.video_mbps,
                audio_kbps: t.audio_kbps,
                ..StreamSettings::default()
            };
            let dest = gw
                .add_with(dest, settings, false)
                .map_err(|e| (ErrorCode::Conflict, format!("cannot stream: {e}")))?;
            Self::attach(&mut g, shared, screen, &gw);
            tracing::info!(target = %t.name, to = %shown, screen = %screen_name, "stream started");
            g.streams.insert(
                id.to_string(),
                Stream {
                    screen,
                    gw,
                    dest,
                    name: t.name.clone(),
                    started: Instant::now(),
                    started_at: unix_ms(),
                },
            );
        }
        Ok(self.status(shared))
    }

    /// `POST /streaming/targets/{id}/stop`.
    pub fn stop_stream(&self, id: &str) -> Result<(), GwError> {
        let (s, feed) = {
            let mut g = lock(&self.inner);
            let Some(s) = g.streams.remove(id) else {
                return Err((ErrorCode::Conflict, "that target isn't streaming".into()));
            };
            let feed = Self::detach(&mut g, s.screen, &s.gw);
            (s, feed)
        };
        if let Some(f) = feed {
            f.shutdown();
        }
        s.gw.close(Duration::from_secs(2));
        tracing::info!(target = %s.name, "stream stopped");
        Ok(())
    }

    /// Something is recorded or streamed.
    pub fn active(&self) -> bool {
        let g = lock(&self.inner);
        g.recording.is_some() || !g.streams.is_empty()
    }

    /// The recording is running.
    pub fn recording(&self) -> bool {
        lock(&self.inner).recording.is_some()
    }

    /// A stream is running.
    pub fn streaming(&self) -> bool {
        !lock(&self.inner).streams.is_empty()
    }

    /// `GET /streaming/status`.
    pub fn status(&self, shared: &Shared) -> StreamingStatus {
        let g = lock(&self.inner);
        let screen_name = |id: ScreenId| {
            shared
                .screens()
                .iter()
                .find(|r| r.id == id)
                .map(|r| r.screen().name)
                .unwrap_or_default()
        };
        let recording = g.recording.as_ref().map(|r| {
            let st = r.gw.status();
            let d = st.iter().find(|s| s.id == r.dest);
            let (state, detail) = d.map(|d| map_state(&d.state)).unwrap_or_default();
            // Bytes of the parts the gateway itself split off are in its status too.
            let bytes: u64 = r.bytes_before + st.iter().map(|s| s.bytes).sum::<u64>();
            RecordingStatus {
                screen: r.screen,
                screen_name: screen_name(r.screen),
                file: r
                    .file
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                state,
                detail,
                started_at: r.started_at,
                seconds: r.started.elapsed().as_secs_f64(),
                bytes,
                part: r.part,
                video_kbps: d.map(|d| d.video_kbps).unwrap_or(0),
                skipped_pictures: d.map(|d| d.skipped_pictures).unwrap_or(0),
                dropped: d.map(|d| d.dropped).unwrap_or(0),
                free_bytes: r.free.load(Ordering::Relaxed),
            }
        });
        let mut streams: Vec<StreamStatus> = g
            .streams
            .iter()
            .map(|(id, s)| {
                let st = s.gw.status();
                let d = st.iter().find(|x| x.id == s.dest);
                let (state, detail) = d.map(|d| map_state(&d.state)).unwrap_or_default();
                StreamStatus {
                    id: id.clone(),
                    name: s.name.clone(),
                    screen: s.screen,
                    state,
                    detail,
                    started_at: s.started_at,
                    seconds: s.started.elapsed().as_secs_f64(),
                    bytes: d.map(|d| d.bytes).unwrap_or(0),
                    sent_kbps: d.map(|d| d.sent_kbps).unwrap_or(0),
                    video_kbps: d.map(|d| d.video_kbps).unwrap_or(0),
                    reconnects: d.map(|d| d.reconnects).unwrap_or(0),
                    dropped: d.map(|d| d.dropped).unwrap_or(0),
                    skipped_pictures: d.map(|d| d.skipped_pictures).unwrap_or(0),
                    lag_ms: d.map(|d| d.lag_ms).unwrap_or(0),
                }
            })
            .collect();
        streams.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        StreamingStatus { recording, streams }
    }

    /// What the disk check of `screen`'s recording needs: its folder, minimum, the free
    /// space reading to update and its gateway.
    fn disk_watch(
        &self,
        screen: ScreenId,
    ) -> Option<(PathBuf, u64, Arc<AtomicU64>, Weak<Gateway>)> {
        let g = lock(&self.inner);
        let r = g.recording.as_ref().filter(|r| r.screen == screen)?;
        Some((
            r.folder.clone(),
            r.min_free,
            r.free.clone(),
            Arc::downgrade(&r.gw),
        ))
    }

    /// Stops everything (server shutdown): files finished, streams closed.
    pub fn shutdown(&self) {
        let (rec, streams, feeds) = {
            let mut g = lock(&self.inner);
            (
                g.recording.take(),
                std::mem::take(&mut g.streams),
                std::mem::take(&mut g.feeds),
            )
        };
        for (_, f) in feeds {
            f.shutdown();
        }
        if let Some(r) = rec {
            r.gw.close(Duration::from_secs(3));
        }
        for (_, s) in streams {
            s.gw.close(Duration::from_secs(1));
        }
    }
}

impl Drop for Gateways {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Stops a recording on `screen` whose disk is running out (under half its minimum). The
/// disk is measured without the gateways' lock and without holding the server's state (a
/// stalled network folder blocks only this thread).
fn check_disk(shared: &Weak<Shared>, screen: ScreenId) {
    let Some((folder, min_free, free_now, gw)) = shared
        .upgrade()
        .and_then(|sh| sh.gateways.disk_watch(screen))
    else {
        return;
    };
    let Ok(space) = vesper_codex::disk::space(&folder) else {
        return;
    };
    let free = space.available;
    free_now.store(free, Ordering::Relaxed);
    if free >= min_free / 2 {
        return;
    }
    let Some(sh) = shared.upgrade() else { return };
    if sh.gateways.stop_recording_of(Some(&gw)).is_err() {
        return;
    }
    tracing::warn!(
        free,
        "the recordings disk is nearly full: recording stopped"
    );
    sh.notice_keyed(
        NoticeLevel::Warning,
        format!(
            "Recording stopped: only {:.1} GB left on the recordings disk",
            free as f64 / 1e9
        ),
        Some("recording-disk".into()),
    );
}

/// The program audio for a feed's gateways.
struct GatewayAudio {
    sinks: Arc<Mutex<Vec<Arc<Gateway>>>>,
    sent: u64,
}

impl output::AudioSink for GatewayAudio {
    fn send(
        &mut self,
        interleaved: &[f32],
        channels: usize,
        sample_rate: u32,
        timecode: i64,
    ) -> anyhow::Result<()> {
        let ch = channels.max(1);
        let n = interleaved.len() / ch;
        let planar: Vec<Vec<f32>> = (0..ch)
            .map(|c| {
                interleaved
                    .iter()
                    .skip(c)
                    .step_by(ch)
                    .copied()
                    .take(n)
                    .collect()
            })
            .collect();
        let f = TappedFrame::sound(
            planar,
            sample_rate,
            (30, 1),
            timecode,
            epoch_unix_100ns() + timecode,
        );
        for g in lock(&self.sinks).iter() {
            g.send(&f);
        }
        self.sent += n as u64;
        Ok(())
    }

    fn samples_sent(&self) -> u64 {
        self.sent
    }
}

/// Converts premultiplied RGBA8 to 4:2:2 Y'CbCr planes (BT.709 limited, over black);
/// `width` even.
fn to_planes(rgba: &[u8], width: usize, height: usize, uyvy: &mut Vec<u8>) -> Vec<Plane> {
    uyvy.resize(width * height * 2, 0);
    output::convert::premul_rgba_to_uyvy(rgba, uyvy);
    let cw = width / 2;
    let mut y = Vec::with_capacity(width * height);
    let mut cb = Vec::with_capacity(cw * height);
    let mut cr = Vec::with_capacity(cw * height);
    for q in uyvy.as_chunks::<4>().0 {
        cb.push(q[0]);
        y.push(q[1]);
        cr.push(q[2]);
        y.push(q[3]);
    }
    vec![
        Plane {
            width,
            height,
            data: y,
        },
        Plane {
            width: cw,
            height,
            data: cb,
        },
        Plane {
            width: cw,
            height,
            data: cr,
        },
    ]
}

/// A frame converted for the gateways: the frame's pointer and sequence, its planes, its size.
type Converted = (usize, u64, Arc<Vec<Plane>>, usize, usize);

/// A feed's picture thread: the screen's latest frame on its frame grid, converted once per
/// new frame, to every gateway of the feed. Checks the recordings disk every 5 s (one check
/// at a time). Holds the server's state only to look up the screen, never across its work:
/// the last `Arc<Shared>` must not be dropped here (X3-1).
fn feed_video(
    shared: Weak<Shared>,
    screen: ScreenId,
    sinks: Arc<Mutex<Vec<Arc<Gateway>>>>,
    stop: Arc<AtomicBool>,
) {
    let mut fr = model::FrameRate::FPS30;
    let mut period = crate::engine::period_of(fr);
    let mut next_due = crate::engine::on_grid(Instant::now(), period);
    // The last frame converted (its pointer and sequence), its planes and size.
    let mut last: Option<Converted> = None;
    let mut uyvy = Vec::new();
    let mut last_check = Instant::now();
    let checking = Arc::new(AtomicBool::new(false));
    while !stop.load(Ordering::Acquire) {
        let now = Instant::now();
        if now < next_due {
            std::thread::sleep((next_due - now).min(Duration::from_millis(50)));
            continue;
        }
        let rt = {
            let Some(sh) = shared.upgrade() else { break };
            sh.screen(screen)
        };
        if let Some(rt) = &rt {
            let cur = rt.frame_rate();
            if cur != fr {
                fr = cur;
                period = crate::engine::period_of(fr);
            }
            let frame = lock(&rt.latest).clone();
            if let Some(f) = frame {
                let key = (Arc::as_ptr(&f) as usize, f.sequence);
                if last.as_ref().is_none_or(|l| (l.0, l.1) != key) {
                    // An odd width loses its last column (4:2:2 needs pairs).
                    let (w, h) = ((f.width & !1) as usize, f.height as usize);
                    let planes = if w as u32 == f.width {
                        to_planes(&f.data, w, h, &mut uyvy)
                    } else {
                        let (_, _, px) = crate::outputs::crop_rgba(
                            &f.data,
                            f.width,
                            f.height,
                            (0, 0, w as u32, h as u32),
                        );
                        to_planes(&px, w, h, &mut uyvy)
                    };
                    last = Some((key.0, key.1, Arc::new(planes), w, h));
                }
                if let Some((_, _, planes, w, h)) = &last
                    && *w > 0
                    && *h > 0
                {
                    let tc = audio::timecode(next_due);
                    let tf = TappedFrame::picture(
                        Fourcc::Shq2,
                        *w,
                        *h,
                        planes.clone(),
                        (fr.num, fr.den),
                        tc,
                        epoch_unix_100ns() + tc,
                    );
                    for g in lock(&sinks).iter() {
                        g.send(&tf);
                    }
                }
            }
        }
        drop(rt);
        if last_check.elapsed() >= Duration::from_secs(5) && !checking.swap(true, Ordering::AcqRel)
        {
            last_check = Instant::now();
            let (w, busy) = (shared.clone(), checking.clone());
            // Off this thread: stopping the recording joins it, and the disk may stall.
            let spawned = std::thread::Builder::new()
                .name("rec disk check".into())
                .spawn(move || {
                    check_disk(&w, screen);
                    busy.store(false, Ordering::Release);
                });
            if spawned.is_err() {
                checking.store(false, Ordering::Release);
            }
        }
        next_due += period;
        if next_due < Instant::now() {
            next_due = crate::engine::on_grid(Instant::now(), period);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Set by a test: the next [`Gateways::file_name`] waits for it (a stalled folder).
    static STALLED: Mutex<Option<std::sync::mpsc::Receiver<()>>> = Mutex::new(None);

    /// Called where a recording's folder is first used.
    pub(super) fn in_folder() {
        let stalled = lock(&STALLED).take();
        if let Some(rx) = stalled {
            let _ = rx.recv();
        }
    }

    /// H05-2: a split whose recordings folder stalls (a network share that stopped
    /// answering) holds up only that request: the status, settings and streams answer
    /// meanwhile, and the split finishes once the folder answers.
    #[test]
    fn a_stalled_split_does_not_hold_the_gateways() {
        let Ok((gw, codec)) = Gateways::new_gateway(StreamCodec::H264) else {
            eprintln!("no H.264 encoder on this machine: skipped");
            return;
        };
        let dir = std::env::temp_dir().join(format!("vesper-split-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let gws = Arc::new(Gateways::load(&dir));
        let settings = StreamSettings {
            codec,
            ..StreamSettings::default()
        };
        let first = dir.join("first.mp4");
        let dest = gw
            .add_with(
                Destination::File(first.clone(), Container::Mp4),
                settings.clone(),
                false,
            )
            .unwrap();
        lock(&gws.inner).recording = Some(Recording {
            screen: ScreenId(uuid::Uuid::from_u128(1)),
            gw: gw.clone(),
            settings,
            dest,
            folder: dir.clone(),
            file: first,
            part: 1,
            bytes_before: 0,
            started: Instant::now(),
            started_at: 0,
            min_free: 0,
            free: Arc::new(AtomicU64::new(0)),
        });
        let (go, stalled) = std::sync::mpsc::channel();
        *lock(&STALLED) = Some(stalled);
        let split = std::thread::spawn({
            let gws = gws.clone();
            move || gws.split_with(|_| "Screen".into())
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while lock(&STALLED).is_some() {
            assert!(
                Instant::now() < deadline,
                "the split never reached its folder"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // The split waits in the folder: the gateways answer meanwhile.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn({
            let gws = gws.clone();
            move || {
                let _ = gws.settings();
                let _ = tx.send(gws.stop_stream("none").is_err());
            }
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)),
            Ok(true),
            "the gateways' lock was held by the split"
        );
        // A second split meanwhile is refused as busy, truthfully.
        let e = gws.split_with(|_| "Screen".into()).unwrap_err();
        assert_eq!(e.0, ErrorCode::Conflict);
        assert!(e.1.contains("not answering"), "{}", e.1);
        // R1-1: only that folder is held: a start in another folder may go ahead.
        assert!(gws.claim_folder(&dir).is_none());
        let other = dir.join("elsewhere");
        let claim = gws.claim_folder(&other).expect("another folder is free");
        drop(claim);
        assert!(gws.claim_folder(&other).is_some(), "released when dropped");
        go.send(()).unwrap();
        split.join().unwrap().unwrap();
        assert_eq!(lock(&gws.inner).recording.as_ref().unwrap().part, 2);
        assert!(
            gws.claim_folder(&dir).is_some(),
            "the split released its folder"
        );
        gws.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn srt_query_values_are_hidden() {
        assert_eq!(
            redact_srt("srt://192.0.2.10:9000?streamid=abc&passphrase=secret1234"),
            "srt://192.0.2.10:9000?streamid=***&passphrase=***"
        );
        assert_eq!(redact_srt("srt://192.0.2.10:9000"), "srt://192.0.2.10:9000");
    }

    #[test]
    fn rtmp_keys_are_appended_and_never_shown() {
        let t = StreamTarget {
            name: "Platform".into(),
            url: "rtmp://192.0.2.5/live/".into(),
            key: Some("made-up-key-123".into()),
            ..StreamTarget::default()
        };
        assert_eq!(send_url(&t), "rtmp://192.0.2.5/live/made-up-key-123");
        let p = public_target(&t);
        assert!(p.key.is_none() && p.has_key);
        assert!(!serde_json::to_string(&p).unwrap().contains("made-up-key"));
    }

    /// R1-2: an SRT target never keeps a stream key (a key typed before switching the form
    /// to SRT is not saved with it).
    #[test]
    fn srt_targets_keep_no_key() {
        let dir = std::env::temp_dir().join(format!("vesper-srt-key-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let gws = Gateways::load(&dir);
        let t = StreamTarget {
            name: "Link".into(),
            kind: StreamKind::Srt,
            url: "srt://192.0.2.5:9000".into(),
            key: Some("made-up-key".into()),
            ..StreamTarget::default()
        };
        let added = gws.add_target(t.clone()).unwrap();
        assert!(!added.has_key);
        let updated = gws.update_target(&added.id, t).unwrap();
        assert!(!updated.has_key);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn targets_are_checked() {
        let mut t = StreamTarget {
            name: "X".into(),
            url: "http://192.0.2.5/live".into(),
            ..StreamTarget::default()
        };
        assert!(check_target(&t).is_err());
        t.url = "rtmps://192.0.2.5/live".into();
        assert!(check_target(&t).is_ok());
        t.kind = StreamKind::Srt;
        assert!(check_target(&t).is_err());
        t.url = "srt://192.0.2.5:9000".into();
        assert!(check_target(&t).is_ok());
        t.video_mbps = 0.0;
        assert!(check_target(&t).is_err());
    }

    #[test]
    fn planes_are_bt709_limited() {
        // White and black pixels, opaque.
        let px = [255u8, 255, 255, 255, 0, 0, 0, 255];
        let p = to_planes(&px, 2, 1, &mut Vec::new());
        assert_eq!(p[0].data, vec![235, 16]);
        assert_eq!((p[1].data[0], p[2].data[0]), (128, 128));
    }

    #[test]
    fn stamps_look_like_dates() {
        let s = stamp();
        assert_eq!(s.len(), 19, "{s}");
        assert_eq!(&s[4..5], "-");
    }

    /// F05-6: file names carry the local time (not UTC).
    #[test]
    fn stamps_are_in_local_time() {
        let mut got = Vec::new();
        for _ in 0..3 {
            let want = chrono::Local::now().format("%Y-%m-%d %H-%M-%S").to_string();
            let s = stamp();
            if s == want {
                return;
            }
            got.push((s, want));
        }
        panic!("{got:?}");
    }
}
