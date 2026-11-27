//! Vesper Codex (`/api/v1/codex/...`): the site's content store, its collections, this box's
//! active collection, each collection's change log and the trash, media, search, and on show
//! boxes the copy they follow. Settings backups are in [`crate::backup`]. See the protocol
//! README's Codex section. Servers with it announce the `codex` capability.
//!
//! - **Status** ([`CodexStatus`]): the Codex's ID, this box's role and active collection, basic
//!   counts and its media library; on a show box how its copy syncs ([`CodexSync`]).
//! - **Collections** ([`CollectionSummary`]): CRUD with revisions (`If-Match`, `409` with
//!   `current`); a deleted collection goes to the collection trash ([`DeletedCollection`]),
//!   intact, until it is restored or deleted for good. The collection a box shows can't be
//!   deleted.
//! - **Assignment** ([`SetAssignment`]): the collection this box shows.
//! - **Change log** ([`CollectionChanges`]): what changed since a `seq`; `resync` when the log
//!   no longer reaches back that far.
//! - **Trash** ([`TrashItem`]): deleted documents, restorable for 30 days with their IDs.
//! - **Uploads** ([`CreateUpload`], [`UploadInfo`], [`UploadFinished`]): resumable media uploads
//!   (`Content-Range` chunks, SHA-256 checked, content-addressed and deduplicated).
//! - **Media** ([`CollectionMedia`], [`RelinkTarget`], [`CodexStorage`]): availability now,
//!   relinks, storage usage.
//! - **Search** ([`SearchResults`]): documents matching a query, in one collection or all.
//! - **Show boxes**: a show box's `409 codexElsewhere` names the Codex box ([`CodexBox`]).
//! - **Events:** `codexChanged {parts, collection?}` ([`CodexPart`]): refetch those parts.

use model::{
    AssetLocation, ChangeBy, ChangeEntry, CodexId, CodexRole, Collection, CollectionId,
    DocumentKind, MediaAsset, MediaAssetId, MediaBinId, MediaBinItemId, MediaRootId, RelinkMethod,
};
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

/// `GET /api/v1/codex`: this box's view of the site's Codex.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CodexStatus {
    /// The Codex's ID.
    pub id: CodexId,
    /// This box's part in it (`host`: it holds the Codex).
    pub role: CodexRole,
    /// The collection this box shows (its active collection); the content routes act on it.
    pub collection: CollectionId,
    /// Its name.
    pub collection_name: String,
    /// Collections in the Codex.
    pub collections: u32,
    /// Documents in every collection.
    pub documents: u64,
    /// Entries in the trash.
    pub trash: u32,
    /// Collections in the collection trash.
    #[serde(default)]
    pub deleted_collections: u32,
    /// The Codex's media library (on the box that hosts the Codex).
    #[serde(default)]
    pub media_library: MediaLibraryStatus,
    /// On a show box (role `client`, or `dormant` while the first copy arrives): how its
    /// mirror of the site's Codex stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sync: Option<CodexSync>,
    /// This box's own Codex is dormant (role `client`): its active collection, shown again
    /// when the box leaves the site. The own Codex is kept intact meanwhile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub dormant_collection: Option<CollectionId>,
    /// On the box that hosts the Codex: the show boxes that followed a collection in the last
    /// 24 hours, from their requests over the box-to-Codex link (G14).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<CodexBoxSync>>", optional)]
    pub boxes: Vec<CodexBoxSync>,
}

/// A show box following a collection of this Codex (`CodexStatus.boxes`, on the Codex box).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CodexBoxSync {
    /// Its IDN device fingerprint in the site.
    pub fingerprint: String,
    /// Its name in the site.
    pub name: String,
    /// The collection it shows.
    pub collection: CollectionId,
    /// The change-log position it has been sent (`null` until it asked for changes).
    pub seq: Option<u64>,
    /// How many change-log entries it is behind the collection's newest (`null` while its
    /// position isn't known).
    pub behind: Option<u64>,
    /// When it last asked (RFC 3339 UTC).
    pub last_seen: String,
    /// It asked in the last 10 minutes (a box asks every 30 s, and at once after a change).
    pub online: bool,
}

