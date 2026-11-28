//! REST bodies (`/api/v1/...`).
//!
//! Full model documents (`Presentation`, `Look`, `Screen`, `ClearGroup`, `MediaAsset`) are
//! served as they are stored; the types here are the lightweight summaries and the bodies that
//! are not documents. Errors are [`ErrorResponse`](crate::ErrorResponse) with the HTTP status
//! of their [`ErrorCode`](crate::ErrorCode).

use crate::state::ShowState;
use model::{
    BinBrowser, ContentDestination, LibraryId, MediaAsset, MediaAssetId, MediaBinId,
    MediaBinItemId, MediaKind, Message, OutputBinding, PlaylistId, PlaylistItemId,
    PlaylistItemKind, PlaylistPurpose, PresentationId, PresentationKind, Prop, Screen, Size,
    SlideId, StageLayout, TemplateSlideId, Theme, ThemeId, Timer,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `GET /api/v1/server`: who the server is and what it can do. Needs no token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    /// Stable server ID (UUID).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Server software version.
    pub version: String,
    /// Protocol major version.
    pub protocol_version: u32,
    /// Feature flags (`ws`, `commands`, `thumbnails`, `preview`, `pairing`, `mdns`,
    /// `mediaFiles`, `reload`, …, `site`, `codex`; the protocol README lists them). Clients
    /// hide what is missing and ignore flags they don't know.
    pub capabilities: Vec<String>,
    /// How this request was authenticated.
    pub auth: AuthStatus,
    /// The current pairing code: only for requests from the server machine itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pairing_code: Option<String>,
    /// `true` when this request came with a credential from before site API keys: swap it
    /// once with `POST /api/v1/pair/upgrade` (no one pairs again). Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub upgrade_credential: Option<bool>,
}

/// How a request was authenticated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum AuthStatus {
    /// From the server machine itself (trusted without a token).
    Local,
    /// With a valid token.
    Paired,
    /// No (valid) token: only `GET /api/v1/server` and `POST /api/v1/pair` work.
    Unpaired,
}

/// `POST /api/v1/pair` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    /// The 6-digit code shown by the server. Without it, the pairing waits for a site admin
    /// to approve it (`202 PairPending`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub code: Option<String>,
    /// A name for this client ("Booth laptop"), shown in the client list.
    pub client_name: String,
    /// The client is a web browser (not the desktop app): its key lives a year, not ten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub browser: Option<bool>,
    /// A Vesper Launcher joining the site: its machine ID (`LauncherReport.machineId`). The
    /// key made is that machine's **Launcher key** (audience `app:vesper-launcher`), not a
    /// client's: it only reports as that machine and picks up its actions on the update
    /// channel (`PUT /api/v1/updates/launchers/{machineId}`) and downloads packages; every
    /// other route refuses it. Paired at the site's server only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub launcher: Option<String>,
}

/// `202` answer to `POST /api/v1/pair` (and `GET /api/v1/pair/{pairingId}` while it waits):
/// the pairing waits for a site admin's approval. Ask `GET /api/v1/pair/{pairingId}` every
/// second or so: `200 PairResponse` once approved, `403 forbidden` when refused (the message
/// says why), `404 notFound` when it ended unanswered (10 minutes) or was already answered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PairPending {
    /// The pairing's secret id: only the client asking knows it.
    pub pairing_id: String,
    /// Seconds until it ends unanswered.
    pub expires_in_seconds: u64,
    /// Paired at a relay (another server of the site) with a code the site's server found
    /// wrong or expired: the pairing still waits for an admin's approval, and the client may
    /// pair again with the right code. (At the site's server itself a wrong code is `401`.)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub code_wrong: bool,
}

/// `POST /api/v1/pair/upgrade` body (optional).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpgradeCredential {
    /// The client is a web browser: its key lives a year, not ten.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub browser: Option<bool>,
}

