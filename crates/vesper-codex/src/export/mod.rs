//! The Codex's encrypted export and import (M13; see `docs/CODEX-PLAN.md`).
//!
//! **The file** is an IDN backup ([`idn_open_protocol::backup`], kind [`KIND`]: Argon2id and
//! ChaCha20-Poly1305 with a passphrase; no crypto of Vesper's own), made of a staging folder
//! assembled under `<data>/` and removed after:
//!
//! ```text
//! export.json                              ExportManifest (what it holds, who made it)
//! codex/codex.json                         the Codex's identity
//! codex/collections/<id>/...               every collection: collection.json, portable/,
//!                                          changes.jsonl (as on disk)
//! codex/backups/sets/ snapshots/ blobs/    the settings backups
//! site/site-manager.idnbackup              on the site's server only, in scheduled exports
//!                                          and in manual ones asked with `includeSite`: the
//!                                          Site Manager's folder as IDN's own site backup
//!                                          makes it (kind `site-manager`, IDN's skip list),
//!                                          encrypted with the same passphrase
//! ```
//!
//! **Documents only, never media**: media references stay as they are (library items by ID
//! and SHA-256); the media folder needs its own backup ([`protocol::MEDIA_NOTE`]). Not in it
//! either: the item trash and the collection trash, uploads, thumbnails, the media library,
//! `machine/` (device identities, the stored passphrase).
//!
//! - **Manual export** (`POST /codex/export`): its own passphrase, the file as a download; the
//!   site's identity only when asked (`includeSite`, off by default; owner, 2026-11-23).
//! - **Scheduled exports** ([`schedule`]): nightly into a folder with the passphrase the admin
//!   stored (`machine/codex-export-passphrase`, mode 0600, never answered back), with
//!   retention; on the site's server always with the site's identity.
//! - **Import** ([`import`]): upload, preview with the passphrase, apply chosen parts through
//!   the Codex writer; the site part is staged with IDN's `backup::stage` and put in place by
//!   the Site Manager at the next start (`host::start` runs `backup::apply_staged` before it
//!   opens its folder).
//!
//! Every route is the site's admin's (the server's `codex::api`); on a show box they answer `409
//! codexElsewhere`.

pub mod import;
pub mod schedule;

use crate::Shared;
use idn_open_protocol::backup;
use model::{CollectionId, Document};
use protocol::{ApiError, ErrorCode, ExportManifest, ExportedCollection};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// The IDN backup kind of a Codex export.
pub const KIND: &str = "vesper-codex";
/// `export.json`'s `format`.
pub const FORMAT: &str = "vesper-codex";
/// The export format's version.
pub const FORMAT_VERSION: u32 = 1;
/// The manifest's file in an export.
pub const MANIFEST: &str = "export.json";
/// The Site Manager's folder in an export (on the site's server).
pub const SITE_FILE: &str = "site/site-manager.idnbackup";
/// The IDN backup kind of the Site Manager's folder.
pub const SITE_KIND: &str = "site-manager";
/// Export files' ending.
pub const EXTENSION: &str = "vcodex";

/// When scheduled exports are looked at and how long uploaded imports wait
/// ([`crate::CodexOptions::export`]).
#[derive(Clone, Debug)]
pub struct ExportOptions {
    /// How often the schedule is looked at (default 1 min).
    pub tick: Duration,
    /// Tests: a scheduled export this often instead of nightly at the set time.
    pub every: Option<Duration>,
    /// An uploaded import not applied is dropped after this (default 1 h).
    pub import_expiry: Duration,
    /// A scheduled export that failed is tried again this long after (default 30 min), not
    /// at every tick.
    pub retry_after: Duration,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            tick: Duration::from_secs(60),
            every: None,
            import_expiry: Duration::from_secs(3600),
            retry_after: Duration::from_secs(1800),
        }
    }
}

/// Where the export's files live in a data directory.
#[derive(Clone, Debug)]
pub struct Paths {
    data: PathBuf,
}

impl Paths {
    /// The paths of the data directory `data`.
    pub fn new(data: &Path) -> Paths {
        Paths {
            data: data.to_path_buf(),
        }
    }