/// The Vespers showing a collection (`CollectionSummary.boxes`, on the Codex box): online show
/// boxes following it, plus the Codex box itself when it is its active collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CollectionBoxes {
    /// Vespers showing it.
    pub showing: u32,
    /// Of those, the ones with every change ("Synced to 2 of 3 Vespers").
    pub synced: u32,
}

/// A show box's mirror of the site's Codex (`GET /api/v1/codex`, `sync`): the box shows its
/// local copy of one collection and plays media from its own disk only; this says how far
/// behind the Codex it is. `codexChanged` (`status`) when it changes meaningfully.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CodexSync {
    /// The site's Codex answered the last exchange.
    pub reachable: bool,
    /// The server ID of the box holding the site's Codex, once found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub server_id: Option<String>,
    /// Its name (the Codex box's server name), once heard from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Its IDN device fingerprint in the site, once found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fingerprint: Option<String>,
    /// When this box last heard from the Codex (RFC 3339 UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_sync: Option<String>,
    /// The change-log position this box's copy is at.
    pub seq: u64,
    /// The Codex's newest position, as last heard.
    pub codex_seq: u64,
    /// Changes this box hasn't applied yet (as far as it knows).
    pub behind: u64,
    /// What keeps the mirror from following (the Codex can't be reached, a document from a
    /// newer schema).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
    /// The media this box keeps (IDN's library cache).
    pub media: CodexCacheStatus,
}

/// The box that holds the site's Codex, as a show box knows it (`ErrorResponse.codex` of a
/// show box's `409 codexElsewhere`): send uploads and Codex management there.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CodexBox {
    /// Its server ID (`GET /api/v1/server`'s `id`; mDNS announces it too).
    pub server_id: String,
    /// Its name, once heard from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Its IDN device fingerprint in the site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fingerprint: Option<String>,
    /// Base URLs of its REST API this box knows (`http://host:port`, without `/api/v1`): the
    /// site server's address with the Codex's API port, and the addresses the Codex box
    /// listens on. Empty while this box hasn't heard from it since it started.
    pub addresses: Vec<String>,
}

/// A show box's media cache: the library items its playlists and bins use are kept (pinned);
/// others are copied when triggered, and removed again when the cache needs room.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CodexCacheStatus {
    /// Items kept for the box's playlists and media bins.
    pub pinned: u32,
    /// Items on this box's disk, ready to play.
    pub cached: u32,
    /// Items being copied or waiting to be.
    pub copying: u32,
    /// Items the Codex's library doesn't have.
    pub missing: u32,
    /// Items whose last copy failed (tried again).
    pub failed: u32,
    /// Bytes on this box's disk.
    pub bytes: u64,
    /// The cache's size cap, bytes.
    pub cap_bytes: u64,
}

/// The Codex's media library (IDN's site media library on the Codex box), as `GET /codex` says
/// it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaLibraryStatus {
    /// It is open: library media are available (`false` on a box that doesn't host the Codex,
    /// or while `problem` says why not).
    pub available: bool,
    /// Why it isn't available (its index can't be read: nothing was changed or recreated;
    /// the library's media are missing until it opens again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
    /// It is served to the site's machines (once the Codex is a member of its site).
    #[serde(default)]
    pub served: bool,
}

/// Documents in a collection, per kind.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CollectionCounts {
    /// Presentations.
    pub presentations: u32,
    /// Libraries (the sidebar shelves).
    pub libraries: u32,
    /// Playlists and playlist folders.
    pub playlists: u32,
    /// Themes.
    pub themes: u32,
    /// Media assets.
    pub media_assets: u32,
    /// Media and audio bins.
    pub media_bins: u32,
    /// Global groups.
    pub global_groups: u32,
    /// Timers.
    pub timers: u32,
    /// Messages.
    pub messages: u32,
    /// Props.
    pub props: u32,
    /// Macros.
    pub macros: u32,
    /// Clear groups.
    pub clear_groups: u32,
    /// Stage layouts.
    pub stage_layouts: u32,
}

