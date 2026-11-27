//! Vesper signage (`docs/SIGNAGE-PLAN.md`): channels rendered by the site's server, their
//! schedules, playlists, zone layouts and data feeds, the weather and the override
//! (`/api/v1/signage/…`).
//!
//! A **channel** ("Lobby") is a screen of the site's server with an IDN output named after it
//! (`HOST (Lobby)`): Mosaic screens, walls and channels follow it like any other source. Its
//! **schedule** picks a **playlist** (day, time and date-range slots with priorities, else the
//! channel's default); the playlist's **items** play in turn with their durations and
//! transitions in the main zone of the channel's **layout**; the other zones show the clock,
//! the countdown to the next service, the weather and a scrolling ticker. An **override**
//! puts an urgent message on all or chosen channels at once.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The zones of a channel's screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SignageLayout {
    /// The playlist on the whole screen.
    #[default]
    Full,
    /// The playlist above a scrolling ticker.
    MainTicker,
    /// The playlist beside a sidebar (clock, countdown, weather).
    MainSidebar,
    /// The playlist beside a sidebar, above a ticker.
    MainSidebarTicker,
}

impl SignageLayout {
    /// The layout has a sidebar.
    pub fn sidebar(self) -> bool {
        matches!(self, Self::MainSidebar | Self::MainSidebarTicker)
    }
    /// The layout has a ticker.
    pub fn ticker(self) -> bool {
        matches!(self, Self::MainTicker | Self::MainSidebarTicker)
    }
}

/// A slot of a channel's schedule: a playlist on some days, between two times, optionally
/// only within a date range.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageSlot {
    /// Its id (made by the server when empty).
    #[serde(default)]
    pub id: String,
    /// The playlist it plays.
    pub playlist: String,
    /// Days of the week it repeats on: 0 = Monday … 6 = Sunday (empty: every day).
    #[serde(default)]
    pub days: Vec<u8>,
    /// Start, minutes after midnight (0–1439).
    pub start: u16,
    /// End, minutes after midnight (1–1440); an end at or before the start runs past
    /// midnight into the next day.
    pub end: u16,
    /// First date it plays (`YYYY-MM-DD`, local).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<String>,
    /// Last date it plays (`YYYY-MM-DD`, local, inclusive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub until: Option<String>,
    /// The highest priority of the slots playing now wins (a tie: the one listed first).
    #[serde(default)]
    pub priority: i32,
}

/// A signage channel as stored, with what it plays now (`GET /signage`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageChannel {
    /// Its id.
    pub id: String,
    /// Its name: its screen's and its IDN output's ("Lobby").
    pub name: String,
    /// Its zones.
    #[serde(default)]
    pub layout: SignageLayout,
    /// What plays when no slot does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default_playlist: Option<String>,
    /// Its schedule.
    #[serde(default)]
    pub slots: Vec<SignageSlot>,
    /// The ticker's text (layouts with a ticker); a feed's titles when `tickerFeed` is set.
    #[serde(default)]
    pub ticker: String,
    /// A feed whose rows' titles scroll in the ticker (after `ticker`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ticker_feed: Option<String>,
    /// Its screen on this server (`GET /screens`; not an operator screen).
    #[serde(default)]
    pub screen: String,
    /// Its IDN source, as receivers list it (`HOST (Lobby)`).
    #[serde(default)]
    pub source: String,
    /// What it plays now (answers only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub now: Option<SignageNow>,
}

/// What a channel plays now and next, and how it is.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageNow {
    /// The playlist playing (`None`: nothing scheduled and no default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub playlist: Option<String>,
    /// The slot that picked it (`None`: the default playlist).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub slot: Option<String>,
    /// The item showing (its title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub item: Option<String>,
    /// The item after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next: Option<String>,
    /// `ok`, `idle` (nothing to play) or `problem`.
    pub health: SignageHealth,
    /// The last problem in words (an item skipped, a feed that failed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
    /// An override shows on it.
    #[serde(default)]
    pub overridden: bool,
}

/// How a channel is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SignageHealth {
    /// Playing.
    Ok,
    /// Nothing to play (zones only).
    #[default]
    Idle,
    /// Playing, but something was skipped or a feed failed (see `problem`).
    Problem,
}

/// What a playlist item shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SignageContent {
    /// A slide of a presentation of the server's active collection (by index from 0).
    Slide {
        /// The presentation.
        presentation: String,
        /// The slide, counted from 0 in the presentation's order.
        #[serde(default)]
        slide: u32,
    },
    /// An image (a media asset).
    Image {
        /// The media asset.
        asset: String,
    },
    /// A video, looping, muted (a media asset).
    Video {
        /// The media asset.
        asset: String,
    },
    /// An announcement: a title and a body.
    Text {
        /// The title.
        title: String,
        /// The body.
        #[serde(default)]
        body: String,
    },
    /// A feed's rows through its column mapping (the first `rows` rows).
    Feed {
        /// The feed.
        feed: String,
        /// How many rows show (1–8).
        #[serde(default = "default_rows")]
        rows: u32,
    },
}

fn default_rows() -> u32 {
    4
}

/// One item of a playlist.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageItem {
    /// Its id (made by the server when empty).
    #[serde(default)]
    pub id: String,
    /// What it shows.
    pub content: SignageContent,
    /// How long it shows, seconds (1–3600).
    #[serde(default = "default_duration")]
    pub duration: f64,
    /// How it comes in (`None`: a 1 s dissolve).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub transition: Option<model::Transition>,
}