    /// `<data>/machine/codex-export.json`: the scheduled exports' settings.
    pub fn settings(&self) -> PathBuf {
        self.data.join("machine").join("codex-export.json")
    }

    /// `<data>/machine/codex-export-passphrase`: the stored passphrase (mode 0600).
    pub fn passphrase(&self) -> PathBuf {
        self.data.join("machine").join("codex-export-passphrase")
    }

    /// `<data>/codex/exports`: the scheduled exports' default folder.
    pub fn default_folder(&self) -> PathBuf {
        self.data.join("codex").join("exports")
    }

    /// `<data>/codex-export-staging`: an export being assembled.
    pub fn staging(&self) -> PathBuf {
        self.data.join("codex-export-staging")
    }

    /// `<data>/codex-import`: uploaded imports and their staging.
    pub fn imports(&self) -> PathBuf {
        self.data.join("codex-import")
    }
}

#[derive(Default)]
struct State {
    /// The last scheduled export's failure (cleared by the next success).
    last_error: Option<String>,
    /// When it failed: the next try waits [`ExportOptions::retry_after`] (cleared by a success
    /// and by new settings or a new passphrase).
    failed_at: Option<std::time::Instant>,
    /// Uploaded imports by ID.
    imports: BTreeMap<String, import::Pending>,
}

/// The Codex's exports and imports (one per server, [`crate::Server::exports`]).
pub struct Exports {
    opts: ExportOptions,
    /// One export or import at a time.
    running: tokio::sync::Mutex<()>,
    state: Mutex<State>,
}

pub(crate) fn api(code: ErrorCode, msg: impl Into<String>) -> ApiError {
    ApiError::new(code, msg)
}

pub(crate) fn join_err(e: tokio::task::JoinError) -> ApiError {
    api(ErrorCode::Internal, e.to_string())
}

/// RFC 3339 UTC now.
pub(crate) fn now() -> String {
    crate::auth::rfc3339(SystemTime::now(), true)
}

/// The passphrase is long enough (IDN's [`backup::MIN_PASSPHRASE`]), or `400` with `field`.
pub fn check_passphrase(passphrase: &str, field: &str) -> Result<(), ApiError> {
    if passphrase.chars().count() < backup::MIN_PASSPHRASE {
        return Err(ApiError::invalid(
            field,
            format!(
                "the passphrase needs at least {} characters",
                backup::MIN_PASSPHRASE
            ),
        ));
    }
    Ok(())
}

/// Exports and imports are the Codex box's.
pub(crate) fn host(sh: &Shared) -> Result<(), ApiError> {
    if sh.codex_is_host() {
        Ok(())
    } else {
        Err(api(
            ErrorCode::Conflict,
            "exports and imports are made on the box that holds the site's Codex",
        ))
    }
}

impl Exports {
    /// The exports with these options.
    pub fn new(opts: ExportOptions) -> Exports {
        Exports {
            opts,
            running: tokio::sync::Mutex::new(()),
            state: Mutex::new(State::default()),
        }
    }

    /// The options.
    pub fn options(&self) -> &ExportOptions {
        &self.opts
    }

    /// A manual export (`POST /codex/export`): the encrypted file, made with `passphrase`; the
    /// site's identity in it only with `include_site` (`409` at a box that doesn't run its
    /// site's Site Manager).
    pub async fn export_now(
        &self,
        sh: &Arc<Shared>,
        passphrase: String,
        include_site: bool,
    ) -> Result<(Vec<u8>, ExportManifest), ApiError> {
        host(sh)?;
        check_passphrase(&passphrase, "passphrase")?;
        if include_site && sh.site().handle().is_none() {
            return Err(no_site());
        }
        let _r = self.running.lock().await;
        let site = if include_site { Site::Asked } else { Site::No };
        make(sh, passphrase, "manual", site).await
    }
}

/// `includeSite` asked at a box that doesn't run its site's Site Manager.
fn no_site() -> ApiError {
    api(
        ErrorCode::Conflict,
        "the site's identity is exported on the site's server: this box doesn't run its site's Site Manager now (export without includeSite, or on the site's server)",
    )
}