impl CollectionCounts {
    /// Every document.
    pub fn total(&self) -> u64 {
        [
            self.presentations,
            self.libraries,
            self.playlists,
            self.themes,
            self.media_assets,
            self.media_bins,
            self.global_groups,
            self.timers,
            self.messages,
            self.props,
            self.macros,
            self.clear_groups,
            self.stage_layouts,
        ]
        .iter()
        .map(|&n| u64::from(n))
        .sum()
    }
}

/// `GET /api/v1/codex/collections` (sorted by sort key, then name) and
/// `GET /api/v1/codex/collections/{id}` (`ETag` = `revision`): a collection with its counts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSummary {
    /// The collection document.
    pub collection: Collection,
    /// Its revision (a content hash of `collection`): send it as `If-Match`.
    pub revision: String,
    /// This box shows it (its active collection).
    pub active: bool,
    /// Written by a newer server version: read only.
    #[serde(default)]
    pub read_only: bool,
    /// Its documents, per kind.
    pub counts: CollectionCounts,
    /// Bytes of its documents on disk.
    pub document_bytes: u64,
    /// Bytes of the managed media files its media assets use (each file counted once; a file
    /// can be shared with other collections).
    pub media_bytes: u64,
    /// The Vespers showing it and how many are up to date (G14).
    #[serde(default)]
    pub boxes: CollectionBoxes,
}

/// `POST /api/v1/codex/collections` (admins): a new, empty collection.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateCollection {
    /// Name (1–200 characters).
    pub name: String,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub description: Option<String>,
    /// Colour in lists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color: Option<model::Color>,
    /// What it is for (default `content`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub purpose: Option<model::CollectionPurpose>,
    /// Order key (default: last).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sort_key: Option<String>,
}

fn double<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

/// `PUT /api/v1/codex/collections/{id}` (admins, `If-Match`): change a collection. Omitted
/// fields stay; `null` clears `description` and `color`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCollection {
    /// Name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Description (`null` clears).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double"
    )]
    #[ts(optional)]
    pub description: Option<Option<String>>,
    /// Colour (`null` clears).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double"
    )]
    #[ts(optional)]
    pub color: Option<Option<model::Color>>,
    /// What it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub purpose: Option<model::CollectionPurpose>,
    /// Order key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sort_key: Option<String>,
}

/// `PUT /api/v1/codex/assignment` (admins): the collection this box shows. The box switches
/// to it at once (the show keeps running; its content routes then act on that collection).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SetAssignment {
    /// The collection.
    pub collection: CollectionId,
    /// Switch even though something is live on this box (a slide, media, a prop or a
    /// message): without it such a switch answers `409 liveShow`, naming what is live.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub confirm: bool,
}

/// `GET /api/v1/codex/collections/{id}/changes?since=<seq>`: the collection's changes after
/// `since`, oldest first. With `resync: true` the log no longer reaches back to `since` (it was
/// compacted, or `since` is from a log this one doesn't continue): `changes` is empty, refetch
/// everything and continue from `to`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CollectionChanges {
    /// The `since` the changes follow (the log's floor when `resync`).
    pub from: u64,
    /// The newest `seq`: ask with `since=<to>` next time.
    pub to: u64,
    /// The changes (`seq` from `from + 1` to `to`), at most `limit` (default 1000): with more,
    /// `to` is the last one sent, and asking again continues.
    pub changes: Vec<ChangeEntry>,
    /// The log doesn't reach back to `since`: refetch everything.
    pub resync: bool,
}

