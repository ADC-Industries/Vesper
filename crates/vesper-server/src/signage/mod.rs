//! Signage on the site's server (`docs/SIGNAGE-PLAN.md`): channels rendered as screens of this
//! server with an IDN output each, their schedules, playlists and zones, data feeds, the
//! weather and the override. Routes: [`crate::api`]'s `signage` area.
//!
//! - **Channels** are screens of kind `signage` ([`model::ScreenKind::Signage`]) with an IDN
//!   output named after the channel; [`ensure`] makes, renames and removes them to match the
//!   channels (at start and after every channel change), so a deleted output or a restored
//!   backup comes back by itself.
//! - **The tick** ([`Signage::tick`], 4 a second) picks each channel's playlist with
//!   [`schedule`], moves on to the next item when an item's time is up, skips what can't be
//!   shown (logged once per item, never a stop), builds the channel's slides ([`compose`]) and
//!   wakes the renderer when something drawn changed. The render thread asks [`Signage::plan`]
//!   for a signage screen's plan instead of planning it from the show state.
//! - **Kept** in `<data>/machine/signage.json` (channels, playlists, feeds, settings, the
//!   override) and `signage-cache.json` (the last good rows of each feed, the last forecast).
//! - **Feeds** are read by IDN's `idn-feeds` on their own interval, off the async threads; a
//!   failed read keeps the last good rows. **Weather** is Open-Meteo's, every 30 minutes; it
//!   hides when the last forecast is older than 6 hours.
//!
//! Only the site's server (the one running the Site Manager) renders signage.

pub mod compose;
pub mod feeds;
pub mod schedule;
pub mod weather;

use crate::config::ConfigOp;
use crate::data::ShowData;
use crate::show::{LayerChange, PlannedSlide, ScreenPlan};
use crate::{Shared, lock};
use chrono::{Datelike, Timelike};
use feeds::Row;
use model::{Layer, Screen, ScreenKind, Size, Slide, Transition};
use protocol::{
    ApiError, ErrorCode, PutSignageChannel, SignageChannel, SignageContent, SignageFeed,
    SignageHealth, SignageItem, SignageNow, SignageOverride, SignagePlaylist, SignageSettings,
    SignageSlot, SignageState, SitePart,
};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// How often channels are looked at.
pub const TICK: Duration = Duration::from_millis(250);
/// The longest name, characters.
pub const MAX_NAME: usize = 48;
/// The most items in a playlist, slots in a schedule.
pub const MAX_ITEMS: usize = 200;
/// The longest override or ticker text, characters.
pub const MAX_TEXT: usize = 500;