/// `POST /api/v1/pair` (and `POST /api/v1/pair/upgrade`) response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PairResponse {
    /// The bearer token: a named site API key (`idn1.…`, a few hundred characters), signed by
    /// the site and valid on every Vesper server of the site. Store it; it is shown once.
    pub token: String,
    /// The key's id (`DELETE /api/v1/site/api-keys/{id}` revokes it).
    pub client_id: String,
}

/// `GET /api/v1/pairing/code` (server machine only).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    /// 6 digits.
    pub code: String,
    /// Seconds until it rotates (it also rotates after a successful pairing).
    pub expires_in_seconds: u64,
}

/// A client still on a pairing credential from before site API keys (`GET /api/v1/clients`,
/// server machine only). It swaps it for a key the next time it connects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    /// Client ID.
    pub id: String,
    /// Name given when pairing.
    pub name: String,
    /// When it paired (RFC 3339).
    pub created_at: String,
    /// Last authenticated request (RFC 3339, minute precision).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_seen: Option<String>,
}

/// `GET /api/v1/state`: the current state, for scripts that do not keep a WebSocket open.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StateSnapshot {
    /// Revision.
    pub revision: u64,
    /// State.
    pub state: ShowState,
}

/// Body of `POST /api/v1/commands` responses (HTTP 200 when `ok`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CommandResponse {
    /// Whether the command was applied.
    pub ok: bool,
    /// Revision of the state that includes the command's effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub revision: Option<u64>,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<crate::ApiError>,
}

/// `GET /api/v1/libraries`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySummary {
    /// Library.
    pub id: LibraryId,
    /// Name.
    pub name: String,
    /// Sidebar order key (compare as strings).
    pub sort_key: String,
    /// Presentations in it.
    pub presentation_count: u32,
}

/// `GET /api/v1/libraries/{id}/presentations` and `GET /api/v1/presentations`: one row of a
/// library list. Sorted by name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PresentationSummary {
    /// Presentation.
    pub id: PresentationId,
    /// Name.
    pub name: String,
    /// Library (`null` = none).
    pub library: Option<LibraryId>,
    /// Free-form category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub category: Option<String>,
    /// Slides (all sections, enabled or not).
    pub slide_count: u32,
    /// Slide or announcements layer.
    pub destination: ContentDestination,
    /// Last modification (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub modified_at: Option<String>,
    /// Song, scripture, text or media (read off the document, see `Presentation::kind`).
    #[serde(default)]
    pub kind: PresentationKind,
    /// Author / writer credit, verbatim (song metadata; omitted when unknown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub authors: Option<String>,
    /// Copyright year (omitted when unknown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub copyright_year: Option<u32>,
    /// CCLI song number, verbatim (omitted when unknown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ccli_number: Option<String>,
    /// The theme its slides follow, else the one they were made from (`templateOrigin`;
    /// omitted when none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub theme: Option<ThemeId>,
    /// That theme's name (the theme's own, else the name the link kept; omitted when none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub theme_name: Option<String>,
}

/// `GET /api/v1/playlists`: the playlist tree (folders and playlists, no items).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistTreeNode {
    /// Node.
    pub id: PlaylistId,
    /// Name.
    pub name: String,
    /// Folder or playlist.
    pub kind: PlaylistNodeType,
    /// What a playlist is for (`null` for folders).
    pub purpose: Option<PlaylistPurpose>,
    /// Items (playlists only).
    pub item_count: u32,
    /// Child nodes in order (folders only).
    pub children: Vec<PlaylistTreeNode>,
}

/// Folder or playlist.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PlaylistNodeType {
    /// Folder.
    Folder,
    /// Playlist.
    Playlist,
}

/// `GET /api/v1/playlists/{id}`: a playlist with its items resolved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistDetail {
    /// Playlist.
    pub id: PlaylistId,
    /// Name.
    pub name: String,
    /// Parent folder.
    pub parent: Option<PlaylistId>,
    /// Service, media or audio.
    pub purpose: PlaylistPurpose,
    /// Items in order.
    pub items: Vec<PlaylistItemDetail>,
}

