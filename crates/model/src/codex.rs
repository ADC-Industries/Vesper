//! Vesper Codex: the site's content store (see `docs/CODEX-PLAN.md`).
//!
//! One Codex per site, on the site's server. Its content is split into **collections** (per
//! campus, ministry or room), each holding the collection kinds (presentations, libraries,
//! playlists, themes, media, bins, global groups, and the documents slide actions refer to).
//! The sidebar shelves of presentations stay [`Library`](crate::Library) documents inside a
//! collection.
//!
//! ```text
//! <data>/codex/codex.json                          CodexInfo
//! <data>/codex/collections/<id>/collection.json    Collection
//! <data>/codex/collections/<id>/portable/<kind>/   the collection's documents
//! <data>/codex/media/<sha256>.<ext>                managed media, shared by all collections
//! <data>/codex/collections/<id>/changes.jsonl     the collection's change log (ChangeEntry)
//! <data>/codex/trash/<entry>.json                  a deleted document (TrashEntry)
//! <data>/codex/trash.jsonl                         the trash's index (TrashEntry without documents)
//! <data>/codex/collection-trash/<id>/              a deleted collection, intact, with its
//!                                                  deleted.json (CollectionDeletion)
//! <data>/machine/codex.json                        CodexConfig (this box: role, active collection)
//! ```

use crate::common::{Color, SortKey, is_default};
use crate::ids::{CodexId, CollectionId};
use crate::{Document, DocumentKind};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The Codex itself (`codex/codex.json`, singleton on the box that holds the site's Codex).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CodexInfo {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// The Codex's ID.
    pub id: CodexId,
    /// When it was created (RFC 3339 UTC).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
}

/// What a collection is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum CollectionPurpose {
    /// Presentation content for Vesper and Oriel.
    #[default]
    Content,
    /// Signage content for Vesper Narthex.
    Signage,
    /// Value from a newer schema.
    #[serde(other)]
    Unknown,
}

/// One collection of the Codex (`codex/collections/<id>/collection.json`): a campus, ministry or
/// room's content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Collection ID.
    pub id: CollectionId,
    /// Name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub description: Option<String>,
    /// Colour in lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub color: Option<Color>,
    /// Order in the collection list.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// What the collection is for.
    #[serde(default, skip_serializing_if = "is_default")]
    pub purpose: CollectionPurpose,
    /// When it was created (RFC 3339 UTC).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
}

/// This box's part in the site's Codex.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum CodexRole {
    /// This box holds the site's Codex (`codex/`).
    #[default]
    Host,
    /// This box mirrors one collection of another box's Codex (`codex-cache/`); its own Codex
    /// is kept intact, dormant, until it leaves that site.
    Client,
    /// This box joined a site whose server hosts a Codex: its own Codex is dormant (kept
    /// intact, read only) while the first copy of the site's collection arrives; then the box
    /// is a `client`. Leaving the site wakes it.
    Dormant,
    /// Value from a newer schema.
    #[serde(other)]
    Unknown,
}

/// The Codex a client box follows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CodexLink {
    /// Server ID of the box holding the Codex.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_id: String,
    /// Its IDN device fingerprint.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fingerprint: String,
}

/// This box's Codex settings (`machine/codex.json`, machine-local singleton).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CodexConfig {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// This box's role (always written).
    #[serde(default)]
    pub role: CodexRole,
    /// The collection this box shows (its active collection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub collection: Option<CollectionId>,
    /// The Codex a client box follows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub codex: Option<CodexLink>,
    /// While this box follows another site's Codex (role `client`): its own Codex's active
    /// collection, shown again when the box leaves that site (its own Codex is kept intact,
    /// dormant, meanwhile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub own_collection: Option<CollectionId>,
}

/// How a document changed (change log).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum DocChange {
    /// A new document (also a restored one).
    Added,
    /// Its content changed.
    Changed,
    /// Deleted.
    Removed,
    /// Value from a newer schema.
    #[serde(other)]
    Unknown,
}