/// `GET /api/v1/codex/trash`: a deleted document, newest first. Restore it with
/// `POST /api/v1/codex/trash/{id}/restore`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrashItem {
    /// The trash entry's ID.
    pub id: String,
    /// The collection it was deleted from.
    pub collection: CollectionId,
    /// The document's kind.
    pub kind: DocumentKind,
    /// The document's ID (kept when restored).
    pub doc_id: String,
    /// Its name.
    pub name: String,
    /// When it was deleted (RFC 3339 UTC).
    pub deleted_at: String,
    /// When it leaves the trash for good (RFC 3339 UTC).
    pub expires_at: String,
    /// Who deleted it.
    pub by: ChangeBy,
    /// Documents deleted and restored with it (a folder's sub-folders and playlists).
    #[serde(default)]
    pub subtree: u32,
}

/// `GET /api/v1/codex/collection-trash` (newest first): a deleted collection, kept intact on
/// disk until it is restored (`POST /api/v1/codex/collection-trash/{id}/restore`) or deleted for
/// good (`DELETE /api/v1/codex/collection-trash/{id}`). No expiry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeletedCollection {
    /// The collection document.
    pub collection: Collection,
    /// Its revision: `If-Match` of the final delete.
    pub revision: String,
    /// Its documents, per kind.
    pub counts: CollectionCounts,
    /// Bytes of its documents on disk.
    pub document_bytes: u64,
    /// Bytes of the managed media files its media assets use (each once).
    pub media_bytes: u64,
    /// Entries of the item trash deleted from it (they come back to the trash list with it).
    pub trash_items: u32,
    /// When it was deleted (RFC 3339 UTC).
    pub deleted_at: String,
    /// Who deleted it.
    pub by: ChangeBy,
}

/// `POST /api/v1/codex/trash/{id}/restore`: what was restored, into which collection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrashRestored {
    /// The collection.
    pub collection: CollectionId,
    /// The document's kind.
    pub kind: DocumentKind,
    /// The restored documents' IDs (the entry's document first).
    pub documents: Vec<String>,
}

/// `DELETE /api/v1/codex/trash` (admins): how many entries were purged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrashPurged {
    /// Entries removed for good.
    pub purged: u32,
}

/// `POST /api/v1/codex/uploads`: starts a resumable media upload into a collection (a content
/// edit: any paired client).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateUpload {
    /// The collection the new media asset goes into.
    pub collection: CollectionId,
    /// The file's name with its extension (`Sunrise.jpg`): the asset's name (without the
    /// extension) and kind (image, video or audio, from the extension).
    pub name: String,
    /// The file's size in bytes.
    pub size: u64,
    /// The file's SHA-256 (64 lowercase hex digits), now or with `finish`: the server checks
    /// it before keeping the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sha256: Option<String>,
    /// A manual bin of the collection that gets the new asset as its last item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bin: Option<MediaBinId>,
    /// Where the file was on the client (`Backgrounds/Sunrise.jpg`), kept as the asset's
    /// origin (relink searches use it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub relative_path: Option<String>,
    /// Relink this media asset of the collection to the uploaded file instead of making a new
    /// asset (a manual relink: `bin` is ignored).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub replaces: Option<MediaAssetId>,
}

/// An upload in progress (`POST /api/v1/codex/uploads` → `201`, `GET`/`PUT
/// /api/v1/codex/uploads/{id}`). Send the bytes from `offset` on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UploadInfo {
    /// The upload's ID.
    pub id: String,
    /// The collection.
    pub collection: CollectionId,
    /// The file's name.
    pub name: String,
    /// The file's size in bytes.
    pub size: u64,
    /// Bytes received so far: the next chunk starts here.
    pub offset: u64,
    /// The expected SHA-256, when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sha256: Option<String>,
    /// The bin the asset goes into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bin: Option<MediaBinId>,
    /// When it was started (RFC 3339 UTC).
    pub created_at: String,
    /// When its last chunk came (RFC 3339 UTC).
    pub updated_at: String,
    /// When it is dropped unless more bytes come (RFC 3339 UTC).
    pub expires_at: String,
}

