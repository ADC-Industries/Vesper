//! Resumable media uploads into a collection.
//!
//! ```text
//! codex/uploads/<id>.json   UploadState: collection, name, size, expected SHA-256, bin, who
//! codex/uploads/<id>.part   the bytes received so far (its length is the upload's offset)
//! ```
//!
//! 1. `POST /codex/uploads` checks the collection, the bin, the size limit, the free space (what
//!    every open upload still needs, plus 5 % of the disk and 2 GiB) and the number of open
//!    uploads.
//! 2. `PUT /codex/uploads/{id}` with `Content-Range: bytes <start>-<end>/<size>` appends one
//!    chunk, streamed to the `.part` file in blocks of at most 1 MiB, with the SHA-256 computed
//!    on the fly (the file is never held in memory). A chunk that doesn't start at the offset
//!    answers `409` with the offset. A chunk that breaks off keeps the bytes that came (the
//!    client asks the offset and goes on). At most [`super::CodexOptions::max_concurrent_transfers`]
//!    chunks are received at once (`429` past that). After a restart the hash is caught up by
//!    reading the `.part` file once.
//! 3. `POST /codex/uploads/{id}/finish` checks the size and the SHA-256, then takes the file
//!    into the Codex's media library ([`super::library`]: IDN's `import`, moved, under the
//!    uploaded file's name, into the collection's folder; staged as `uploads/<id>/<name>`
//!    first, on a thread of its own at low priority, since IDN reads the file for its
//!    SHA-256). The same bytes in the library already: that item (dedup), and the upload's
//!    file goes. Then a new media asset is saved with the item's `library {id, sha256}`
//!    location ([`super::writer::Writer::media`]: the writer's lock is held only for that save)
//!    and the item is marked in use. When the collection has an asset for those bytes already
//!    (the same SHA-256, not a theme's), that one is answered instead (`existing`; pointed at
//!    the item when it was elsewhere, added to the named bin when it isn't in it). Probing and
//!    the thumbnail follow on the media worker ([`super::worker`]). Without an open library the finish answers `503` and the upload
//!    stays (finish it again later).
//! 4. `DELETE /codex/uploads/{id}` cancels. An upload without a new chunk for
//!    [`super::CodexOptions::upload_expiry`] is dropped by the hourly upkeep ([`Uploads::clean`]).

use super::blobs;
use super::disk;
use crate::Shared;
use crate::Show;
use axum::body::Body;
use futures_util::StreamExt;
use model::{
    AssetLocation, AssetScope, ChangeBy, CollectionId, MediaAsset, MediaBinId, MediaBinItem,
    MediaBinItemId, MediaBinKind, MediaKind, MediaLayer, MediaOrigin, MediaStatus, MediaTrigger,
};
use protocol::{ApiError, CreateUpload, ErrorCode, FinishUpload, UploadFinished, UploadInfo};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use vesper_core::change::ConfigError;

/// Bytes written (and hashed) at a time.
const BLOCK: usize = 1 << 20;

/// An upload as stored (`codex/uploads/<id>.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadState {
    /// Schema version.
    pub schema_version: u32,
    /// The upload's ID.
    pub id: uuid::Uuid,
    /// The collection.
    pub collection: CollectionId,
    /// The file's name.
    pub name: String,
    /// The file's size.
    pub size: u64,
    /// The expected SHA-256.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The bin the asset goes into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<MediaBinId>,
    /// Where the file was on the client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<String>,
    /// The asset the file relinks (instead of a new asset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<model::MediaAssetId>,
    /// When it was started (RFC 3339 UTC).
    pub created_at: String,
    /// Who started it.
    #[serde(default)]
    pub by: ChangeBy,
}

/// One upload in memory.
pub struct Session {
    state: UploadState,
    /// Bytes in the `.part` file.
    offset: u64,
    /// The hash of the first `hashed` bytes.
    hasher: Sha256,
    hashed: u64,
    /// When the last chunk came.
    updated: SystemTime,
    /// Finished, cancelled or dropped: gone.
    done: bool,
}

/// Why a chunk or a finish was refused.
#[derive(Debug)]
pub enum UploadError {
    /// An API error.
    Api(ConfigError),
    /// The chunk doesn't start where the upload stands (or bytes are missing): `409` with the
    /// offset.
    Offset(ApiError, u64),
}

impl From<ApiError> for UploadError {
    fn from(e: ApiError) -> Self {
        UploadError::Api(e.into())
    }
}

impl From<ConfigError> for UploadError {
    fn from(e: ConfigError) -> Self {
        UploadError::Api(e)
    }
}

fn api(code: ErrorCode, msg: impl Into<String>) -> ApiError {
    ApiError::new(code, msg)
}

fn now_str(t: SystemTime) -> String {
    crate::auth::rfc3339(t, true)
}