/// A playlist item with what it refers to resolved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistItemDetail {
    /// Item.
    pub id: PlaylistItemId,
    /// What to show in the list: the item's own name, else the presentation / media / header
    /// name.
    pub display_name: String,
    /// Disabled items are skipped by `next` / `prev`.
    pub enabled: bool,
    /// The item as stored.
    pub kind: PlaylistItemKind,
    /// For presentation items: the presentation (`null` = not in the library).
    pub presentation: Option<PresentationSummary>,
}

/// `GET /api/v1/themes`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThemeSummary {
    /// Theme.
    pub id: ThemeId,
    /// Name.
    pub name: String,
    /// Canvas size of its template slides.
    pub canvas_size: Size,
    /// Template slides.
    pub slides: Vec<TemplateSlideSummary>,
    /// Presentations that follow it (linked themes).
    #[serde(default)]
    pub used_by: u32,
}

/// `GET /api/v1/themes/{id}`: the theme and its revision (also the `ETag`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThemeEntry {
    /// The theme.
    pub theme: Theme,
    /// Revision (`If-Match` for `PUT` / `DELETE`).
    pub revision: String,
}

/// A template slide of a theme.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSlideSummary {
    /// Template slide.
    pub id: TemplateSlideId,
    /// Name.
    pub name: String,
    /// Its slide's ID (in the theme's document: `/themes/{id}/slides/{slide}/thumbnail`).
    pub slide: SlideId,
}

/// `GET /api/v1/media/{id}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaAssetInfo {
    /// The asset document.
    pub asset: MediaAsset,
    /// The file is on this server (`GET /api/v1/media/{id}/file` serves it).
    pub online: bool,
    /// Why it is offline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
}

/// One document that uses a media asset or a prop (`GET /api/v1/media/{id}/used-by`,
/// `GET /api/v1/props/{id}/used-by`): the Library's "Used by".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentUse {
    /// Its kind: `presentation`, `playlistNode`, `prop`, `macro`, `theme`, `mediaBin`,
    /// `stageLayout`, `message`, …
    pub kind: model::DocumentKind,
    /// Its ID.
    pub id: String,
    /// Its name.
    pub name: String,
}

/// `GET /api/v1/media-bins`: one node of the media bin tree. Folders come from the bins'
/// folder paths (a folder exists while a bin is in it); bins are the leaves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaBinNode {
    /// The bin (`null` for folders).
    pub id: Option<MediaBinId>,
    /// Bin or folder name.
    pub name: String,
    /// Folder path of this node, outermost first: for a folder its own path, for a bin the
    /// folder holding it (`[]` = top level).
    pub path: Vec<String>,
    /// Folder, manual bin or watched-folder bin.
    pub kind: MediaBinNodeType,
    /// Media or audio browser (bins; folders say `media`).
    pub browser: BinBrowser,
    /// Items in the bin (folders: in every bin below it).
    pub item_count: u32,
    /// Sub-folders first, then bins, each in order (folders only).
    pub children: Vec<MediaBinNode>,
}

/// What a media bin tree node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MediaBinNodeType {
    /// A folder of bins.
    Folder,
    /// A hand-curated bin.
    Manual,
    /// A watched folder (the items of its last scan).
    Watched,
}

/// `GET /api/v1/media-bins/{id}`: a bin with its items resolved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaBinDetail {
    /// The bin.
    pub id: MediaBinId,
    /// Name.
    pub name: String,
    /// Folder path, outermost first.
    pub path: Vec<String>,
    /// Manual or watched.
    pub kind: MediaBinNodeType,
    /// Media or audio browser.
    pub browser: BinBrowser,
    /// Items in display order.
    pub items: Vec<MediaBinItemInfo>,
}

