//! # `vesper-codex` — Vesper Codex, the site's content store (see `docs/CODEX-PLAN.md`)
//!
//! A crate of the Vesper server (`vesper-core` ← `vesper-site` ← `vesper-codex` ←
//! `vesper-server`), which re-exports it as `codex` (and [`data`], [`store`] under their old
//! paths). The Codex reaches the server only through [`Server`] (the server's state: the
//! Codex's parts it holds, the documents, the site, a few hooks) and [`Show`] (the show
//! actor); the `/api/v1/codex` routes, the write-through middleware and presence over the link
//! stay in the server (`vesper_server::codex::{api, server, site_presence}`).
//!
//! ```text
//! <data>/codex/                          only on the box that holds the site's Codex
//!   codex.json                           CodexInfo {schemaVersion, id, createdAt}
//!   migration.json                       the migration of a standalone install (below)
//!   collections/<id>/collection.json     Collection
//!   collections/<id>/portable/<kind>/<id>.json   the collection's documents (same files and
//!                                        rules as before: atomic writes, never over a newer
//!                                        schema)
//!   collections/<id>/changes.jsonl       the collection's change log ([`log`])
//!   library/                             the Codex's media library: IDN's site media library,
//!                                        a folder per collection, items by lasting ID
//!                                        ([`library`]; IDN keeps its index in `library/.library/`)
//!   library-migration.json               the move of the old store into the library
//!                                        ([`media_migration`]); `library-import/` its staging
//!   media/<sha256>.<ext>                 the old managed media store (what the library doesn't
//!                                        take; emptied into the library by [`media_migration`])
//!   thumbs/<sha256>-320.jpg              media thumbnails by SHA-256 ([`worker`])
//!   uploads/<id>.json, <id>.part         resumable uploads in progress ([`uploads`])
//!   trash/<entry>.json, trash.jsonl      deleted documents and the trash's index ([`trash`])
//!   collection-trash/<id>/               deleted collections, intact ([`collection_trash`])
//!   backups/sets/ snapshots/ blobs/      settings backups ([`backups`])
//!   exports/                             scheduled exports' default folder ([`export`])
//! <data>/codex-cache/                    a show box's copy of the site's collection and its
//!                                        media cache ([`mirror`])
//! <data>/machine/codex.json              CodexConfig: this box's role (host, dormant, client), its
//!                                        active collection, the Codex it follows, and its own
//!                                        Codex's active collection while that is dormant
//! <data>/machine/codex-member/           the Codex's own identity in its site ([`member`];
//!                                        never in a settings backup)
//! <data>/machine/codex-export.json       scheduled exports' settings; `codex-export-passphrase`
//!                                        their stored passphrase (0600; never answered back,
//!                                        never in a snapshot or an export) ([`export`])
//! <data>/codex-import/                   uploaded imports and their staging ([`export`])
//! ```
//!
//! [`load`] runs the [`migration`] when needed, loads every collection and builds the show's
//! [`ShowData`] from this box's documents and the **active collection**'s content. The
//! collection kinds are [`data::COLLECTION_KINDS`]; screens, looks, show settings, masks,
//! inputs and everything in `machine/` stay with the box (`<data>/portable/`,
//! `<data>/machine/`).
//!
//! **One source of truth per collection's content.** The active collection's documents are
//! [`ShowData::content`] (what the show renders, what the content routes read, what edits
//! replace); [`CodexState`] holds every *other* collection's content
//! ([`LoadedCollection::content`] is `None` for the active one). Switching the active collection
//! moves the content between the two in one step of the show actor. Read a collection's
//! content with [`Server::collection_content`].
//!
//! Every collection write goes through the one serialized [`writer::Writer`]: collection CRUD,
//! this box's assignment, and content writes (plan, `store::apply`, notify; for the active
//! collection the show actor swaps the new content in and the show follows). A reload from
//! disk (`reloadData`) holds the writer too.
//!
//! - [`host`]: what the Codex needs from the server ([`Server`], [`Show`]) and the types they
//!   share ([`CollectionUpdate`], [`Forwarded`]).
//! - [`data`]: loading a data directory (this box's documents, a collection's content) and
//!   [`store`]: atomic document writes.
//! - [`log`]: each collection's change log (`changes.jsonl`: seq, compaction, resync).
//! - [`trash`]: deleted documents, restorable for 30 days with their IDs.
//! - [`collection_trash`]: deleted collections, kept intact until restored or deleted for good.
//!
//! - [`library`]: the Codex's media library (IDN's), its items' files, "in use" marks, serving.
//! - [`media_migration`]: the old managed media store moved into the library.
//! - [`uploads`]: resumable media uploads (SHA-256 checked, taken into the library, deduplicated
//!   by it).
//! - [`worker`]: probing and thumbnails at low priority.
//! - [`disk`]: free space, cross-filesystem moves.
//! - [`storage`]: storage usage and which library items the documents use (the marks).
//! - [`relink`]: missing media (availability now) and the relink paths.
//! - [`member`]: the Codex's own membership of its site (`machine/codex-member/`), for the
//!   box-to-Codex link.
//! - [`link`]: the box-to-Codex link over IDN's `channel/1`: the Codex's one channel (Vesper's
//!   requests and the media library's), its pushes to show boxes.
//! - [`mirror`]: a show box's copy of the site's collection and its media cache; the roles
//!   `dormant` and `client`, waking on leave.
//! - [`adopt`]: a show box uploads its own, dormant collection to the site's Codex (G15).
//! - presence and edit locks site-wide over the box-to-Codex link: the server's
//!   `codex::site_presence` (the link's [`link::PRESENCE`], [`link::LOCK`] and
//!   [`link::PRESENCE_CHANGED`] reach it through [`Server`]).
//! - [`search`]: the in-memory search index of every collection (`GET /codex/search`).
//! - [`write_through`]: a show box's content writes forwarded to the site's Codex (M10); its
//!   uploads and Codex management answered with where the Codex box is.
//!
//! - [`backups`]: settings backups (sets, snapshots, retention) and restoring them; show
//!   boxes offer theirs and take restores queued for them over the box-to-Codex link.
//! - [`export`]: the encrypted export (documents only, never media; the manual download and
//!   scheduled exports with retention) and importing one (preview, then apply), with the
//!   site's Site Manager folder on the site's server.