fn api(code: ErrorCode, m: impl Into<String>) -> ApiError {
    ApiError::new(code, m)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    channels: Vec<SignageChannel>,
    playlists: Vec<SignagePlaylist>,
    feeds: Vec<SignageFeed>,
    settings: SignageSettings,
    #[serde(rename = "override", skip_serializing_if = "Option::is_none")]
    over: Option<SignageOverride>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct FeedCache {
    columns: Vec<String>,
    rows: Vec<Row>,
    #[serde(skip_serializing_if = "Option::is_none")]
    read_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    problem: Option<String>,
    /// When it was last tried (ms; not kept: a restart reads every feed again).
    #[serde(skip)]
    tried: u64,
    /// Counts the feed's changes: a read started before one is dropped (H02-6).
    #[serde(skip)]
    generation: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Cache {
    feeds: BTreeMap<String, FeedCache>,
    #[serde(skip_serializing_if = "Option::is_none")]
    weather: Option<protocol::SignageWeather>,
    #[serde(skip)]
    weather_tried: u64,
    /// Counts changes of the place or unit: a read started before one is dropped (H02-6).
    #[serde(skip)]
    weather_generation: u64,
}

/// A channel's playback.
struct Player {
    playlist: Option<String>,
    index: usize,
    item: Option<String>,
    started: Instant,
    change: LayerChange,
    /// Problems already logged (each once).
    logged: HashSet<String>,
    problem: Option<String>,
}

/// What a channel's screen draws now.
pub struct Frame {
    main: Option<(Slide, Size)>,
    /// The screen's size: the zones' canvas.
    size: Size,
    chrome: Slide,
    change: LayerChange,
    fingerprint: u64,
}

/// Signage on this server.
pub struct Signage {
    path: PathBuf,
    cache_path: PathBuf,
    /// The signage feeds folder (`<data>/signage-feeds/`): the only files feeds read.
    feed_files: PathBuf,
    managed: AtomicBool,
    store: Mutex<Stored>,
    cache: Mutex<Cache>,
    players: Mutex<HashMap<String, Player>>,
    nows: Mutex<HashMap<String, SignageNow>>,
    /// Why a channel's screen or output couldn't be made (shown as its problem).
    setup: Mutex<HashMap<String, String>>,
    frames: Mutex<HashMap<String, Arc<Frame>>>,
    reading: Mutex<HashSet<String>>,
    last: Mutex<Option<SignageState>>,
    seq: AtomicU64,
    /// The store holds a change `signage.json` doesn't have yet (its save failed): saved
    /// again at the next ticks.
    unsaved: AtomicBool,
    /// One write of `signage-cache.json` at a time, each of the latest cache.
    cache_writing: Mutex<()>,
    /// Wakes the channel upkeep ([`ensure`]) after a channel change.
    pub kick: tokio::sync::Notify,
    /// One [`ensure`] at a time (the upkeep's and a request's would both make a screen).
    ensuring: tokio::sync::Mutex<()>,
    /// Open-Meteo's forecast and geocoding addresses (tests point them elsewhere).
    pub endpoints: Mutex<(String, String)>,
}

impl Signage {
    /// Loads `<data>/machine/signage.json` (a damaged file is kept aside and signage starts
    /// afresh).
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("machine").join("signage.json");
        let cache_path = data_dir.join("machine").join("signage-cache.json");
        let feed_files = data_dir.join(feeds::FOLDER);
        // There for the admin to put feed files in.
        let _ = std::fs::create_dir_all(&feed_files);
        let read = |p: &Path| -> Option<Vec<u8>> { std::fs::read(p).ok() };
        let store: Stored = read(&path)
            .map(|b| {
                serde_json::from_slice(&b).unwrap_or_else(|e| {
                    tracing::warn!(
                        "{}: {e}; starting afresh (kept as .damaged)",
                        path.display()
                    );
                    let _ = std::fs::rename(&path, path.with_extension("json.damaged"));
                    Stored::default()
                })
            })
            .unwrap_or_default();
        let cache: Cache = read(&cache_path)
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Signage {
            path,
            cache_path,
            feed_files,
            managed: AtomicBool::new(false),
            store: Mutex::new(store),
            cache: Mutex::new(cache),
            players: Mutex::new(HashMap::new()),
            nows: Mutex::new(HashMap::new()),
            setup: Mutex::new(HashMap::new()),
            frames: Mutex::new(HashMap::new()),
            reading: Mutex::new(HashSet::new()),
            last: Mutex::new(None),
            seq: AtomicU64::new(1),
            unsaved: AtomicBool::new(false),
            cache_writing: Mutex::new(()),
            kick: tokio::sync::Notify::new(),
            ensuring: tokio::sync::Mutex::new(()),
            endpoints: Mutex::new((weather::FORECAST.into(), weather::GEOCODING.into())),
        }
    }

    fn write(path: &Path, bytes: Vec<u8>) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)
    }

    fn save(&self, s: &Stored) -> Result<(), ApiError> {
        let mut s = s.clone();
        for c in &mut s.channels {
            c.now = None;
        }
        for f in &mut s.feeds {
            f.columns.clear();
            f.rows.clear();
            f.read_at = None;
            f.problem = None;
        }
        Self::write(
            &self.path,
            serde_json::to_vec_pretty(&s).unwrap_or_default(),
        )
        .map_err(|e| api(ErrorCode::Internal, format!("signage.json: {e}")))
    }

    /// Saves the store `s` (the caller holds its lock, so writes keep their order, H02-9);
    /// a failure is logged once and retried at the next tick.
    fn save_now(&self, s: &Stored) {
        match self.save(s) {
            Ok(()) => {
                if self.unsaved.swap(false, Ordering::AcqRel) {
                    tracing::info!("signage.json: saved again");
                }
            }
            Err(e) => {
                if !self.unsaved.swap(true, Ordering::AcqRel) {
                    tracing::warn!("{} (tried again until it works)", e.message);
                }
            }
        }
    }

    fn save_cache(&self) {
        // The cache is copied under the writer's lock: the last write is of the latest cache.
        let _one = lock(&self.cache_writing);
        let c = lock(&self.cache).clone();
        if let Err(e) = Self::write(&self.cache_path, serde_json::to_vec(&c).unwrap_or_default()) {
            tracing::warn!("signage-cache.json: {e}");
        }
    }

    /// This server renders the site's signage (it runs the Site Manager).
    pub fn managed(&self) -> bool {
        self.managed.load(Ordering::Acquire)
    }

    /// Follows whether this server is the site's server.
    pub fn follow(&self, site: &crate::site::Site) {
        self.managed
            .store(site.handle().is_some(), Ordering::Release);
    }

    /// `409` unless this server renders the site's signage.
    pub fn require_managed(&self) -> Result<(), ApiError> {
        if self.managed() {
            Ok(())
        } else {
            Err(api(
                ErrorCode::Conflict,
                "the site's signage runs on the site's server",
            ))
        }
    }

    // ---- reading ------------------------------------------------------------------------

    /// `GET /api/v1/signage`.
    pub fn state(&self) -> SignageState {
        let s = lock(&self.store).clone();
        let cache = lock(&self.cache).clone();
        let nows = lock(&self.nows).clone();
        let setup = lock(&self.setup).clone();
        let channels = s
            .channels
            .into_iter()
            .map(|mut c| {
                let mut now = nows.get(&c.id).cloned().unwrap_or_default();
                if let Some(why) = setup.get(&c.id) {
                    now.health = SignageHealth::Problem;
                    now.problem = Some(why.clone());
                }
                c.now = Some(now);
                c
            })
            .collect();
        let feeds = s
            .feeds
            .into_iter()
            .map(|mut f| {
                if let Some(c) = cache.feeds.get(&f.id) {
                    f.columns = c.columns.clone();
                    f.rows = c.rows.clone();
                    f.read_at = c.read_at;
                    f.problem = c.problem.clone();
                }
                f
            })
            .collect();
        SignageState {
            managed: self.managed(),
            channels,
            playlists: s.playlists,
            feeds,
            weather: fresh_weather(&cache, &s.settings, now_ms()).cloned(),
            settings: s.settings,
            override_: s.over,
        }
    }

    /// `siteChanged` (`signage`) when what `GET /signage` answers changed.
    pub fn announce(&self, site: &crate::site::Site) {
        let now = self.state();
        let mut last = lock(&self.last);
        if last.as_ref() != Some(&now) {
            *last = Some(now);
            drop(last);
            site.changed([SitePart::Signage]);
        }
    }

    /// The channel `id`.
    pub fn channel(&self, id: &str) -> Option<SignageChannel> {
        lock(&self.store)
            .channels
            .iter()
            .find(|c| c.id == id)
            .cloned()
    }

    // ---- changes ------------------------------------------------------------------------

    fn change<T>(&self, f: impl FnOnce(&mut Stored) -> Result<T, ApiError>) -> Result<T, ApiError> {
        self.require_managed()?;
        let mut s = lock(&self.store);
        let mut next = s.clone();
        let r = f(&mut next)?;
        self.save(&next)?;
        self.unsaved.store(false, Ordering::Release);
        *s = next;
        Ok(r)
    }

    /// `POST /signage/channels`: a new channel (its screen and output follow).
    pub fn create_channel(&self, put: PutSignageChannel) -> Result<SignageChannel, ApiError> {
        let name = put.name.clone().unwrap_or_default();
        self.change(|s| {
            let name = check_name(&name)?;
            if s.channels
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case(&name))
            {
                return Err(api(
                    ErrorCode::Conflict,
                    format!("a channel is already called “{name}”"),
                ));
            }
            let mut c = SignageChannel {
                id: new_id(),
                name,
                layout: Default::default(),
                default_playlist: None,
                slots: Vec::new(),
                ticker: String::new(),
                ticker_feed: None,
                screen: String::new(),
                source: String::new(),
                now: None,
            };
            apply_channel(&mut c, put, s)?;
            s.channels.push(c.clone());
            Ok(c)
        })
    }

    /// `PUT /signage/channels/{id}`.
    pub fn update_channel(
        &self,
        id: &str,
        put: PutSignageChannel,
    ) -> Result<SignageChannel, ApiError> {
        self.change(|s| {
            let others: Vec<String> = s
                .channels
                .iter()
                .filter(|c| c.id != id)
                .map(|c| c.name.to_lowercase())
                .collect();
            let mut c = s
                .channels
                .iter()
                .find(|c| c.id == id)
                .cloned()
                .ok_or_else(|| api(ErrorCode::NotFound, format!("no channel {id}")))?;
            if let Some(n) = &put.name {
                let n = check_name(n)?;
                if others.contains(&n.to_lowercase()) {
                    return Err(api(
                        ErrorCode::Conflict,
                        format!("a channel is already called “{n}”"),
                    ));
                }
                c.name = n;
            }
            apply_channel(&mut c, put, s)?;
            if let Some(slot) = s.channels.iter_mut().find(|x| x.id == id) {
                *slot = c.clone();
            }
            Ok(c)
        })
    }

    /// `DELETE /signage/channels/{id}` (its screen and output go with it).
    pub fn delete_channel(&self, id: &str) -> Result<(), ApiError> {
        self.change(|s| {
            let n = s.channels.len();
            s.channels.retain(|c| c.id != id);
            if s.channels.len() == n {
                return Err(api(ErrorCode::NotFound, format!("no channel {id}")));
            }
            // An override of named channels that are all gone goes too: an empty list would
            // mean every channel (H02-4).
            if let Some(o) = &mut s.over
                && !o.channels.is_empty()
            {
                o.channels.retain(|c| c != id);
                if o.channels.is_empty() {
                    s.over = None;
                }
            }
            Ok(())
        })?;
        lock(&self.players).remove(id);
        lock(&self.frames).remove(id);
        lock(&self.nows).remove(id);
        Ok(())
    }

    /// `POST /signage/playlists`, `PUT /signage/playlists/{id}` (`id`: `None` = new).
    pub fn put_playlist(
        &self,
        id: Option<&str>,
        mut p: SignagePlaylist,
    ) -> Result<SignagePlaylist, ApiError> {
        self.change(|s| {
            p.name = check_name(&p.name)?;
            if p.items.len() > MAX_ITEMS {
                return Err(api(
                    ErrorCode::BadRequest,
                    format!("at most {MAX_ITEMS} items"),
                ));
            }
            let mut seen = HashSet::new();
            for it in &mut p.items {
                check_item(it, s)?;
                if it.id.is_empty() || !seen.insert(it.id.clone()) {
                    it.id = new_id();
                    seen.insert(it.id.clone());
                }
            }
            match id {
                None => {
                    p.id = new_id();
                    s.playlists.push(p.clone());
                }
                Some(id) => {
                    p.id = id.to_string();
                    let slot = s
                        .playlists
                        .iter_mut()
                        .find(|x| x.id == id)
                        .ok_or_else(|| api(ErrorCode::NotFound, format!("no playlist {id}")))?;
                    *slot = p.clone();
                }
            }
            Ok(p)
        })
    }

    /// `DELETE /signage/playlists/{id}`: `409` while a channel plays it by default or in its
    /// schedule.
    pub fn delete_playlist(&self, id: &str) -> Result<(), ApiError> {
        self.change(|s| {
            if let Some(c) = s.channels.iter().find(|c| {
                c.default_playlist.as_deref() == Some(id)
                    || c.slots.iter().any(|sl| sl.playlist == id)
            }) {
                return Err(api(
                    ErrorCode::Conflict,
                    format!("the channel {} plays it", c.name),
                ));
            }
            let n = s.playlists.len();
            s.playlists.retain(|p| p.id != id);
            if s.playlists.len() == n {
                return Err(api(ErrorCode::NotFound, format!("no playlist {id}")));
            }
            Ok(())
        })
    }

    /// `POST /signage/feeds`, `PUT /signage/feeds/{id}` (`id`: `None` = new). A new or
    /// changed address is read at the next tick.
    pub fn put_feed(&self, id: Option<&str>, mut f: SignageFeed) -> Result<SignageFeed, ApiError> {
        let f = self.change(|s| {
            f.name = check_name(&f.name)?;
            f.url = f.url.trim().to_string();
            if f.url.is_empty() {
                return Err(ApiError::invalid(
                    "url",
                    "a feed needs an address or a file",
                ));
            }
            feeds::source(&f.url, &self.feed_files).map_err(|e| ApiError::invalid("url", e))?;
            feeds::format(&f.format).map_err(|e| ApiError::invalid("format", e))?;
            f.refresh = f.refresh.clamp(60, 86_400);
            f.columns.clear();
            f.rows.clear();
            f.read_at = None;
            f.problem = None;
            match id {
                None => {
                    f.id = new_id();
                    s.feeds.push(f.clone());
                }
                Some(id) => {
                    f.id = id.to_string();
                    let slot = s
                        .feeds
                        .iter_mut()
                        .find(|x| x.id == id)
                        .ok_or_else(|| api(ErrorCode::NotFound, format!("no feed {id}")))?;
                    *slot = f.clone();
                }
            }
            Ok(f)
        })?;
        {
            // Read at the next tick; a read of the old address still running is dropped.
            let mut c = lock(&self.cache);
            let e = c.feeds.entry(f.id.clone()).or_default();
            e.tried = 0;
            e.generation += 1;
        }
        Ok(f)
    }

    /// `DELETE /signage/feeds/{id}`: `409` while an item or a ticker uses it.
    pub fn delete_feed(&self, id: &str) -> Result<(), ApiError> {
        self.change(|s| {
            let used = s.playlists.iter().find(|p| {
                p.items
                    .iter()
                    .any(|i| matches!(&i.content, SignageContent::Feed { feed, .. } if feed == id))
            });
            if let Some(p) = used {
                return Err(api(
                    ErrorCode::Conflict,
                    format!("the playlist {} shows it", p.name),
                ));
            }
            if let Some(c) = s
                .channels
                .iter()
                .find(|c| c.ticker_feed.as_deref() == Some(id))
            {
                return Err(api(
                    ErrorCode::Conflict,
                    format!("the channel {}'s ticker shows it", c.name),
                ));
            }
            let n = s.feeds.len();
            s.feeds.retain(|f| f.id != id);
            if s.feeds.len() == n {
                return Err(api(ErrorCode::NotFound, format!("no feed {id}")));
            }
            Ok(())
        })?;
        lock(&self.cache).feeds.remove(id);
        Ok(())
    }

    /// `PUT /signage/settings`. A new place or unit reads the weather again.
    pub fn put_settings(&self, mut st: SignageSettings) -> Result<SignageSettings, ApiError> {
        let st = self.change(|s| {
            if let Some(p) = &mut st.place {
                p.name = p.name.trim().chars().take(120).collect();
                if !(-90.0..=90.0).contains(&p.latitude) || !(-180.0..=180.0).contains(&p.longitude)
                {
                    return Err(ApiError::invalid(
                        "place",
                        "latitude or longitude out of range",
                    ));
                }
            }
            if st.services.len() > 50 {
                return Err(ApiError::invalid("services", "at most 50 service times"));
            }
            for sv in &mut st.services {
                if sv.day > 6 || sv.minute >= schedule::DAY {
                    return Err(ApiError::invalid("services", "a day is 0–6, a time 0–1439"));
                }
                sv.name = sv.name.trim().chars().take(MAX_NAME).collect();
            }
            let moved = s.settings.place != st.place || s.settings.fahrenheit != st.fahrenheit;
            s.settings = st.clone();
            Ok((st, moved))
        })?;
        if st.1 {
            let mut c = lock(&self.cache);
            c.weather = None;
            c.weather_tried = 0;
            c.weather_generation += 1;
        }
        Ok(st.0)
    }

    /// `POST /signage/override`: shows at once on its channels (all when none are named).
    pub fn push_override(&self, mut o: SignageOverride) -> Result<SignageOverride, ApiError> {
        let o = self.change(|s| {
            o.text = o.text.trim().to_string();
            if o.text.is_empty() || o.text.chars().count() > MAX_TEXT {
                return Err(ApiError::invalid(
                    "text",
                    format!("the message needs 1–{MAX_TEXT} characters"),
                ));
            }
            o.channels.sort();
            o.channels.dedup();
            if let Some(bad) = o
                .channels
                .iter()
                .find(|id| !s.channels.iter().any(|c| &&c.id == id))
            {
                return Err(api(ErrorCode::NotFound, format!("no channel {bad}")));
            }
            let since = now_ms();
            o.since = Some(since);
            o.until = o
                .seconds
                .filter(|s| *s > 0)
                .map(|s| since + u64::from(s.min(7 * 86_400)) * 1000);
            s.over = Some(o.clone());
            Ok(o)
        })?;
        Ok(o)
    }

    /// `DELETE /signage/override`.
    pub fn clear_override(&self) -> Result<bool, ApiError> {
        self.change(|s| Ok(s.over.take().is_some()))
    }

    /// A feed read once (`POST /signage/feeds/preview`; blocking).
    pub fn preview(&self, f: &SignageFeed) -> Result<protocol::SignageFeedPreview, ApiError> {
        let (mut rows, columns) = feeds::read(f, &self.feed_files)
            .map_err(|e| api(ErrorCode::BadRequest, format!("can't read it: {e}")))?;
        rows.truncate(20);
        Ok(protocol::SignageFeedPreview { columns, rows })
    }

    // ---- playing ------------------------------------------------------------------------

    /// What a signage screen draws (the render thread; see the module docs).
    pub fn plan(&self, channel: &str) -> ScreenPlan<'static> {
        let mut plan = ScreenPlan::default();
        let Some(f) = lock(&self.frames).get(channel).cloned() else {
            plan.blank = true;
            plan.fingerprint = 3;
            return plan;
        };
        if let Some((slide, canvas)) = &f.main {
            plan.slides.push(PlannedSlide {
                slide: Cow::Owned(slide.clone()),
                canvas: *canvas,
                layer: Layer::Slide,
            });
        }
        plan.message = Some(PlannedSlide {
            slide: Cow::Owned(f.chrome.clone()),
            canvas: f.size,
            layer: Layer::Messages,
        });
        plan.changes = Some(HashMap::from([(Layer::Slide, f.change.clone())]));
        plan.fingerprint = f.fingerprint;
        plan
    }

    /// One look at every channel (see the module docs). True when a screen must be drawn
    /// again.
    pub fn tick(
        &self,
        data: &ShowData,
        local: chrono::DateTime<chrono::Local>,
        at: Instant,
    ) -> bool {
        let ms = now_ms();
        let now = local.naive_local();
        let store = {
            let mut s = lock(&self.store);
            // An override whose time is up clears itself, in memory at once; the file follows
            // (a failed save is retried at the next ticks, never a reason to keep it, H02-2).
            if s.over
                .as_ref()
                .is_some_and(|o| o.until.is_some_and(|u| ms >= u))
            {
                s.over = None;
                self.unsaved.store(true, Ordering::Release);
            }
            if self.unsaved.load(Ordering::Acquire) {
                self.save_now(&s);
            }
            s.clone()
        };
        let cache = lock(&self.cache).clone();
        let feed_rows = |id: &str| -> Option<(SignageFeed, Vec<Row>)> {
            let f = store.feeds.iter().find(|f| f.id == id)?.clone();
            let mut f = f;
            let rows = cache.feeds.get(id).map(|c| {
                f.columns = c.columns.clone();
                f.problem = c.problem.clone();
                c.rows.clone()
            });
            Some((f, rows.unwrap_or_default()))
        };
        let weather = fresh_weather(&cache, &store.settings, ms);
        let unit = if store.settings.fahrenheit {
            "°F"
        } else {
            "°C"
        };
        let clock = (
            format!("{:02}:{:02}", now.hour(), now.minute()),
            format!("{} {} {}", now.format("%A"), now.day(), now.format("%B")),
        );
        let countdown = schedule::next_service(&store.settings.services, now).map(|(t, s)| {
            let secs = schedule::seconds_until(&local, t);
            let name = if s.name.is_empty() {
                "Next service".to_string()
            } else {
                s.name.clone()
            };
            (name, schedule::countdown(secs))
        });
        let mut dirty = false;
        let mut players = lock(&self.players);
        players.retain(|id, _| store.channels.iter().any(|c| &c.id == id));
        let mut nows = HashMap::new();
        let mut frames = HashMap::new();
        for ch in &store.channels {
            let size = data
                .screens
                .iter()
                .find(|s| matches!(&s.kind, ScreenKind::Signage { channel } if *channel == ch.id))
                .map_or(Size::HD, |s| s.size);
            let zones = compose::zones(ch.layout, size);
            let picked = schedule::pick(&ch.slots, ch.default_playlist.as_deref(), now);
            let playlist = picked.and_then(|(p, _)| store.playlists.iter().find(|x| x.id == p));
            let p = players.entry(ch.id.clone()).or_insert_with(|| Player {
                playlist: None,
                index: 0,
                item: None,
                started: at,
                change: LayerChange {
                    seq: 0,
                    transition: Transition::CUT,
                    at,
                },
                logged: HashSet::new(),
                problem: None,
            });
            let pid = playlist.map(|p| p.id.clone());
            let mut advance = false;
            if p.playlist != pid {
                p.playlist = pid;
                p.index = 0;
                p.item = None;
                advance = true;
            } else if let Some(pl) = playlist {
                let cur = pl.items.iter().position(|i| Some(&i.id) == p.item.as_ref());
                match cur {
                    Some(i) => {
                        p.index = i;
                        let d = Duration::from_secs_f64(pl.items[i].duration.clamp(1.0, 3600.0));
                        if at.duration_since(p.started) >= d {
                            p.index = i + 1;
                            advance = true;
                        }
                    }
                    // The item was removed (or nothing showed yet): the next one.
                    None => advance = true,
                }
            }
            let mut main = None;
            let mut shown: Option<&SignageItem> = None;
            if let Some(pl) = playlist.filter(|pl| !pl.items.is_empty()) {
                let n = pl.items.len();
                let start = p.index % n;
                if advance && start == 0 {
                    // A new round: problems are looked for afresh.
                    p.problem = None;
                }
                for k in 0..n {
                    let i = (start + k) % n;
                    let it = &pl.items[i];
                    let feed = match &it.content {
                        SignageContent::Feed { feed, .. } => feed_rows(feed),
                        _ => None,
                    };
                    match compose::item(&it.content, data, feed.as_ref(), zones.main, size) {
                        Ok(mut m) => {
                            // A presentation's slide keeps its own IDs; built ones get stable
                            // ones (H02-1).
                            if !matches!(it.content, SignageContent::Slide { .. }) {
                                compose::stamp(&mut m.0, &format!("{}:{}", ch.id, it.id));
                            }
                            let other = p.item.as_deref() != Some(it.id.as_str());
                            if other || advance {
                                p.index = i;
                                p.item = Some(it.id.clone());
                                p.started = at;
                            }
                            if other {
                                p.change = LayerChange {
                                    seq: self.seq.fetch_add(1, Ordering::Relaxed),
                                    transition: it
                                        .transition
                                        .clone()
                                        .unwrap_or_else(|| Transition::dissolve(1.0)),
                                    at,
                                };
                            }
                            main = Some(m);
                            shown = Some(it);
                            break;
                        }
                        Err(why) => {
                            if p.logged.insert(format!("{}:{why}", it.id)) {
                                tracing::warn!(channel = %ch.name, "signage: skipped an item: {why}");
                            }
                            if p.logged.len() > 200 {
                                p.logged.clear();
                            }
                            p.problem = Some(why);
                        }
                    }
                }
            }
            if shown.is_none() {
                p.item = None;
            }
            let over = store
                .over
                .as_ref()
                .filter(|o| o.channels.is_empty() || o.channels.contains(&ch.id));
            let mut ticker = ch.ticker.trim().to_string();
            if let Some((_, rows)) = ch.ticker_feed.as_deref().and_then(feed_rows) {
                let f = store
                    .feeds
                    .iter()
                    .find(|f| Some(&f.id) == ch.ticker_feed.as_ref());
                let cols = cache
                    .feeds
                    .get(ch.ticker_feed.as_deref().unwrap_or_default())
                    .map(|c| c.columns.clone())
                    .unwrap_or_default();
                for r in rows.iter().take(20) {
                    let m = f.map(|f| f.mapping.clone()).unwrap_or_default();
                    let (when, title, _) = feeds::mapped(r, &m, &cols);
                    let t = if when.is_empty() {
                        title
                    } else {
                        format!("{when} · {title}")
                    };
                    if !t.is_empty() {
                        if !ticker.is_empty() {
                            ticker.push_str("   •   ");
                        }
                        ticker.push_str(&t);
                    }
                }
            }
            let mut chrome = compose::chrome(
                &compose::Chrome {
                    layout: ch.layout,
                    clock: clock.clone(),
                    countdown: countdown.clone(),
                    weather: weather.map(|w| (w, unit)),
                    ticker,
                    over,
                },
                size,
            );
            compose::stamp(&mut chrome, &format!("{}:zones", ch.id));
            let mut h = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_vec(&main).unwrap_or_default().hash(&mut h);
            serde_json::to_vec(&chrome).unwrap_or_default().hash(&mut h);
            size.width.to_bits().hash(&mut h);
            size.height.to_bits().hash(&mut h);
            p.change.seq.hash(&mut h);
            let fingerprint = h.finish();
            let next = playlist.and_then(|pl| {
                let n = pl.items.len();
                (n > 1).then(|| title(&pl.items[(p.index + 1) % n], data, &store))
            });
            nows.insert(
                ch.id.clone(),
                SignageNow {
                    playlist: playlist.map(|p| p.id.clone()),
                    slot: picked.and_then(|(_, s)| s.map(str::to_string)),
                    item: shown.map(|i| title(i, data, &store)),
                    next,
                    health: match (shown, &p.problem) {
                        (None, None) => SignageHealth::Idle,
                        (Some(_), None) => SignageHealth::Ok,
                        _ => SignageHealth::Problem,
                    },
                    problem: p.problem.clone(),
                    overridden: over.is_some(),
                },
            );
            frames.insert(
                ch.id.clone(),
                Frame {
                    main,
                    size,
                    chrome,
                    change: p.change.clone(),
                    fingerprint,
                },
            );
        }
        drop(players);
        *lock(&self.nows) = nows;
        let mut current = lock(&self.frames);
        for (id, f) in frames {
            let same = current
                .get(&id)
                .is_some_and(|o| o.fingerprint == f.fingerprint);
            if !same {
                current.insert(id, Arc::new(f));
                dirty = true;
            }
        }
        let n = current.len();
        current.retain(|id, _| store.channels.iter().any(|c| &c.id == id));
        dirty |= current.len() != n;
        dirty
    }

    /// Stops drawing the channels (this server is no longer the site's server): true when a
    /// screen must be drawn again (blank).
    pub fn stand_down(&self) -> bool {
        lock(&self.players).clear();
        lock(&self.nows).clear();
        let mut frames = lock(&self.frames);
        let had = !frames.is_empty();
        frames.clear();
        had
    }

    /// Feeds and the weather due a read: read each off the async threads.
    fn reads_due(self: &Arc<Self>) {
        let store = lock(&self.store).clone();
        let ms = now_ms();
        for f in store.feeds {
            let (due, generation) = {
                let mut c = lock(&self.cache);
                let e = c.feeds.entry(f.id.clone()).or_default();
                (
                    ms.saturating_sub(e.tried) >= u64::from(f.refresh) * 1000,
                    e.generation,
                )
            };
            // One read per feed and version of it: a changed feed is read at once, even while
            // a read of its old address hangs (that one's result is dropped).
            let key = format!("{}#{generation}", f.id);
            if !due || !lock(&self.reading).insert(key.clone()) {
                continue;
            }
            let me = self.clone();
            let files = self.feed_files.clone();
            std::thread::spawn(move || {
                let r = feeds::read(&f, &files);
                if me.took_feed(&f, generation, r) {
                    me.save_cache();
                }
                lock(&me.reading).remove(&key);
            });
        }
        let Some(place) = store.settings.place.clone() else {
            return;
        };
        let (due, generation) = {
            let c = lock(&self.cache);
            (
                ms.saturating_sub(c.weather_tried) >= weather::REFRESH.as_millis() as u64,
                c.weather_generation,
            )
        };
        let key = format!("\u{0}weather#{generation}");
        if !due || !lock(&self.reading).insert(key.clone()) {
            return;
        }
        let me = self.clone();
        let base = lock(&self.endpoints).0.clone();
        let f = store.settings.fahrenheit;
        std::thread::spawn(move || {
            let r = weather::read(&base, &place, f, now_ms());
            if me.took_weather(generation, r) {
                me.save_cache();
            }
            lock(&me.reading).remove(&key);
        });
    }

    /// A feed's read, started at its `generation`, is kept: false when the feed was changed
    /// or deleted meanwhile (the rows aren't its rows; its next read stays due, H02-6).
    fn took_feed(
        &self,
        f: &SignageFeed,
        generation: u64,
        r: Result<(Vec<Row>, Vec<String>), String>,
    ) -> bool {
        let mut c = lock(&self.cache);
        let Some(e) = c
            .feeds
            .get_mut(&f.id)
            .filter(|e| e.generation == generation)
        else {
            return false;
        };
        e.tried = now_ms();
        match r {
            Ok((rows, columns)) => {
                e.rows = rows;
                e.columns = columns;
                e.read_at = Some(e.tried);
                e.problem = None;
            }
            Err(why) => {
                tracing::warn!(feed = %f.name, "signage: a feed read failed: {why}");
                e.problem = Some(why);
            }
        }
        true
    }

    /// A forecast read at the settings' `generation` is kept: false when the place or unit
    /// changed meanwhile (H02-6).
    fn took_weather(&self, generation: u64, r: Result<protocol::SignageWeather, String>) -> bool {
        let mut c = lock(&self.cache);
        if c.weather_generation != generation {
            return false;
        }
        c.weather_tried = now_ms();
        match r {
            Ok(w) => c.weather = Some(w),
            Err(why) => tracing::warn!("signage: the weather couldn't be read: {why}"),
        }
        true
    }

    /// Places found by name (`GET /signage/places?q=`; blocking).
    pub fn places(&self, q: &str) -> Result<Vec<protocol::SignagePlace>, ApiError> {
        if q.trim().chars().count() < 2 {
            return Ok(Vec::new());
        }
        let base = lock(&self.endpoints).1.clone();
        weather::places(&base, q).map_err(|e| {
            api(
                ErrorCode::Unavailable,
                format!("the place search couldn't be reached: {e}"),
            )
        })
    }

    /// Sets the last forecast (tests; a place must be set for it to show).
    pub fn set_weather(&self, w: protocol::SignageWeather) {
        let mut c = lock(&self.cache);
        c.weather = Some(w);
        c.weather_tried = now_ms();
    }
}

