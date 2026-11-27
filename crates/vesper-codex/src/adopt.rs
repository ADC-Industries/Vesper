//! Uploading a show box's dormant collection to the site's Codex (G15; see
//! `docs/MEDIA-BINS-CODEX-PLAN.md`).
//!
//! A show box that joined a site with a Codex keeps its own Codex dormant
//! (`machine/codex.json ownCollection`, `<data>/codex/collections/<own>/`, media in its own
//! library under `<data>/codex/library/`). This copies that collection into the site's Codex as
//! a **new** collection, over the box-to-Codex link ([`super::link`]):
//!
//! | Kind | Request | Answer |
//! |---|---|---|
//! | [`ADOPT`] | [`AdoptRequest`] `{name, sourceCodex, sourceCollection, machine, boxName}` | [`AdoptAnswer`] `{collection, live, name}`: the collection made for this source (made now, or the one made before: never two) |
//! | [`ADOPT_DOCS`] | [`AdoptDocs`] `{collection, kind, docs: [{id, body}]}` (≤ 8 MiB) | `{written}`: the files, as they are, into the collection's staging folder |
//! | [`ADOPT_FINISH`] | [`AdoptFinish`] `{collection, machine}` | `{collection, documents}`: the staged collection moved into the Codex, every document logged as added (by the show box) |
//! | [`UPLOAD_BEGIN`] | [`UploadBegin`] `{collection, asset, name, size, sha256, machine}` | [`UploadBegun`]: `relinked` (the Codex's library has those bytes: the asset points at them now) or the upload (`codex::uploads`, its `offset`; an open upload of the same bytes for the collection is answered again, so a drop resumes) |
//! | [`UPLOAD_CHUNK`] | JSON [`UploadChunk`] `{upload, offset, size, machine}`, a NUL byte, then the bytes (≤ 4 MiB) | `UploadInfo` |
//! | [`UPLOAD_FINISH`] | [`UploadFinish`] `{upload, machine}` | `UploadFinished` (the upload relinks the asset: `replaces`) |
//!
//! The Codex runs the media requests through its own REST routes (as the show box's machine,
//! like a forwarded write), so uploads keep every rule they have (size, space, SHA-256,
//! deduplication in the library). What it made for which source is kept in
//! `codex/adopted.json`; a collection still being filled is staged in `codex/adopting/<id>/`.
//!
//! **The show box** ([`Adopter`]): `POST /codex/dormant/upload` starts a job on a thread of its
//! own: [`ADOPT`], the documents in pages, [`ADOPT_FINISH`], then every media file the
//! documents use (library items found through the dormant library, opened for the job and
//! closed after; old managed files from `codex/media/`), each one relinked when it lands. Its
//! state is in `machine/codex-adopt.json` (the assets done), so starting again, or the box's
//! restart, resumes. A document of a newer schema than the Codex reads stops it with the
//! reason. The dormant collection is never changed.

use super::link::{self, VERSION};
use crate::Shared;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::ConnectInfo;
use axum::http::Method;
use idn_open_protocol::channel::{Channel, Reply};
use model::{
    AssetLocation, ChangeBy, CodexId, CodexInfo, CodexRole, Collection, CollectionId, DocChange,
    MediaAsset, MediaAssetId,
};
use protocol::{
    ApiError, CodexPart, DormantUpload, DormantUploadState, ErrorCode, StartDormantUpload,
    TransferCount, UploadFinished, UploadInfo,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::{Read as _, Seek as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tower::ServiceExt;

/// `vesper.codex.adopt`.
pub const ADOPT: &str = "vesper.codex.adopt";
/// `vesper.codex.adopt_docs`.
pub const ADOPT_DOCS: &str = "vesper.codex.adopt_docs";
/// `vesper.codex.adopt_finish`.
pub const ADOPT_FINISH: &str = "vesper.codex.adopt_finish";
/// `vesper.codex.upload_begin`.
pub const UPLOAD_BEGIN: &str = "vesper.codex.upload_begin";
/// `vesper.codex.upload_chunk`.
pub const UPLOAD_CHUNK: &str = "vesper.codex.upload_chunk";
/// `vesper.codex.upload_finish`.
pub const UPLOAD_FINISH: &str = "vesper.codex.upload_finish";
/// The link kinds of this module (the Codex's handler sends them here).
pub const KINDS: &[&str] = &[
    ADOPT,
    ADOPT_DOCS,
    ADOPT_FINISH,
    UPLOAD_BEGIN,
    UPLOAD_CHUNK,
    UPLOAD_FINISH,
];
/// Bytes of media per chunk.
pub const CHUNK: usize = 4 << 20;
/// How long one request may take.
const ASK: Duration = Duration::from_secs(60);
/// How long [`ADOPT_FINISH`] waits for the Codex's writer (an import or export holds it for
/// minutes), well within the box's [`ASK`]: a finish the box gave up on is refused, never
/// applied later (the box may be sending the documents again by then).
const FINISH_WAIT: Duration = super::write_through::WRITER_WAIT;
/// A finish that only got going this late is refused as well (the box's [`ASK`] is near).
const FINISH_LATE: Duration = Duration::from_secs(30);

fn v1() -> u32 {
    VERSION
}

fn api(code: ErrorCode, msg: impl Into<String>) -> ApiError {
    ApiError::new(code, msg)
}

fn now() -> String {
    crate::auth::rfc3339(std::time::SystemTime::now(), true)
}

// ---- the wire -------------------------------------------------------------------------------

/// [`ADOPT`]'s request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptRequest {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The name wanted.
    pub name: String,
    /// The show box's own Codex.
    pub source_codex: CodexId,
    /// Its dormant collection.
    pub source_collection: CollectionId,
    /// The show box's server ID.
    pub machine: String,
    /// Its name (a name in use gets " (from <box>)").
    #[serde(default)]
    pub box_name: String,
    /// The box remembers no earlier upload of this source (its first, or the first since it
    /// left the site and joined again): a source already in the Codex becomes a new
    /// collection, so changes made since aren't skipped (R3-1). An older Codex ignores it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fresh: bool,
}

/// [`ADOPT`]'s answer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptAnswer {
    /// The link's version.
    pub v: u32,
    /// The collection made for this source.
    pub collection: CollectionId,
    /// Its name.
    pub name: String,
    /// It is in the Codex already (its documents are done).
    pub live: bool,
}

