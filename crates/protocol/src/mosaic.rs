//! Vesper Mosaic (`docs/MOSAIC-PLAN.md`): the site's display boxes, their screens and the
//! site-wide channels, managed by the site's server (`/api/v1/mosaic/…`).
//!
//! A **display box** is a machine of the site whose IDN Node runs IDN's display service; Vesper
//! sets each of its **screens**' feed (a source, a channel, or nothing) and settings through that
//! service. A **channel** is a site-wide named feed pointing at one source: every screen following
//! it moves when its source changes. A **video wall** is a grid of screens (of one box or
//! several) showing one picture across them, in step.

use crate::site::DeviceHealth;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a screen shows (`PUT /api/v1/mosaic/boxes/{machine}/screens/{n}/feed`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MosaicFeed {
    /// Nothing: the screen goes back to the box's desktop.
    #[default]
    None,
    /// A network video source, as sources are listed (`HOST (name)`).
    Source {
        /// The source.
        source: String,
    },
    /// A site-wide channel (its id): the screen shows whatever the channel points at.
    Channel {
        /// The channel's id.
        channel: String,
    },
}

/// How the picture fills a screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MosaicFit {
    /// The whole picture, its aspect kept, black around.
    #[default]
    Fit,
    /// The whole screen, its aspect kept, the edges cut.
    Fill,
    /// The whole screen, stretched.
    Stretch,
}

/// What a screen shows when its feed goes (after `fallbackAfterSeconds` of the last picture).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MosaicFallback {
    /// The site's logo (pushed to the box's media folder and given to each screen; black
    /// without one).
    #[default]
    Logo,
    /// The last picture, for good.
    Last,
    /// Black.
    Black,
}

/// A screen's sound: the feed's sound on one of the box's sound cards.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicSound {
    /// The sound card (`default`: the system's), e.g. an HDMI projector's speakers.
    pub device: String,
    /// Volume, dB (−60 to 12).
    pub volume_db: f32,
    /// Muted.
    #[serde(default)]
    pub mute: bool,
}

/// A screen's tile of a video wall: its place in the wall's grid, the bezels and the screen's
/// visible size (IDN's `settings.wall`). Every tile of a wall has the same geometry and group,
/// and the wall shows one feed. A screen in a [`MosaicWall`] gets its tile from the wall.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicTile {
    /// The wall's rows (1 to 16).
    pub rows: u16,
    /// The wall's columns (1 to 16).
    pub columns: u16,
    /// Its row, from 0.
    pub row: u16,
    /// Its column, from 0.
    pub column: u16,
    /// The bezels between screens, horizontal and vertical, millimetres (0 to 10000).
    pub bezel_mm: [f32; 2],
    /// The screen's visible width and height, millimetres (needed with a bezel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub screen_mm: Option<[f32; 2]>,
    /// The wall's sync group (default: `wall:` and the feed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub group: Option<String>,
}

/// A screen's settings (`PUT …/screens/{n}/settings`, admins).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
pub struct MosaicScreenSettings {
    /// Fit, fill or stretch.
    pub fit: MosaicFit,
    /// Rotation: 0, 90, 180 or 270 degrees.
    pub rotation: u16,
    /// Overscan, percent (0 to 20).
    pub overscan: f32,
    /// The feed's sound on a sound card (none: no sound).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sound: Option<MosaicSound>,
    /// What to show when the feed goes.
    pub fallback: MosaicFallback,
    /// Seconds of the last picture before the fallback (default 5, 0 to 3600).
    pub fallback_after_seconds: u32,
    /// Its tile of a video wall (set by the wall it is in; see [`MosaicWall`]; ignored in a
    /// `PUT …/settings`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tile: Option<MosaicTile>,
}

impl Default for MosaicScreenSettings {
    fn default() -> Self {
        MosaicScreenSettings {
            fit: MosaicFit::Fit,
            rotation: 0,
            overscan: 0.0,
            sound: None,
            fallback: MosaicFallback::Logo,
            fallback_after_seconds: 5,
            tile: None,
        }
    }
}