/// The forecast when a place is set and it isn't stale.
fn fresh_weather<'a>(
    c: &'a Cache,
    s: &SignageSettings,
    ms: u64,
) -> Option<&'a protocol::SignageWeather> {
    s.place.as_ref()?;
    c.weather
        .as_ref()
        .filter(|w| ms.saturating_sub(w.read_at) <= weather::STALE.as_millis() as u64)
}

/// An item's title for now/next.
fn title(it: &SignageItem, data: &ShowData, s: &Stored) -> String {
    match &it.content {
        SignageContent::Slide {
            presentation,
            slide,
        } => presentation
            .parse::<uuid::Uuid>()
            .ok()
            .and_then(|u| data.content.presentation(model::PresentationId(u)))
            .map_or_else(
                || "A missing presentation".into(),
                |p| format!("{} · slide {}", p.name, slide + 1),
            ),
        SignageContent::Image { asset } | SignageContent::Video { asset } => asset
            .parse::<uuid::Uuid>()
            .map(|u| data.content.asset_name(model::MediaAssetId(u)))
            .unwrap_or_else(|_| "A missing media file".into()),
        SignageContent::Text { title, .. } => title.clone(),
        SignageContent::Feed { feed, .. } => s
            .feeds
            .iter()
            .find(|f| &f.id == feed)
            .map_or_else(|| "A deleted feed".into(), |f| format!("Feed: {}", f.name)),
    }
}