/// One document of [`AdoptDocs`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptDoc {
    /// Its ID.
    pub id: uuid::Uuid,
    /// Its file.
    pub body: String,
}

/// [`ADOPT_DOCS`]'s request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptDocs {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The collection ([`AdoptAnswer::collection`]).
    pub collection: CollectionId,
    /// The documents' kind (its folder name).
    pub kind: String,
    /// The documents.
    pub docs: Vec<AdoptDoc>,
}

/// [`ADOPT_FINISH`]'s request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptFinish {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The collection.
    pub collection: CollectionId,
    /// The show box's server ID (the change log names it).
    pub machine: String,
}

/// [`UPLOAD_BEGIN`]'s request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadBegin {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The collection.
    pub collection: CollectionId,
    /// The asset the file relinks.
    pub asset: MediaAssetId,
    /// The file's name.
    pub name: String,
    /// Its size.
    pub size: u64,
    /// Its SHA-256.
    pub sha256: String,
    /// The show box's server ID.
    pub machine: String,
}

/// [`UPLOAD_BEGIN`]'s answer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadBegun {
    /// The link's version.
    pub v: u32,
    /// The Codex's library had the bytes: the asset points at them; nothing to send.
    #[serde(default)]
    pub relinked: bool,
    /// Otherwise the upload (send from its `offset`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload: Option<UploadInfo>,
}

/// [`UPLOAD_CHUNK`]'s header (before the NUL byte and the bytes).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadChunk {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The upload.
    pub upload: String,
    /// Where the bytes start.
    pub offset: u64,
    /// The file's size.
    pub size: u64,
    /// The show box's server ID.
    pub machine: String,
}

/// [`UPLOAD_FINISH`]'s request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadFinish {
    /// The link's version.
    #[serde(default = "v1")]
    pub v: u32,
    /// The upload.
    pub upload: String,
    /// The show box's server ID.
    pub machine: String,
}

fn parse<'a, T: Deserialize<'a>>(payload: &'a [u8]) -> Result<T, ApiError> {
    serde_json::from_slice(payload)
        .map_err(|e| api(ErrorCode::BadRequest, format!("bad request: {e}")))
}

// ---- the Codex's side -----------------------------------------------------------------------

/// What the Codex made for which source (`codex/adopted.json`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Adopted {
    #[serde(default)]
    list: Vec<AdoptedEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdoptedEntry {
    source_codex: CodexId,
    source_collection: CollectionId,
    collection: CollectionId,
    name: String,
    by: String,
    at: String,
}

/// One adoption at a time on the Codex (the registry and the staging folders).
static ADOPTING: Mutex<()> = Mutex::new(());

fn registry_path(sh: &Shared) -> PathBuf {
    sh.codex().layout.codex().join("adopted.json")
}

fn staging(sh: &Shared, id: CollectionId) -> PathBuf {
    sh.codex()
        .layout
        .codex()
        .join("adopting")
        .join(id.to_string())
}