/// One item of a media bin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaBinItemInfo {
    /// Item.
    pub id: MediaBinItemId,
    /// Display name (the item's own name, else the asset's).
    pub name: String,
    /// Image, video or audio.
    pub kind: MediaKind,
    /// The asset (`triggerMedia`, `GET /api/v1/media/{asset}/thumbnail`).
    pub asset: MediaAssetId,
    /// Whether the file is on this server.
    pub availability: MediaAvailability,
    /// Disabled items stay in the bin but should not be triggered.
    pub enabled: bool,
    /// Length of a video or audio file, seconds (`null` = still image or unknown).
    pub duration: Option<f64>,
}

/// Whether a media file can be found on the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MediaAvailability {
    /// The file is there.
    Online,
    /// The file cannot be found (not imported, drive not bound, deleted).
    Missing,
    /// On a show box: the file is in the site's Codex but not on this box yet (being copied,
    /// or copied when triggered). Until it is here it behaves like missing media.
    Copying,
}

/// `POST /api/v1/presentations/{id}/thumbnails?w=320` response: rendering continues in the
/// background.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThumbnailPrefetch {
    /// Slides in the presentation.
    pub slides: u32,
    /// Already cached (memory or disk).
    pub cached: u32,
    /// Queued for rendering.
    pub queued: u32,
}

/// `GET /api/v1/diagnostics`: render and load issues since start.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsReport {
    /// GPU adapter in use.
    pub adapter: Option<String>,
    /// Seconds since start.
    pub uptime_seconds: u64,
    /// Fonts that did not resolve exactly, most used first.
    pub fonts: Vec<FontIssueInfo>,
    /// Media that could not be drawn.
    pub missing_media: Vec<IssueCount>,
    /// Unsupported features met while rendering.
    pub unsupported: Vec<IssueCount>,
    /// Other render notes.
    pub notes: Vec<IssueCount>,
    /// Documents that failed to load.
    pub load_issues: Vec<LoadIssueInfo>,
}

/// A font that did not resolve exactly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FontIssueInfo {
    /// Requested family.
    pub family: String,
    /// Requested weight.
    pub weight: u32,
    /// Requested italic.
    pub italic: bool,
    /// Family used.
    pub used_family: String,
    /// `substituted` (machine table), `fallback` (not installed), `family` / `postScript`
    /// (matched, with synthetic styling).
    #[serde(rename = "match")]
    pub matched: String,
    /// Bold was synthesized.
    pub synthetic_bold: bool,
    /// Italic was synthesized.
    pub synthetic_italic: bool,
    /// Text runs rendered this way.
    pub uses: u64,
}

/// A counted issue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IssueCount {
    /// What.
    pub what: String,
    /// Detail.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Occurrences.
    pub count: u64,
}

/// A document that failed to load.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LoadIssueInfo {
    /// File (relative to the data directory).
    pub file: String,
    /// What went wrong.
    pub error: String,
    /// From a newer schema (refused, never overwritten).
    pub too_new: bool,
}

// ---- screen configuration ------------------------------------------------------------------

/// A screen document with its revision: `GET /api/v1/screens` (array), `GET`, `POST`, `PUT
/// /api/v1/screens/{id}`. On the wire it is the stored [`Screen`] plus a `revision` field, so
/// clients that only read screens see the same document as before.
///
/// Edits are optimistic: send the `revision` you read as `If-Match: "<revision>"` with `PUT`
/// or `DELETE`; if the document changed since, the server answers `409 conflict` with the
/// current entry in [`ErrorResponse::current`](crate::ErrorResponse::current).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScreenEntry {
    /// Opaque content revision (also sent as the `ETag` header of single-document responses).
    pub revision: String,
    /// The document.
    #[serde(flatten)]
    pub screen: Screen,
}

/// An output binding with its revision: `GET /api/v1/outputs` (array), `GET`, `POST`, `PUT
/// /api/v1/outputs/{id}`. Same rules as [`ScreenEntry`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OutputEntry {
    /// Opaque content revision.
    pub revision: String,
    /// The document.
    #[serde(flatten)]
    pub output: OutputBinding,
}

// ---- timers, messages, props ---------------------------------------------------------------