/// A screen's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MosaicScreenStatus {
    /// Its feed's pictures arrive.
    Showing,
    /// Connecting to its feed.
    Connecting,
    /// Its feed went; the last picture holds (`fallbackAfterSeconds`).
    Lost,
    /// Its feed is gone: it shows its fallback.
    Fallback,
    /// No feed (the box's desktop).
    Idle,
    /// The box doesn't answer, or is offline in the site (as last known).
    Offline,
    /// The box refused the last change, or the screen has a problem (`problem`).
    Error,
}

/// One screen of a display box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicScreen {
    /// Its number on the box, from 1.
    pub number: u32,
    /// Its name, as the box's display service gives it (the desktop's output name, `DP-2`).
    pub name: String,
    /// Its size in pixels, when the box says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub width: Option<u32>,
    /// Its height in pixels, when the box says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub height: Option<u32>,
    /// What Vesper has it show (`none` also while Vesper never set it: the box keeps its own,
    /// `showing`).
    pub feed: MosaicFeed,
    /// The feed the box has now (empty: none), as it last said.
    pub showing: String,
    /// A test pattern is up on it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub test_pattern: bool,
    /// Its settings: Vesper's, or the box's own (as it last said) while Vesper never set them.
    pub settings: MosaicScreenSettings,
    /// Its state.
    pub status: MosaicScreenStatus,
    /// Why the last change failed (`error`), or why the box can't be reached (`offline`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
    /// The video wall it is a tile of (its id): the wall sets its feed and its `tile`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub wall: Option<String>,
    /// Its frame sync, as the box last said (a wall's tile that shows a picture).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sync: Option<MosaicSync>,
}

/// A screen's frame sync (IDN's `stats.sync`): every tile of a wall shows each picture at the
/// source's time plus a shared delay, within one refresh of each other.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicSync {
    /// The sync group it is in (a wall's: `wall:` and the wall's name).
    pub group: String,
    /// The service's sync mode, as it says.
    pub mode: String,
    /// The shared delay, milliseconds.
    pub delay_ms: f32,
    /// Pictures that came too late to show in step (dropped), as the box counts them.
    pub late: u64,
    /// Of those, the ones since the read before (a few seconds).
    pub late_recently: u64,
}

/// Where a display box's address came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MosaicAddressFrom {
    /// An admin set it (it overrides the others).
    Admin,
    /// The Site Manager sees the box's IDN Node at it (its trust connection).
    Site,
    /// The box asked to join the site from it.
    Join,
}

/// A display box: a machine of the site whose IDN Node runs the display service.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicBox {
    /// The machine's fingerprint (its id in the site).
    pub machine: String,
    /// Its name in the site.
    pub name: String,
    /// Its health in the site.
    pub health: DeviceHealth,
    /// Its display service's state, as its Node reports it (`running`, `no_session`, …; empty:
    /// not reported: an admin added the box).
    pub service: String,
    /// Where Vesper reaches its display service: an address (`192.0.2.40`, its port 29223) or a
    /// full URL; absent until known (one an admin set, else where the Site Manager sees its IDN
    /// Node, else the address it asked to join from).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub address: Option<String>,
    /// How its display service runs: as its IDN Node reports it (`serviceModeReported`), else
    /// as Vesper last switched it (`PUT …/boxes/{machine}/service`); absent: neither known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub service_mode: Option<MosaicServiceMode>,
    /// `serviceMode` is what the box's Node reports (not only what Vesper last set).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub service_mode_reported: bool,
    /// Its system is Linux, so the display service can run headless (no desktop).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub headless_ok: bool,
    /// The product that updates its IDN Node instead of the Site Manager (`Vesper`: the
    /// vesper-idn-node package's marker, IDN Beta14), as its Node reports it
    /// (`SiteDevice.updateManagedBy`); absent: the Site Manager's updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub update_managed_by: Option<String>,
    /// Where the site's logo (`MosaicState.logo`) is with this box (absent: no logo, or not
    /// sent yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub logo_state: Option<MosaicLogoState>,
    /// Where `address` came from (absent while there is none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub address_from: Option<MosaicAddressFrom>,
    /// An admin added it (its Node doesn't report the display service).
    pub added: bool,
    /// The machine runs a Vesper server: its screens are driven by Vesper's own display
    /// outputs (Configure Screens), never by Mosaic, which doesn't assign them (`409`). Listed
    /// only when its display service is on too (`conflict`).
    pub vesper: bool,
    /// Two programs would drive the same screens: a Vesper server and the display service
    /// both run on it. Switch the Node's display service off there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub conflict: Option<String>,
    /// Its display service answered the last time Vesper asked.
    pub reachable: bool,
    /// Why it can't be reached or used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
    /// Its screens (as last known).
    pub screens: Vec<MosaicScreen>,
}