fn read_registry(sh: &Shared) -> Adopted {
    std::fs::read(registry_path(sh))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_registry(sh: &Shared, r: &Adopted) -> Result<(), ApiError> {
    let json = serde_json::to_vec_pretty(r).map_err(|e| api(ErrorCode::Internal, e.to_string()))?;
    crate::store::write_atomic(&registry_path(sh), &json).map_err(|e| {
        api(
            ErrorCode::Internal,
            format!("saving codex/adopted.json: {e}"),
        )
    })
}

fn entry_for(sh: &Shared, collection: CollectionId) -> Result<AdoptedEntry, ApiError> {
    read_registry(sh)
        .list
        .into_iter()
        .find(|e| e.collection == collection)
        .ok_or_else(|| {
            api(
                ErrorCode::NotFound,
                format!("collection {collection} isn't one being uploaded from a show box"),
            )
        })
}

/// A request of this module's kinds from the show box `from` (the link checked it is a
/// machine of the site).
pub fn serve(sh: Arc<Shared>, l: &link::Link, kind: &str, payload: &[u8]) -> Reply {
    let r: Result<Reply, ApiError> = (|| {
        Ok(match kind {
            ADOPT => {
                let req: AdoptRequest = parse(payload)?;
                link_ok(&adopt(&sh, req)?)
            }
            ADOPT_DOCS => {
                let req: AdoptDocs = parse(payload)?;
                link_ok(&adopt_docs(&sh, req)?)
            }
            ADOPT_FINISH => {
                let req: AdoptFinish = parse(payload)?;
                l.run(adopt_finish(sh, req))
            }
            UPLOAD_BEGIN => {
                let req: UploadBegin = parse(payload)?;
                let router = l.router().ok_or_else(not_up)?;
                l.run(upload_begin(sh, router, req))
            }
            UPLOAD_CHUNK => {
                let nul = payload.iter().position(|b| *b == 0).ok_or_else(|| {
                    api(
                        ErrorCode::BadRequest,
                        "a chunk is its JSON, a NUL, its bytes",
                    )
                })?;
                let head: UploadChunk = parse(&payload[..nul])?;
                let bytes = Bytes::copy_from_slice(&payload[nul + 1..]);
                let router = l.router().ok_or_else(not_up)?;
                l.run(upload_chunk(sh, router, head, bytes))
            }
            UPLOAD_FINISH => {
                let req: UploadFinish = parse(payload)?;
                let router = l.router().ok_or_else(not_up)?;
                l.run(upload_finish(sh, router, req))
            }
            _ => Reply::Unknown,
        })
    })();
    r.unwrap_or_else(|e| super::backups::remote::error_reply(&e))
}

fn not_up() -> ApiError {
    api(ErrorCode::Unavailable, "the Codex's routes aren't up yet")
}

fn link_ok<T: Serialize>(v: &T) -> Reply {
    match serde_json::to_vec(v) {
        Ok(b) => Reply::Ok(b),
        Err(e) => Reply::Error(e.to_string()),
    }
}

/// [`ADOPT`]: the collection for this source, made now (staged) or found.
fn adopt(sh: &Shared, req: AdoptRequest) -> Result<AdoptAnswer, ApiError> {
    super::writer::writable(sh)?;
    let _g = crate::lock(&ADOPTING);
    let codex = sh.codex();
    let mut reg = read_registry(sh);
    if let Some(i) = reg.list.iter().position(|e| {
        e.source_codex == req.source_codex && e.source_collection == req.source_collection
    }) {
        let e = reg.list[i].clone();
        if let Some(c) = codex.collection(e.collection) {
            if !req.fresh {
                return Ok(AdoptAnswer {
                    v: VERSION,
                    collection: e.collection,
                    name: c.collection.name.clone(),
                    live: true,
                });
            }
            // Uploaded before, and the box has changed it since (it left the site and joined
            // again): uploaded anew as a new collection; the earlier one stays as it is.
            tracing::info!(earlier = %e.collection, machine = %req.machine, "Codex: a show box uploads its own collection again (a new collection)");
            reg.list.remove(i);
        } else {
            let dir = staging(sh, e.collection);
            if dir.join("collection.json").is_file() {
                // The box sends every document again: what it sent before goes (a document it
                // no longer has isn't kept).
                let _ = std::fs::remove_dir_all(dir.join("portable"));
                return Ok(AdoptAnswer {
                    v: VERSION,
                    collection: e.collection,
                    name: e.name,
                    live: false,
                });
            }
            // (Deleted since: made again.)
            reg.list.remove(i);
        }
    }
    let mut name: String = req
        .name
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(180)
        .collect();
    if name.is_empty() {
        name = "Uploaded collection".into();
    }
    let taken = |n: &str| {
        codex
            .collections
            .iter()
            .any(|c| c.collection.name.to_lowercase() == n.to_lowercase())
    };
    if taken(&name) {
        let from = if req.box_name.trim().is_empty() {
            "a show box"
        } else {
            req.box_name.trim()
        };
        name = format!("{name} (from {from})");
    }
    let last = codex
        .collections
        .iter()
        .map(|c| c.collection.sort_key.as_str())
        .filter(|k| !k.is_empty())
        .max();
    let c = Collection {
        schema_version: model::SCHEMA_VERSION,
        id: CollectionId::new(),
        name: name.clone(),
        sort_key: model::sort_key_between(last, None),
        created_at: now(),
        ..Collection::default()
    };
    let dir = staging(sh, c.id);
    std::fs::create_dir_all(dir.join("portable"))
        .map_err(|e| api(ErrorCode::Internal, format!("staging the collection: {e}")))?;
    crate::store::write_atomic(
        &dir.join("collection.json"),
        &serde_json::to_vec_pretty(&c).unwrap_or_default(),
    )
    .map_err(|e| api(ErrorCode::Internal, format!("staging the collection: {e}")))?;
    reg.list.push(AdoptedEntry {
        source_codex: req.source_codex,
        source_collection: req.source_collection,
        collection: c.id,
        name: name.clone(),
        by: req.machine.clone(),
        at: now(),
    });
    write_registry(sh, &reg)?;
    tracing::info!(collection = %c.id, name = %name, machine = %req.machine, "Codex: a show box uploads its own collection");
    Ok(AdoptAnswer {
        v: VERSION,
        collection: c.id,
        name,
        live: false,
    })
}

/// The schema a document's file says it was written with, when newer than this server reads.
fn too_new(body: &serde_json::Value) -> Option<u64> {
    body.get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .filter(|v| *v > u64::from(model::NEWEST_SCHEMA_VERSION))
}

/// [`ADOPT_DOCS`]: documents into the staging folder.
fn adopt_docs(sh: &Shared, req: AdoptDocs) -> Result<serde_json::Value, ApiError> {
    super::writer::writable(sh)?;
    let _g = crate::lock(&ADOPTING);
    entry_for(sh, req.collection)?;
    if sh.codex().collection(req.collection).is_some() {
        return Err(api(
            ErrorCode::Conflict,
            format!(
                "collection {} is in the Codex already: its documents are done",
                req.collection
            ),
        ));
    }
    let dir = staging(sh, req.collection);
    if !dir.join("collection.json").is_file() {
        return Err(api(
            ErrorCode::NotFound,
            format!("collection {} isn't staged here", req.collection),
        ));
    }
    if link::collection_kind(&req.kind).is_none() {
        return Err(ApiError::invalid(
            "kind",
            format!("{}: not a collection kind", req.kind),
        ));
    }
    let kind_dir = dir.join("portable").join(&req.kind);
    std::fs::create_dir_all(&kind_dir)
        .map_err(|e| api(ErrorCode::Internal, format!("staging: {e}")))?;
    let mut written = 0u32;
    for d in &req.docs {
        let v: serde_json::Value = serde_json::from_str(&d.body).map_err(|e| {
            ApiError::invalid("docs", format!("{} {} isn't JSON: {e}", req.kind, d.id))
        })?;
        if let Some(s) = too_new(&v) {
            return Err(api(
                ErrorCode::Conflict,
                format!(
                    "{} {} was written by a newer server (schema {s}; the site's server reads up to {}): update the site's server first",
                    req.kind,
                    d.id,
                    model::NEWEST_SCHEMA_VERSION
                ),
            ));
        }
        crate::store::write_atomic(&kind_dir.join(format!("{}.json", d.id)), d.body.as_bytes())
            .map_err(|e| api(ErrorCode::Internal, format!("staging {}: {e}", d.id)))?;
        written += 1;
    }
    Ok(serde_json::json!({ "v": VERSION, "written": written }))
}

/// [`ADOPT_FINISH`]: the staged collection moved into the Codex and logged.
async fn adopt_finish(sh: Arc<Shared>, req: AdoptFinish) -> Result<serde_json::Value, ApiError> {
    let arrived = std::time::Instant::now();
    let busy = || {
        api(
            ErrorCode::Unavailable,
            "the site's server is busy with a long job (an import or export): the upload stopped before its documents went in, start it again in a minute",
        )
    };
    let Ok(_w) = tokio::time::timeout(FINISH_WAIT, sh.codex_writer().exclusive()).await else {
        return Err(busy());
    };
    super::writer::writable(&sh)?;
    entry_for(&sh, req.collection)?;
    let id = req.collection;
    if let Some(c) = sh.codex().collection(id) {
        let n = c.content.as_ref().map(|c| c.hashes.len()).unwrap_or(0);
        return Ok(serde_json::json!({"v": VERSION, "collection": id, "documents": n}));
    }
    let codex = sh.codex();
    let layout = codex.layout.clone();
    let (from, roots, lib) = (
        staging(&sh, id),
        sh.data().content.media_roots.clone(),
        sh.library().clone(),
    );
    let loaded = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let _g = crate::lock(&ADOPTING);
        if arrived.elapsed() >= FINISH_LATE {
            return Ok(None);
        }
        anyhow::ensure!(
            from.join("collection.json").is_file(),
            "collection {id} isn't staged here"
        );
        let to = layout.collection(id);
        anyhow::ensure!(!to.exists(), "collection {id} appeared meanwhile");
        std::fs::create_dir_all(layout.collections())?;
        std::fs::rename(&from, &to)?;
        let mut issues = Vec::new();
        let c = super::load_collection(&layout, &to, &lib, &roots, &mut issues)
            .ok_or_else(|| anyhow::anyhow!("collection {id} can't be read after the upload"))?;
        for i in issues {
            tracing::warn!("Codex: an uploaded collection: {}: {}", i.file, i.error);
        }
        Ok(Some(c))
    })
    .await
    .map_err(|e| api(ErrorCode::Internal, e.to_string()))?
    .map_err(|e| api(ErrorCode::Internal, format!("{e:#}")))?
    .ok_or_else(busy)?;
    let mut entries: Vec<super::log::NewChange> = loaded
        .content
        .as_ref()
        .map(|c| {
            c.hashes
                .keys()
                .map(|(kind, doc)| super::log::NewChange {
                    kind: *kind,
                    id: *doc,
                    change: DocChange::Added,
                    revision: None,
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by_key(|e| (crate::data::kind_dir(e.kind), e.id));
    let n = entries.len();
    sh.set_codex(Arc::new(sh.codex().with_collection(loaded)));
    sh.library().marks_due();
    // Added by the show box's server (the machine names the box).
    let by = ChangeBy {
        client: crate::site::SERVER_MACHINE.to_string(),
        client_id: None,
        machine: req.machine.clone(),
    };
    sh.codex_writer().log(&sh, id, entries, &by).await;
    super::writer::codex_changed(&sh, &[CodexPart::Status, CodexPart::Collections], Some(id));
    tracing::info!(collection = %id, documents = n, "Codex: a show box's collection is in");
    Ok(serde_json::json!({"v": VERSION, "collection": id, "documents": n}))
}

/// Runs a REST request through the Codex's own routes as the show box's machine.
async fn rest<T: serde::de::DeserializeOwned>(
    router: Router,
    machine: &str,
    collection: CollectionId,
    method: Method,
    path: &str,
    headers: &[(&str, String)],
    body: Body,
) -> Result<T, ApiError> {
    let mut b = axum::http::Request::builder()
        .method(method)
        .uri(format!("{}{path}", protocol::API_PREFIX));
    for (k, v) in headers {
        b = b.header(*k, v.as_str());
    }
    let mut r = b
        .body(body)
        .map_err(|e| api(ErrorCode::Internal, e.to_string()))?;
    r.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([0, 0, 0, 0], 0))));
    r.extensions_mut().insert(crate::Forwarded {
        access: crate::auth::Access::Local,
        machine: machine.to_string(),
        collection,
    });
    let resp = match router.oneshot(r).await {
        Ok(r) => r,
        Err(e) => match e {},
    };
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
        .await
        .map_err(|e| api(ErrorCode::Internal, format!("the answer: {e}")))?;
    if !status.is_success() {
        return Err(serde_json::from_slice::<protocol::ErrorResponse>(&bytes)
            .map(|e| e.error)
            .unwrap_or_else(|_| {
                api(
                    ErrorCode::Internal,
                    format!("{path}: {status}: {}", String::from_utf8_lossy(&bytes)),
                )
            }));
    }
    serde_json::from_slice(&bytes).map_err(|e| {
        api(
            ErrorCode::Internal,
            format!("{path}: unreadable answer: {e}"),
        )
    })
}