fn check_name(n: &str) -> Result<String, ApiError> {
    let n = n.trim();
    if n.is_empty() || n.chars().count() > MAX_NAME || n.chars().any(|c| c.is_control()) {
        return Err(ApiError::invalid(
            "name",
            format!("a name needs 1–{MAX_NAME} characters"),
        ));
    }
    Ok(n.to_string())
}

fn check_item(it: &mut SignageItem, s: &Stored) -> Result<(), ApiError> {
    if !it.duration.is_finite() || !(1.0..=3600.0).contains(&it.duration) {
        return Err(ApiError::invalid(
            "duration",
            "an item shows 1–3600 seconds",
        ));
    }
    match &mut it.content {
        SignageContent::Slide { presentation, .. } => {
            presentation
                .parse::<uuid::Uuid>()
                .map_err(|_| ApiError::invalid("presentation", "not an ID"))?;
        }
        SignageContent::Image { asset } | SignageContent::Video { asset } => {
            asset
                .parse::<uuid::Uuid>()
                .map_err(|_| ApiError::invalid("asset", "not an ID"))?;
        }
        SignageContent::Text { title, body } => {
            *title = title.trim().chars().take(MAX_TEXT).collect();
            *body = body.trim().chars().take(MAX_TEXT * 4).collect();
            if title.is_empty() {
                return Err(ApiError::invalid("title", "an announcement needs a title"));
            }
        }
        SignageContent::Feed { feed, rows } => {
            if !s.feeds.iter().any(|f| &f.id == feed) {
                return Err(api(ErrorCode::NotFound, format!("no feed {feed}")));
            }
            *rows = (*rows).clamp(1, 8);
        }
    }
    Ok(())
}

