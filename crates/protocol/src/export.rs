//! The Codex's encrypted export and import (`/api/v1/codex/export…`, admins only): a manual
//! encrypted download, scheduled exports into a folder with retention, and importing an export
//! (preview first, then apply with explicit choices). See the protocol README's Codex section.
//! Servers with it announce the `codexExport` capability.
//!
//! **Documents only, never media:** an export holds the Codex's collections (their documents
//! and change logs) and its settings backups, plus on the site's server its Site Manager's
//! folder (the site's keys and machines, restored only when asked, at the next start). Media
//! files are not in it: media references stay as they are (library items by ID and SHA-256),
//! so **the media folder needs its own backup** (e.g. a copy of the library folder on a NAS).
//!
//! - **Manual export** ([`CreateExport`]): its own passphrase in the request (never the
//!   stored one), the file as a download.
//! - **Scheduled exports** ([`CodexExportStatus`], [`UpdateExportSettings`],
//!   [`SetExportPassphrase`], [`ScheduledExport`]): nightly at a set time into a folder, with
//!   the passphrase the admin stored (never answered back; without one nothing runs and the
//!   status says why), keeping some nightly and weekly ones.
//! - **Import** ([`ImportUpload`], [`OpenImport`], [`ImportPreview`], [`ImportRequest`],
//!   [`ImportResult`]): upload the file, preview it with its passphrase, apply chosen parts.

use model::{BackupSetId, CodexId, CollectionId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What every export and import answer says about media.
pub const MEDIA_NOTE: &str = "Media files are not in a Codex export (documents only): the media folder needs its own backup, e.g. a copy of the Codex's library folder on a NAS. After an import, media are found again by their SHA-256 once the media folder is restored or added as a media root (relink search).";

/// The scheduled exports' settings (`GET /api/v1/codex/export/settings`'s `settings`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CodexExportSettings {
    /// Scheduled exports run (they also need a stored passphrase).
    pub enabled: bool,
    /// The folder they go to (an absolute path on the Codex box); `None`: the default
    /// (`<data>/codex/exports/`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub folder: Option<String>,
    /// When, in the Codex box's local time (`HH:MM`, 24 h).
    pub time: String,
    /// Nightly exports kept (the newest of each of this many days that have exports).
    pub keep_nightly: u32,
    /// Weekly exports kept besides (the newest of each of this many weeks that have exports).
    pub keep_weekly: u32,
}

impl Default for CodexExportSettings {
    fn default() -> Self {
        CodexExportSettings {
            enabled: true,
            folder: None,
            time: "02:00".into(),
            keep_nightly: 7,
            keep_weekly: 4,
        }
    }
}

/// One scheduled export in the folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledExport {
    /// Its file name in the folder.
    pub file: String,
    /// When it was made (RFC 3339, from its name).
    pub created_at: String,
    /// Its size.
    pub bytes: u64,
}

/// `GET /api/v1/codex/export/settings` (admins): the scheduled exports' settings and how they
/// stand. Never the stored passphrase, only whether one is set.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CodexExportStatus {
    /// The settings.
    pub settings: CodexExportSettings,
    /// The folder exports go to now (the setting, or the default).
    pub folder: String,
    /// An admin stored the scheduled exports' passphrase.
    pub passphrase_set: bool,
    /// Scheduled exports will run (on, a passphrase stored).
    pub active: bool,
    /// Why they won't run, or what went wrong with the last one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
    /// The newest scheduled export in the folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last: Option<ScheduledExport>,
    /// The next one is due (RFC 3339), while active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next_at: Option<String>,
    /// Always `false`: exports hold documents only.
    pub media_included: bool,
    /// [`MEDIA_NOTE`].
    pub note: String,
}

/// `PUT /api/v1/codex/export/settings` (admins): the fields to change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateExportSettings {
    /// On or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub enabled: Option<bool>,
    /// The folder (an absolute path; `""`: back to the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub folder: Option<String>,
    /// `HH:MM`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub time: Option<String>,
    /// 1–366.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub keep_nightly: Option<u32>,
    /// 0–520.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub keep_weekly: Option<u32>,
}

/// `PUT /api/v1/codex/export/passphrase` (admins): the scheduled exports' passphrase (at least
/// 8 characters). Stored on the Codex box only, never answered back, never in a settings
/// snapshot or an export.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SetExportPassphrase {
    /// The passphrase.
    pub passphrase: String,
}

impl std::fmt::Debug for SetExportPassphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SetExportPassphrase { .. }")
    }
}

/// `POST /api/v1/codex/export` (admins): a manual export with its own passphrase (at least 8
/// characters; never the stored one). The answer is the encrypted file.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateExport {
    /// The passphrase the file is encrypted with.
    pub passphrase: String,
    /// Include the site's identity (the Site Manager's folder; `false` by default, owner
    /// 2026-11-23). Only on the site's server, which runs its Site Manager (`409` elsewhere).
    /// Scheduled exports always include it there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub include_site: bool,
}

impl std::fmt::Debug for CreateExport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CreateExport { .. }")
    }
}

/// What an export holds (`export.json` inside it; [`ImportPreview::manifest`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExportManifest {
    /// `vesper-codex`.
    pub format: String,
    /// The export format's version (1).
    pub format_version: u32,
    /// When it was made (RFC 3339 UTC).
    pub created_at: String,
    /// `manual` or `scheduled`.
    pub reason: String,
    /// The Codex it was made from.
    pub codex_id: CodexId,
    /// The Codex box's server ID.
    pub server_id: String,
    /// The Codex box's name.
    pub server_name: String,
    /// The Vesper version that made it.
    pub version: String,
    /// Its collections.
    pub collections: Vec<ExportedCollection>,
    /// Its settings backup sets.
    pub backup_sets: u32,
    /// Their snapshots.
    pub snapshots: u32,
    /// It holds the site's Site Manager folder: a scheduled export made on the site's server,
    /// or a manual one made there with `includeSite`.
    pub site: bool,
    /// Always `false`: documents only.
    pub media_included: bool,
}