fn join_err(e: tokio::task::JoinError) -> ApiError {
    api(ErrorCode::Internal, e.to_string())
}

fn io_err(what: &str, e: impl std::fmt::Display) -> ApiError {
    api(ErrorCode::Internal, format!("{what}: {e}"))
}

/// Every upload of the server ([`crate::Server::uploads`]).
pub struct Uploads {
    /// The limits.
    pub options: super::CodexOptions,
    sessions: Mutex<Option<HashMap<uuid::Uuid, Arc<tokio::sync::Mutex<Session>>>>>,
    transfers: Arc<tokio::sync::Semaphore>,
}

type Sessions = HashMap<uuid::Uuid, Arc<tokio::sync::Mutex<Session>>>;

fn part(dir: &Path, id: uuid::Uuid) -> PathBuf {
    dir.join(format!("{id}.part"))
}

fn meta(dir: &Path, id: uuid::Uuid) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// The uploads on disk (each `.json` with its `.part`; a missing `.part` is an empty one).
fn load_dir(dir: &Path) -> Sessions {
    let mut out = Sessions::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(state) = std::fs::read(&p)
            .ok()
            .and_then(|b| serde_json::from_slice::<UploadState>(&b).ok())
        else {
            tracing::warn!(file = %p.display(), "upload: unreadable state, skipped");
            continue;
        };
        let pp = part(dir, state.id);
        let m = std::fs::metadata(&pp).ok();
        let offset = m.as_ref().map_or(0, |m| m.len());
        let updated = m
            .and_then(|m| m.modified().ok())
            .or_else(|| e.metadata().ok()?.modified().ok())
            .unwrap_or_else(SystemTime::now);
        out.insert(
            state.id,
            Arc::new(tokio::sync::Mutex::new(Session {
                state,
                offset,
                hasher: Sha256::new(),
                hashed: 0,
                updated,
                done: false,
            })),
        );
    }
    out
}