/// A site-wide channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicChannel {
    /// Its id.
    pub id: String,
    /// Its name ("Main", "Lobby").
    pub name: String,
    /// The source it points at (empty: none: its screens show their fallback).
    pub source: String,
    /// How many screens follow it.
    pub screens: u32,
    /// The IDN router carrying it on the site's server (`HOST (Channel Main)`): the screens
    /// following it show that, and a new source is one change there. Absent: no IDN Node on the
    /// site's server (or not set yet): each screen gets the channel's source itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub router: Option<String>,
}

/// A cell of a video wall with the screen in it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicWallCell {
    /// Its row, from 0 (the top).
    pub row: u16,
    /// Its column, from 0 (the left).
    pub column: u16,
    /// The display box (its fingerprint).
    pub machine: String,
    /// The box's screen, from 1.
    pub screen: u32,
}

/// A video wall: screens of one or several display boxes, side by side in a grid, showing one
/// picture across them (each screen its tile, with the bezels left out), in step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicWall {
    /// Its id.
    pub id: String,
    /// Its name ("Front wall"); its sync group is `wall:` and the name (at most 64 bytes).
    pub name: String,
    /// Rows (1 to 16).
    pub rows: u16,
    /// Columns (1 to 16).
    pub columns: u16,
    /// The bezels between screens, horizontal and vertical, millimetres (0 to 10000).
    pub bezel_mm: [f32; 2],
    /// A screen's visible width and height, millimetres (needed with a bezel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub screen_mm: Option<[f32; 2]>,
    /// What the wall shows (every tile the same).
    pub feed: MosaicFeed,
    /// The cells with a screen (a cell may stay empty).
    pub cells: Vec<MosaicWallCell>,
    /// The wall's state: its tiles' worst.
    pub status: MosaicScreenStatus,
    /// Every tile showing is in the wall's sync group, and none dropped late pictures lately.
    pub synced: bool,
    /// What is wrong (a tile's problem, tiles out of step), for people.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
    /// The largest sync delay of its tiles, milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub delay_ms: Option<f32>,
    /// Late pictures of all its tiles since the read before.
    pub late_recently: u64,
}

/// `POST /api/v1/mosaic/walls` and `PUT /api/v1/mosaic/walls/{id}` (admins): a wall, whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PutMosaicWall {
    /// Its name (1 to 48 characters, unique; `wall:` and the name at most 64 bytes).
    pub name: String,
    /// Rows (1 to 16).
    pub rows: u16,
    /// Columns (1 to 16).
    pub columns: u16,
    /// The bezels, horizontal and vertical, millimetres (default none).
    #[serde(default)]
    pub bezel_mm: [f32; 2],
    /// A screen's visible width and height, millimetres (needed with a bezel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub screen_mm: Option<[f32; 2]>,
    /// What the wall shows (default nothing).
    #[serde(default)]
    #[ts(optional, as = "Option<MosaicFeed>")]
    pub feed: MosaicFeed,
    /// The cells with a screen.
    #[serde(default)]
    #[ts(optional, as = "Option<Vec<MosaicWallCell>>")]
    pub cells: Vec<MosaicWallCell>,
}