/// `409` to a chunk that doesn't start at the upload's offset (or a `finish` before every byte
/// came): continue from `offset`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UploadConflict {
    /// The error (`conflict`).
    pub error: crate::ApiError,
    /// Bytes the server has: the next chunk starts here.
    pub offset: u64,
}

/// `POST /api/v1/codex/uploads/{id}/finish` (optional body): the SHA-256, when `CreateUpload`
/// didn't carry it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FinishUpload {
    /// The file's SHA-256 (64 lowercase hex digits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sha256: Option<String>,
}

/// `POST /api/v1/codex/uploads/{id}/finish` → `201`: the file is checked and kept, and the
/// collection has a new media asset for it (or the one it had for those bytes already,
/// `existing`). Probing and the thumbnail follow in the background
/// (the asset's `documentChanged` says when its `probe` is filled).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UploadFinished {
    /// The collection.
    pub collection: CollectionId,
    /// The new media asset (`location`: `library`, the media library's item and its
    /// SHA-256).
    pub asset: MediaAsset,
    /// The asset's revision.
    pub revision: String,
    /// The media library had the file already (the same SHA-256): the uploaded bytes were
    /// dropped and the asset refers to the item that was there.
    pub deduplicated: bool,
    /// The bin item made for it, when the upload named a bin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bin_item: Option<MediaBinItemId>,
    /// The upload relinked an existing asset (`CreateUpload.replaces`): `asset` is that one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub relinked: bool,
    /// The collection had a media asset for these bytes already (the same SHA-256): `asset` is
    /// that one, no second asset was made (and `binItem` is its item in the named bin, added
    /// when it wasn't there). Not with `replaces`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub existing: bool,
}

/// Files and their bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FileUsage {
    /// Files.
    pub files: u64,
    /// Their bytes.
    pub bytes: u64,
}

/// The disk the Codex is on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiskUsage {
    /// Bytes on the disk.
    pub total: u64,
    /// Bytes still free for the server.
    pub available: u64,
    /// Bytes uploads leave free (5 % of the disk plus 2 GiB, at least what the media library
    /// keeps free).
    pub keep_free: u64,
}

/// One collection's share of the Codex's storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CollectionStorage {
    /// The collection.
    pub collection: CollectionId,
    /// Its name.
    pub name: String,
    /// Its documents.
    pub documents: u64,
    /// Bytes of its documents on disk.
    pub document_bytes: u64,
    /// Bytes of the media library's items (and old managed files) its assets use (each
    /// once).
    pub media_bytes: u64,
    /// Bytes of the media only this collection uses (what deleting its media from the library
    /// would free once it is deleted for good).
    pub unique_media_bytes: u64,
    /// Its media assets.
    pub assets: u32,
    /// Its media assets whose file can't be found on this box.
    pub missing: u32,
}

/// The trashes' share of the Codex's storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TrashUsage {
    /// Entries in the item trash (those of deleted collections included).
    pub items: u64,
    /// Bytes of the item trash.
    pub item_bytes: u64,
    /// Collections in the collection trash.
    pub collections: u64,
    /// Bytes of their documents and change logs.
    pub collection_bytes: u64,
    /// Bytes of the library items only deleted collections and trashed documents use.
    pub media_bytes: u64,
}

/// The media library's own figures (IDN's `space`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryUsage {
    /// Bytes of its items and of its trash.
    pub bytes: u64,
    /// Its items.
    pub items: u64,
    /// Items in its trash (deleted there; restorable until they expire).
    pub trash_items: u64,
    /// Days a deleted item stays in its trash.
    pub trash_days: u64,
}