/// Copies `from`'s files (every folder) into `to`, but temporary files (`.…`); adds their
/// sizes to `total` and fails once it passes IDN's [`backup::MAX_SIZE`].
fn copy_tree(from: &Path, to: &Path, total: &mut u64) -> anyhow::Result<()> {
    let Ok(rd) = std::fs::read_dir(from) else {
        return Ok(());
    };
    for e in rd.flatten() {
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let (src, dst) = (e.path(), to.join(&name));
        let t = e.file_type()?;
        if t.is_dir() {
            copy_tree(&src, &dst, total)?;
        } else if t.is_file() {
            *total += e.metadata()?.len();
            anyhow::ensure!(*total <= backup::MAX_SIZE, TooLarge);
            std::fs::create_dir_all(to)?;
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

/// The Codex's documents are larger than an export may be.
#[derive(Debug)]
struct TooLarge;

impl std::fmt::Display for TooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the Codex's documents and settings backups are larger than an export may be ({} MB)",
            backup::MAX_SIZE >> 20
        )
    }
}

impl std::error::Error for TooLarge {}

/// The newest change's time in a change log's text (`None`: no entries).
pub(crate) fn newest_change(text: &str) -> Option<String> {
    text.lines().rev().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        v.get("seq")?;
        v.get("at")?.as_str().map(str::to_string)
    })
}

/// The number of documents (`portable/<kind>/<id>.json`) among a collection folder's
/// relative paths.
pub(crate) fn is_document(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    parts.len() == 3 && parts[0] == "portable" && parts[2].ends_with(".json")
}

/// A collection folder's documents: relative path → SHA-256.
pub(crate) fn documents_on_disk(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(kinds) = std::fs::read_dir(dir.join("portable")) else {
        return out;
    };
    for k in kinds.flatten() {
        let kind = k.file_name().to_string_lossy().to_string();
        let Ok(files) = std::fs::read_dir(k.path()) else {
            continue;
        };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || !name.ends_with(".json") {
                continue;
            }
            if let Ok(bytes) = std::fs::read(f.path()) {
                out.insert(
                    format!("portable/{kind}/{name}"),
                    crate::backups::store::sha256(&bytes),
                );
            }
        }
    }
    out
}

/// A collection as it is here (for the manifest and the preview).
pub(crate) fn collection_here(sh: &Shared, id: CollectionId) -> Option<ExportedCollection> {
    let codex = sh.codex();
    let c = codex.collection(id)?;
    let dir = codex.layout.collection(id);
    let log = std::fs::read_to_string(crate::log::path(&dir)).unwrap_or_default();
    Some(ExportedCollection {
        id,
        name: c.collection.name.clone(),
        documents: documents_on_disk(&dir).len() as u32,
        seq: sh.codex_writer().head(sh, id),
        last_change_at: newest_change(&log)
            .or_else(|| Some(c.collection.created_at.clone()).filter(|s| !s.is_empty())),
    })
}

/// Whether an export holds the site's identity (the Site Manager's folder).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Site {
    /// A scheduled export: on the site's server (it runs the Site Manager), always.
    Here,
    /// A manual export asked with `includeSite`: `409` when this box doesn't run it.
    Asked,
    /// A manual export without it.
    No,
}