fn apply_channel(
    c: &mut SignageChannel,
    put: PutSignageChannel,
    s: &Stored,
) -> Result<(), ApiError> {
    if let Some(l) = put.layout {
        c.layout = l;
    }
    let playlist = |id: &str| {
        if s.playlists.iter().any(|p| p.id == id) {
            Ok(())
        } else {
            Err(api(ErrorCode::NotFound, format!("no playlist {id}")))
        }
    };
    if let Some(d) = put.default_playlist {
        if d.is_empty() {
            c.default_playlist = None;
        } else {
            playlist(&d)?;
            c.default_playlist = Some(d);
        }
    }
    if let Some(mut slots) = put.slots {
        if slots.len() > MAX_ITEMS {
            return Err(ApiError::invalid(
                "slots",
                format!("at most {MAX_ITEMS} slots"),
            ));
        }
        let mut seen = HashSet::new();
        for sl in &mut slots {
            check_slot(sl)?;
            playlist(&sl.playlist)?;
            if sl.id.is_empty() || !seen.insert(sl.id.clone()) {
                sl.id = new_id();
                seen.insert(sl.id.clone());
            }
        }
        c.slots = slots;
    }
    if let Some(t) = put.ticker {
        if t.chars().count() > MAX_TEXT * 4 {
            return Err(ApiError::invalid("ticker", "the ticker text is too long"));
        }
        c.ticker = t.trim().to_string();
    }
    if let Some(f) = put.ticker_feed {
        if f.is_empty() {
            c.ticker_feed = None;
        } else if s.feeds.iter().any(|x| x.id == f) {
            c.ticker_feed = Some(f);
        } else {
            return Err(api(ErrorCode::NotFound, format!("no feed {f}")));
        }
    }
    Ok(())
}

