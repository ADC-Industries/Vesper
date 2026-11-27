//! Settings backups kept by the Codex (see `docs/CODEX-PLAN.md`, M11/M12).
//!
//! One [`BackupSet`] per product and machine (Vesper: one per box, by its server ID; Oriel:
//! one per paired client, by its key's id). A set holds [`SettingsSnapshot`]s; a snapshot lists
//! its [`SnapshotItem`]s, each a content-addressed blob (its SHA-256) carrying its own
//! `schemaVersion`, so a product that doesn't exist yet needs no Codex code: it offers items,
//! the Codex keeps their bytes.
//!
//! ```text
//! <data>/codex/backups/sets/<set>.json                  BackupSet
//! <data>/codex/backups/snapshots/<set>/<snapshot>.json  SettingsSnapshot
//! <data>/codex/backups/blobs/<sha256>                   an item's bytes
//! ```
//!
//! Never in a snapshot: device identities (`machine/site-member/`, `machine/codex-member/`,
//! the Site Manager's folder), `cache/`, `clients.json`, tokens, keys and PIN hashes.

use crate::codex::ChangeBy;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A backup set's ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
pub struct BackupSetId(pub Uuid);

/// A settings snapshot's ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(transparent)]
pub struct SnapshotId(pub Uuid);

macro_rules! uuid_id {
    ($($t:ident),*) => {$(
        impl $t {
            /// A fresh random ID.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }
        impl Default for $t {
            fn default() -> Self {
                Self::new()
            }
        }
        impl std::fmt::Display for $t {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.hyphenated().fmt(f)
            }
        }
        impl std::str::FromStr for $t {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    )*};
}
uuid_id!(BackupSetId, SnapshotId);

/// The product of Vesper's own sets (a box's settings).
pub const PRODUCT_VESPER: &str = "vesper";
/// The product of Oriel's sets (a client's preferences).
pub const PRODUCT_ORIEL: &str = "oriel";

/// One product's settings on one machine (`codex/backups/sets/<id>.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BackupSet {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// The set's ID.
    pub id: BackupSetId,
    /// Whose settings: `vesper` (a box), `oriel` (a client's preferences), or a product to
    /// come.
    pub product: String,
    /// Which machine: a Vesper box's server ID, an Oriel client's key id (`local` for the
    /// server machine's own).
    pub machine: String,
    /// Its name when it last offered a snapshot (the server's name, the client's key name).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// The box's fingerprint in its site when it last offered a snapshot (a box that is a
    /// machine of the site): a replacement box may take over its server ID only once the site
    /// lists this fingerprint as removed. Not a secret, and never restored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub fingerprint: Option<String>,
    /// When the set was made (RFC 3339 UTC).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
}

/// Why a snapshot was taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum SnapshotReason {
    /// The daily snapshot.
    #[default]
    Scheduled,
    /// Settings changed (taken a while after the last change).
    Change,
    /// Asked for (`POST /codex/backups/snapshots`).
    Manual,
    /// Taken just before a restore: the restore's undo.
    BeforeRestore,
    /// Uploaded by the product itself (Oriel's preferences).
    Upload,
    /// Value from a newer schema.
    #[serde(other)]
    Unknown,
}

/// One item of a snapshot: a file's bytes, kept by their SHA-256
/// (`codex/backups/blobs/<sha256>`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SnapshotItem {
    /// Its key, unique in the snapshot. For Vesper the file's path in the data folder
    /// (`portable/screen/<id>.json`, `machine/outputBinding/<id>.json`, `machine/server.json`);
    /// for other products their own name (`preferences`).
    pub key: String,
    /// What it is (a document kind such as `screen` or `look`, `server`, `orielPreferences`).
    pub kind: String,
    /// The document's ID, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub id: Option<String>,
    /// Its name, for lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub name: Option<String>,
    /// The item's own schema version (its document's `schemaVersion`).
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// SHA-256 of its bytes (lowercase hex).
    pub sha256: String,
    /// Its size in bytes.
    pub size: u64,
}

/// A set's settings at one moment (`codex/backups/snapshots/<set>/<id>.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// The snapshot's ID.
    pub id: SnapshotId,
    /// Its set.
    pub set: BackupSetId,
    /// Its position in the set (1, 2, …; never reused).
    pub seq: u64,
    /// When it was taken (RFC 3339 UTC).
    pub taken_at: String,
    /// Why.
    #[serde(default)]
    pub reason: SnapshotReason,
    /// Who asked for it (the server machine for scheduled and change snapshots).
    #[serde(default)]
    pub by: ChangeBy,
    /// SHA-256 over the items' keys and SHA-256s: two snapshots with the same digest hold the
    /// same settings.
    pub digest: String,
    /// The product's version that offered it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// Its items, sorted by key.
    pub items: Vec<SnapshotItem>,
}

impl SettingsSnapshot {
    /// Bytes of its items.
    pub fn bytes(&self) -> u64 {
        self.items.iter().map(|i| i.size).sum()
    }
}

/// The digest of a snapshot's items ([`SettingsSnapshot::digest`]): SHA-256 over each item's
/// `key`, `schemaVersion` and `sha256`, in key order. `hash` is the SHA-256 function.
pub fn snapshot_digest(items: &[SnapshotItem], hash: impl Fn(&[u8]) -> String) -> String {
    let mut sorted: Vec<&SnapshotItem> = items.iter().collect();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));
    let mut text = String::new();
    for i in sorted {
        text.push_str(&i.key);
        text.push('\0');
        text.push_str(&i.schema_version.to_string());
        text.push('\0');
        text.push_str(&i.sha256);
        text.push('\n');
    }
    hash(text.as_bytes())
}