/// [`UPLOAD_BEGIN`].
async fn upload_begin(
    sh: Arc<Shared>,
    router: Router,
    req: UploadBegin,
) -> Result<UploadBegun, ApiError> {
    entry_for(&sh, req.collection)?;
    let content = sh.collection_content(req.collection).ok_or_else(|| {
        api(
            ErrorCode::Conflict,
            format!("collection {} isn't in the Codex yet", req.collection),
        )
    })?;
    let asset = content.assets.get(&req.asset).cloned().ok_or_else(|| {
        ApiError::invalid(
            "asset",
            format!("media asset {} is not in the collection", req.asset.0),
        )
    })?;
    // The library has the bytes already: the asset points at them (unless it does).
    if let Some((item, _)) = sh.library().by_sha(&req.sha256) {
        let here = matches!(&asset.location, AssetLocation::Library { id, .. } if *id == item);
        if !here {
            let (_, ext) = super::blobs::split_name(&req.name);
            let body = serde_json::json!({"type": "blob", "sha256": req.sha256, "extension": ext});
            let _: serde_json::Value = rest(
                router,
                &req.machine,
                req.collection,
                Method::POST,
                &format!(
                    "/codex/collections/{}/media/{}/relink",
                    req.collection, req.asset.0
                ),
                &[("content-type", "application/json".into())],
                Body::from(body.to_string()),
            )
            .await?;
        }
        return Ok(UploadBegun {
            v: VERSION,
            relinked: true,
            upload: None,
        });
    }
    // An open upload of these bytes for the collection: answered again (resumes).
    let open = sh.uploads().list(&sh).await.into_iter().find(|u| {
        u.collection == req.collection
            && u.sha256.as_deref() == Some(req.sha256.as_str())
            && u.size == req.size
    });
    if let Some(u) = open {
        return Ok(UploadBegun {
            v: VERSION,
            relinked: false,
            upload: Some(u),
        });
    }
    let body = protocol::CreateUpload {
        collection: req.collection,
        name: req.name.clone(),
        size: req.size,
        sha256: Some(req.sha256.clone()),
        replaces: Some(req.asset),
        ..Default::default()
    };
    let info: UploadInfo = rest(
        router,
        &req.machine,
        req.collection,
        Method::POST,
        "/codex/uploads",
        &[("content-type", "application/json".into())],
        Body::from(serde_json::to_vec(&body).unwrap_or_default()),
    )
    .await?;
    Ok(UploadBegun {
        v: VERSION,
        relinked: false,
        upload: Some(info),
    })
}