pub mod adopt;
pub mod backups;
pub mod blobs;
pub mod collection_trash;
pub mod data;
pub mod disk;
pub mod export;
pub mod host;
pub mod library;
pub mod link;
pub mod log;
pub mod media_migration;
pub mod member;
pub mod migration;
pub mod mirror;
pub mod prep;
pub mod relink;
pub mod search;
pub mod storage;
pub mod store;
pub mod trash;
pub mod uploads;
pub mod worker;
pub mod write_through;
pub mod writer;

pub use host::{CollectionUpdate, ContentPlanned, Forwarded, Server, Show, ShowFuture};

/// The server, as the Codex's modules see it (see [`Server`]).
pub(crate) type Shared = dyn Server;

// The helpers and the site every part of the server shares (`crates/vesper-core`,
// `crates/vesper-site`), under the paths the Codex's modules have always used.
use vesper_core::lock;
use vesper_site::{self as site, auth};

use crate::data::{CollectionContent, LoadIssue, LoadReport, ShowData};
use model::{CodexConfig, CodexInfo, Collection, CollectionId, Document, MediaRootId};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Limits of the Codex's media handling (the server's configuration).
#[derive(Clone, Debug)]
pub struct CodexOptions {
    /// Largest media upload, bytes (default 64 GiB, the IDN file channel's limit).
    pub max_upload_bytes: u64,
    /// Uploads started and not finished, at most (default 64).
    pub max_open_uploads: usize,
    /// Chunks received at the same time, at most (default 4; more answer `429`).
    pub max_concurrent_transfers: usize,
    /// Free space an upload must leave on the disk: this share of it… (default 5 %)
    pub keep_free_percent: f64,
    /// …plus these bytes (default 2 GiB).
    pub keep_free_bytes: u64,
    /// Media probing and thumbnail threads, at low CPU priority (default 1).
    pub media_workers: usize,
    /// New media are prepared automatically ([`prep`]) when the Codex box's setting is on too.
    /// Off by default here (tests, embedders); the `vesper` server turns it on.
    pub media_prep: bool,
    /// How media preparation's conversions run: in a child process (`vesper media-convert`,
    /// [`prep::child`]) when a program is set, which the `vesper` server does (itself); else in
    /// this process (the default here: tests, embedders).
    pub converter: prep::child::Converter,
    /// An upload without a new chunk for this long is dropped (default 24 h).
    pub upload_expiry: std::time::Duration,
    /// The UDP port of the box-to-Codex link (IDN's `channel/1`), on the Codex box and on show
    /// boxes: `0` (the default) picks a free one of IDN's media range, which the site's
    /// directory tells the other machines. Set a fixed port where firewalls sit between
    /// buildings (the VPN routed, without NAT) and open it for UDP.
    pub channel_port: u16,
    /// A show box's media cache cap, bytes (default 32 GiB): IDN removes unpinned copies,
    /// least recently used first, to stay under it.
    pub cache_bytes: u64,
    /// How often a show box asks the site's Codex for changes besides the Codex's pushes
    /// (default 30 s).
    pub mirror_poll: std::time::Duration,
    /// The Codex tells its show boxes about changes at once (default on; off only in tests of
    /// the poll).
    pub push_changes: bool,
    /// Settings backups: when and how many ([`backups`]).
    pub backups: backups::BackupOptions,
    /// Scheduled exports and uploaded imports ([`export`]).
    pub export: export::ExportOptions,
}