/// A timer document with its revision: `GET /api/v1/timers` (array), `GET`, `POST`, `PUT
/// /api/v1/timers/{id}`. Same rules as [`ScreenEntry`] (`If-Match`, `409` with `current`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TimerEntry {
    /// Opaque content revision.
    pub revision: String,
    /// The document.
    #[serde(flatten)]
    pub timer: Timer,
}

/// A message document with its revision (`/api/v1/messages`, as [`TimerEntry`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MessageEntry {
    /// Opaque content revision.
    pub revision: String,
    /// The document.
    #[serde(flatten)]
    pub message: Message,
}

/// A prop document with its revision (`/api/v1/props`, as [`TimerEntry`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PropEntry {
    /// Opaque content revision.
    pub revision: String,
    /// The document.
    #[serde(flatten)]
    pub prop: Prop,
}

/// A stage layout document with its revision (`/api/v1/stage-layouts`, as [`TimerEntry`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StageLayoutEntry {
    /// Opaque content revision.
    pub revision: String,
    /// The document.
    #[serde(flatten)]
    pub layout: StageLayout,
}

/// A built-in stage layout template (`GET /api/v1/stage-layout-templates`). `POST
/// /api/v1/stage-layouts` with `template: "<id>"` starts a new layout from it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StageLayoutTemplate {
    /// `currentNext`, `lyricsClockTimer`, `notes` or `musician` (chord charts).
    pub id: String,
    /// Display name ("Current / Next").
    pub name: String,
}

/// Where a new prop's content comes from (`POST /api/v1/props` field `fromSlide`): a copy of a
/// presentation slide's elements (fresh IDs; no actions), on that presentation's canvas size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SlideRef {
    /// The presentation.
    pub presentation: PresentationId,
    /// The slide.
    pub slide: SlideId,
}

/// Body of `DELETE /api/v1/screens/{id}` (200): what went with the screen.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScreenDeleted {
    /// Output bindings deleted with it.
    pub outputs: Vec<model::OutputId>,
    /// Looks that listed the screen (its entry was removed).
    pub looks: Vec<model::LookId>,
}

/// One kind of output transport and whether this server can run it
/// (`GET /api/v1/output-transports`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OutputTransportInfo {
    /// The `transport.type` of an output binding: `idn`, `omt`, `display`, `sdi`.
    pub kind: String,
    /// Display name ("IDN Open Protocol (compatible with NDI®)").
    pub label: String,
    /// Outputs of this kind can be created (`false`: shown, but not selectable).
    pub supported: bool,
    /// Outputs of this kind run on this server right now.
    pub available: bool,
    /// Runtime / library version when known ("6.3.2.0"; the full string is in `detail`).
    pub version: Option<String>,
    /// One short line for the UI ("built in (IDN Open Protocol <version>)", "not yet supported").
    pub status: String,
    /// The full explanation (where to install a missing runtime, …) for a tooltip.
    pub detail: Option<String>,
}

/// `GET /api/v1/fonts`: the font families the server can render (system fonts plus
/// `<data>/fonts`), and the machine's substitution table. Font pickers offer these families;
/// anything else renders through a substitution or the fallback family.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FontList {
    /// Installed families, sorted by name (case-insensitive).
    pub families: Vec<FontFamilyInfo>,
    /// Substitution rules in the order they are tried.
    pub substitutions: Vec<FontSubstitutionInfo>,
    /// Family used when neither the font nor a substitution is installed.
    pub fallback_family: String,
}

/// One installed font family.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FontFamilyInfo {
    /// Family name ("Carlito").
    pub family: String,
    /// Its faces, by weight then upright before italic.
    pub faces: Vec<FontFaceInfo>,
}

/// One installed face of a family.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FontFaceInfo {
    /// CSS weight (100–900).
    pub weight: u16,
    /// Italic or oblique.
    pub italic: bool,
    /// PostScript name ("Carlito-BoldItalic"), when the face has one.
    pub postscript_name: Option<String>,
}