/// Makes an export (manual or scheduled): assembles the staging folder while the Codex
/// writer and the settings backups are held (one consistent copy), adds the site's folder on
/// the site's server when `site` asks for it (scheduled: always; manual: `includeSite`; `409`
/// when a manual export asks for it and this box doesn't run the site's Site Manager), and
/// seals it all with IDN's `backup::make`.
pub(crate) async fn make(
    sh: &Arc<Shared>,
    passphrase: String,
    reason: &str,
    site: Site,
) -> Result<(Vec<u8>, ExportManifest), ApiError> {
    let paths = Paths::new(&sh.data().root);
    let staging_root = paths.staging();
    let stage = staging_root.join(uuid::Uuid::new_v4().to_string());
    let identity = sh.auth().identity();
    let mut manifest = ExportManifest {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION,
        created_at: now(),
        reason: reason.into(),
        codex_id: sh.codex().info.id,
        server_id: identity.id.to_string(),
        server_name: identity.name.clone(),
        version: sh.version().into(),
        collections: Vec::new(),
        backup_sets: 0,
        snapshots: 0,
        site: false,
        media_included: false,
    };
    let copied = {
        let _w = sh.codex_writer().exclusive().await;
        let _b = sh.backups().hold().await;
        let codex = sh.codex();
        manifest.collections = codex
            .collections
            .iter()
            .filter(|c| !c.too_new)
            .filter_map(|c| collection_here(sh, c.collection.id))
            .collect();
        let ids: Vec<CollectionId> = manifest.collections.iter().map(|c| c.id).collect();
        let (layout, stage2, root) = (codex.layout.clone(), stage.clone(), staging_root.clone());
        tokio::task::spawn_blocking(move || -> anyhow::Result<(u32, u32)> {
            // (Leftovers of an export cut short go first: one export runs at a time.)
            let _ = std::fs::remove_dir_all(&root);
            let codex_dir = stage2.join("codex");
            std::fs::create_dir_all(&codex_dir)?;
            let mut total = 0u64;
            std::fs::copy(layout.codex_json(), codex_dir.join("codex.json"))?;
            for id in ids {
                copy_tree(
                    &layout.collection(id),
                    &codex_dir.join("collections").join(id.to_string()),
                    &mut total,
                )?;
            }
            let store = crate::backups::store::Store::new(&layout.codex());
            let mut sets = 0;
            let mut snaps = 0;
            let to = codex_dir.join("backups");
            for set in store.sets() {
                sets += 1;
                snaps += store.snapshots(set.id).len() as u32;
            }
            copy_tree(&store.sets_dir(), &to.join("sets"), &mut total)?;
            copy_tree(
                &layout.codex().join("backups").join("snapshots"),
                &to.join("snapshots"),
                &mut total,
            )?;
            copy_tree(&store.blobs_dir(), &to.join("blobs"), &mut total)?;
            Ok((sets, snaps))
        })
        .await
        .map_err(join_err)?
    };
    let result = async {
        let (sets, snaps) = copied.map_err(|e| {
            if e.is::<TooLarge>() {
                api(ErrorCode::Conflict, e.to_string())
            } else {
                api(ErrorCode::Internal, format!("assembling the export: {e:#}"))
            }
        })?;
        manifest.backup_sets = sets;
        manifest.snapshots = snaps;
        // On the site's server, when asked: the Site Manager's folder, as IDN's own site
        // backup makes it.
        let site_dir = (site != Site::No)
            .then(|| {
                sh.site()
                    .handle()
                    .map(|_| sh.site().idn_dir().to_path_buf())
            })
            .flatten();
        if site == Site::Asked && site_dir.is_none() {
            return Err(no_site());
        }
        manifest.site = site_dir.is_some();
        let (stage2, pass, m) = (stage.clone(), passphrase.clone(), manifest.clone());
        tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ApiError> {
            if let Some(dir) = site_dir {
                let site = backup::make(&dir, SITE_KIND, idn_site_manager::BACKUP_SKIP, &pass)
                    .map_err(|e| {
                        api(
                            ErrorCode::Internal,
                            format!("backing up the site's Site Manager folder: {e}"),
                        )
                    })?;
                let p = stage2.join(SITE_FILE);
                std::fs::create_dir_all(p.parent().expect("a folder"))
                    .and_then(|()| std::fs::write(&p, site))
                    .map_err(|e| api(ErrorCode::Internal, format!("assembling the export: {e}")))?;
            }
            let json = serde_json::to_vec_pretty(&m)
                .map_err(|e| api(ErrorCode::Internal, e.to_string()))?;
            std::fs::write(stage2.join(MANIFEST), json)
                .map_err(|e| api(ErrorCode::Internal, format!("assembling the export: {e}")))?;
            backup::make(&stage2, KIND, &[], &pass).map_err(|e| match e.kind() {
                std::io::ErrorKind::InvalidInput => ApiError::invalid("passphrase", e.to_string()),
                _ => api(ErrorCode::Conflict, format!("sealing the export: {e}")),
            })
        })
        .await
        .map_err(join_err)?
    }
    .await;
    let s = stage.clone();
    let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(s)).await;
    let bytes = result?;
    tracing::info!(
        reason,
        bytes = bytes.len(),
        collections = manifest.collections.len(),
        site = manifest.site,
        "Codex export made"
    );
    Ok((bytes, manifest))
}