fn check_slot(sl: &mut SignageSlot) -> Result<(), ApiError> {
    sl.days.sort_unstable();
    sl.days.dedup();
    if sl.days.iter().any(|d| *d > 6) {
        return Err(ApiError::invalid(
            "slots",
            "days are 0 (Monday) to 6 (Sunday)",
        ));
    }
    if sl.start >= schedule::DAY || sl.end == 0 || sl.end > schedule::DAY {
        return Err(ApiError::invalid(
            "slots",
            "a start is 0–1439 minutes, an end 1–1440",
        ));
    }
    for d in [&sl.from, &sl.until].into_iter().flatten() {
        if chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err() {
            return Err(ApiError::invalid(
                "slots",
                format!("{d:?} isn't a YYYY-MM-DD date"),
            ));
        }
    }
    Ok(())
}

// ---- the channels' screens and outputs ----------------------------------------------------

/// A channel's screen's name.
pub fn screen_name(channel: &str) -> String {
    format!("Signage: {channel}")
}

/// Makes, renames and removes the channels' screens and IDN outputs to match the channels;
/// records each channel's screen and source. Problems are logged (the next run tries again).
pub async fn ensure(shared: &Arc<Shared>, show: &crate::actor::ShowHandle) {
    let sg = &shared.signage;
    if !sg.managed() {
        return;
    }
    let _one = sg.ensuring.lock().await;
    let channels = lock(&sg.store).channels.clone();
    let mut found: HashMap<String, (String, String)> = HashMap::new();
    let mut problems: HashMap<String, String> = HashMap::new();
    for ch in &channels {
        let data = shared.data();
        let name = screen_name(&ch.name);
        let screen = data
            .screens
            .iter()
            .find(|s| matches!(&s.kind, ScreenKind::Signage { channel } if *channel == ch.id))
            .cloned();
        let screen: Screen = match screen {
            Some(s) if s.name == name => s,
            Some(mut s) => {
                s.name = name.clone();
                let body = serde_json::to_value(&s).unwrap_or_default();
                match show
                    .configure(ConfigOp::UpdateScreen {
                        id: s.id,
                        body,
                        if_match: None,
                    })
                    .await
                {
                    Ok(_) => s,
                    Err(e) => {
                        problems.insert(
                            ch.id.clone(),
                            format!("its screen can't be renamed: {}", config_error(&e)),
                        );
                        s
                    }
                }
            }
            None => {
                let body = serde_json::json!({
                    "name": name,
                    "kind": {"type": "signage", "channel": ch.id},
                });
                match show.configure(ConfigOp::CreateScreen(body)).await {
                    Ok(crate::config::ConfigResult::Screen(e)) => e.screen,
                    Ok(_) => continue,
                    Err(e) => {
                        problems.insert(
                            ch.id.clone(),
                            format!("its screen can't be made: {}", config_error(&e)),
                        );
                        continue;
                    }
                }
            }
        };
        let data = shared.data();
        let out = data.outputs.iter().find(|o| {
            o.screen == screen.id && matches!(o.transport, model::OutputTransport::Idn { .. })
        });
        match out {
            Some(o) => {
                if !matches!(&o.transport, model::OutputTransport::Idn { source_name, .. } if *source_name == ch.name)
                {
                    let mut o = o.clone();
                    o.name = name.clone();
                    o.transport = model::OutputTransport::Idn {
                        source_name: ch.name.clone(),
                        groups: Vec::new(),
                    };
                    let body = serde_json::to_value(&o).unwrap_or_default();
                    if let Err(e) = show
                        .configure(ConfigOp::UpdateOutput {
                            id: o.id,
                            body,
                            if_match: None,
                        })
                        .await
                    {
                        problems.insert(
                            ch.id.clone(),
                            format!("its output can't be renamed: {}", config_error(&e)),
                        );
                    }
                }
            }
            None => {
                let body = serde_json::json!({
                    "name": name,
                    "screen": screen.id,
                    "transport": {"type": "idn", "sourceName": ch.name},
                });
                if let Err(e) = show.configure(ConfigOp::CreateOutput(body)).await {
                    problems.insert(
                        ch.id.clone(),
                        format!("its output can't be made: {}", config_error(&e)),
                    );
                }
            }
        }
        let source =
            output::idn::full_name(&ch.name).unwrap_or_else(|_| format!("HOST ({})", ch.name));
        found.insert(ch.id.clone(), (screen.id.0.to_string(), source));
    }
    // Screens of channels that are gone.
    let data = shared.data();
    for s in &data.screens {
        if let ScreenKind::Signage { channel } = &s.kind
            && !channels.iter().any(|c| &c.id == channel)
            && let Err(e) = show
                .configure(ConfigOp::DeleteScreen {
                    id: s.id,
                    if_match: None,
                })
                .await
        {
            tracing::warn!("signage: the screen {} can't be removed: {e:?}", s.name);
        }
    }
    for (id, why) in &problems {
        tracing::warn!(channel = %id, "signage: {why}");
    }
    *lock(&sg.setup) = problems;
    // Saved under the store's lock, so a change made meanwhile is never overwritten by an
    // older copy (H02-9).
    let mut moved: Vec<(String, String)> = Vec::new();
    {
        let mut st = lock(&sg.store);
        let mut changed = false;
        for c in &mut st.channels {
            if let Some((screen, source)) = found.get(&c.id)
                && (c.screen != *screen || c.source != *source)
            {
                if !c.source.is_empty() && c.source != *source {
                    moved.push((c.source.clone(), source.clone()));
                }
                c.screen = screen.clone();
                c.source = source.clone();
                changed = true;
            }
        }
        if changed {
            sg.save_now(&st);
        }
    }
    if !moved.is_empty() {
        let sh = shared.clone();
        let _ = tokio::task::spawn_blocking(move || follow_in_mosaic(&sh, &moved)).await;
    }
}