/// The SHA-256 of a file's first `len` bytes, read in blocks.
fn hash_prefix(path: &Path, len: u64) -> std::io::Result<Sha256> {
    let mut h = Sha256::new();
    let mut f = std::fs::File::open(path)?.take(len);
    let mut buf = vec![0u8; BLOCK];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `bytes <start>-<end>/<total>`.
fn parse_range(v: &str) -> Option<(u64, u64, u64)> {
    let rest = v.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (a, b) = range.split_once('-')?;
    let (a, b, t) = (
        a.trim().parse().ok()?,
        b.trim().parse().ok()?,
        total.trim().parse().ok()?,
    );
    (a <= b).then_some((a, b, t))
}

/// The open file of an upload and its running hash, moved to the blocking pool per block.
struct Sink {
    file: std::fs::File,
    hasher: Sha256,
}

impl Sink {
    async fn write(mut self, buf: Vec<u8>) -> (Sink, std::io::Result<()>, Vec<u8>) {
        let r = tokio::task::spawn_blocking(move || {
            let r = self.file.write_all(&buf);
            if r.is_ok() {
                self.hasher.update(&buf);
            }
            (self, r, buf)
        })
        .await;
        match r {
            Ok(t) => t,
            Err(e) => panic!("upload writer task: {e}"),
        }
    }
}

impl Uploads {
    /// No uploads yet (they are read from disk on first use).
    pub fn new(options: super::CodexOptions) -> Uploads {
        let transfers = Arc::new(tokio::sync::Semaphore::new(
            options.max_concurrent_transfers.max(1),
        ));
        Uploads {
            options,
            sessions: Mutex::new(None),
            transfers,
        }
    }

    fn dir(sh: &Shared) -> PathBuf {
        sh.codex().layout.uploads()
    }

    fn sessions(&self, sh: &Shared) -> std::sync::MutexGuard<'_, Option<Sessions>> {
        let mut g = crate::lock(&self.sessions);
        if g.is_none() {
            *g = Some(load_dir(&Self::dir(sh)));
        }
        g
    }

    fn session(
        &self,
        sh: &Shared,
        id: uuid::Uuid,
    ) -> Result<Arc<tokio::sync::Mutex<Session>>, ApiError> {
        self.sessions(sh)
            .as_ref()
            .and_then(|m| m.get(&id).cloned())
            .ok_or_else(|| api(ErrorCode::NotFound, format!("upload {id} not found")))
    }

    fn all(&self, sh: &Shared) -> Vec<Arc<tokio::sync::Mutex<Session>>> {
        self.sessions(sh)
            .iter()
            .flat_map(|m| m.values().cloned())
            .collect()
    }

    fn forget(&self, sh: &Shared, id: uuid::Uuid) {
        if let Some(m) = self.sessions(sh).as_mut() {
            m.remove(&id);
        }
    }

    fn info(&self, s: &Session) -> UploadInfo {
        UploadInfo {
            id: s.state.id.to_string(),
            collection: s.state.collection,
            name: s.state.name.clone(),
            size: s.state.size,
            offset: s.offset,
            sha256: s.state.sha256.clone(),
            bin: s.state.bin,
            created_at: s.state.created_at.clone(),
            updated_at: now_str(s.updated),
            expires_at: now_str(s.updated + self.options.upload_expiry),
        }
    }

    /// The free space an upload of `bytes` more must leave (`507` otherwise).
    fn check_space(&self, dir: &Path, bytes: u64) -> Result<(), ApiError> {
        check_space(&self.options, dir, bytes)
    }

    /// `POST /codex/uploads`.
    pub async fn create(
        &self,
        sh: &Arc<Shared>,
        body: CreateUpload,
        by: ChangeBy,
    ) -> Result<UploadInfo, ApiError> {
        super::writer::writable(sh)?;
        let content = sh.collection_content(body.collection).ok_or_else(|| {
            ApiError::invalid(
                "collection",
                format!("collection {} is not in the Codex", body.collection),
            )
        })?;
        let name = body.name.trim().to_string();
        if name.is_empty() || name.chars().count() > 255 || name.chars().any(char::is_control) {
            return Err(ApiError::invalid(
                "name",
                "a file name of 1–255 characters, without control characters",
            ));
        }
        let (_, ext) = blobs::split_name(&name);
        if blobs::kind_for(&ext).is_none() || !super::library::takes(base_name(&name)) {
            return Err(ApiError::invalid(
                "name",
                format!(
                    "“{name}” isn't an image, video or audio file the Codex's media library takes"
                ),
            ));
        }
        if body.size == 0 {
            return Err(ApiError::invalid("size", "the file is empty"));
        }
        if body.size > self.options.max_upload_bytes {
            return Err(ApiError::invalid(
                "size",
                format!(
                    "the file is larger than the {} MiB an upload may be",
                    self.options.max_upload_bytes >> 20
                ),
            ));
        }
        if let Some(s) = &body.sha256
            && !blobs::is_sha256(s)
        {
            return Err(ApiError::invalid(
                "sha256",
                "a SHA-256 is 64 lowercase hex digits",
            ));
        }
        if let Some(bin) = body.bin {
            match content.media_bins.iter().find(|b| b.id == bin) {
                Some(b) if matches!(b.kind, MediaBinKind::Manual { .. }) => {}
                Some(_) => {
                    return Err(ApiError::invalid(
                        "bin",
                        "a watched bin fills itself: choose a manual bin",
                    ));
                }
                None => {
                    return Err(ApiError::invalid(
                        "bin",
                        format!("bin {} is not in the collection", bin.0),
                    ));
                }
            }
        }
        if let Some(a) = body.replaces
            && !content.assets.contains_key(&a)
        {
            return Err(ApiError::invalid(
                "replaces",
                format!("media asset {} is not in the collection", a.0),
            ));
        }
        let dir = Self::dir(sh);
        // What the open uploads still need.
        let (open, pending) = {
            let list = self.all(sh);
            let mut pending = 0u64;
            for s in &list {
                let s = s.lock().await;
                pending += s.state.size.saturating_sub(s.offset);
            }
            (list.len(), pending)
        };
        if open >= self.options.max_open_uploads {
            return Err(api(
                ErrorCode::RateLimited,
                format!("{open} uploads are open: finish or cancel some first"),
            ));
        }
        let now = SystemTime::now();
        let state = UploadState {
            schema_version: model::SCHEMA_VERSION,
            id: uuid::Uuid::new_v4(),
            collection: body.collection,
            name,
            size: body.size,
            sha256: body.sha256,
            bin: body.bin,
            relative_path: body
                .relative_path
                .map(|p| p.trim().replace('\\', "/"))
                .filter(|p| !p.is_empty()),
            replaces: body.replaces,
            created_at: now_str(now),
            by,
        };
        let (d, st) = (dir.clone(), state.clone());
        let opts = self.options.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&d).map_err(|e| io_err("creating the uploads folder", e))?;
            check_space(&opts, &d, pending.saturating_add(st.size))?;
            std::fs::File::create(part(&d, st.id)).map_err(|e| io_err("starting the upload", e))?;
            let json = serde_json::to_vec_pretty(&st).map_err(|e| io_err("saving", e))?;
            crate::store::write_atomic(&meta(&d, st.id), &json)
                .map_err(|e| io_err("saving the upload", e))
        })
        .await
        .map_err(join_err)??;
        let session = Session {
            state: state.clone(),
            offset: 0,
            hasher: Sha256::new(),
            hashed: 0,
            updated: now,
            done: false,
        };
        let info = self.info(&session);
        self.sessions(sh)
            .get_or_insert_with(Sessions::new)
            .insert(state.id, Arc::new(tokio::sync::Mutex::new(session)));
        tracing::info!(upload = %state.id, collection = %state.collection, size = state.size, "Codex: upload started");
        Ok(info)
    }

    /// `GET /codex/uploads/{id}`.
    pub async fn get(&self, sh: &Shared, id: uuid::Uuid) -> Result<UploadInfo, ApiError> {
        let s = self.session(sh, id)?;
        let s = s.lock().await;
        if s.done {
            return Err(api(ErrorCode::NotFound, format!("upload {id} not found")));
        }
        Ok(self.info(&s))
    }

    /// `GET /codex/uploads`: every open upload, oldest first.
    pub async fn list(&self, sh: &Shared) -> Vec<UploadInfo> {
        let all = self.all(sh);
        let mut out = Vec::new();
        for s in all {
            let s = s.lock().await;
            if !s.done {
                out.push(self.info(&s));
            }
        }
        out.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        out
    }

    /// Brings the session's hash up to its offset (after a restart: reads the `.part` file).
    async fn catch_up(dir: &Path, s: &mut Session) -> Result<(), ApiError> {
        if s.hashed == s.offset {
            return Ok(());
        }
        let (p, len) = (part(dir, s.state.id), s.offset);
        let h = tokio::task::spawn_blocking(move || hash_prefix(&p, len))
            .await
            .map_err(join_err)?
            .map_err(|e| io_err("reading the upload", e))?;
        s.hasher = h;
        s.hashed = len;
        Ok(())
    }

    /// `PUT /codex/uploads/{id}` with `Content-Range`: appends one chunk.
    pub async fn chunk(
        &self,
        sh: &Arc<Shared>,
        id: uuid::Uuid,
        range: Option<&str>,
        body: Body,
    ) -> Result<UploadInfo, UploadError> {
        let Ok(_permit) = self.transfers.clone().try_acquire_owned() else {
            return Err(api(
                ErrorCode::RateLimited,
                "the server is receiving as many chunks as it takes at once: try again shortly",
            )
            .into());
        };
        let sess = self.session(sh, id)?;
        let mut s = sess.lock().await;
        if s.done {
            return Err(api(ErrorCode::NotFound, format!("upload {id} not found")).into());
        }
        let Some((start, end, total)) = range.and_then(parse_range) else {
            return Err(ApiError::invalid(
                "Content-Range",
                "send each chunk with Content-Range: bytes <start>-<end>/<size>",
            )
            .into());
        };
        if total != s.state.size || end >= total {
            return Err(ApiError::invalid(
                "Content-Range",
                format!("the upload is {} bytes", s.state.size),
            )
            .into());
        }
        if start != s.offset {
            return Err(UploadError::Offset(
                api(
                    ErrorCode::Conflict,
                    format!("the server has {} bytes: continue from there", s.offset),
                ),
                s.offset,
            ));
        }
        let expected = end - start + 1;
        let dir = Self::dir(sh);
        self.check_space(&dir, expected)?;
        Self::catch_up(&dir, &mut s).await?;
        let path = part(&dir, id);
        // Written at the offset, whatever the file holds past it (bytes of a block that failed
        // part-way, or of a chunk whose request went away mid-write): the file stays the
        // bytes the hash covers.
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|mut f| {
                f.set_len(start)?;
                f.seek(std::io::SeekFrom::Start(start))?;
                Ok(f)
            })
            .map_err(|e| io_err("opening the upload", e))?;
        let before = s.hasher.clone();
        let mut sink = Sink {
            file,
            hasher: s.hasher.clone(),
        };
        let mut stream = body.into_data_stream();
        let mut buf: Vec<u8> = Vec::with_capacity(BLOCK);
        let mut written = 0u64;
        let mut failure: Option<ApiError> = None;
        let mut too_long = false;
        loop {
            match stream.next().await {
                Some(Ok(bytes)) => {
                    if written + (buf.len() + bytes.len()) as u64 > expected {
                        too_long = true;
                        break;
                    }
                    buf.extend_from_slice(&bytes);
                    if buf.len() >= BLOCK {
                        let (s2, r, mut b) = sink.write(std::mem::take(&mut buf)).await;
                        sink = s2;
                        if let Err(e) = r {
                            failure = Some(io_err("writing the upload", e));
                            break;
                        }
                        written += b.len() as u64;
                        b.clear();
                        buf = b;
                    }
                }
                Some(Err(e)) => {
                    failure = Some(api(
                        ErrorCode::BadRequest,
                        format!("the chunk broke off: {e}"),
                    ));
                    break;
                }
                None => break,
            }
        }
        if too_long {
            // Not what Content-Range said: the whole chunk goes.
            let (p, keep) = (path.clone(), start);
            drop(sink);
            let _ = tokio::task::spawn_blocking(move || {
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&p)
                    .and_then(|f| f.set_len(keep))
            })
            .await;
            s.hasher = before;
            s.hashed = start;
            s.offset = start;
            return Err(ApiError::invalid(
                "Content-Range",
                format!("the chunk is longer than the {expected} bytes its Content-Range says"),
            )
            .into());
        }
        if failure.is_none() && !buf.is_empty() {
            let (s2, r, b) = sink.write(std::mem::take(&mut buf)).await;
            sink = s2;
            match r {
                Ok(()) => written += b.len() as u64,
                Err(e) => failure = Some(io_err("writing the upload", e)),
            }
        }
        // The bytes that came stay (a broken chunk is resumed from the new offset).
        s.hasher = sink.hasher.clone();
        s.offset = start + written;
        s.hashed = s.offset;
        s.updated = SystemTime::now();
        drop(sink);
        if failure.is_none() && written < expected {
            failure = Some(api(
                ErrorCode::BadRequest,
                format!(
                    "the chunk ended after {written} of the {expected} bytes its Content-Range says: continue from {}",
                    s.offset
                ),
            ));
        }
        match failure {
            Some(e) => Err(e.into()),
            None => Ok(self.info(&s)),
        }
    }

    /// `DELETE /codex/uploads/{id}`.
    pub async fn cancel(&self, sh: &Shared, id: uuid::Uuid) -> Result<(), ApiError> {
        let sess = self.session(sh, id)?;
        let mut s = sess.lock().await;
        if s.done {
            return Err(api(ErrorCode::NotFound, format!("upload {id} not found")));
        }
        s.done = true;
        let dir = Self::dir(sh);
        let _ = tokio::task::spawn_blocking(move || remove_files(&dir, id)).await;
        drop(s);
        self.forget(sh, id);
        tracing::info!(upload = %id, "Codex: upload cancelled");
        Ok(())
    }

    /// `POST /codex/uploads/{id}/finish`: checks the size and the SHA-256, keeps the file
    /// (deduplicated) and saves its media asset.
    #[allow(clippy::too_many_arguments)]
    pub async fn finish(
        &self,
        sh: &Arc<Shared>,
        show: &dyn Show,
        jobs: &super::worker::MediaWorker,
        id: uuid::Uuid,
        body: FinishUpload,
        client_id: Option<String>,
        by: ChangeBy,
    ) -> Result<UploadFinished, UploadError> {
        let sess = self.session(sh, id)?;
        let mut s = sess.lock().await;
        if s.done {
            return Err(api(ErrorCode::NotFound, format!("upload {id} not found")).into());
        }
        if s.offset != s.state.size {
            return Err(UploadError::Offset(
                api(
                    ErrorCode::Conflict,
                    format!(
                        "{} of the {} bytes came: send the rest first",
                        s.offset, s.state.size
                    ),
                ),
                s.offset,
            ));
        }
        let Some(want) = body.sha256.or_else(|| s.state.sha256.clone()) else {
            return Err(ApiError::invalid(
                "sha256",
                "the file's SHA-256 is needed to finish (here or when starting the upload)",
            )
            .into());
        };
        if !blobs::is_sha256(&want) {
            return Err(ApiError::invalid("sha256", "a SHA-256 is 64 lowercase hex digits").into());
        }
        sh.collection_content(s.state.collection).ok_or_else(|| {
            api(
                ErrorCode::Conflict,
                format!(
                    "collection {} is no longer in the Codex",
                    s.state.collection
                ),
            )
        })?;
        let dir = Self::dir(sh);
        Self::catch_up(&dir, &mut s).await?;
        let got = hex(&s.hasher.clone().finalize());
        if got != want {
            // The bytes aren't the file: nothing to resume.
            s.done = true;
            let d = dir.clone();
            let _ = tokio::task::spawn_blocking(move || remove_files(&d, id)).await;
            drop(s);
            self.forget(sh, id);
            return Err(ApiError::invalid(
                "sha256",
                format!(
                    "the bytes received have the SHA-256 {got}, not {want}: upload the file again"
                ),
            )
            .into());
        }
        let (stem, ext) = blobs::split_name(&s.state.name);
        let kind = blobs::kind_for(&ext).unwrap_or(MediaKind::Unknown);
        if !sh.library().is_open() {
            return Err(api(
                ErrorCode::Unavailable,
                sh.library().problem().unwrap_or_else(|| {
                    "the Codex's media library isn't open: finish the upload again later".into()
                }),
            )
            .into());
        }
        let folder = sh
            .codex()
            .collection(s.state.collection)
            .map(|c| super::library::folder_for(&c.collection.name, c.collection.id))
            .unwrap_or_default();
        let (lib, d, name) = (
            sh.library().clone(),
            dir.clone(),
            staged_name(&s.state.name, id),
        );
        let imported = super::relink::low_priority(move || take_in(&lib, &d, id, &name, &folder))
            .await
            .ok_or_else(|| api(ErrorCode::Internal, "the upload's import stopped"))?
            .map_err(|e| io_err("taking the file into the media library", e))?;
        let (item, sha, deduplicated) = imported;
        s.done = true;
        if sha != got {
            // The library holds other bytes than were uploaded: no asset points at them.
            let state = s.state.clone();
            drop(s);
            self.forget(sh, id);
            tracing::error!(upload = %id, item = %item, name = %state.name, "the media library worked out another SHA-256 ({sha}) than the upload's ({got}): not saved");
            return Err(api(
                ErrorCode::Internal,
                format!(
                    "the media library read other bytes than were uploaded for “{}”: upload the file again",
                    state.name
                ),
            )
            .into());
        }
        let state = s.state.clone();
        drop(s);
        self.forget(sh, id);

        let location = AssetLocation::Library {
            id: item.clone(),
            sha256: sha,
        };
        if let Some(target) = state.replaces {
            // A manual relink of an existing asset to the uploaded file.
            let mut updated = None;
            let revisions = sh
                .codex_writer()
                .media(
                    sh,
                    show,
                    state.collection,
                    |content| {
                        let cur = content.assets.get(&target).ok_or_else(|| {
                            api(
                                ErrorCode::Conflict,
                                format!("media asset {} is no longer in the collection", target.0),
                            )
                        })?;
                        let a = super::relink::manual(content, cur, location.clone())
                            .unwrap_or_else(|| cur.clone());
                        updated = Some(a.clone());
                        Ok(super::writer::MediaPlan {
                            assets: vec![a],
                            bins: Vec::new(),
                        })
                    },
                    client_id,
                    by,
                )
                .await?;
            sh.library().add_marks(std::slice::from_ref(&item));
            let asset = updated.unwrap_or_default();
            let revision = revisions
                .into_iter()
                .next()
                .unwrap_or_else(|| vesper_core::change::revision(&asset));
            jobs.probe(state.collection, target);
            tracing::info!(upload = %id, asset = %target.0, deduplicated, "Codex: upload relinked a media asset");
            return Ok(UploadFinished {
                collection: state.collection,
                asset,
                revision,
                deduplicated,
                bin_item: None,
                relinked: true,
                existing: false,
            });
        }
        let now = SystemTime::now();
        let new_asset = MediaAsset {
            kind,
            name: stem,
            location: location.clone(),
            origin: Some(MediaOrigin {
                relative_path: Some(
                    state
                        .relative_path
                        .clone()
                        .unwrap_or_else(|| state.name.clone()),
                ),
                ..MediaOrigin::default()
            }),
            status: MediaStatus::Online,
            checked_at: Some(now_str(now)),
            ..MediaAsset::default()
        };
        let bin = state.bin;
        // (the asset answered, whether it was written, whether it was there already)
        let mut outcome = None;
        let mut bin_item = None;
        let revisions = sh
            .codex_writer()
            .media(
                sh,
                show,
                state.collection,
                |content| {
                    let mut plan = super::writer::MediaPlan {
                        assets: Vec::new(),
                        bins: Vec::new(),
                    };
                    // The collection has an asset for these bytes already: that one, no second
                    // asset. It points at the uploaded file when it was elsewhere (a missing
                    // item, the old store): the bytes are the same. An asset these bytes were
                    // the original of (media preparation) stays as it is: it plays its
                    // prepared copy, and relinking would undo the preparation.
                    let (asset, existing) = match same_content(content, &location) {
                        Some(cur) if !own_file(cur, &location) => (cur.clone(), true),
                        Some(cur) => match super::relink::manual(content, cur, location.clone()) {
                            Some(a) => {
                                plan.assets.push(a.clone());
                                (a, true)
                            }
                            None => (cur.clone(), true),
                        },
                        None => {
                            plan.assets.push(new_asset);
                            (plan.assets[0].clone(), false)
                        }
                    };
                    if let Some(b) = bin.and_then(|b| content.media_bins.iter().find(|x| x.id == b))
                        && let MediaBinKind::Manual { items } = &b.kind
                    {
                        if let Some(i) = items.iter().find(|i| i.trigger.asset == asset.id) {
                            bin_item = Some(i.id);
                        } else {
                            let (mut b, mut items) = (b.clone(), items.clone());
                            let i = bin_item_for(asset.id, asset.kind);
                            bin_item = Some(i.id);
                            items.push(i);
                            b.kind = MediaBinKind::Manual { items };
                            plan.bins.push(b);
                        }
                    }
                    outcome = Some((asset, !plan.assets.is_empty(), existing));
                    Ok(plan)
                },
                client_id,
                by,
            )
            .await?;
        let Some((asset, written, existing)) = outcome else {
            return Err(api(
                ErrorCode::Internal,
                "the upload's media asset wasn't planned",
            )
            .into());
        };
        sh.library().add_marks(std::slice::from_ref(&item));
        if written {
            jobs.probe(state.collection, asset.id);
        }
        // Without a write (the asset was there as it is): its revision as stored.
        let revision = revisions
            .into_iter()
            .next()
            .filter(|_| written)
            .unwrap_or_else(|| vesper_core::change::revision(&asset));
        tracing::info!(upload = %id, asset = %asset.id.0, deduplicated, existing, "Codex: upload finished");
        Ok(UploadFinished {
            collection: state.collection,
            asset,
            revision,
            deduplicated,
            bin_item,
            relinked: false,
            existing,
        })
    }

    /// The hourly upkeep: uploads without a new chunk for the expiry time go, and files in
    /// `uploads/` that belong to no upload.
    pub async fn clean(&self, sh: &Shared, now: SystemTime) -> usize {
        let dir = Self::dir(sh);
        let all: Vec<(uuid::Uuid, Arc<tokio::sync::Mutex<Session>>)> = self
            .sessions(sh)
            .iter()
            .flat_map(|m| m.iter().map(|(k, v)| (*k, v.clone())))
            .collect();
        let mut gone = Vec::new();
        for (id, sess) in &all {
            let Ok(mut s) = sess.try_lock() else { continue };
            if s.done || s.updated + self.options.upload_expiry <= now {
                s.done = true;
                gone.push(*id);
            }
        }
        for id in &gone {
            self.forget(sh, *id);
        }
        let keep: Vec<uuid::Uuid> = self
            .sessions(sh)
            .iter()
            .flat_map(|m| m.keys().copied())
            .collect();
        let n = gone.len();
        let expiry = self.options.upload_expiry;
        let _ = tokio::task::spawn_blocking(move || {
            for id in &gone {
                remove_files(&dir, *id);
            }
            // Leftovers: files of no upload, once as old as an expired upload (a new upload's
            // files are written just before it is listed).
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.flatten() {
                    let name = e.file_name();
                    let Some(stem) = name.to_str().and_then(|n| n.split('.').next()) else {
                        continue;
                    };
                    if matches!(stem.parse::<uuid::Uuid>(), Ok(id) if keep.contains(&id)) {
                        continue;
                    }
                    let old = e
                        .metadata()
                        .and_then(|m| m.modified())
                        .is_ok_and(|t| t + expiry <= now);
                    if old {
                        if e.file_type().is_ok_and(|t| t.is_dir()) {
                            let _ = std::fs::remove_dir_all(e.path());
                        } else {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                }
            }
        })
        .await;
        if n > 0 {
            tracing::info!(dropped = n, "Codex: stale uploads dropped");
        }
        n
    }

    /// Bytes and number of the uploads in progress.
    pub fn usage(&self, sh: &Shared) -> (u64, u64) {
        disk::dir_usage(&Self::dir(sh))
    }
}