/// A collection in an export.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ExportedCollection {
    /// Its ID.
    pub id: CollectionId,
    /// Its name.
    pub name: String,
    /// Its documents.
    pub documents: u32,
    /// Its change log's position.
    pub seq: u64,
    /// Its newest change (RFC 3339 UTC; its creation when it has none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_change_at: Option<String>,
}

/// `POST /api/v1/codex/export/imports` (admins; the body is the export file) → `201`: the
/// file waits here for its preview and apply (an hour; `DELETE …/imports/{id}` drops it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ImportUpload {
    /// Its ID.
    pub id: String,
    /// Its size.
    pub bytes: u64,
    /// When it is dropped (RFC 3339 UTC).
    pub expires_at: String,
}

/// `POST /api/v1/codex/export/imports/{id}/preview` (admins): the file's passphrase.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenImport {
    /// The passphrase the file was encrypted with.
    pub passphrase: String,
}

impl std::fmt::Debug for OpenImport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OpenImport { .. }")
    }
}

/// How a collection of the file stands against this Codex.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ImportComparison {
    /// Not here: added by default.
    #[default]
    New,
    /// Here with the same documents.
    Same,
    /// Here; the file's newest change is newer.
    Newer,
    /// Here; the file's newest change is older (or as old, with other documents).
    Older,
    /// In this Codex's collection trash: restore it or delete it for good before importing.
    InTrash,
}

/// A collection of the file in the preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ImportCollection {
    /// As in the file.
    pub collection: ExportedCollection,
    /// Against this Codex.
    pub comparison: ImportComparison,
    /// The collection here with the same ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub here: Option<ExportedCollection>,
    /// This box shows it (it can't be replaced: assign another collection first).
    #[serde(default)]
    pub active: bool,
    /// Written by a newer server: it can't be imported here.
    #[serde(default)]
    pub too_new: bool,
}

/// A settings backup set of the file in the preview.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ImportBackupSet {
    /// Its ID.
    pub id: BackupSetId,
    /// Its product.
    pub product: String,
    /// Its machine.
    pub machine: String,
    /// Its name.
    pub name: String,
    /// Its snapshots in the file.
    pub snapshots: u32,
    /// This Codex has a set for the same product and machine (kept as it is; the file's is
    /// not imported).
    pub present: bool,
}

/// `POST /api/v1/codex/export/imports/{id}/preview` (admins): what the file holds and how it
/// stands against this Codex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    /// The upload's ID.
    pub id: String,
    /// The file's manifest.
    pub manifest: ExportManifest,
    /// When IDN's backup format says it was made (local time of the box that made it).
    pub sealed_at: String,
    /// Its collections.
    pub collections: Vec<ImportCollection>,
    /// Its settings backup sets.
    pub backup_sets: Vec<ImportBackupSet>,
    /// It holds the site's Site Manager folder (restored only when asked, at the next start).
    pub site: bool,
    /// Always `false`.
    pub media_included: bool,
    /// [`MEDIA_NOTE`].
    pub note: String,
}

/// `POST /api/v1/codex/export/imports/{id}/apply` (admins): what to import. By default the
/// collections that aren't here are added, collections here are kept, settings backup sets
/// this Codex lacks are added, and the site is not touched.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    /// The file's passphrase.
    pub passphrase: String,
    /// Collections here to replace with the file's (the current one goes to the collection
    /// trash first: the undo). Not the one this box shows.
    #[serde(default)]
    #[ts(as = "Option<Vec<CollectionId>>", optional)]
    pub replace: Vec<CollectionId>,
    /// New collections not to add.
    #[serde(default)]
    #[ts(as = "Option<Vec<CollectionId>>", optional)]
    pub skip: Vec<CollectionId>,
    /// Add the settings backup sets this Codex lacks (default `true`).
    #[serde(default = "yes")]
    #[ts(as = "Option<bool>", optional)]
    pub backups: bool,
    /// Restore the site's Site Manager folder from the file (default `false`): staged now,
    /// put in place by the Site Manager at the server's next start.
    #[serde(default)]
    #[ts(as = "Option<bool>", optional)]
    pub site: bool,
}

fn yes() -> bool {
    true
}

impl std::fmt::Debug for ImportRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportRequest")
            .field("replace", &self.replace)
            .field("skip", &self.skip)
            .field("backups", &self.backups)
            .field("site", &self.site)
            .finish_non_exhaustive()
    }
}

/// `POST /api/v1/codex/export/imports/{id}/apply`: what was imported.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    /// Collections added.
    pub added: Vec<CollectionId>,
    /// Collections replaced (their previous copies are in the collection trash).
    pub replaced: Vec<CollectionId>,
    /// Collections of the file left out.
    pub skipped: Vec<CollectionId>,
    /// Settings backup sets added.
    pub backup_sets: u32,
    /// Their snapshots.
    pub snapshots: u32,
    /// The site's Site Manager folder is staged: it takes the place of this site's at the
    /// server's next start.
    pub site_staged: bool,
    /// A restart of the server is needed (the site part).
    pub restart_needed: bool,
    /// Always `false`.
    pub media_included: bool,
    /// [`MEDIA_NOTE`].
    pub note: String,
}