impl Default for CodexOptions {
    fn default() -> Self {
        CodexOptions {
            max_upload_bytes: 64 << 30,
            max_open_uploads: 64,
            max_concurrent_transfers: 4,
            keep_free_percent: 5.0,
            keep_free_bytes: 2 << 30,
            media_workers: 1,
            media_prep: false,
            converter: prep::child::Converter::default(),
            upload_expiry: std::time::Duration::from_secs(86_400),
            channel_port: 0,
            cache_bytes: 32 << 30,
            mirror_poll: std::time::Duration::from_secs(30),
            push_changes: true,
            backups: backups::BackupOptions::default(),
            export: export::ExportOptions::default(),
        }
    }
}

/// Where the Codex's files live in a data directory.
#[derive(Clone, Debug)]
pub struct Layout {
    data: PathBuf,
}

impl Layout {
    /// The layout of the data directory `data`.
    pub fn new(data: &Path) -> Layout {
        Layout {
            data: data.to_path_buf(),
        }
    }

    /// The data directory.
    pub fn data(&self) -> &Path {
        &self.data
    }

    /// `<data>/codex`.
    pub fn codex(&self) -> PathBuf {
        self.data.join("codex")
    }

    /// `<data>/codex/codex.json`.
    pub fn codex_json(&self) -> PathBuf {
        self.codex().join("codex.json")
    }

    /// `<data>/codex/migration.json`.
    pub fn migration_json(&self) -> PathBuf {
        self.codex().join("migration.json")
    }

    /// `<data>/codex/collections`.
    pub fn collections(&self) -> PathBuf {
        self.codex().join("collections")
    }

    /// `<data>/codex/collections/<id>`: the collection's content root.
    pub fn collection(&self, id: CollectionId) -> PathBuf {
        self.collections().join(id.to_string())
    }

    /// `<data>/codex/collections/<id>/collection.json`.
    pub fn collection_json(&self, id: CollectionId) -> PathBuf {
        self.collection(id).join("collection.json")
    }

    /// `<data>/codex/collection-trash`: deleted collections.
    pub fn collection_trash(&self) -> PathBuf {
        self.codex().join("collection-trash")
    }

    /// `<data>/codex/collection-trash/<id>`: a deleted collection's folder.
    pub fn deleted_collection(&self, id: CollectionId) -> PathBuf {
        self.collection_trash().join(id.to_string())
    }

    /// `<data>/codex/uploads`: uploads in progress.
    pub fn uploads(&self) -> PathBuf {
        self.codex().join("uploads")
    }

    /// `<data>/codex/media`: the old managed media store.
    pub fn media(&self) -> PathBuf {
        self.codex().join("media")
    }

    /// `<data>/codex/library`: the Codex's media library.
    pub fn library(&self) -> PathBuf {
        self.codex().join(library::DIR)
    }

    /// `<data>/codex-cache`: a show box's copy of the site's Codex ([`mirror`]).
    pub fn codex_cache(&self) -> PathBuf {
        self.data.join("codex-cache")
    }

    /// `<data>/codex-cache/collections/<id>`: a show box's copy of a collection.
    pub fn mirror_collection(&self, id: CollectionId) -> PathBuf {
        self.codex_cache().join("collections").join(id.to_string())
    }