/// The file name of an upload's name (without a client's folders).
/// The collection's asset for the bytes at `location` (the same SHA-256): a shared-library
/// asset, preferably one that is there already, then the lowest ID (the same answer each time).
fn same_content<'a>(
    content: &'a crate::data::CollectionContent,
    location: &AssetLocation,
) -> Option<&'a MediaAsset> {
    let sha = sha_of(location)?;
    content
        .assets
        .values()
        .filter(|a| !matches!(a.scope, AssetScope::Theme { .. }))
        .filter(|a| {
            // Its file, or the original it was converted from (media preparation).
            let original = a.prep.as_ref().and_then(|p| p.original.as_ref());
            std::iter::once(&a.location)
                .chain(original.map(|o| &o.location))
                .any(|l| sha_of(l).is_some_and(|s| s.eq_ignore_ascii_case(sha)))
        })
        .min_by_key(|a| (a.location != *location, a.id.0))
}

/// True when `a`'s own file (not the original it was prepared from) has the bytes at
/// `location`.
fn own_file(a: &MediaAsset, location: &AssetLocation) -> bool {
    match (sha_of(&a.location), sha_of(location)) {
        (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
        _ => false,
    }
}

/// The SHA-256 a location names (library items and the old store's files).
fn sha_of(location: &AssetLocation) -> Option<&str> {
    match location {
        AssetLocation::Library { sha256, .. } | AssetLocation::Managed { sha256, .. } => {
            Some(sha256)
        }
        _ => None,
    }
}

/// A new bin item that plays `asset` (audio on the audio layer, else as the background).
fn bin_item_for(asset: model::MediaAssetId, kind: MediaKind) -> MediaBinItem {
    let layer = if kind == MediaKind::Audio {
        MediaLayer::Audio
    } else {
        MediaLayer::Background
    };
    MediaBinItem {
        id: MediaBinItemId::new(),
        name: None,
        enabled: true,
        trigger: MediaTrigger::new(asset, layer),
    }
}

fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// The file name a finished upload is taken into the library with: its own, made a plain
/// file name on every platform (a Windows Codex box refuses `:` `?` `*` `"` `<` `>` `|`, a
/// trailing dot or space and device names such as `CON`), else the upload's ID with its
/// ending.
fn staged_name(name: &str, id: uuid::Uuid) -> String {
    let name = base_name(name);
    super::media_migration::plain_file_name(name)
        .filter(|n| blobs::split_name(n).1 == blobs::split_name(name).1)
        .unwrap_or_else(|| match blobs::split_name(name).1 {
            ext if ext.is_empty() => id.to_string(),
            ext => format!("{id}.{ext}"),
        })
}

/// `uploads/<id>/`: where a finished upload's file waits, under its own name, for the library.
fn staging(dir: &Path, id: uuid::Uuid) -> PathBuf {
    dir.join(id.to_string())
}

/// Takes a finished upload's file into the media library (blocking; see the module docs): the
/// item, its SHA-256 and whether the library had those bytes already. On failure the file is
/// back as the upload's `.part`.
fn take_in(
    lib: &super::library::MediaLibrary,
    dir: &Path,
    id: uuid::Uuid,
    name: &str,
    folder: &str,
) -> std::io::Result<(String, String, bool)> {
    let p = part(dir, id);
    let stage = staging(dir, id);
    std::fs::create_dir_all(&stage)?;
    let file = stage.join(name);
    std::fs::File::open(&p)?.sync_all()?;
    std::fs::rename(&p, &file)?;
    match lib.import(&file, folder) {
        Ok((item, sha, existed)) => {
            if existed {
                std::fs::remove_file(&file)?;
            }
            let _ = std::fs::remove_dir(&stage);
            let _ = std::fs::remove_file(meta(dir, id));
            Ok((item, sha, existed))
        }
        Err(e) => {
            let _ = std::fs::rename(&file, &p);
            let _ = std::fs::remove_dir(&stage);
            Err(e)
        }
    }
}

/// The free space `bytes` more must leave on the disk holding `dir` (`507` otherwise).
pub fn check_space(opts: &super::CodexOptions, dir: &Path, bytes: u64) -> Result<(), ApiError> {
    let s = disk::space(dir).map_err(|e| io_err("reading the free disk space", e))?;
    let keep = s.reserve(opts.keep_free_percent, opts.keep_free_bytes);
    if s.available < keep.saturating_add(bytes) {
        return Err(api(
            ErrorCode::InsufficientStorage,
            format!(
                "not enough disk space: {} MiB more would leave less than the {} MiB the server keeps free ({} MiB free)",
                bytes >> 20,
                keep >> 20,
                s.available >> 20
            ),
        ));
    }
    Ok(())
}

fn remove_files(dir: &Path, id: uuid::Uuid) {
    for p in [part(dir, id), meta(dir, id)] {
        if let Err(e) = std::fs::remove_file(&p)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(file = %p.display(), "removing an upload: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_names_are_plain_file_names_everywhere() {
        let id = uuid::Uuid::nil();
        assert_eq!(staged_name("Easter: Intro.mp4", id), "Easter_ Intro.mp4");
        assert_eq!(staged_name("a/b/What?*.png", id), "What__.png");
        assert_eq!(staged_name("CON.mp4", id), "_CON.mp4");
        assert_eq!(staged_name("Intro .mp4", id), "Intro .mp4");
        assert_eq!(staged_name("<|>.mp4", id), "___.mp4");
        assert_eq!(staged_name("Sunrise.jpg", id), "Sunrise.jpg");
        let long = format!("{}.mp4", "x".repeat(250));
        assert_eq!(staged_name(&long, id), format!("{id}.mp4"));
    }

    #[test]
    fn content_range() {
        assert_eq!(parse_range("bytes 0-99/200"), Some((0, 99, 200)));
        assert_eq!(parse_range(" bytes 100-199/200 "), Some((100, 199, 200)));
        assert_eq!(parse_range("bytes 5-4/200"), None);
        assert_eq!(parse_range("bytes */200"), None);
        assert_eq!(parse_range("items 0-1/2"), None);
    }

    #[test]
    fn hash_of_a_prefix() {
        let d = std::env::temp_dir().join(format!("codex-upl-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("x");
        std::fs::write(&p, b"hello world").unwrap();
        let h = hash_prefix(&p, 5).unwrap();
        assert_eq!(hex(&h.finalize()), hex(&Sha256::digest(b"hello")));
        std::fs::remove_dir_all(&d).ok();
    }
}