/// [`UPLOAD_CHUNK`].
async fn upload_chunk(
    sh: Arc<Shared>,
    router: Router,
    head: UploadChunk,
    bytes: Bytes,
) -> Result<UploadInfo, ApiError> {
    let id: uuid::Uuid = head
        .upload
        .parse()
        .map_err(|_| ApiError::invalid("upload", "not an upload ID"))?;
    let info = sh.uploads().get(&sh, id).await?;
    if bytes.is_empty() {
        return Ok(info);
    }
    let end = head.offset + bytes.len() as u64 - 1;
    let range = format!("bytes {}-{end}/{}", head.offset, head.size);
    rest(
        router,
        &head.machine,
        info.collection,
        Method::PUT,
        &format!("/codex/uploads/{id}"),
        &[("content-range", range)],
        Body::from(bytes),
    )
    .await
}

/// [`UPLOAD_FINISH`].
async fn upload_finish(
    sh: Arc<Shared>,
    router: Router,
    req: UploadFinish,
) -> Result<UploadFinished, ApiError> {
    let id: uuid::Uuid = req
        .upload
        .parse()
        .map_err(|_| ApiError::invalid("upload", "not an upload ID"))?;
    let info = sh.uploads().get(&sh, id).await?;
    rest(
        router,
        &req.machine,
        info.collection,
        Method::POST,
        &format!("/codex/uploads/{id}/finish"),
        &[],
        Body::empty(),
    )
    .await
}

// ---- the show box's side --------------------------------------------------------------------

/// The job's state as saved (`machine/codex-adopt.json`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    upload: DormantUpload,
    #[serde(default)]
    assets_done: BTreeSet<MediaAssetId>,
    /// The name asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// The site's Codex the job uploads to (its box's server ID; empty in a file of an older
    /// server). A job for another site's Codex is over: the offer is made again.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    codex: String,
}

impl Saved {
    /// The job is this dormant collection's, for this site's Codex.
    fn belongs(&self, own: CollectionId, codex: &str) -> bool {
        self.upload.source == own && (self.codex.is_empty() || self.codex == codex)
    }

    /// [`ADOPT`] answered `target`: media sent to another collection (one deleted since,
    /// another Codex's) are sent again.
    fn target(&mut self, target: CollectionId) {
        if self.upload.collection.is_some_and(|c| c != target) {
            self.assets_done.clear();
            self.upload.media = TransferCount::default();
            self.upload.bytes = TransferCount::default();
        }
    }

    /// Cancels a job no thread runs (a restart, the Codex out of reach): it isn't resumed.
    fn cancel_idle(&mut self) -> bool {
        if self.upload.state != DormantUploadState::Copying {
            return false;
        }
        self.upload.state = DormantUploadState::Cancelled;
        self.upload.finished_at = Some(now());
        true
    }
}

/// The site's Codex this box follows (its box's server ID; empty: none).
fn site_codex(sh: &Shared) -> String {
    sh.codex()
        .config
        .codex
        .as_ref()
        .map(|c| c.server_id.clone())
        .unwrap_or_default()
}

/// A show box's upload of its dormant collection (one per server, in [`crate::Server::adopt`]).
#[derive(Default)]
pub struct Adopter {
    saved: Mutex<Option<Saved>>,
    running: AtomicBool,
    cancel: AtomicBool,
    /// Held by [`Adopter::start`] from taking `running` until the job is set up, and by the
    /// job's thread from publishing its last state until `running` is reset: a start right
    /// after a client saw the old job end starts a new one, and the old thread never writes
    /// over the new job's state (M-1).
    turn: Mutex<()>,
    /// Tests: how long the job's thread waits between publishing its last state and
    /// resetting `running` (widens M-1's window).
    tail_pause: Mutex<Option<std::time::Duration>>,
}

fn saved_path(sh: &Shared) -> PathBuf {
    sh.codex()
        .layout
        .data()
        .join("machine")
        .join("codex-adopt.json")
}

/// The dormant collection: its ID and folder.
fn dormant(sh: &Shared) -> Option<(CollectionId, PathBuf)> {
    let codex = sh.codex();
    if codex.config.role != CodexRole::Client {
        return None;
    }
    let own = codex.config.own_collection?;
    let dir = codex.layout.collection(own);
    dir.join("collection.json").is_file().then_some((own, dir))
}

fn no_dormant() -> ApiError {
    api(
        ErrorCode::NotFound,
        "this box has no dormant collection of its own (it isn't a show box of a site's Codex, or its own Codex was empty)",
    )
}

fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

/// The collection's documents: (kind folder, ID, path).
fn documents(dir: &Path) -> Vec<(String, uuid::Uuid, PathBuf)> {
    let mut out = Vec::new();
    for kind in crate::data::COLLECTION_KINDS {
        let name = crate::data::kind_dir(*kind);
        let Ok(rd) = std::fs::read_dir(dir.join("portable").join(&name)) else {
            continue;
        };
        let mut files: Vec<(uuid::Uuid, PathBuf)> = rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let p = e.path();
                let id = p
                    .file_name()?
                    .to_str()?
                    .strip_suffix(".json")?
                    .parse()
                    .ok()?;
                Some((id, p))
            })
            .collect();
        files.sort();
        out.extend(files.into_iter().map(|(id, p)| (name.clone(), id, p)));
    }
    out
}

impl Adopter {
    /// Nothing loaded yet (read from `machine/codex-adopt.json` on first use).
    pub fn new() -> Adopter {
        Adopter::default()
    }

    fn with_saved<R>(&self, sh: &Shared, f: impl FnOnce(&mut Option<Saved>) -> R) -> R {
        let mut g = crate::lock(&self.saved);
        if g.is_none() {
            *g = read_json(&saved_path(sh));
        }
        f(&mut g)
    }

    fn save(&self, sh: &Shared) {
        // Written under the lock: a [`Adopter::forget`] meanwhile (which removes the file under
        // it) must not be undone by a save of the state from before it.
        let g = crate::lock(&self.saved);
        if let Some(s) = &*g {
            let p = saved_path(sh);
            if let Ok(json) = serde_json::to_vec_pretty(s)
                && let Err(e) = crate::store::write_atomic(&p, &json)
            {
                tracing::warn!("saving {}: {e}", p.display());
            }
        }
    }

    fn update(&self, sh: &Shared, f: impl FnOnce(&mut DormantUpload)) {
        self.with_saved(sh, |s| {
            if let Some(s) = s {
                f(&mut s.upload);
            }
        });
    }