    /// `<data>/codex-cache/media`: a show box's media cache (IDN's).
    pub fn cache_media(&self) -> PathBuf {
        self.codex_cache().join("media")
    }

    /// `<data>/machine/codex.json`: this box's Codex settings.
    pub fn config_json(&self) -> PathBuf {
        self.data.join("machine").join("codex.json")
    }

    /// `<data>/portable/<kind>` of a standalone install (before the Codex).
    pub fn old_kind_dir(&self, kind: model::DocumentKind) -> PathBuf {
        self.data.join("portable").join(crate::data::kind_dir(kind))
    }

    /// `<data>/media` of a standalone install (before the Codex).
    pub fn old_media(&self) -> PathBuf {
        self.data.join("media")
    }
}

/// One collection of the Codex, loaded.
#[derive(Clone)]
pub struct LoadedCollection {
    /// Its `collection.json` (for one written by a newer server: its ID and name only).
    pub collection: Collection,
    /// Its `collection.json` comes from a newer schema: never rewritten.
    pub too_new: bool,
    /// Its documents; `None` for this box's active collection, whose documents are
    /// [`ShowData::content`] (see the module docs).
    pub content: Option<Arc<CollectionContent>>,
}

/// The Codex as loaded: its identity, this box's settings and every collection.
#[derive(Clone)]
pub struct CodexState {
    /// Where its files live.
    pub layout: Layout,
    /// `codex.json`.
    pub info: CodexInfo,
    /// `machine/codex.json` as this box uses it.
    pub config: CodexConfig,
    /// The collection this box shows.
    pub active: CollectionId,
    /// Every collection, sorted by sort key, then name, then ID.
    pub collections: Vec<LoadedCollection>,
    /// The collection trash, newest deletion first (no content loaded).
    pub deleted: Vec<collection_trash::DeletedState>,
}

impl CodexState {
    /// A collection by ID.
    pub fn collection(&self, id: CollectionId) -> Option<&LoadedCollection> {
        self.collections.iter().find(|c| c.collection.id == id)
    }

    fn collection_mut(&mut self, id: CollectionId) -> Option<&mut LoadedCollection> {
        self.collections.iter_mut().find(|c| c.collection.id == id)
    }

    /// A copy with a collection added or replaced (kept sorted).
    pub fn with_collection(&self, c: LoadedCollection) -> CodexState {
        let mut s = self.clone();
        s.collections.retain(|x| x.collection.id != c.collection.id);
        s.collections.push(c);
        sort_collections(&mut s.collections);
        s
    }

    /// A copy without a collection.
    pub fn without_collection(&self, id: CollectionId) -> CodexState {
        let mut s = self.clone();
        s.collections.retain(|x| x.collection.id != id);
        s
    }

    /// A deleted collection by ID.
    pub fn deleted(&self, id: CollectionId) -> Option<&collection_trash::DeletedState> {
        self.deleted.iter().find(|d| d.collection.id == id)
    }

    /// A collection is in the collection trash.
    pub fn is_deleted(&self, id: CollectionId) -> bool {
        self.deleted(id).is_some()
    }

    /// A copy with an inactive collection's content replaced.
    pub fn with_content(&self, id: CollectionId, content: Arc<CollectionContent>) -> CodexState {
        let mut s = self.clone();
        if let Some(c) = s.collection_mut(id) {
            c.content = Some(content);
        }
        s
    }

    /// A copy where `to` is active: `old_content` (the content the show had) goes back to the
    /// collection that was active, `to`'s content is taken out (returned).
    pub fn switched(
        &self,
        to: CollectionId,
        old_content: Arc<CollectionContent>,
    ) -> Option<(CodexState, Arc<CollectionContent>)> {
        let mut s = self.clone();
        let content = s.collection_mut(to)?.content.take()?;
        let was = s.active;
        if let Some(c) = s.collection_mut(was) {
            c.content = Some(old_content);
        }
        s.active = to;
        s.config.collection = Some(to);
        Some((s, content))
    }
}