/// `GET /api/v1/codex/storage`: where the Codex's disk space goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CodexStorage {
    /// The disk.
    pub disk: DiskUsage,
    /// Per collection, in the Codex's order.
    pub collections: Vec<CollectionStorage>,
    /// The media library (all zero while it isn't open).
    #[serde(default)]
    pub library: LibraryUsage,
    /// Every item of the media library.
    pub media: FileUsage,
    /// Library items used by more than one collection.
    pub shared: FileUsage,
    /// Library items no document uses (live or in a trash): not marked in use, so the
    /// library's owner may delete them; the Codex never deletes them itself.
    pub unreferenced: FileUsage,
    /// Media files left in the old managed store (`codex/media/`: endings the library
    /// doesn't take).
    #[serde(default)]
    pub managed: FileUsage,
    /// The trashes.
    pub trash: TrashUsage,
    /// Stored media thumbnails.
    pub thumbs: FileUsage,
    /// Uploads in progress.
    pub uploads: FileUsage,
}

/// `GET /api/v1/codex/collections/{id}/media?status=`: a media asset of a collection with
/// its availability on this box, worked out now (the stored `status` is the last known one).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CollectionMedia {
    /// The asset.
    pub asset: MediaAsset,
    /// Its revision.
    pub revision: String,
    /// Its file can be found on this box now.
    pub availability: crate::MediaAvailability,
    /// Why not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
}

/// `POST /api/v1/codex/collections/{id}/media/{asset}/relink`: a manual relink to a file the
/// operator chose.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RelinkTarget {
    /// An item of the Codex's media library, by its ID (an upload's `UploadFinished` names
    /// one in its asset's location).
    Library {
        /// The item's ID.
        id: String,
    },
    /// A media file the Codex has, by its SHA-256: the library's item with those bytes (or,
    /// for media the library doesn't take, a managed file of the old store).
    Blob {
        /// Its SHA-256.
        sha256: String,
        /// Its extension.
        extension: String,
    },
    /// A file under a media root (`path`: relative, `/`-separated).
    Linked {
        /// The root.
        root: MediaRootId,
        /// The path below it.
        path: String,
    },
}

/// A relinked asset (`POST …/relink` answers with it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelinkedAsset {
    /// The asset after the relink (its `relinks` end with this one).
    pub asset: MediaAsset,
    /// Its revision.
    pub revision: String,
}

/// `POST /api/v1/codex/collections/{id}/media/search`: looks for missing media under the media
/// roots bound on this box: the same relative path under a bound root (`rootBinding`), then a
/// file with the same name found once (`fileName`), then, for managed media, a file with the
/// same SHA-256 (`contentHash`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelinkSearch {
    /// The assets to look for (default: every asset of the collection missing on this box).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub assets: Option<Vec<MediaAssetId>>,
    /// Only report what would be relinked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub dry_run: bool,
}

/// One asset a relink found a file for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelinkFound {
    /// The asset.
    pub asset: MediaAssetId,
    /// Its name.
    pub name: String,
    /// How the file was found.
    pub method: RelinkMethod,
    /// Where it is.
    pub location: AssetLocation,
}

/// One asset with several files that could be it (nothing relinked).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelinkAmbiguous {
    /// The asset.
    pub asset: MediaAssetId,
    /// Its name.
    pub name: String,
    /// The files (at most 10).
    pub candidates: Vec<AssetLocation>,
}

/// What a relink search or re-root did (or, with `dryRun`, would do).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RelinkReport {
    /// Assets looked at.
    pub checked: u32,
    /// Relinked (each with a new `relinks` record and a `documentChanged`).
    pub relinked: Vec<RelinkFound>,
    /// Several candidates: choose one with a manual relink.
    pub ambiguous: Vec<RelinkAmbiguous>,
    /// Nothing found.
    pub not_found: Vec<MediaAssetId>,
    /// Assets whose stored `status` changed (online ↔ missing) without a relink.
    pub status_changed: u32,
}

/// A file in a folder the client picked (`POST …/media/match`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ClientFile {
    /// Its name.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its path within the picked folder (`Backgrounds/Sunrise.jpg`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub relative_path: Option<String>,
}

/// `POST /api/v1/codex/collections/{id}/media/match`: which of the client's files are the
/// collection's missing media. Nothing changes: the client uploads each match with
/// `CreateUpload.replaces`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchRequest {
    /// The files of the picked folder.
    pub files: Vec<ClientFile>,
    /// The assets to match (default: every asset of the collection missing on this box).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub assets: Option<Vec<MediaAssetId>>,
}