/// A renamed channel's source has a new name: Mosaic's channels, screens and walls that showed
/// the old one show the new one (X3-4), through Mosaic's own changes, so routers and boxes
/// follow. Problems are logged (a box that is away is set again when it answers).
fn follow_in_mosaic(shared: &Shared, moved: &[(String, String)]) {
    let state = shared.mosaic.state();
    let site = &shared.site;
    for (old, new) in moved {
        tracing::info!("signage: the source {old} is now {new}; Mosaic follows");
        let was = protocol::MosaicFeed::Source {
            source: old.clone(),
        };
        let now = protocol::MosaicFeed::Source {
            source: new.clone(),
        };
        for c in state.channels.iter().filter(|c| c.source == *old) {
            if let Err(e) = shared.mosaic.put_channel(site, &c.id, None, Some(new)) {
                tracing::warn!(
                    "signage: Mosaic's channel {} keeps {old}: {}",
                    c.name,
                    e.message
                );
            }
        }
        for w in state.walls.iter().filter(|w| w.feed == was) {
            if let Err(e) = shared.mosaic.set_wall_feed(site, &w.id, now.clone()) {
                tracing::warn!("signage: the wall {} keeps {old}: {}", w.name, e.message);
            }
        }
        for b in &state.boxes {
            for sc in b.screens.iter().filter(|sc| sc.feed == was) {
                // A wall's tile follows its wall (above).
                if state.walls.iter().any(|w| {
                    w.cells
                        .iter()
                        .any(|t| t.machine == b.machine && t.screen == sc.number)
                }) {
                    continue;
                }
                if let Err(e) = shared
                    .mosaic
                    .set_feed(site, &b.machine, sc.number, now.clone())
                {
                    tracing::warn!(
                        "signage: {} screen {} keeps {old}: {}",
                        b.name,
                        sc.number,
                        e.message
                    );
                }
            }
        }
    }
}

/// Starts signage's upkeep: the tick, the reads, and [`ensure`] at start and on every kick.
pub fn start(
    shared: &Arc<Shared>,
    show: crate::actor::ShowHandle,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    let sh = shared.clone();
    let mut stop2 = stop.clone();
    tokio::spawn(async move {
        loop {
            sh.signage.follow(&sh.site);
            ensure(&sh, &show).await;
            sh.signage.announce(&sh.site);
            tokio::select! {
                _ = sh.signage.kick.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
                _ = stop2.wait_for(|s| *s) => break,
            }
        }
    });
    let sh = shared.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut n: u64 = 0;
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = stop.wait_for(|s| *s) => break,
            }
            n += 1;
            if !sh.signage.managed() {
                // No longer the site's server: the screens go blank rather than freeze on
                // their last frame (H02-8).
                if sh.signage.stand_down() {
                    sh.wake_render();
                }
                continue;
            }
            if lock(&sh.signage.store).channels.is_empty() {
                continue;
            }
            let s = sh.clone();
            let r = tokio::task::spawn_blocking(move || {
                let data = s.data();
                let dirty = s.signage.tick(&data, chrono::Local::now(), Instant::now());
                if dirty {
                    s.wake_render();
                }
                if n.is_multiple_of(4) {
                    s.signage.announce(&s.site);
                }
            })
            .await;
            if r.is_err() {
                break;
            }
            if n % 4 == 1 {
                sh.signage.reads_due();
            }
        }
    });
}