/// Reads a JSON document, migrated. `Err((message, too_new))`; a missing file is `Ok(None)`.
pub(crate) fn read_json(
    path: &Path,
    kind: model::DocumentKind,
) -> Result<Option<Value>, (String, bool)> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err((format!("read failed: {e}"), false)),
    };
    let mut v: Value =
        serde_json::from_str(&text).map_err(|e| (format!("not valid JSON: {e}"), false))?;
    model::migrate(kind, &mut v).map_err(|e| {
        let too_new = matches!(e, model::MigrationError::TooNew { .. });
        (e.to_string(), too_new)
    })?;
    Ok(Some(v))
}

/// Reads and decodes a document; see [`read_json`].
pub(crate) fn read_doc<T: Document>(path: &Path) -> Result<Option<T>, (String, bool)> {
    match read_json(path, T::KIND)? {
        None => Ok(None),
        Some(v) => serde_json::from_value(v)
            .map(Some)
            .map_err(|e| (format!("invalid document: {e}"), false)),
    }
}

/// Writes a document atomically, never over one from a newer schema.
pub(crate) fn write_doc<T: Document>(path: &Path, doc: &T) -> anyhow::Result<()> {
    if let Err(e) = crate::store::check_writable(path) {
        anyhow::bail!("{e}");
    }
    crate::store::write_atomic(path, &serde_json::to_vec_pretty(doc)?)
}

fn issue(path: &Path, error: String, too_new: bool) -> LoadIssue {
    tracing::warn!(file = %path.display(), too_new, "Codex: {error}");
    LoadIssue {
        file: path.display().to_string(),
        error,
        too_new,
    }
}

/// Loads one collection directory; `None` when it holds no `collection.json` at all.
pub(crate) fn load_collection(
    layout: &Layout,
    dir: &Path,
    lib: &Arc<library::MediaLibrary>,
    media_roots: &Arc<HashMap<MediaRootId, PathBuf>>,
    issues: &mut Vec<LoadIssue>,
) -> Option<LoadedCollection> {
    let id: CollectionId = dir.file_name()?.to_str()?.parse().ok()?;
    let path = dir.join("collection.json");
    let (collection, too_new) = match read_doc::<Collection>(&path) {
        Ok(Some(c)) if c.id == id => (c, false),
        Ok(Some(c)) => {
            issues.push(issue(
                &path,
                format!("collection ID {} does not match its folder", c.id),
                false,
            ));
            return None;
        }
        Ok(None) => return None,
        Err((error, too_new)) => {
            issues.push(issue(&path, error, too_new));
            if !too_new {
                return None;
            }
            // A newer server's collection: still shown (its documents are each checked), with
            // the name it has; never rewritten.
            let name = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            (
                Collection {
                    id,
                    name,
                    ..Collection::default()
                },
                true,
            )
        }
    };
    let content = CollectionContent::load(dir, layout.media(), lib.clone(), media_roots.clone());
    Some(LoadedCollection {
        collection,
        too_new,
        content: Some(Arc::new(content)),
    })
}

pub(crate) fn sort_collections(list: &mut [LoadedCollection]) {
    list.sort_by(|a, b| {
        let (a, b) = (&a.collection, &b.collection);
        a.sort_key
            .cmp(&b.sort_key)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.0.cmp(&b.id.0))
    });
}