/// How a client file was matched.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MatchBy {
    /// The end of its relative path is the asset's.
    RelativePath,
    /// Its name is the asset's file name, and no other file has it.
    FileName,
    /// A value from a newer server.
    #[serde(other)]
    Unknown,
}

/// A client file that is a missing asset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatch {
    /// The asset.
    pub asset: MediaAssetId,
    /// Its name.
    pub name: String,
    /// The file.
    pub file: ClientFile,
    /// How it matched.
    pub by: MatchBy,
}

/// `POST …/media/match` → the matches, and the assets no file matched.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchReport {
    /// Matched assets.
    pub matches: Vec<MediaMatch>,
    /// Assets several files could be.
    pub ambiguous: Vec<MediaAssetId>,
    /// Assets no file matched.
    pub unmatched: Vec<MediaAssetId>,
}

fn yes() -> bool {
    true
}

/// `POST /api/v1/codex/collections/{id}/media/reroot`: moves many linked assets at once, e.g.
/// after a media folder moved: every asset under `from` gets the same path under `to`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RerootRequest {
    /// Which assets: those linked under `root` whose path starts with `prefix`; without `root`,
    /// those whose original path (`origin.absoluteHint`) starts with `prefix` (ignoring case,
    /// and backslashes taken as slashes).
    pub from: RerootFrom,
    /// Where to: the root and the folder that replaces `from.prefix`.
    pub to: RerootTo,
    /// Only assets whose new file exists on this box (default `true`).
    #[serde(default = "yes")]
    pub only_existing: bool,
    /// Only report what would be relinked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub dry_run: bool,
}

/// The assets a re-root moves.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RerootFrom {
    /// Assets linked under this root (else: by their original path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub root: Option<MediaRootId>,
    /// The folder they are in (`""`: anywhere).
    #[serde(default)]
    pub prefix: String,
}

/// Where a re-root moves them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RerootTo {
    /// The root.
    pub root: MediaRootId,
    /// The folder below it that replaces `from.prefix` (`""`: the root itself).
    #[serde(default)]
    pub prefix: String,
}

/// `GET /api/v1/codex/library?q=&kind=&limit=`: the media library's items (for a relink to
/// any of them), by path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItems {
    /// The items matching, at most `limit`.
    pub items: Vec<LibraryItem>,
    /// How many matched in all.
    pub total: u32,
}

/// One item of the Codex's media library (IDN's).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItem {
    /// Its lasting ID (relink with `{"type": "library", id}`).
    pub id: String,
    /// Its path in the library (`Sunday/Welcome.mp4`).
    pub path: String,
    /// What it is called.
    pub title: String,
    /// `video`, `picture` or `sound`.
    pub kind: String,
    /// Bytes.
    pub size: u64,
    /// Its SHA-256, once the library worked it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sha256: Option<String>,
    /// Seconds (video and sound), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub duration: Option<f64>,
}

/// A part of the Codex that changed (`codexChanged`): refetch it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum CodexPart {
    /// `GET /api/v1/codex` (this box's active collection, counts).
    Status,
    /// `GET /api/v1/codex/collections` (a collection added, changed or deleted, or its counts
    /// changed; `collection` names it).
    Collections,
    /// `GET /api/v1/codex/trash`.
    Trash,
    /// `GET /api/v1/codex/collection-trash`.
    CollectionTrash,
    /// `GET /api/v1/codex/collections/{id}/media`: library items the collection uses changed
    /// or vanished (availability may differ).
    Media,
    /// `GET /api/v1/codex/backups` (a settings snapshot taken or removed).
    Backups,
    /// `GET /api/v1/codex/backups/restores` (a restore queued for a show box moved on).
    Restores,
    /// `GET /api/v1/codex/export/settings` and `GET …/export/scheduled` (a scheduled export
    /// ran or failed, the settings or the stored passphrase changed: last run, next run,
    /// problem).
    Exports,
}