/// `GET /api/v1/mosaic`: every display box with its screens, the channels and the walls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicState {
    /// This server manages the site's displays (it is the site's server).
    pub managed: bool,
    /// The display boxes, by name.
    pub boxes: Vec<MosaicBox>,
    /// The channels, by name.
    pub channels: Vec<MosaicChannel>,
    /// The video walls, by name (absent from a server before walls).
    #[serde(default)]
    #[ts(optional, as = "Option<Vec<MosaicWall>>")]
    pub walls: Vec<MosaicWall>,
    /// Why the channels aren't on routers on the site's server's IDN Node (they are resolved
    /// per screen meanwhile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub routing_problem: Option<String>,
    /// The site's logo the boxes show while their feed is gone (`PUT /mosaic/logo`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub logo: Option<MosaicLogo>,
    /// Why the logo can't be pushed to the boxes now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub logo_problem: Option<String>,
    /// The site's other machines with an IDN Node that could be display boxes ("Use as a
    /// display box"): neither boxes yet nor Vesper machines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<Vec<MosaicMachine>>")]
    pub machines: Vec<MosaicMachine>,
}

/// The site's logo for the display boxes' fallback (kept as PNG on the site's server, pushed to
/// each box's media folder with its IDN Node: `logo-a.png` / `logo-b.png` in turns).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicLogo {
    /// Its content's hash (changes with it: a cache key for `GET /mosaic/logo`).
    pub hash: String,
    /// Width, pixels.
    pub width: u32,
    /// Height, pixels.
    pub height: u32,
}

/// The site's logo on a display box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MosaicLogoState {
    /// Sent to its IDN Node (under a name of its own); its screens are given it once it has
    /// had time to arrive.
    Sending,
    /// Every screen it lists has it as its fallback logo.
    OnBox,
    /// Its Node didn't take it, or a screen refused it (`MosaicState.logoProblem`).
    Failed,
}

/// How a box's IDN display service runs (its Node's `apps.display` / `apps.display_boot`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MosaicServiceMode {
    /// Off (the box gives its screens back to its desktop).
    Off,
    /// In the desktop session (someone logged on, or auto-logon).
    Desktop,
    /// From boot with no desktop (Linux; recommended for a dedicated box).
    Headless,
}

/// `PUT /api/v1/mosaic/boxes/{machine}/service` (admins): "Use as a display box".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PutMosaicService {
    /// How the display service runs from now on.
    pub mode: MosaicServiceMode,
}

/// A machine of the site with an IDN Node that isn't a display box (yet).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicMachine {
    /// Its fingerprint.
    pub machine: String,
    /// Its name in the site.
    pub name: String,
    /// Its health in the site.
    pub health: DeviceHealth,
    /// Its system is Linux (headless possible).
    pub headless_ok: bool,
}

/// `PUT /api/v1/mosaic/boxes/{machine}` (admins): add a machine of the site as a display box,
/// or set where its display service is reached.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PutMosaicBox {
    /// An address (`192.0.2.40`, `host:port`) or a full URL; absent or empty: the address it
    /// asked to join from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub address: Option<String>,
}

/// `POST /api/v1/mosaic/channels` (admins) and `PUT /api/v1/mosaic/channels/{id}` (a new name:
/// admins; only the source: operators).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PutMosaicChannel {
    /// Its name (needed to create one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// The source it points at (empty: none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub source: Option<String>,
}

/// `POST /api/v1/mosaic/boxes/{machine}/screens/{n}/identify` and `POST /api/v1/mosaic/identify`
/// (every screen of every box): the screens show their number and name (optional body).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicIdentify {
    /// How long, seconds (0.5 to 600; default 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub seconds: Option<f32>,
}

/// `POST /api/v1/mosaic/boxes/{machine}/screens/{n}/test-pattern` and
/// `POST /api/v1/mosaic/test-pattern` (every screen of every box).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MosaicTestPattern {
    /// Up (`true`) or down.
    pub on: bool,
    /// For how long, seconds (1 to 3600; absent: until taken down).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub seconds: Option<u32>,
}