/// One rule of the machine's font substitution table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FontSubstitutionInfo {
    /// Requested family the rule matches ("Calibri").
    pub family: Option<String>,
    /// Requested PostScript name the rule matches (checked before `family`).
    pub postscript_name: Option<String>,
    /// Family rendered instead ("Carlito").
    pub replacement_family: String,
    /// The replacement family is installed on this server.
    pub installed: bool,
}

/// `GET /api/v1/audio-devices`: an audio output the server can play on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceInfo {
    /// The name to store in the audio device configuration (`hw:1,0`, `default`).
    pub name: String,
    /// What it is (`HDA Intel PCH: ALC892 Analog`).
    pub description: String,
    /// The device can be opened right now (`false`: in use, or no permission; see `problem`).
    pub available: bool,
    /// Why it cannot be opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
    /// Output channels (when known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub channels: Option<u32>,
}

// ---- looks and masks -------------------------------------------------------------------------

/// A look with its revision: `GET`, `POST`, `PUT /api/v1/looks/{id}`, `POST …/duplicate`. Same
/// rules as [`ScreenEntry`] (`If-Match`, `409` with `current`). `GET /api/v1/looks` stays the
/// plain `Look[]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LookEntry {
    /// Opaque content revision.
    pub revision: String,
    /// The document.
    #[serde(flatten)]
    pub look: model::Look,
}

/// `POST /api/v1/looks/{id}/duplicate`, `/masks/{id}/duplicate`: a copy with a new ID, right
/// after the original.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateLook {
    /// Name of the copy (default "<name> copy").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
}

/// A mask with its revision: `GET /api/v1/masks` (array), `GET`, `POST`, `PUT
/// /api/v1/masks/{id}`, `POST …/duplicate`. Same rules as [`ScreenEntry`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MaskEntry {
    /// Opaque content revision.
    pub revision: String,
    /// The document.
    #[serde(flatten)]
    pub mask: model::Mask,
}

/// A logical video input ("Camera 1") with this box's binding and a revision: `GET
/// /api/v1/video-inputs` (array), `GET`, `POST`, `PUT /api/v1/video-inputs/{id}`. The body of
/// `POST` (partial) and `PUT` (whole) is the same shape without `revision`; `source` absent or
/// `null` = not bound on this box. `PUT` and `DELETE` take `If-Match`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct VideoInputEntry {
    /// Opaque content revision (of the input and its binding).
    pub revision: String,
    /// The portable document.
    #[serde(flatten)]
    pub input: model::VideoInput,
    /// Where it comes from on this box (its `InputBinding`); `None`: not bound here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub source: Option<model::InputSource>,
}

/// Body of `DELETE /api/v1/masks/{id}` (200): the looks it was removed from (`?detach=true`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MaskDeleted {
    /// Looks that used the mask (their entries no longer name it).
    pub looks: Vec<model::LookId>,
    /// Stage screens that used the mask (their `mask` is cleared).
    #[serde(default)]
    pub screens: Vec<model::ScreenId>,
}

/// `GET /api/v1/server/details` (G9): this Vesper's own numbers, for Site ▸ Devices and the
/// status bar. Other machines' details come from the site's device list (`SiteDevice`: IDN's
/// build and services, and `machine`: what their Nodes report).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServerDetails {
    /// The server's name.
    pub name: String,
    /// Vesper's version.
    pub version: String,
    /// The operating system (`Ubuntu 24.04.1 LTS`, `Windows`).
    pub os: String,
    /// The CPU's model, when the system says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cpu: Option<String>,
    /// Logical CPUs.
    pub cpus: u32,
    /// The GPU adapter rendering (`None` until the renderer runs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gpu: Option<String>,
    /// Seconds since the server started.
    pub uptime_seconds: u64,
    /// The products this box runs (`Vesper`, `Codex`, `Site Manager`).
    pub products: Vec<String>,
}