/// Who made a change: the client (a paired client's key name and id, or the server machine)
/// and the machine the write came in through.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChangeBy {
    /// The client's name (`server machine` for the server machine itself, `reload` for a reload
    /// from disk).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client: String,
    /// The paired client's id (its site key's id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub client_id: Option<String>,
    /// The server ID of the box the write came in through.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub machine: String,
}

/// One entry of a collection's change log (`changes.jsonl`): a document was added, changed or
/// removed. `seq` grows by one per entry and never repeats within a collection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChangeEntry {
    /// Position in the log (1, 2, …).
    pub seq: u64,
    /// When (RFC 3339 UTC).
    pub at: String,
    /// The document's kind.
    pub kind: DocumentKind,
    /// The document's ID.
    pub id: Uuid,
    /// What happened.
    pub change: DocChange,
    /// The document's revision after the change (not for removals).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub revision: Option<String>,
    /// Who.
    #[serde(default)]
    pub by: ChangeBy,
}

/// A deleted document in the Codex trash (`codex/trash/<id>.json`), kept 30 days. Restoring it
/// recreates the document with its ID.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntry {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// The trash entry's ID.
    pub id: Uuid,
    /// The collection the document was deleted from.
    pub collection: CollectionId,
    /// The document's kind.
    pub kind: DocumentKind,
    /// The document's ID.
    pub doc_id: Uuid,
    /// Its name, for lists.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// When it was deleted (RFC 3339 UTC).
    pub deleted_at: String,
    /// When it leaves the trash for good (RFC 3339 UTC).
    pub expires_at: String,
    /// Who deleted it.
    #[serde(default)]
    pub by: ChangeBy,
    /// The document as it was (absent in the trash's index, `trash.jsonl`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<serde_json::Value>,
    /// Documents deleted with it, restored with it: a playlist folder's sub-folders and
    /// playlists, parents first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtree: Vec<serde_json::Value>,
    /// How many documents `subtree` holds (kept in the index, which leaves `subtree` out).
    #[serde(default, skip_serializing_if = "is_default")]
    pub subtree_count: usize,
}

/// Why a collection is in the collection trash (`codex/collection-trash/<id>/deleted.json`,
/// next to its `collection.json`). A deleted collection keeps every file it had; it stays until
/// it is restored or deleted for good (no expiry).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionDeletion {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// When it was deleted (RFC 3339 UTC).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub deleted_at: String,
    /// Who deleted it.
    #[serde(default)]
    pub by: ChangeBy,
}

impl Default for CodexInfo {
    fn default() -> Self {
        CodexInfo {
            schema_version: crate::SCHEMA_VERSION,
            id: CodexId::new(),
            created_at: String::new(),
        }
    }
}

impl Default for Collection {
    fn default() -> Self {
        Collection {
            schema_version: crate::SCHEMA_VERSION,
            id: CollectionId::new(),
            name: String::new(),
            description: None,
            color: None,
            sort_key: SortKey::new(),
            purpose: CollectionPurpose::Content,
            created_at: String::new(),
        }
    }
}

impl Default for CodexConfig {
    fn default() -> Self {
        CodexConfig {
            schema_version: crate::SCHEMA_VERSION,
            role: CodexRole::Host,
            collection: None,
            codex: None,
            own_collection: None,
        }
    }
}

impl Document for CodexInfo {
    const KIND: DocumentKind = DocumentKind::CodexInfo;
    fn id(&self) -> Uuid {
        Uuid::nil()
    }
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

impl Document for Collection {
    const KIND: DocumentKind = DocumentKind::Collection;
    fn id(&self) -> Uuid {
        self.id.0
    }
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
}

impl Document for CodexConfig {
    const KIND: DocumentKind = DocumentKind::CodexConfig;
    fn id(&self) -> Uuid {
        Uuid::nil()
    }
    fn schema_version(&self) -> u32 {
        self.schema_version
    }
    fn portable() -> bool {
        false
    }
}