/// `GET /api/v1/codex/search?q=&collection=&kinds=&limit=`: the documents matching a query,
/// best first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    /// The matches, best first: at most `limit`.
    pub hits: Vec<SearchHit>,
    /// How many documents matched in all (more than `hits` when `limit` cut the list).
    pub total: u32,
}

/// One document matching a search.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// The document's kind.
    pub kind: DocumentKind,
    /// The document's ID.
    pub id: String,
    /// The collection holding it.
    pub collection: CollectionId,
    /// That collection's name.
    pub collection_name: String,
    /// The document's name.
    pub title: String,
    /// Where the snippet comes from.
    pub field: SearchField,
    /// A short piece of that field with the matching words marked (`mark: true`); line breaks
    /// are shown as ` / `, and `…` stands for text left out.
    pub snippet: Vec<SnippetPart>,
    /// Where in the document: for a presentation the slide (0-based over all slides in
    /// document order, as `SlidePosition.slideIndex`; for a group its first slide), for a theme
    /// the template slide (0-based). Absent for the document's own fields (its name, notes,
    /// details).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub slide_index: Option<u32>,
    /// The presentation's slide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub slide: Option<model::SlideId>,
    /// The presentation's section (group).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub section: Option<model::SectionId>,
}

/// The part of a document a search hit's snippet comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SearchField {
    /// The document's name.
    Name,
    /// A presentation's details: song details (title, artist, authors, publisher, CCLI
    /// number), category, scripture reference.
    Details,
    /// A group (section) name of a presentation.
    Group,
    /// A slide's label.
    Label,
    /// Slide text (a presentation's or a prop's), or a message's text.
    Text,
    /// Notes (a slide's or the presentation's).
    Notes,
    /// A theme's template slide name.
    Template,
    /// A media asset's original file reference (where it was imported or uploaded from).
    Origin,
}

/// A piece of a search snippet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SnippetPart {
    /// The text.
    pub text: String,
    /// It matches the query.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub mark: bool,
}

// ---- uploading a show box's dormant collection (G15) ------------------------------------------

/// `POST /api/v1/codex/dormant/upload` (on a show box, admins): upload this box's own, dormant
/// collection to the site's Codex as a new collection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StartDormantUpload {
    /// The new collection's name (default: the dormant collection's; a name in use gets
    /// " (from <box>)").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
}

/// How an upload of the dormant collection stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum DormantUploadState {
    /// Not started (the offer).
    #[default]
    Idle,
    /// Documents, then media, are being copied.
    Copying,
    /// Everything is in the site's Codex.
    Done,
    /// Stopped with `error` (start it again to resume).
    Failed,
    /// Cancelled (start it again to resume).
    Cancelled,
}

/// Done of total (documents, media files, bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TransferCount {
    /// Done.
    pub done: u64,
    /// In all.
    pub total: u64,
}

/// `GET /api/v1/codex/dormant/upload` (on a show box with a dormant collection): the offer and
/// the progress of uploading it to the site's Codex. `codexChanged {status}` when it starts and
/// ends; poll while `copying`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DormantUpload {
    /// Where it stands.
    pub state: DormantUploadState,
    /// The dormant collection (this box's own).
    pub source: CollectionId,
    /// Its name.
    pub name: String,
    /// Its songs (presentations), for the offer.
    pub songs: u32,
    /// The collection made from it in the site's Codex, once known (uploading again resumes
    /// into it: never a second one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub collection: Option<CollectionId>,
    /// That collection's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub collection_name: Option<String>,
    /// Documents sent.
    pub documents: TransferCount,
    /// Media files sent (or found in the site's media library already).
    pub media: TransferCount,
    /// Media bytes sent.
    pub bytes: TransferCount,
    /// Why it stopped (`failed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// When it started (RFC 3339 UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub started_at: Option<String>,
    /// When it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub finished_at: Option<String>,
}