/// `GET /api/v1/server/stats`: the server's own health numbers since it started, for soak runs
/// and diagnostics (frame pacing per screen, decoder counters per live video, the process's
/// memory, threads and open files). Counters only go up; a client compares two readings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ServerStats {
    /// Seconds since the server started.
    pub uptime_seconds: f64,
    /// The process (Linux: from `/proc/self`; other systems: fields absent).
    pub process: ProcessStats,
    /// WebSocket clients connected now.
    pub ws_clients: u32,
    /// Every running screen.
    pub screens: Vec<ScreenPacing>,
    /// Every live video player (background, foreground, video elements).
    pub videos: Vec<VideoStats>,
    /// Buffer underruns of the audio output since the server started.
    pub audio_xruns: u64,
    /// What the server's caches hold now (the renderer's, published every few seconds while
    /// it works; the thumbnails'), to tell a growing cache from a leak.
    #[serde(default)]
    pub caches: Vec<CacheSize>,
}

/// One cache's size now (see [`ServerStats`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CacheSize {
    /// What it caches (`decoded images (CPU)`, `live slide layers (GPU)`, …).
    pub name: String,
    /// Entries.
    pub entries: u64,
    /// Bytes held (0: not counted).
    pub bytes: u64,
}

/// The server process's resources (see [`ServerStats`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProcessStats {
    /// Resident memory, bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rss_bytes: Option<u64>,
    /// Threads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub threads: Option<u32>,
    /// Open file descriptors (files, sockets, pipes, GPU handles).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub open_files: Option<u32>,
    /// Heap bytes allocated and in use (Linux with glibc: `mallinfo2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub heap_in_use_bytes: Option<u64>,
    /// Heap bytes the allocator holds free (Linux with glibc): resident memory beyond what is
    /// in use, kept for reuse or fragmented.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub heap_free_bytes: Option<u64>,
    /// Memory the server's Rust allocator (mimalloc) has committed, bytes: what Rust code holds
    /// in use and what mimalloc keeps for reuse (not all of it resident). Present when the
    /// server runs on mimalloc (the default build); the two heap fields above then count only
    /// the C libraries' memory (FFmpeg, whisper, GPU drivers), which still uses glibc's malloc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rust_heap_bytes: Option<u64>,
}

/// One screen's frame pacing since it started (see [`ServerStats`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScreenPacing {
    /// The screen.
    pub id: model::ScreenId,
    /// Its name.
    pub name: String,
    /// Clock ticks (frames offered to the outputs).
    pub ticks: u64,
    /// Ticks the clock skipped because it was late (frames never sent).
    pub late_ticks: u64,
    /// Tick renders (videos, transitions) that finished after their frame was due (the
    /// previous frame was sent again).
    pub late_renders: u64,
    /// Renders.
    pub renders: u64,
    /// Slowest render, ms.
    pub max_render_ms: f64,
    /// 95th percentile of the recent renders, ms (`null` before the first).
    pub p95_render_ms: Option<f64>,
    /// Frames its outputs dropped because they were busy (sum over the outputs).
    pub frames_dropped: u64,
}

/// One live video player's decoder counters (see [`ServerStats`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct VideoStats {
    /// The video's name.
    pub name: String,
    /// Distinct frames shown.
    pub shown: u64,
    /// Frames passed over (by design when the file's rate is above the screen's).
    pub skipped: u64,
    /// Times the decoder had not reached the wanted frame yet (the previous one was shown).
    pub late: u64,
    /// 1 once decoding failed.
    pub errors: u64,
}

/// `POST /api/v1/chordpro/parse`: a ChordPro song read into a new (unsaved) presentation.
/// Create it with `POST /api/v1/presentations {document, library}`; put its chords on an
/// existing song with the `applyChordPro` edit instead.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChordProParse {
    /// The ChordPro text (`.cho`, `.chordpro`, `.pro`, or pasted).
    pub text: String,
    /// The presentation's name (default: `{title}`, else "Untitled").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Lines per slide (default 0: one slide per stanza; stanzas never share a slide).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub lines_per_slide: Option<u32>,
    /// Style the slides' text like this presentation's (its canvas too); else the default text
    /// element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub like: Option<model::PresentationId>,
    /// A two-language file: how its second language is written (`model::chordpro` docs:
    /// alternating lines, a `---` line, a separator inside each line, `{ci: …}` comments, or
    /// `auto`: what the file's `{meta: second_language_split …}` says, else a `---` line).
    /// The second language becomes each slide's second text (`Slide.translation`, language from
    /// `{meta: second_language …}`); chords stay on the first language. Default: one language.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub split: Option<model::chordpro::SecondSplit>,
}