/// Files of an export: (path relative to a folder, bytes).
pub type Files = Vec<(String, Vec<u8>)>;

/// An export opened with its passphrase, sorted into its parts.
pub struct Opened {
    /// IDN's manifest of the file.
    pub sealed: backup::Manifest,
    /// `export.json`.
    pub manifest: ExportManifest,
    /// Each collection's files (path relative to its folder, bytes).
    pub collections: BTreeMap<CollectionId, Files>,
    /// The settings backups' files (relative to `codex/backups/`).
    pub backups: Files,
    /// The Site Manager's folder (an IDN site backup), when it holds one.
    pub site: Option<Vec<u8>>,
}

/// Opens an export (`400` with `field: passphrase` for a wrong passphrase or a damaged file;
/// `400` for a file that isn't a Codex export; `409` for one from a newer format).
pub fn open(data: &[u8], passphrase: &str) -> Result<Opened, ApiError> {
    check_passphrase(passphrase, "passphrase")?;
    let (sealed, files) = backup::open(data, passphrase)
        .map_err(|e| ApiError::invalid("passphrase", format!("the file can't be opened: {e}")))?;
    if sealed.kind != KIND {
        return Err(api(
            ErrorCode::BadRequest,
            format!(
                "this file isn't a Vesper Codex export (an IDN backup of kind “{}”)",
                sealed.kind
            ),
        ));
    }
    let mut manifest = None;
    let mut collections: BTreeMap<CollectionId, Files> = BTreeMap::new();
    let mut backups = Vec::new();
    let mut site = None;
    for (rel, bytes) in files {
        if rel == MANIFEST {
            manifest = Some(
                serde_json::from_slice::<ExportManifest>(&bytes).map_err(|e| {
                    api(
                        ErrorCode::BadRequest,
                        format!("the export's manifest can't be read: {e}"),
                    )
                })?,
            );
        } else if rel == SITE_FILE {
            site = Some(bytes);
        } else if let Some(rest) = rel.strip_prefix("codex/collections/") {
            let Some((id, inner)) = rest.split_once('/') else {
                continue;
            };
            let Ok(id) = id.parse::<CollectionId>() else {
                continue;
            };
            collections
                .entry(id)
                .or_default()
                .push((inner.to_string(), bytes));
        } else if let Some(rest) = rel.strip_prefix("codex/backups/") {
            backups.push((rest.to_string(), bytes));
        }
    }
    let manifest = manifest.ok_or_else(|| {
        api(
            ErrorCode::BadRequest,
            "this file isn't a Vesper Codex export (no export.json)",
        )
    })?;
    if manifest.format != FORMAT {
        return Err(api(
            ErrorCode::BadRequest,
            format!(
                "this file isn't a Vesper Codex export (format “{}”)",
                manifest.format
            ),
        ));
    }
    if manifest.format_version > FORMAT_VERSION {
        return Err(api(
            ErrorCode::Conflict,
            format!(
                "this export was made by a newer Vesper (format {}; this server reads {FORMAT_VERSION})",
                manifest.format_version
            ),
        ));
    }
    Ok(Opened {
        sealed,
        manifest,
        collections,
        backups,
        site,
    })
}

/// A collection's `collection.json` in an export: `Ok(Some)`, `Ok(None)` when missing or
/// unreadable, `Err(())` when it comes from a newer schema.
pub(crate) fn parse_collection(
    files: &[(String, Vec<u8>)],
) -> Result<Option<model::Collection>, ()> {
    let Some((_, bytes)) = files.iter().find(|(r, _)| r == "collection.json") else {
        return Ok(None);
    };
    let Ok(mut v) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Ok(None);
    };
    match model::migrate(model::Collection::KIND, &mut v) {
        Ok(()) => Ok(serde_json::from_value(v).ok()),
        Err(model::MigrationError::TooNew { .. }) => Err(()),
        Err(_) => Ok(None),
    }
}