/// Loads a data directory: runs the [`migration`] when needed (`server_name` names a new
/// collection), opens the media library on the box that hosts the Codex (`lib`; once) and
/// moves the old media store into it ([`media_migration`]), loads every collection and builds
/// the show's data from this box's documents and the active collection. Fails only when `data`
/// is not a data directory or the Codex can't be set up (I/O); bad documents are reported in
/// [`ShowData::report`].
pub fn load(
    data: &Path,
    server_name: &str,
    lib: &Arc<library::MediaLibrary>,
) -> anyhow::Result<(ShowData, CodexState)> {
    anyhow::ensure!(
        data.join("portable").is_dir(),
        "{} is not a data directory (no portable/ folder)",
        data.display()
    );
    let layout = Layout::new(data);
    // A show box: the copy of the site's collection (its own Codex is dormant, untouched).
    if let Ok(Some(config)) = read_doc::<CodexConfig>(&layout.config_json())
        && config.role == model::CodexRole::Client
    {
        return load_client(layout, config, lib);
    }
    if layout.codex().is_dir() {
        collection_trash::finish_moves(&layout);
    }
    let outcome = migration::run(&layout, server_name)?;
    let mut issues = outcome.issues;
    if outcome.config.role == model::CodexRole::Host {
        lib.ensure_open(&layout.library());
        // (Rewrites are noted for the change logs unless the Codex was only made now: no
        // show box can be following it yet.)
        let followed = !outcome.migrated;
        if let Err(e) = media_migration::run(&layout, lib, followed, &mut issues) {
            tracing::error!("moving the old media store into the media library: {e:#}");
        }
    }

    let info = match read_doc::<CodexInfo>(&layout.codex_json()) {
        Ok(Some(i)) => i,
        Ok(None) => anyhow::bail!("{} is missing", layout.codex_json().display()),
        Err((error, too_new)) => {
            issues.push(issue(&layout.codex_json(), error, too_new));
            CodexInfo::default()
        }
    };

    // This box's documents come first: the collections resolve linked media through its media
    // root bindings.
    let own = ShowData::load_from(
        data,
        &layout.collection(outcome.active),
        layout.media(),
        lib.clone(),
    )?;
    let media_roots = own.content.media_roots.clone();
    let mut collections = Vec::new();
    if let Ok(rd) = std::fs::read_dir(layout.collections()) {
        let mut dirs: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            if dir.file_name().and_then(|n| n.to_str()) == Some(&outcome.active.to_string()) {
                continue;
            }
            if let Some(c) = load_collection(&layout, &dir, lib, &media_roots, &mut issues) {
                collections.push(c);
            }
        }
    }
    // The active collection: its content is the one the show data loaded.
    let active_path = layout.collection_json(outcome.active);
    let (collection, too_new) = match read_doc::<Collection>(&active_path) {
        Ok(Some(c)) => (c, false),
        Ok(None) => (
            Collection {
                id: outcome.active,
                ..Collection::default()
            },
            false,
        ),
        Err((error, too_new)) => {
            issues.push(issue(&active_path, error, too_new));
            (
                Collection {
                    id: outcome.active,
                    ..Collection::default()
                },
                too_new,
            )
        }
    };
    collections.push(LoadedCollection {
        collection,
        too_new,
        content: None,
    });
    sort_collections(&mut collections);

    let mut show = own;
    if !issues.is_empty() {
        let mut report: LoadReport = (*show.report).clone();
        report.issues.extend(issues);
        show.report = Arc::new(report);
    }
    tracing::info!(
        codex = %info.id,
        collections = collections.len(),
        active = %outcome.active,
        "Codex loaded"
    );
    let deleted = collection_trash::load(&layout);
    let state = CodexState {
        layout,
        info,
        config: outcome.config,
        active: outcome.active,
        collections,
        deleted,
    };
    Ok((show, state))
}

/// Loads a show box (Codex role `client`): this box's documents and its copy of the site's
/// collection ([`mirror`]); the box's own Codex stays as it is (dormant). Library media come
/// from the media cache (none until the mirror has found the Codex).
fn load_client(
    layout: Layout,
    config: CodexConfig,
    lib: &Arc<library::MediaLibrary>,
) -> anyhow::Result<(ShowData, CodexState)> {
    if !lib.is_remote() {
        lib.set_remote(None);
    }
    let active = config.collection.unwrap_or_default();
    let root = layout.mirror_collection(active);
    let state = mirror::read_state(&root).unwrap_or_default();
    let mut issues = Vec::new();
    let collection = match read_doc::<Collection>(&root.join("collection.json")) {
        Ok(Some(c)) => c,
        Ok(None) => Collection {
            id: active,
            ..Collection::default()
        },
        Err((error, too_new)) => {
            issues.push(issue(&root.join("collection.json"), error, too_new));
            Collection {
                id: active,
                ..Collection::default()
            }
        }
    };
    let mut show = ShowData::load_from(layout.data(), &root, layout.media(), lib.clone())?;
    if !issues.is_empty() {
        let mut report: LoadReport = (*show.report).clone();
        report.issues.extend(issues);
        show.report = Arc::new(report);
    }
    tracing::info!(codex = %state.codex_id, collection = %active, seq = state.seq, "Codex: this box shows its copy of the site's collection");
    let state = CodexState {
        info: CodexInfo {
            schema_version: model::SCHEMA_VERSION,
            id: state.codex_id,
            created_at: String::new(),
        },
        config,
        active,
        collections: vec![LoadedCollection {
            collection,
            too_new: false,
            content: None,
        }],
        deleted: Vec::new(),
        layout,
    };
    Ok((show, state))
}
