//! Settings backups in the Codex (`/api/v1/codex/backups…`): backup sets (one per product and
//! machine), their snapshots, restore preview and restore, and Oriel's preferences. See the
//! protocol README's Codex section.
//!
//! - **Sets and snapshots** ([`BackupSetSummary`], [`SnapshotSummary`]): listed for every
//!   paired client; a snapshot's contents ([`SnapshotContents`]) only for admins.
//! - **Restore** ([`RestorePreview`], [`RestoreRequest`], [`RestoreResult`], admins): what would
//!   change, per item; restoring all or chosen items, after a `beforeRestore` snapshot (the
//!   undo).
//! - **Oriel's preferences** ([`OrielPreferences`], [`OrielPreferencesBackup`]): each client
//!   uploads and fetches its own.

use model::{
    BackupSet, BackupSetId, ChangeBy, DocChange, SnapshotId, SnapshotItem, SnapshotReason,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A snapshot without its items' contents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSummary {
    /// Its ID.
    pub id: SnapshotId,
    /// Its set.
    pub set: BackupSetId,
    /// Its position in the set.
    pub seq: u64,
    /// When it was taken (RFC 3339 UTC).
    pub taken_at: String,
    /// Why.
    pub reason: SnapshotReason,
    /// Who asked for it.
    pub by: ChangeBy,
    /// Same digest, same settings.
    pub digest: String,
    /// Its items.
    pub items: u32,
    /// Their bytes.
    pub bytes: u64,
}

/// `GET /api/v1/codex/backups`: a backup set (one per product and machine), with its newest
/// snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BackupSetSummary {
    /// The set.
    pub set: BackupSet,
    /// Its snapshots.
    pub snapshots: u32,
    /// The newest one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub latest: Option<SnapshotSummary>,
    /// This box's own settings (Vesper on the box answering).
    #[serde(default)]
    pub this_box: bool,
    /// The asking client's own preferences (Oriel).
    #[serde(default)]
    pub yours: bool,
}

/// One item of a snapshot with its content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotItemContent {
    /// The item.
    pub item: SnapshotItem,
    /// Its content (the JSON document as it was).
    #[ts(type = "unknown")]
    pub content: serde_json::Value,
}

/// `GET /api/v1/codex/backups/{set}/snapshots/{snapshot}` (admins): a snapshot with its items'
/// contents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotContents {
    /// The snapshot.
    pub snapshot: SnapshotSummary,
    /// Its items, sorted by key.
    pub items: Vec<SnapshotItemContent>,
}

/// A snapshot asked for or offered: the new one, or the newest one when nothing changed
/// since (`created: false`; identical settings make no new snapshot).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotTaken {
    /// The snapshot.
    pub snapshot: SnapshotSummary,
    /// A new snapshot was made.
    pub created: bool,
}

/// How one item would change (preview) or changed (restore), seen from the box: `added` (the
/// snapshot has it, the box doesn't), `changed`, `removed` (the box has it, the snapshot
/// doesn't).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RestoreChange {
    /// The item's key (choose items by it in [`RestoreRequest::items`]).
    pub key: String,
    /// What it is (`screen`, `look`, `server`, …).
    pub kind: String,
    /// The document's ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<String>,
    /// Its name (the snapshot's, else the box's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// How.
    pub change: DocChange,
    /// Takes effect only after the server restarts (the server's name, font substitutions).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub restart: bool,
}

/// `GET /api/v1/codex/backups/{set}/snapshots/{snapshot}/preview` (admins): what restoring the
/// snapshot onto its box would change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreview {
    /// The snapshot.
    pub snapshot: SnapshotSummary,
    /// Compared with the box's settings as of this snapshot of them (another box: its newest
    /// snapshot here); absent: as they are now (this box).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub current_as_of: Option<String>,
    /// The items that would change.
    pub changes: Vec<RestoreChange>,
    /// Items that are the same.
    pub unchanged: u32,
    /// In words: `3 screens, 2 looks changed; 1 output binding added`.
    pub summary: String,
    /// What would need a restart of the server to take effect.
    pub restart: Vec<String>,
}