/// `POST /api/v1/chordpro/parse`'s answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ChordProParsed {
    /// The new presentation (fresh IDs, no library).
    pub presentation: model::Presentation,
    /// Sections and slides made.
    pub sections: u32,
    /// See `sections`.
    pub slides: u32,
    /// Lines with chords.
    pub chord_lines: u32,
    /// Slides with a second-language text (a two-language file read with `split`).
    #[serde(default)]
    pub second_language_slides: u32,
    /// Stanzas split into several slides (`lines_per_slide`) whose second-language lines don't
    /// pair line by line with their lyric lines: all of them stay on the stanza's first slide
    /// (check those slides).
    #[serde(default)]
    pub unpaired_stanzas: u32,
    /// `{capo}` of the file (a guitarist's capo: set it on a stage layout's chord view).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub capo: Option<u8>,
}

/// `POST /api/v1/songselect/parse`: a song a church's subscriber downloaded from CCLI
/// SongSelect (lyrics `.txt`, SongShow Plus `.usr` / `.bin`, ChordPro), or SongSelect lyrics
/// pasted as text, read into a new (unsaved) presentation with its song metadata
/// (`model::songselect`). Vesper never contacts CCLI. Create it with
/// `POST /api/v1/presentations {document, library}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SongSelectParse {
    /// The file's bytes, base64 (UTF-8 with or without a byte-order mark, UTF-16 or
    /// Windows-1252; at most 2 MiB). Give this or `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub data: Option<String>,
    /// Pasted text (SongSelect lyrics, or a whole file's text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<String>,
    /// The file's name: a hint for its format (`.usr` / `.bin`, `.cho` / `.chordpro` /
    /// `.pro`); the content decides first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub file_name: Option<String>,
    /// The presentation's name (default: the song's title, else "Untitled").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Lines per slide (default 0: one slide per stanza; stanzas never share a slide).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub lines_per_slide: Option<u32>,
    /// Style the slides' text like this presentation's (its canvas too); else the default text
    /// element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub like: Option<model::PresentationId>,
}

/// The format a SongSelect file was read as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SongSelectFormat {
    /// Lyrics text (`.txt` or pasted).
    Lyrics,
    /// SongShow Plus (`.usr`, `.bin`).
    Usr,
    /// ChordPro (SongSelect Premium).
    ChordPro,
}

/// A presentation already in the library with the same CCLI song number.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SongSelectDuplicate {
    /// The presentation.
    pub id: model::PresentationId,
    /// Its name.
    pub name: String,
}

/// `POST /api/v1/songselect/parse`'s answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SongSelectParsed {
    /// How the file was read.
    pub format: SongSelectFormat,
    /// The new presentation (fresh IDs, no library): sections linked to the global groups of
    /// the same name, `songInfo` with the title, authors, CCLI number, copyright (year and
    /// owners as `publisher`) and key; `songInfo.display` on when it has a CCLI number, so the
    /// copyright line (Settings ▸ CCLI) shows under the words.
    pub presentation: model::Presentation,
    /// Sections and slides made.
    pub sections: u32,
    /// See `sections`.
    pub slides: u32,
    /// Lines with chords (ChordPro).
    #[serde(default)]
    pub chord_lines: u32,
    /// The church's CCLI licence number the file carries (`CCLI License # …`); the copyright
    /// line uses the one in the settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub license_number: Option<String>,
    /// Presentations with the same CCLI song number (skip, replace or keep both).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<SongSelectDuplicate>>", optional)]
    pub duplicates: Vec<SongSelectDuplicate>,
}