fn default_duration() -> f64 {
    10.0
}

/// A playlist of signage items, played in turn and looping.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignagePlaylist {
    /// Its id.
    #[serde(default)]
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its items.
    #[serde(default)]
    pub items: Vec<SignageItem>,
}

/// Which columns of a feed's rows fill a feed slide's fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageMapping {
    /// The row's title.
    #[serde(default)]
    pub title: String,
    /// The row's text under its title.
    #[serde(default)]
    pub body: String,
    /// When (shown before the title).
    #[serde(default)]
    pub when: String,
}

/// A data feed: rows read from an address or a file every so often (IDN's `idn-feeds`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageFeed {
    /// Its id.
    #[serde(default)]
    pub id: String,
    /// Its name.
    pub name: String,
    /// An `http(s)://` address (a Google Sheet's link or its published CSV, a CSV, TSV, JSON,
    /// XML or RSS address) or a file on the server.
    pub url: String,
    /// `auto`, `csv`, `tsv`, `json` or `xml` (RSS too).
    #[serde(default = "default_format")]
    pub format: String,
    /// Seconds between reads (60–86400).
    #[serde(default = "default_refresh")]
    pub refresh: u32,
    /// The column mapping.
    #[serde(default)]
    pub mapping: SignageMapping,
    /// The columns of the last good read (answers only).
    #[serde(default)]
    pub columns: Vec<String>,
    /// The last good read's rows, at most 50 (answers only).
    #[serde(default)]
    pub rows: Vec<std::collections::BTreeMap<String, String>>,
    /// When it was last read well (Unix ms; answers only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub read_at: Option<u64>,
    /// Why the last read failed (answers only; the last good rows stay).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
}

fn default_format() -> String {
    "auto".into()
}

fn default_refresh() -> u32 {
    900
}

/// A feed read once to see its rows before it is added (`POST /signage/feeds/preview`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageFeedPreview {
    /// The columns.
    pub columns: Vec<String>,
    /// The first rows (at most 20).
    pub rows: Vec<std::collections::BTreeMap<String, String>>,
}

/// A weekly service time the countdown counts to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageServiceTime {
    /// 0 = Monday … 6 = Sunday.
    pub day: u8,
    /// Minutes after midnight.
    pub minute: u16,
    /// Its name ("Sunday service").
    #[serde(default)]
    pub name: String,
}

/// Where the weather is for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignagePlace {
    /// Its name ("Springfield, Illinois").
    pub name: String,
    /// Latitude, degrees.
    pub latitude: f64,
    /// Longitude, degrees.
    pub longitude: f64,
}

/// Signage settings (`PUT /signage/settings`, admins).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageSettings {
    /// The weather's place (`None`: no weather).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub place: Option<SignagePlace>,
    /// Temperatures in °F rather than °C.
    #[serde(default)]
    pub fahrenheit: bool,
    /// The services the countdown counts to.
    #[serde(default)]
    pub services: Vec<SignageServiceTime>,
}

/// The weather now (Open-Meteo).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageWeather {
    /// Temperature now, in the settings' unit.
    pub temperature: f64,
    /// Today's high and low.
    pub high: f64,
    /// Today's low.
    pub low: f64,
    /// The sky in words ("Partly cloudy").
    pub summary: String,
    /// When it was read (Unix ms).
    #[ts(type = "number")]
    pub read_at: u64,
    /// The data's credit, shown with it.
    pub credit: String,
}

/// How an override looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SignageOverrideStyle {
    /// White on red, full screen.
    #[default]
    Urgent,
    /// White on blue, full screen.
    Info,
}

/// An urgent message over all or chosen channels (`POST /signage/override`, operators).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageOverride {
    /// The message.
    pub text: String,
    /// How it looks.
    #[serde(default)]
    pub style: SignageOverrideStyle,
    /// The channels it shows on (empty: all).
    #[serde(default)]
    pub channels: Vec<String>,
    /// It clears itself after this many seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub seconds: Option<u32>,
    /// When it was pushed (Unix ms; answers only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub since: Option<u64>,
    /// When it clears itself (Unix ms; answers only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, type = "number")]
    pub until: Option<u64>,
}

/// `GET /api/v1/signage`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignageState {
    /// This server renders the site's signage (it is the site's server).
    pub managed: bool,
    /// The channels, with what they play now.
    pub channels: Vec<SignageChannel>,
    /// The playlists.
    pub playlists: Vec<SignagePlaylist>,
    /// The feeds, with their last rows.
    pub feeds: Vec<SignageFeed>,
    /// The settings.
    pub settings: SignageSettings,
    /// The weather now (`None`: no place set, or the last forecast is older than 6 h).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub weather: Option<SignageWeather>,
    /// The override showing.
    #[serde(default, rename = "override", skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub override_: Option<SignageOverride>,
}

/// `POST /signage/channels`, `PUT /signage/channels/{id}` (admins): only fields sent change.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PutSignageChannel {
    /// A new name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// A new layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub layout: Option<SignageLayout>,
    /// A new default playlist (`""`: none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub default_playlist: Option<String>,
    /// A new schedule (whole).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub slots: Option<Vec<SignageSlot>>,
    /// New ticker text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ticker: Option<String>,
    /// A new ticker feed (`""`: none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ticker_feed: Option<String>,
}

/// A place found by name (`GET /signage/places?q=`, Open-Meteo's geocoding).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SignagePlaces {
    /// The places found, best first.
    pub places: Vec<SignagePlace>,
}