    /// `GET /codex/dormant/upload`: the offer (`idle`) or the job.
    pub fn status(&self, sh: &Shared) -> Result<DormantUpload, ApiError> {
        let (own, dir) = dormant(sh).ok_or_else(no_dormant)?;
        let codex = site_codex(sh);
        let saved = self.with_saved(sh, |s| {
            s.as_ref()
                .filter(|s| s.belongs(own, &codex))
                .map(|s| s.upload.clone())
        });
        if let Some(u) = saved {
            return Ok(u);
        }
        let c: Option<Collection> = read_json(&dir.join("collection.json"));
        let songs = std::fs::read_dir(dir.join("portable").join("presentation"))
            .map(|rd| rd.filter(|e| e.is_ok()).count() as u32)
            .unwrap_or(0);
        Ok(DormantUpload {
            state: DormantUploadState::Idle,
            source: own,
            name: c.map(|c| c.name).unwrap_or_default(),
            songs,
            ..DormantUpload::default()
        })
    }

    /// `POST /codex/dormant/upload`: starts (or resumes) the job.
    pub fn start(
        &self,
        sh: &Arc<Shared>,
        body: StartDormantUpload,
    ) -> Result<DormantUpload, ApiError> {
        let (own, _) = dormant(sh).ok_or_else(no_dormant)?;
        if !sh.mirror().linked() {
            return Err(api(
                ErrorCode::Unavailable,
                "the site's Codex can't be reached from this box now",
            ));
        }
        let turn = crate::lock(&self.turn);
        if self.running.swap(true, Ordering::SeqCst) {
            drop(turn);
            return self.status(sh);
        }
        let offer = match self.status(sh) {
            Ok(o) => o,
            Err(e) => {
                self.running.store(false, Ordering::SeqCst);
                return Err(e);
            }
        };
        self.cancel.store(false, Ordering::SeqCst);
        let codex = site_codex(sh);
        self.with_saved(sh, |s| {
            let keep = s.as_ref().is_some_and(|s| s.belongs(own, &codex));
            if !keep {
                *s = Some(Saved::default());
            }
            let s = s.as_mut().expect("saved");
            s.codex = codex;
            if body.name.is_some() {
                s.name = body.name.clone();
            }
            s.upload = DormantUpload {
                state: DormantUploadState::Copying,
                error: None,
                started_at: Some(now()),
                finished_at: None,
                ..offer
            };
        });
        self.save(sh);
        super::writer::codex_changed(sh, &[CodexPart::Status], None);
        let weak = Arc::downgrade(sh);
        let spawned = std::thread::Builder::new()
            .name("codex-adopt".into())
            .spawn(move || {
                let Some(sh) = weak.upgrade() else { return };
                let r = sh.adopt().run(&sh);
                let turn = crate::lock(&sh.adopt().turn);
                let cancelled = sh.adopt().cancel.load(Ordering::SeqCst);
                sh.adopt().update(&sh, |u| {
                    u.finished_at = Some(now());
                    match &r {
                        Ok(()) => u.state = DormantUploadState::Done,
                        Err(_) if cancelled => u.state = DormantUploadState::Cancelled,
                        Err(e) => {
                            u.state = DormantUploadState::Failed;
                            u.error = Some(e.message.clone());
                        }
                    }
                });
                match &r {
                    Ok(()) => {
                        tracing::info!("Codex: this box's own collection is in the site's Codex")
                    }
                    Err(e) => tracing::warn!(
                        "Codex: uploading this box's own collection stopped: {}",
                        e.message
                    ),
                }
                sh.adopt().save(&sh);
                let pause = *crate::lock(&sh.adopt().tail_pause);
                if let Some(d) = pause {
                    std::thread::sleep(d);
                }
                sh.adopt().running.store(false, Ordering::SeqCst);
                drop(turn);
                super::writer::codex_changed(&sh, &[CodexPart::Status], None);
            });
        if let Err(e) = spawned {
            self.running.store(false, Ordering::SeqCst);
            return Err(api(
                ErrorCode::Internal,
                format!("the upload didn't start: {e}"),
            ));
        }
        drop(turn);
        self.status(sh)
    }

    /// Tests only: the job's thread waits `d` between publishing its last state and taking
    /// itself off as running.
    #[doc(hidden)]
    pub fn pause_tail_for_tests(&self, d: Option<std::time::Duration>) {
        *crate::lock(&self.tail_pause) = d;
    }

    /// `DELETE /codex/dormant/upload`: stops the job (what is in the Codex stays; starting
    /// again resumes). A job no thread runs (saved as copying) is cancelled as saved, so it
    /// isn't resumed later.
    pub fn cancel(&self, sh: &Shared) -> Result<(), ApiError> {
        dormant(sh).ok_or_else(no_dormant)?;
        if self.running.load(Ordering::SeqCst) {
            self.cancel.store(true, Ordering::SeqCst);
            return Ok(());
        }
        let stopped = self.with_saved(sh, |s| s.as_mut().is_some_and(Saved::cancel_idle));
        if stopped {
            self.save(sh);
            super::writer::codex_changed(sh, &[CodexPart::Status], None);
        }
        Ok(())
    }