/// A configuration change's refusal in words.
fn config_error(e: &crate::config::ConfigError) -> String {
    e.error.message.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fx {
        dir: PathBuf,
        sg: Arc<Signage>,
        data: ShowData,
    }

    impl Drop for Fx {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn fx() -> Fx {
        let dir = std::env::temp_dir().join(format!("vesper-signage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("portable")).unwrap();
        let data = ShowData::load(&dir).unwrap();
        let sg = Arc::new(Signage::load(&dir));
        sg.managed.store(true, Ordering::Release);
        Fx { dir, sg, data }
    }

    fn of<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
        serde_json::from_value(v).unwrap()
    }

    /// A channel with the zones and a ticker playing one announcement.
    fn channel(sg: &Signage, name: &str) -> SignageChannel {
        let p = sg
            .put_playlist(
                None,
                of(json!({"name": "Welcome", "items": [
                    {"content": {"kind": "text", "title": "Welcome", "body": "Coffee after"},
                     "duration": 60}
                ]})),
            )
            .unwrap();
        sg.create_channel(of(json!({"name": name, "defaultPlaylist": p.id,
                                    "layout": "mainSidebarTicker", "ticker": "Hello all"})))
            .unwrap()
    }

    fn frame(sg: &Signage, ch: &str) -> Arc<Frame> {
        lock(&sg.frames).get(ch).cloned().expect("a frame")
    }

    fn local(s: &str) -> chrono::DateTime<chrono::Local> {
        use chrono::TimeZone;
        let t = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        chrono::Local.from_local_datetime(&t).earliest().unwrap()
    }

    /// H02-1: unchanged inputs give the same frame (nothing redrawn, the renderer keeps the
    /// slides' clocks); a new minute changes the clock's text only, under the same slide and
    /// element IDs, so the ticker keeps scrolling and the item keeps playing.
    #[test]
    fn a_frame_changes_only_when_its_content_does() {
        let f = fx();
        let ch = channel(&f.sg, "Lobby");
        let t0 = Instant::now();
        let now = local("2026-11-22 09:00:10");
        assert!(f.sg.tick(&f.data, now, t0), "the first frame");
        let first = frame(&f.sg, &ch.id);
        for k in 1..=4 {
            let at = t0 + TICK * k;
            let now = now + chrono::Duration::milliseconds(250 * i64::from(k));
            assert!(
                !f.sg.tick(&f.data, now, at),
                "tick {k} redrew the same content"
            );
        }
        assert!(Arc::ptr_eq(&first, &frame(&f.sg, &ch.id)));
        // The next minute: the clock changes, the slides stay the same slides.
        assert!(f.sg.tick(&f.data, local("2026-11-22 09:01:00"), t0 + TICK * 5));
        let next = frame(&f.sg, &ch.id);
        assert_ne!(first.fingerprint, next.fingerprint);
        assert_eq!(first.chrome.id, next.chrome.id, "the zones keep their ID");
        let ids = |s: &Slide| s.elements.iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids(&first.chrome), ids(&next.chrome));
        let (a, b) = (first.main.as_ref().unwrap(), next.main.as_ref().unwrap());
        assert_eq!(
            (a.0.id, ids(&a.0)),
            (b.0.id, ids(&b.0)),
            "the item keeps its IDs"
        );
        assert_eq!(
            first.change.seq, next.change.seq,
            "no new change: no transition"
        );
        // Another channel's slides are other slides.
        let other = channel(&f.sg, "Hall");
        f.sg.tick(&f.data, local("2026-11-22 09:01:00"), t0 + TICK * 6);
        assert_ne!(frame(&f.sg, &other.id).chrome.id, next.chrome.id);
    }

    /// H02-2 / X3-1: an override whose time is up while signage.json can't be written is
    /// cleared at once anyway (no recursion), and saved once the file can be written.
    #[test]
    fn an_expired_override_clears_even_when_it_cant_be_saved() {
        let f = fx();
        let ch = channel(&f.sg, "Lobby");
        f.sg.push_override(of(json!({"text": "Closed", "seconds": 600})))
            .unwrap();
        lock(&f.sg.store).over.as_mut().unwrap().until = Some(now_ms() - 1);
        // The data folder can't take the file: its temporary name is a folder.
        let tmp = f.dir.join("machine").join("signage.json.tmp");
        std::fs::create_dir_all(tmp.join("x")).unwrap();
        let now = chrono::Local::now();
        f.sg.tick(&f.data, now, Instant::now());
        assert!(lock(&f.sg.store).over.is_none(), "cleared in memory");
        assert!(f.sg.unsaved.load(Ordering::Acquire));
        assert!(!frame(&f.sg, &ch.id).chrome.elements.is_empty());
        let on_disk = std::fs::read_to_string(f.dir.join("machine/signage.json")).unwrap();
        assert!(on_disk.contains("Closed"), "the file still has it");
        // The next ticks try again; once it can be written, it is.
        f.sg.tick(&f.data, now, Instant::now());
        std::fs::remove_dir_all(&tmp).unwrap();
        f.sg.tick(&f.data, now, Instant::now());
        assert!(!f.sg.unsaved.load(Ordering::Acquire));
        let on_disk = std::fs::read_to_string(f.dir.join("machine/signage.json")).unwrap();
        assert!(!on_disk.contains("Closed"), "{on_disk}");
    }

    /// H02-4: deleting the only channel an override names ends the override (an empty list
    /// would mean every channel); an override of every channel stays.
    #[test]
    fn deleting_an_overrides_last_channel_ends_it() {
        let f = fx();
        let a = channel(&f.sg, "Kids hallway");
        let b = channel(&f.sg, "Lobby");
        f.sg.push_override(of(json!({"text": "Kids wing closed", "channels": [a.id]})))
            .unwrap();
        f.sg.delete_channel(&a.id).unwrap();
        assert!(lock(&f.sg.store).over.is_none());
        let c = channel(&f.sg, "Cafe");
        f.sg.push_override(of(json!({"text": "Welcome"}))).unwrap();
        f.sg.delete_channel(&c.id).unwrap();
        assert!(lock(&f.sg.store).over.is_some(), "an override of all stays");
        let _ = b;
    }

    /// H02-6: a read started before the feed (or the weather's place or unit) changed
    /// doesn't count for it, and the changed one is read at once.
    #[test]
    fn a_read_of_an_old_feed_address_is_dropped() {
        let f = fx();
        let dir = f.dir.join(feeds::FOLDER);
        std::fs::write(dir.join("new.csv"), "Event\nNew\n").unwrap();
        let feed =
            f.sg.put_feed(None, of(json!({"name": "Events", "url": "old.csv"})))
                .unwrap();
        let old = lock(&f.sg.cache).feeds[&feed.id].generation;
        // A read of the old address is under way (it hangs) while the address is corrected.
        lock(&f.sg.reading).insert(format!("{}#{old}", feed.id));
        let mut changed = feed.clone();
        changed.url = "new.csv".into();
        f.sg.put_feed(Some(&feed.id), changed.clone()).unwrap();
        // The old read ends (failing): dropped, the new address is still due.
        assert!(!f.sg.took_feed(&feed, old, Err("timed out".into())));
        let e = lock(&f.sg.cache).feeds[&feed.id].clone();
        assert_eq!((e.tried, e.problem), (0, None));
        // The new address is read at once, the old read still running or not.
        f.sg.reads_due();
        let deadline = Instant::now() + Duration::from_secs(10);
        while lock(&f.sg.cache).feeds[&feed.id].read_at.is_none() {
            assert!(Instant::now() < deadline, "never read");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(lock(&f.sg.cache).feeds[&feed.id].rows[0]["Event"], "New");

        // The weather: a forecast in °C arriving after the switch to °F is dropped.
        let g = lock(&f.sg.cache).weather_generation;
        f.sg.put_settings(of(json!({"fahrenheit": true, "services": []})))
            .unwrap();
        let w: protocol::SignageWeather = of(json!({"temperature": 20.0, "summary": "Sun",
            "high": 22.0, "low": 12.0, "readAt": now_ms(), "credit": "x"}));
        assert!(!f.sg.took_weather(g, Ok(w.clone())));
        assert!(lock(&f.sg.cache).weather.is_none());
        assert!(f.sg.took_weather(g + 1, Ok(w)));
    }

    /// H02-8: a server that is no longer the site's server blanks its channels' screens.
    #[test]
    fn standing_down_blanks_the_screens() {
        let f = fx();
        let ch = channel(&f.sg, "Lobby");
        f.sg.tick(&f.data, chrono::Local::now(), Instant::now());
        assert!(!f.sg.plan(&ch.id).blank);
        assert!(f.sg.stand_down());
        assert!(f.sg.plan(&ch.id).blank);
        assert!(!f.sg.stand_down(), "once");
    }
}