/// `POST /api/v1/codex/backups/{set}/snapshots/{snapshot}/restore` (admins).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRequest {
    /// The items to restore, by key (from the preview); absent: every item, and what the
    /// snapshot doesn't hold is removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub items: Option<Vec<String>>,
    /// A replacement box takes over the old box's server ID (only once the old box was
    /// removed from the site). The IDN device identity is never restored.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub adopt_server_id: bool,
    /// The box to restore onto, by its server ID; absent: the set's own box. Another box than
    /// the one answering (the Codex box) gets the restore when it next asks the Codex (a show
    /// box polls every 30 s): the answer is `202` with a [`QueuedRestore`]. A replacement box
    /// restoring an old box's set names itself here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target: Option<String>,
}

/// What a restore did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    /// The snapshot taken just before (restore it to undo).
    pub before_restore: SnapshotSummary,
    /// The items changed.
    pub applied: Vec<RestoreChange>,
    /// In words.
    pub summary: String,
    /// What takes effect only after a restart of the server (the rest applied at once).
    pub restart: Vec<String>,
    /// The box took over the old box's server ID (after a restart).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub server_id_adopted: bool,
}

/// Where a restore queued for a show box stands ([`QueuedRestore::state`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RestoreState {
    /// Waiting for the box to ask for it (it does at its next poll of the Codex).
    #[default]
    Queued,
    /// The box took it: its `beforeRestore` snapshot is made, it is writing the files.
    Applying,
    /// Done: `result` says what changed.
    Applied,
    /// It failed: `error` says why (nothing was written when the Codex refused it).
    Failed,
    /// The box didn't ask for it in time (`expiresAt`): nothing was done.
    Expired,
    /// An admin cancelled it before the box took it.
    Cancelled,
    /// Value from a newer server.
    #[serde(other)]
    Unknown,
}

/// A restore of a show box's settings, queued on the Codex box until that box takes it
/// (`202` from `POST …/restore`; `GET /api/v1/codex/backups/restores/{id}`, admins).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueuedRestore {
    /// Its ID.
    pub id: String,
    /// The set restored.
    pub set: BackupSetId,
    /// The snapshot restored.
    pub snapshot: SnapshotId,
    /// The box restored onto: its server ID.
    pub target: String,
    /// That box's name (its backup set's).
    pub target_name: String,
    /// The items chosen (absent: all).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub items: Option<Vec<String>>,
    /// The box takes over the set's server ID (a replacement box).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub adopt_server_id: bool,
    /// Where it stands.
    pub state: RestoreState,
    /// When it was queued (RFC 3339 UTC).
    pub queued_at: String,
    /// When its state last changed.
    pub updated_at: String,
    /// A restore still `queued` then expires.
    pub expires_at: String,
    /// Who asked for it.
    pub by: ChangeBy,
    /// Why it failed (`failed`, `expired`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// What it did (`applied`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub result: Option<RestoreResult>,
}

/// `PUT /api/v1/codex/backups/oriel`: this client's preferences, as Oriel keeps them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OrielPreferences {
    /// Oriel's own schema version of `preferences`.
    pub schema_version: u32,
    /// The preferences (a JSON object, at most 1 MiB).
    #[ts(type = "unknown")]
    pub preferences: serde_json::Value,
}

/// `GET /api/v1/codex/backups/oriel[?snapshot=]`: this client's preferences as backed up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OrielPreferencesBackup {
    /// Oriel's schema version of `preferences`.
    pub schema_version: u32,
    /// The preferences.
    #[ts(type = "unknown")]
    pub preferences: serde_json::Value,
    /// The snapshot they come from.
    pub snapshot: SnapshotSummary,
}