    /// The box left the site (its own Codex wakes): the job and its state are forgotten (a
    /// later site gets the offer again; a running job stops).
    pub fn forget(&self, sh: &Shared) {
        self.cancel.store(true, Ordering::SeqCst);
        let mut g = crate::lock(&self.saved);
        let p = saved_path(sh);
        match std::fs::remove_file(&p) {
            Ok(()) => tracing::info!("Codex: this box left the site: its upload's state is reset"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!("removing {}: {e}", p.display()),
        }
        *g = None;
    }

    /// The box's site tick: a job that was copying when the server stopped goes on (once the
    /// Codex is reachable).
    pub fn resume(&self, sh: &Arc<Shared>) {
        if self.running.load(Ordering::SeqCst) || !sh.mirror().linked() {
            return;
        }
        let copying = self.with_saved(sh, |s| {
            s.as_ref()
                .is_some_and(|s| s.upload.state == DormantUploadState::Copying)
        });
        if copying && dormant(sh).is_some() {
            tracing::info!("Codex: uploading this box's own collection goes on");
            let _ = self.start(sh, StartDormantUpload::default());
        }
    }

    fn stopped(&self) -> Result<(), ApiError> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(api(ErrorCode::Conflict, "cancelled"));
        }
        Ok(())
    }

    /// The job (on its thread).
    fn run(&self, sh: &Arc<Shared>) -> Result<(), ApiError> {
        let (own, dir) = dormant(sh).ok_or_else(no_dormant)?;
        let (channel, codex_fp) = sh.mirror().channel()?;
        let layout = sh.codex().layout.clone();
        let info: CodexInfo = read_json(&layout.codex_json()).ok_or_else(|| {
            api(
                ErrorCode::Internal,
                "this box's own codex.json can't be read",
            )
        })?;
        let identity = sh.auth().identity();
        let machine = identity.id.to_string();
        let collection: Collection = read_json(&dir.join("collection.json")).unwrap_or_default();
        let wanted = self
            .with_saved(sh, |s| s.as_ref().and_then(|s| s.name.clone()))
            .unwrap_or(collection.name.clone());
        // No collection on the Codex known for this job yet (R3-1).
        let fresh = self
            .with_saved(sh, |s| s.as_ref().and_then(|s| s.upload.collection))
            .is_none();

        // 1. The collection.
        let adopted: AdoptAnswer = request(
            &channel,
            &codex_fp,
            ADOPT,
            json_payload(&AdoptRequest {
                v: VERSION,
                name: wanted,
                source_codex: info.id,
                source_collection: own,
                machine: machine.clone(),
                box_name: identity.name.clone(),
                fresh,
            })?,
        )?;
        let target = adopted.collection;
        let docs = documents(&dir);
        self.with_saved(sh, |s| {
            if let Some(s) = s {
                s.target(target);
            }
        });
        self.update(sh, |u| {
            u.collection = Some(target);
            u.collection_name = Some(adopted.name.clone());
            u.documents = TransferCount {
                done: 0,
                total: docs.len() as u64,
            };
        });
        self.save(sh);

        // 2. The documents, in pages.
        if !adopted.live {
            let mut page: Vec<AdoptDoc> = Vec::new();
            let mut page_kind = String::new();
            let mut bytes = 0usize;
            let send = |kind: &str, page: &mut Vec<AdoptDoc>| -> Result<(), ApiError> {
                if page.is_empty() {
                    return Ok(());
                }
                let n = page.len() as u64;
                let _: serde_json::Value = request(
                    &channel,
                    &codex_fp,
                    ADOPT_DOCS,
                    json_payload(&AdoptDocs {
                        v: VERSION,
                        collection: target,
                        kind: kind.to_string(),
                        docs: std::mem::take(page),
                    })?,
                )?;
                self.update(sh, |u| u.documents.done += n);
                Ok(())
            };
            for (kind, id, path) in &docs {
                self.stopped()?;
                let body = std::fs::read_to_string(path)
                    .map_err(|e| api(ErrorCode::Internal, format!("reading {kind} {id}: {e}")))?;
                if *kind != page_kind || (bytes > 0 && bytes + body.len() > link::DOCS_BUDGET) {
                    send(&page_kind, &mut page)?;
                    bytes = 0;
                    page_kind = kind.clone();
                }
                bytes += body.len();
                page.push(AdoptDoc { id: *id, body });
            }
            send(&page_kind, &mut page)?;
            let _: serde_json::Value = request(
                &channel,
                &codex_fp,
                ADOPT_FINISH,
                json_payload(&AdoptFinish {
                    v: VERSION,
                    collection: target,
                    machine: machine.clone(),
                })?,
            )?;
        }
        self.update(sh, |u| u.documents.done = u.documents.total);
        self.save(sh);

        // 3. The media files.
        let files = self.media_files(sh, &layout, &docs)?;
        let done: BTreeSet<MediaAssetId> = self.with_saved(sh, |s| {
            s.as_ref()
                .map(|s| s.assets_done.clone())
                .unwrap_or_default()
        });
        self.update(sh, |u| {
            u.media = TransferCount {
                done: files.iter().filter(|f| done.contains(&f.asset)).count() as u64,
                total: files.len() as u64,
            };
            u.bytes = TransferCount {
                done: files
                    .iter()
                    .filter(|f| done.contains(&f.asset))
                    .map(|f| f.size)
                    .sum(),
                total: files.iter().map(|f| f.size).sum(),
            };
        });
        for f in files.iter().filter(|f| !done.contains(&f.asset)) {
            self.stopped()?;
            self.send_file(sh, &channel, &codex_fp, &machine, target, f)?;
            self.with_saved(sh, |s| {
                if let Some(s) = s {
                    s.assets_done.insert(f.asset);
                    s.upload.media.done += 1;
                }
            });
            self.save(sh);
        }
        Ok(())
    }

    /// Every media file the collection's assets point at on this box (library items through
    /// the dormant library, opened for this and closed after; old managed files).
    fn media_files(
        &self,
        sh: &Shared,
        layout: &super::Layout,
        docs: &[(String, uuid::Uuid, PathBuf)],
    ) -> Result<Vec<MediaFile>, ApiError> {
        let mut assets: Vec<MediaAsset> = docs
            .iter()
            .filter(|(k, _, _)| k == "mediaAsset")
            .filter_map(|(_, _, p)| read_json(p))
            .collect();
        assets.sort_by_key(|a| a.id.0);
        let needs_library = assets
            .iter()
            .any(|a| matches!(a.location, AssetLocation::Library { .. }));
        let library = if needs_library && layout.library().is_dir() {
            match idn_open_protocol::library::server::LibraryServer::open(
                &layout.library(),
                super::library::NAME,
            ) {
                Ok(l) => Some(l),
                Err(e) => {
                    return Err(api(
                        ErrorCode::Internal,
                        format!("this box's own media library can't be opened: {e}"),
                    ));
                }
            }
        } else {
            None
        };
        let _ = sh;
        let mut out = Vec::new();
        for a in assets {
            let path = match &a.location {
                AssetLocation::Library { id, .. } => library.as_ref().and_then(|l| l.path_of(id)),
                AssetLocation::Managed { .. } => super::blobs::managed_name(&a.location)
                    .map(|n| layout.media().join(n))
                    .filter(|p| p.is_file()),
                _ => None,
            };
            let Some(path) = path else { continue };
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let stem: String = a
                .name
                .chars()
                .filter(|c| !c.is_control() && !matches!(c, '/' | '\\'))
                .take(200)
                .collect();
            let stem = if stem.trim().is_empty() {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "media".into())
            } else {
                stem.trim().to_string()
            };
            let name = if ext.is_empty() {
                stem
            } else {
                format!("{stem}.{ext}")
            };
            out.push(MediaFile {
                asset: a.id,
                path,
                name,
                size: meta.len(),
            });
        }
        // (The dormant library closes here: the box's own Codex stays as it was.)
        drop(library);
        Ok(out)
    }

    fn send_file(
        &self,
        sh: &Shared,
        channel: &Channel,
        codex_fp: &str,
        machine: &str,
        collection: CollectionId,
        f: &MediaFile,
    ) -> Result<(), ApiError> {
        let sha = sha_file(&f.path)
            .map_err(|e| api(ErrorCode::Internal, format!("reading {}: {e}", f.name)))?;
        let begin = || -> Result<UploadBegun, ApiError> {
            request(
                channel,
                codex_fp,
                UPLOAD_BEGIN,
                json_payload(&UploadBegin {
                    v: VERSION,
                    collection,
                    asset: f.asset,
                    name: f.name.clone(),
                    size: f.size,
                    sha256: sha.clone(),
                    machine: machine.to_string(),
                })?,
            )
        };
        let begun = begin()?;
        let Some(mut info) = begun.upload else {
            self.update(sh, |u| u.bytes.done += f.size);
            return Ok(());
        };
        self.update(sh, |u| u.bytes.done += info.offset);
        let mut file = std::fs::File::open(&f.path)
            .map_err(|e| api(ErrorCode::Internal, format!("opening {}: {e}", f.name)))?;
        let mut retried = false;
        while info.offset < f.size {
            self.stopped()?;
            let len = (f.size - info.offset).min(CHUNK as u64) as usize;
            let mut buf = vec![0u8; len];
            file.seek(std::io::SeekFrom::Start(info.offset))
                .and_then(|_| file.read_exact(&mut buf))
                .map_err(|e| api(ErrorCode::Internal, format!("reading {}: {e}", f.name)))?;
            let mut payload = serde_json::to_vec(&UploadChunk {
                v: VERSION,
                upload: info.id.clone(),
                offset: info.offset,
                size: f.size,
                machine: machine.to_string(),
            })
            .map_err(|e| api(ErrorCode::Internal, e.to_string()))?;
            payload.push(0);
            payload.extend_from_slice(&buf);
            let before = info.offset;
            match request::<UploadInfo>(channel, codex_fp, UPLOAD_CHUNK, payload) {
                Ok(i) => {
                    retried = false;
                    info = i;
                }
                Err(e) if !retried => {
                    // Where does the upload stand? (A drop, or a chunk the Codex had.)
                    tracing::debug!(
                        "Codex: a chunk of {}: {}: asking where it stands",
                        f.name,
                        e.message
                    );
                    retried = true;
                    match begin()?.upload {
                        Some(i) => info = i,
                        None => return Ok(()),
                    }
                }
                Err(e) => return Err(e),
            }
            let moved = info.offset.saturating_sub(before);
            self.update(sh, |u| u.bytes.done += moved);
        }
        let _: UploadFinished = request(
            channel,
            codex_fp,
            UPLOAD_FINISH,
            json_payload(&UploadFinish {
                v: VERSION,
                upload: info.id.clone(),
                machine: machine.to_string(),
            })?,
        )?;
        Ok(())
    }
}

/// A media file to send.
struct MediaFile {
    asset: MediaAssetId,
    path: PathBuf,
    name: String,
    size: u64,
}

fn sha_file(p: &Path) -> std::io::Result<String> {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    let mut f = std::fs::File::open(p)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn json_payload(v: &impl Serialize) -> Result<Vec<u8>, ApiError> {
    serde_json::to_vec(v).map_err(|e| api(ErrorCode::Internal, e.to_string()))
}

/// A request to the Codex and its answer (errors as the Codex's `ApiError`).
fn request<T: serde::de::DeserializeOwned>(
    channel: &Channel,
    codex: &str,
    kind: &str,
    payload: Vec<u8>,
) -> Result<T, ApiError> {
    let unavailable = |m: String| api(ErrorCode::Unavailable, m);
    match channel.request(codex, kind, &payload, ASK) {
        Ok(Reply::Ok(b)) => serde_json::from_slice(&b).map_err(|e| {
            api(
                ErrorCode::Internal,
                format!("{kind}: unreadable answer from the site's Codex: {e}"),
            )
        }),
        Ok(Reply::Error(e)) => Err(super::backups::remote::error_of(kind, &e)),
        Ok(Reply::Refused) => Err(unavailable(format!(
            "the site's Codex refused this box ({kind}): is it still a machine of the site?"
        ))),
        Ok(Reply::Unknown) => Err(unavailable(format!(
            "the site's Codex doesn't take uploads of a show box's collection ({kind}): update the site's server"
        ))),
        Err(e) => Err(unavailable(format!(
            "the site's Codex can't be reached from this box now: {e}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(own: CollectionId, codex: &str) -> Saved {
        Saved {
            upload: DormantUpload {
                state: DormantUploadState::Done,
                source: own,
                collection: Some(CollectionId::new()),
                media: TransferCount { done: 2, total: 2 },
                bytes: TransferCount { done: 9, total: 9 },
                ..DormantUpload::default()
            },
            assets_done: BTreeSet::from([MediaAssetId::new()]),
            name: None,
            codex: codex.into(),
        }
    }

    #[test]
    fn a_job_is_only_for_the_site_it_was_made_for() {
        let own = CollectionId::new();
        let s = job(own, "codex-a");
        assert!(s.belongs(own, "codex-a"));
        assert!(!s.belongs(own, "codex-b"), "another site's Codex");
        assert!(!s.belongs(CollectionId::new(), "codex-a"), "another source");
        // A file of an older server names no Codex.
        assert!(job(own, "").belongs(own, "codex-b"));
    }

    #[test]
    fn media_sent_to_another_collection_are_sent_again() {
        let own = CollectionId::new();
        let mut s = job(own, "codex-a");
        let same = s.upload.collection.unwrap();
        s.target(same);
        assert_eq!(s.assets_done.len(), 1, "the same collection resumes");
        s.target(CollectionId::new());
        assert!(s.assets_done.is_empty());
        assert_eq!(s.upload.media, TransferCount::default());
        assert_eq!(s.upload.bytes, TransferCount::default());
    }

    #[test]
    fn cancelling_a_job_no_thread_runs_keeps_it_from_resuming() {
        let mut s = job(CollectionId::new(), "codex-a");
        assert!(!s.cancel_idle(), "a finished job stays as it is");
        s.upload.state = DormantUploadState::Copying;
        assert!(s.cancel_idle());
        assert_eq!(s.upload.state, DormantUploadState::Cancelled);
        assert!(s.upload.finished_at.is_some());
    }
}
