//! Vesper Mosaic on the site's server (`docs/MOSAIC-PLAN.md`): the site's display boxes (IDN
//! Nodes running IDN's display service), their screens' feeds and settings, and the site-wide
//! channels. Routes: [`crate::api`]'s `mosaic` area.
//!
//! - **Display boxes** are found among the site's devices ([`Mosaic::refresh`]): a machine whose
//!   Node reports the `display` service, or one an admin added. A machine running a Vesper server
//!   is never one: its screens are Vesper's own display outputs. It is listed only when its
//!   display service is on as well, as a conflict ([`vesper_machine`]).
//! - **Kept** in `<data>/machine/mosaic.json`: each box's address, what each screen should show
//!   and its settings, and the channels. Each box keeps its own feeds and settings as well, so it
//!   keeps showing while this server is down.
//! - **Every `poll`** (5 s) each box's state is read; a screen whose feed isn't the one Vesper set
//!   is set again (a box replaced or restarted), and a change of what `GET /mosaic` answers is
//!   announced with `siteChanged` (`mosaic`). A screen whose feed Vesper never set keeps the
//!   feed the box gives it; settings a box didn't take when they changed (it was away) are sent
//!   again until it does.
//! - **Channels** are resolved here: a screen following a channel gets the channel's source as
//!   its feed, and a channel's new source is set on every screen following it. (IDN's routers are
//!   the intended way, on the site server's own Node: a later round, see the plan.)
//! - **Video walls** ([`Mosaic::create_wall`]): a grid of screens (of one box or several)
//!   showing one picture. Applying a wall gives each of its screens the same `settings.wall`
//!   geometry with its own row and column, the same explicit sync group (`wall:` and the wall's
//!   name) and the wall's feed; a screen leaving the wall loses its tile and keeps the feed. The
//!   wall's state is its tiles' worst, and its sync comes from their `stats.sync`. The poll sets
//!   lost settings (a tile's geometry) again, as it does feeds.
//! - **Tokens:** one per box, issued by the Site Manager this server hosts (engine `display`, the
//!   box's fingerprint, scope `control`, an hour), renewed ten minutes before they end.
//! - **Certificates:** a box is reached over HTTPS accepting only its own LAN certificate
//!   (`Authority::lan_client_config` with its fingerprint), at any address ([`client`]).
//!
//! Only the site's server (the one running the Site Manager) manages displays.

pub mod client;
pub mod node;

pub use node::{IdnNodes, Nodes, Transfer};

use crate::site::Site;
use crate::{Shared, lock};
use idn_site_manager::typed::TokenAud;
use protocol::{
    ApiError, DeviceHealth, ErrorCode, MosaicAddressFrom, MosaicBox, MosaicChannel, MosaicFeed,
    MosaicLogo, MosaicLogoState, MosaicMachine, MosaicScreen, MosaicScreenSettings,
    MosaicScreenStatus, MosaicServiceMode, MosaicState, MosaicSync, MosaicTile, MosaicWall,
    MosaicWallCell, PutMosaicWall, SiteDevice, SitePart,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The site user the boxes' tokens are issued to.
pub const TOKEN_USER: &str = "vesper-mosaic";
/// A box's token lives this long …
const TOKEN_TTL: Duration = Duration::from_secs(3600);
/// … and is renewed when less than this is left.
const TOKEN_RENEW: Duration = Duration::from_secs(600);
/// How often each box is read by default.
pub const DEFAULT_POLL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct StoredScreen {
    /// `None`: never set from Vesper (the box's own feed is left alone).
    #[serde(skip_serializing_if = "Option::is_none")]
    feed: Option<MosaicFeed>,
    /// `None`: never set from Vesper (the box's own are left alone).
    #[serde(skip_serializing_if = "Option::is_none")]
    settings: Option<MosaicScreenSettings>,
    /// Its settings command ([`Store::settings_command`], a wall's tile going included) changed
    /// and the box hasn't taken it yet: the poll sends it until it does.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    unsent: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct StoredBox {
    /// Set by an admin.
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<String>,
    /// The address its join request came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    joined_from: Option<String>,
    /// Added by an admin (its Node doesn't report the display service).
    added: bool,
    screens: BTreeMap<u32, StoredScreen>,
    /// The hash of the site's logo it was sent last ([`Mosaic::put_logo`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    logo: Option<String>,
    /// The name that logo was sent under (at the top of its media folder: `logo-a.png` or
    /// `logo-b.png`, replaced on the box, or `logo-<hash>.png` for a box from before IDN
    /// Beta14), and when (Unix seconds).
    #[serde(skip_serializing_if = "Option::is_none")]
    logo_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    logo_sent_at: Option<u64>,
    /// The offer it went under as a replacement (`replace="true"`, IDN Beta14 boxes): its
    /// screens are given it once this machine's Node says the box fetched all of it
    /// (`logo_arrived`). `None`: sent the old way, its screens given it [`LOGO_SETTLE`] after.
    #[serde(skip_serializing_if = "Option::is_none")]
    logo_offer: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    logo_arrived: bool,
    /// The name its screens were given last as their fallback logo (a replacement that
    /// arrived): the next logo goes under the other name ([`next_logo_name`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    logo_shown: Option<String>,
    /// How Vesper last switched its display service ([`Mosaic::set_service`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    service_mode: Option<MosaicServiceMode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredChannel {
    id: String,
    name: String,
    source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredWall {
    id: String,
    name: String,
    rows: u16,
    columns: u16,
    bezel_mm: [f32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    screen_mm: Option<[f32; 2]>,
    #[serde(default)]
    feed: MosaicFeed,
    #[serde(default)]
    cells: Vec<MosaicWallCell>,
}

/// The longest sync group the display service takes, bytes.
const GROUP_MAX: usize = 64;

impl StoredWall {
    /// Its sync group: one per wall, named after it (cut to [`GROUP_MAX`] bytes for a wall named
    /// before names were limited to fit).
    fn group(&self) -> String {
        let mut g = format!("wall:{}", self.name);
        if g.len() > GROUP_MAX {
            let mut end = GROUP_MAX;
            while !g.is_char_boundary(end) {
                end -= 1;
            }
            g.truncate(end);
        }
        g
    }

    /// The tile of the screen in `cell`.
    fn tile(&self, cell: &MosaicWallCell) -> MosaicTile {
        MosaicTile {
            rows: self.rows,
            columns: self.columns,
            row: cell.row,
            column: cell.column,
            bezel_mm: self.bezel_mm,
            screen_mm: self.screen_mm,
            group: Some(self.group()),
        }
    }

    fn has(&self, machine: &str, n: u32) -> bool {
        self.cells
            .iter()
            .any(|c| c.machine == machine && c.screen == n)
    }
}

/// `mosaic.json`'s format: 1 since a screen's feed can be "never set" (absent).
const STORE_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Store {
    version: u32,
    boxes: BTreeMap<String, StoredBox>,
    channels: Vec<StoredChannel>,
    walls: Vec<StoredWall>,
    /// The IDN Nodes of the site's Vesper boxes, by the reporting box's fingerprint, as each
    /// box reports its own ([`Mosaic::note_vesper_node`]): not display boxes while the box keeps
    /// reporting. (The older `vesperNodes`, kept forever and without a reporter, is dropped:
    /// a running box reports again within [`crate::codex::site_presence::REPORT_EVERY`].)
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    vesper_boxes: BTreeMap<String, VesperNode>,
}

/// A Vesper box's IDN Node, as the box last reported it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct VesperNode {
    /// The Node's fingerprint.
    node: String,
    /// The box's name.
    name: String,
    /// When it last reported (Unix seconds).
    seen: u64,
}

/// A machine stops counting as a Vesper machine this long after its Vesper server was last
/// heard from (its presence report, or its `(Vesper)` device online): it no longer runs one.
pub const VESPER_GONE: Duration = Duration::from_secs(15 * 60);
/// A Vesper server reporting within this long still runs (it reports every 20 s): what counts
/// on a machine whose Launcher has the display role.
const VESPER_LIVE: Duration = Duration::from_secs(60);

/// What says which machines run a Vesper server ([`vesper_machine`]): the site's Vesper boxes'
/// names and Nodes, all (heard from within [`VESPER_GONE`]) and live (running now), and the
/// machines whose Launcher has the display role (for them only live evidence counts).
#[derive(Clone, Debug, Default)]
struct VesperEvidence {
    names: Vec<String>,
    names_live: Vec<String>,
    nodes: Vec<String>,
    nodes_live: Vec<String>,
    display_boxes: Vec<String>,
}

impl VesperEvidence {
    /// Whether `d` runs a Vesper server.
    fn runs_vesper(&self, d: &SiteDevice, own_name: &str) -> bool {
        let display_box = self
            .display_boxes
            .iter()
            .any(|n| n.eq_ignore_ascii_case(d.name.trim()));
        if display_box {
            vesper_machine(d, &self.names_live, own_name, &self.nodes_live)
        } else {
            vesper_machine(d, &self.names, own_name, &self.nodes)
        }
    }
}

/// An RFC 3339 time's Unix seconds.
fn unix_of(t: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(t)
        .ok()
        .and_then(|t| u64::try_from(t.timestamp()).ok())
}

impl Store {
    /// A file of a format before [`STORE_VERSION`]: a feed of "none" outside a wall was also
    /// what a screen got when only its settings were set, so it is taken as never set.
    fn upgrade(&mut self) {
        if self.version < 1 {
            let walled: Vec<(String, u32)> = self
                .walls
                .iter()
                .flat_map(|w| w.cells.iter().map(|c| (c.machine.clone(), c.screen)))
                .collect();
            for (m, b) in &mut self.boxes {
                for (n, sc) in &mut b.screens {
                    if sc.feed == Some(MosaicFeed::None) && !walled.contains(&(m.clone(), *n)) {
                        sc.feed = None;
                    }
                }
            }
        }
        self.version = STORE_VERSION;
    }

    fn screen_mut(&mut self, machine: &str, n: u32) -> &mut StoredScreen {
        self.boxes
            .entry(machine.to_string())
            .or_default()
            .screens
            .entry(n)
            .or_default()
    }

    /// The settings Vesper set for a screen (not its wall's tile).
    fn own_settings(&self, machine: &str, n: u32) -> Option<&MosaicScreenSettings> {
        self.boxes
            .get(machine)
            .and_then(|b| b.screens.get(&n))
            .and_then(|s| s.settings.as_ref())
    }

    /// The tile of a screen in a wall.
    fn tile_of(&self, machine: &str, n: u32) -> Option<MosaicTile> {
        self.wall_of(machine, n).and_then(|w| {
            w.cells
                .iter()
                .find(|c| c.machine == machine && c.screen == n)
                .map(|c| w.tile(c))
        })
    }

    /// The settings `GET /mosaic` reports for a screen: Vesper's ([`Store::settings_of`]), else
    /// the box's own as it last said (`has`), with its wall's tile when it is in one.
    fn reported_settings(
        &self,
        machine: &str,
        n: u32,
        has: Option<&serde_json::Value>,
    ) -> MosaicScreenSettings {
        if self.own_settings(machine, n).is_some() {
            return self.settings_of(machine, n).unwrap_or_default();
        }
        let mut s = has.map(client::read_settings).unwrap_or_default();
        if let Some(t) = self.tile_of(machine, n) {
            s.tile = Some(t);
        }
        s
    }

    /// The wall a screen is a tile of.
    fn wall_of(&self, machine: &str, n: u32) -> Option<&StoredWall> {
        self.walls.iter().find(|w| w.has(machine, n))
    }

    /// The settings a screen should have: its own, with its wall's tile when it is in one
    /// (`None`: never set from Vesper and in no wall).
    fn settings_of(&self, machine: &str, n: u32) -> Option<MosaicScreenSettings> {
        let own = self
            .boxes
            .get(machine)
            .and_then(|b| b.screens.get(&n))
            .and_then(|s| s.settings.clone());
        match self.wall_of(machine, n) {
            Some(w) => {
                let cell = w
                    .cells
                    .iter()
                    .find(|c| c.machine == machine && c.screen == n)?;
                let mut s = own.unwrap_or_default();
                s.tile = Some(w.tile(cell));
                Some(s)
            }
            None => own,
        }
    }

    /// The service's `settings` command for a screen: Vesper's settings whole when they were set
    /// from Vesper (with its wall's tile), else only its wall tile, so the box's own settings
    /// are left alone (`wall: null` when it is `leaving` a wall); `None`: nothing to send.
    fn settings_command(&self, machine: &str, n: u32, leaving: bool) -> Option<serde_json::Value> {
        if self.own_settings(machine, n).is_some() {
            return self
                .settings_of(machine, n)
                .map(|st| client::settings_body(n, &st));
        }
        match self.tile_of(machine, n) {
            Some(t) => Some(serde_json::json!({"screen": n, "wall": client::wall_json(&t)})),
            None if leaving => Some(serde_json::json!({"screen": n, "wall": null})),
            None => None,
        }
    }
}

/// A machine of the site that is (or conflicts as) a display box, as last seen.
#[derive(Clone, Debug)]
struct Candidate {
    device: SiteDevice,
    /// The display service's state (empty: not reported).
    service: String,
    /// It runs a Vesper server.
    vesper: bool,
    /// The address the Site Manager sees its IDN Node at (its trust connection).
    reported: Option<String>,
    /// Its system is Linux (the display service can run headless).
    headless_ok: bool,
    /// Its display service's mode as its Node reports it (`<apps display display_boot>`).
    mode_reported: Option<MosaicServiceMode>,
}

/// What the last read of a box gave.
#[derive(Clone, Debug, Default)]
struct Live {
    /// Its display service answered (`problem` may still be its own error).
    reachable: bool,
    problem: Option<String>,
    screens: Vec<client::ScreenState>,
    /// The last change a screen refused, and when (cleared when one is taken, or when the box
    /// is read as Vesper wants it after that).
    errors: BTreeMap<u32, (String, Instant)>,
    /// Each screen's late pictures (`stats.sync.late`) at the read before, and since then.
    late_before: BTreeMap<u32, u64>,
    late_recently: BTreeMap<u32, u64>,
}

/// Where a box's display service is reached ([`Mosaic::target`]).
struct Target {
    base: String,
    token: String,
    tls: Option<Arc<rustls::ClientConfig>>,
}

impl Target {
    fn client(&self) -> client::Client<'_> {
        client::Client {
            base: &self.base,
            token: &self.token,
            tls: self.tls.as_ref(),
        }
    }
}

/// Mosaic on this server.
pub struct Mosaic {
    path: PathBuf,
    /// How often each box is read.
    pub poll: Duration,
    managed: AtomicBool,
    store: Mutex<Store>,
    candidates: Mutex<Vec<Candidate>>,
    live: Mutex<HashMap<String, Live>>,
    tokens: Mutex<HashMap<String, (String, Instant)>>,
    last: Mutex<Option<MosaicState>>,
    /// Wakes the poll loop (a box added, its address changed).
    pub kick: tokio::sync::Notify,
    /// How long after a box's Node accepted the logo its screens are given it
    /// ([`LOGO_SETTLE`]; tests shorten it).
    pub logo_settle: Duration,
    /// This machine's IDN Node and other machines' Node settings.
    nodes: Arc<dyn Nodes>,
    /// The channels' routers on this server's Node.
    routing: Mutex<Routing>,
    /// The site's other Node machines (not boxes, not Vesper's).
    machines: Mutex<Vec<MosaicMachine>>,
    /// The site's logo as kept (`None`: none).
    logo: Mutex<Option<MosaicLogo>>,
    /// Machines whose Launcher reports the display role (lower-case machine name → when).
    display_roles: Mutex<HashMap<String, Instant>>,
    /// How pushing the logo went lately.
    logo_push: Mutex<LogoPush>,
    /// This machine's Node answered a `replace` send without an offer: it is from before IDN
    /// Beta14 (logos go under a name of their own, as before).
    own_node_legacy: AtomicBool,
}

/// The two names the site's logo is sent under to a box that replaces files (IDN Beta14 and
/// later: `media_send replace="true"`), taking turns: a screen already showing a logo reads
/// its file again only when its path changes (the IDN maintainers' answer, 2026-11-26), so a new
/// logo goes under the name not in use and the screens are then pointed at it.
const LOGO_NAMES: [&str; 2] = ["logo-a.png", "logo-b.png"];

/// The name the next logo goes under to a Beta14 box whose screens show `shown`: the other
/// one of [`LOGO_NAMES`] (`logo-a.png` after anything else).
fn next_logo_name(shown: Option<&str>) -> &'static str {
    if shown == Some(LOGO_NAMES[0]) {
        LOGO_NAMES[1]
    } else {
        LOGO_NAMES[0]
    }
}

/// The name the site's logo is sent under to a box from before IDN Beta14 (one per logo: its
/// Node never overwrites a file, and refuses `replace`).
fn logo_file_name(hash: &str) -> String {
    format!("logo-{hash}.png")
}

/// A name [`logo_file_name`] makes.
fn is_logo_file_name(n: &str) -> bool {
    n.strip_prefix("logo-")
        .and_then(|r| r.strip_suffix(".png"))
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|c| c.is_ascii_hexdigit()))
}

/// How pushing the logo went lately.
#[derive(Default)]
struct LogoPush {
    /// Why it can't be pushed now.
    problem: Option<String>,
    /// The boxes it was tried on lately (tried again after [`LOGO_RETRY`]).
    tried: HashMap<String, Instant>,
    /// Each box's refusal.
    refused: HashMap<String, String>,
    /// Each box's replacement being followed: when its fetching last moved.
    progress: HashMap<String, Progress>,
}

/// How far a box fetching a logo offered as a replacement got, and since when (H03-2).
#[derive(Debug)]
struct Progress {
    offer: String,
    /// The transfer's state and bytes fetched (`None`: not known).
    seen: Option<Transfer>,
    since: Instant,
}

impl Progress {
    /// Notes what `offer`'s transfer looks like at `now`; how long it has looked so (a new
    /// offer, state or byte count starts again).
    fn observe(
        map: &mut HashMap<String, Progress>,
        fp: &str,
        offer: &str,
        seen: Option<&Transfer>,
        now: Instant,
    ) -> Duration {
        let p = map.entry(fp.to_string()).or_insert_with(|| Progress {
            offer: offer.to_string(),
            seen: seen.cloned(),
            since: now,
        });
        if p.offer != offer || p.seen.as_ref() != seen {
            *p = Progress {
                offer: offer.to_string(),
                seen: seen.cloned(),
                since: now,
            };
        }
        now.saturating_duration_since(p.since)
    }
}

/// The largest logo taken, bytes.
pub const LOGO_MAX: usize = 8 << 20;
/// A box that didn't take the logo is tried again after this.
const LOGO_RETRY: Duration = Duration::from_secs(60);
/// A logo sent to a box from before IDN Beta14 is taken as there this long after its Node
/// accepted it (`offered`): whether it arrived shows only on that box (its own "received"),
/// which the Site Manager doesn't pass on. Its screens are given it then (and again until they
/// have it). A Beta14 box's transfer is followed to `sent` instead.
pub const LOGO_SETTLE: Duration = Duration::from_secs(15);
/// A logo offered as a replacement whose fetching hasn't moved for this long (no more bytes,
/// or no longer listed) is sent again; one the box is still fetching is waited for (H03-2: two
/// transfers of one name never run at once).
const LOGO_STUCK: Duration = Duration::from_secs(120);

/// Why a transfer's state (`Transfer::state`) says it ended without the file arriving, as
/// shown: `refused`, `failed: <why>` (IDN Beta21: `failed: interrupted` for one that ended
/// without a result, its sender or the box gone meanwhile); `None` while it goes or once `sent`.
fn transfer_failed(state: &str) -> Option<String> {
    match state {
        "refused" => Some("the box refused it".into()),
        "failed: interrupted" => Some("the transfer was interrupted".into()),
        s => s
            .strip_prefix("failed")
            .map(|why| why.trim_start_matches(':').trim())
            .map(|why| {
                if why.is_empty() {
                    "the transfer failed"
                } else {
                    why
                }
                .to_string()
            }),
    }
}

/// Whether an IDN Node of build `version` (`0.3.4-Beta14 …`, as its report says) takes
/// `media_send replace="true"` (Beta14 and later); `None`: not known.
fn takes_replace(version: Option<&str>) -> Option<bool> {
    let v = version?.split_whitespace().next()?.trim_start_matches('v');
    let (core, tag) = v.split_once('-').unwrap_or((v, ""));
    let mut n = core.split('.').map(|x| x.parse::<u32>().ok());
    let core = (n.next()??, n.next()??, n.next()??);
    Some(match core.cmp(&(0, 3, 4)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal if tag.is_empty() => true,
        std::cmp::Ordering::Equal => tag
            .strip_prefix("Beta")
            .and_then(|b| b.parse::<u32>().ok())
            .is_some_and(|b| b >= 14),
    })
}

/// A Node setting's yes/no (`on`/`off`, `true`/`false`).
fn yes(v: Option<&str>) -> bool {
    matches!(
        v.map(|x| x.trim().to_ascii_lowercase()).as_deref(),
        Some("on" | "true" | "1" | "yes")
    )
}

/// A display service's mode as the box's Node reports it (what the Site Manager's status passes
/// on as `machines[].node.settings`, IDN Beta14): its `<service id="display" on runs>` where it
/// says (`runs="node"`: from boot, headless; `session`: in the desktop session), else its
/// `<apps display display_boot>` (headless needs both on, the desktop session `display` alone).
fn reported_mode(report: &idn_open_protocol::xml::Element) -> Option<MosaicServiceMode> {
    if let Some(svc) = report
        .children_named("service")
        .find(|s| s.attr("id") == Some("display"))
        && svc.attr("on").is_some()
    {
        if !yes(svc.attr("on")) {
            return Some(MosaicServiceMode::Off);
        }
        match svc.attr("runs") {
            Some("node") => return Some(MosaicServiceMode::Headless),
            Some("session") => return Some(MosaicServiceMode::Desktop),
            _ => {}
        }
    }
    let apps = report.child("apps")?;
    apps.attr("display")?;
    Some(
        match (yes(apps.attr("display")), yes(apps.attr("display_boot"))) {
            (true, true) => MosaicServiceMode::Headless,
            (true, false) => MosaicServiceMode::Desktop,
            _ => MosaicServiceMode::Off,
        },
    )
}

/// The same `routers.list`, whatever the separators and order.
fn same_routers(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        let mut v: Vec<String> = s
            .split([';', '\n'])
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect();
        v.sort();
        v
    };
    norm(a) == norm(b)
}

/// Unix seconds now.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A settings command (or none) with the site's logo as the screen's fallback logo: `file`
/// when the box has it (its `fallback` taken from `body`, else as the box has it), else the
/// logo the box has kept, so that a `fallback` sent whole never drops it. What the box has and
/// Vesper can't say is kept too ([`keep_box_extras`]).
fn with_logo(
    body: Option<serde_json::Value>,
    n: u32,
    file: Option<&str>,
    has: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut b = match (body, file) {
        (Some(b), _) => b,
        (None, Some(_)) => serde_json::json!({"screen": n}),
        (None, None) => return None,
    };
    keep_box_extras(&mut b, has);
    if let Some(f) = file {
        if !b["fallback"].is_object() {
            b["fallback"] = has
                .and_then(|h| h.get("fallback"))
                .filter(|f| f.is_object())
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
        }
        b["fallback"]["logo"] = serde_json::json!(f);
    }
    Some(b)
}

/// The service replaces `fallback` and `audio` whole: what the box has in them that Vesper
/// can't say is kept in a command that sends them (X2-5): the fallback's logo, slideshow
/// folder, `slide_s` and `reconnect_cap`, a slideshow where Vesper says "logo" (Vesper reads a
/// slideshow as the logo), and the sound's channel map while the device stays the same.
fn keep_box_extras(b: &mut serde_json::Value, has: Option<&serde_json::Value>) {
    let Some(has) = has else { return };
    if let (Some(bf), Some(hf)) = (
        b.get_mut("fallback").and_then(|f| f.as_object_mut()),
        has.get("fallback").and_then(|f| f.as_object()),
    ) {
        for (k, v) in hf {
            if !bf.contains_key(k) {
                bf.insert(k.clone(), v.clone());
            }
        }
        if bf.get("show").and_then(|s| s.as_str()) == Some("logo")
            && hf.get("show").and_then(|s| s.as_str()) == Some("slideshow")
        {
            bf.insert("show".into(), "slideshow".into());
        }
    }
    if let (Some(ba), Some(ha)) = (
        b.get_mut("audio").and_then(|a| a.as_object_mut()),
        has.get("audio").and_then(|a| a.as_object()),
    ) && ba.get("device") == ha.get("device")
    {
        for (k, v) in ha {
            if !ba.contains_key(k) {
                ba.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Two settings commands alike but for the fallback's logo ([`with_logo`]).
fn same_command(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    let strip = |v: &serde_json::Value| {
        let mut v = v.clone();
        if let Some(f) = v.get_mut("fallback").and_then(|f| f.as_object_mut()) {
            f.remove("logo");
            if f.is_empty() {
                v.as_object_mut().map(|o| o.remove("fallback"));
            }
        }
        v
    };
    strip(a) == strip(b)
}

/// Whether a machine's system is Linux, as its Node reports it. The report names the
/// distribution (`Ubuntu`, `Zorin OS`, `Pop!_OS`) or `Windows 11 Pro`, so anything that isn't
/// Windows or macOS counts as Linux; unknown (no report) doesn't.
fn linux(d: &SiteDevice) -> bool {
    d.machine
        .as_ref()
        .and_then(|m| m.os.as_deref())
        .map(|os| os.trim().to_ascii_lowercase())
        .is_some_and(|os| {
            !os.is_empty()
                && !["windows", "macos", "mac os", "os x", "darwin"]
                    .iter()
                    .any(|x| os.contains(x))
        })
}

/// A logo's hash (its PNG's).
fn logo_hash(png: &[u8]) -> String {
    use sha2::Digest;
    let d = sha2::Sha256::digest(png);
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// The channels' routers on the site server's IDN Node, as last set by this run.
#[derive(Clone, Debug, Default)]
struct Routing {
    /// The Node they are on (its fingerprint).
    host: Option<String>,
    /// The `routers.list` it took. `None`: none set.
    applied: Option<Routers>,
    /// Why they aren't set.
    problem: Option<String>,
    /// The list last refused, and when.
    tried: Option<(Routers, Instant)>,
}

/// Routers on the site server's Node: router name → (channel id, source).
type Routers = BTreeMap<String, (String, String)>;

/// A refused `routers.list` is sent again after this (or at once when the channels change).
const ROUTERS_RETRY: Duration = Duration::from_secs(30);

/// A channel's router name on the site server's Node (`Channel Main`, published as `HOST
/// (Channel Main)`): "Channel" before it, so it can't take the name of one of the server's own
/// sources; no `=`, `;`, brackets or control characters (the setting's syntax).
fn router_name(channel: &str) -> String {
    let clean: String = channel
        .chars()
        .map(|c| {
            if matches!(c, '=' | ';' | '(' | ')') || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("Channel {clean}").trim().to_string()
}

/// Whether a source can go into `routers.list` / `routers.route` (`Name=SOURCE; …`): not
/// with a `;` (the entries' separator) or a line break in its name.
fn routable(source: &str) -> bool {
    !source.contains([';', '\n', '\r'])
}

/// The routers the channels need: router name → (channel id, source); a channel whose router
/// name another one has already, or whose source can't be written in the setting
/// ([`routable`]), isn't on a router (it is resolved per screen).
fn wanted_routers(store: &Store) -> Routers {
    let mut chs: Vec<&StoredChannel> = store
        .channels
        .iter()
        .filter(|c| routable(&c.source))
        .collect();
    chs.sort_by(|a, b| a.id.cmp(&b.id));
    let mut out = BTreeMap::new();
    for c in chs {
        out.entry(router_name(&c.name))
            .or_insert_with(|| (c.id.clone(), c.source.clone()));
    }
    out
}

/// `routers.list`'s value: `Name=SOURCE; …` (an empty source: nowhere yet).
fn routers_list(want: &Routers) -> String {
    want.iter()
        .map(|(name, (_, src))| format!("{name}={src}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// What a feed shows now ([`Mosaic::source_of`]).
fn resolve(store: &Store, routing: &Routing, nodes: &dyn Nodes, feed: &MosaicFeed) -> String {
    match feed {
        MosaicFeed::None => String::new(),
        MosaicFeed::Source { source } => source.clone(),
        MosaicFeed::Channel { channel } => {
            let Some(c) = store.channels.iter().find(|c| &c.id == channel) else {
                return String::new();
            };
            match routed(routing, channel) {
                Some(router) => nodes.source_name(&router),
                None => c.source.clone(),
            }
        }
    }
}

/// The router a channel is on, if it is.
fn routed(routing: &Routing, channel: &str) -> Option<String> {
    routing.host.as_ref()?;
    routing
        .applied
        .as_ref()?
        .iter()
        .find(|(_, (id, _))| id == channel)
        .map(|(name, _)| name.clone())
}

fn api(code: ErrorCode, m: impl Into<String>) -> ApiError {
    ApiError::new(code, m)
}

/// Whether a device runs a Vesper server (see the module docs): a Vesper box of the site
/// (`<name> (Vesper)`), the IDN Node of one (`vesper_nodes`: each Vesper server reads its own
/// Node's fingerprint and reports it), a Node reporting a Vesper service, this server's own
/// machine, or a Node named like one of the site's Vesper boxes (for a box that doesn't report
/// its Node: an older one, or one without a Node yet).
pub fn vesper_machine(
    d: &SiteDevice,
    vesper_names: &[String],
    own_name: &str,
    vesper_nodes: &[String],
) -> bool {
    let name = d.name.trim();
    crate::site::is_vesper_box(&d.kind, &d.name)
        || vesper_nodes.iter().any(|f| f == &d.fingerprint)
        || d.built_in
        || d.services.iter().any(|s| {
            s.id.to_ascii_lowercase().contains("vesper")
                || s.name.to_ascii_lowercase().contains("vesper")
        })
        || name.eq_ignore_ascii_case(own_name.trim())
        || vesper_names.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// A display service state that means it runs (or tries to).
fn service_on(state: &str) -> bool {
    !matches!(state, "" | "off" | "unset" | "unsupported")
}

impl Mosaic {
    /// Reads `<data>/machine/mosaic.json` (a damaged one is kept aside and started afresh).
    pub fn load(data_dir: &Path, poll: Duration, nodes: Arc<dyn Nodes>) -> Self {
        let path = data_dir.join("machine").join("mosaic.json");
        let store = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                tracing::warn!(
                    "{}: {e}; starting afresh (kept as .damaged)",
                    path.display()
                );
                let _ = std::fs::rename(&path, path.with_extension("json.damaged"));
                Store::default()
            }),
            Err(_) => Store::default(),
        };
        let mut store = store;
        store.upgrade();
        Mosaic {
            path,
            poll,
            managed: AtomicBool::new(false),
            store: Mutex::new(store),
            candidates: Mutex::new(Vec::new()),
            live: Mutex::new(HashMap::new()),
            tokens: Mutex::new(HashMap::new()),
            last: Mutex::new(None),
            kick: tokio::sync::Notify::new(),
            logo_settle: LOGO_SETTLE,
            nodes,
            routing: Mutex::new(Routing::default()),
            machines: Mutex::new(Vec::new()),
            logo: Mutex::new(None),
            logo_push: Mutex::new(LogoPush::default()),
            display_roles: Mutex::new(HashMap::new()),
            own_node_legacy: AtomicBool::new(false),
        }
        .with_logo()
    }

    fn logo_path(&self) -> PathBuf {
        self.path.with_file_name("mosaic-logo.png")
    }

    /// Reads the kept logo's details.
    fn with_logo(self) -> Self {
        if let Ok(png) = std::fs::read(self.logo_path())
            && let Ok(img) = image::load_from_memory(&png)
        {
            *lock(&self.logo) = Some(MosaicLogo {
                hash: logo_hash(&png),
                width: img.width(),
                height: img.height(),
            });
        }
        self
    }

    /// `GET /mosaic/logo`: the site's logo (PNG).
    pub fn logo_png(&self) -> Option<Vec<u8>> {
        lock(&self.logo).as_ref()?;
        std::fs::read(self.logo_path()).ok()
    }

    /// `PUT /mosaic/logo` (blocking): the site's logo (PNG or JPEG, kept as PNG), pushed to every
    /// box on the next rounds.
    pub fn put_logo(&self, site: &Site, bytes: &[u8]) -> Result<(), ApiError> {
        self.require_managed()?;
        if bytes.len() > LOGO_MAX {
            return Err(api(ErrorCode::BadRequest, "the logo: at most 8 MB"));
        }
        let img = image::load_from_memory(bytes).map_err(|e| {
            api(
                ErrorCode::BadRequest,
                format!("the logo: a PNG or JPEG ({e})"),
            )
        })?;
        if img.width() > 7680 || img.height() > 4320 {
            return Err(api(
                ErrorCode::BadRequest,
                "the logo: at most 7680 × 4320 pixels",
            ));
        }
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(|e| api(ErrorCode::Internal, format!("the logo: {e}")))?;
        let path = self.logo_path();
        let tmp = path.with_extension("png.tmp");
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))
            .and_then(|()| std::fs::write(&tmp, &png))
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| api(ErrorCode::Internal, format!("the logo: {e}")))?;
        *lock(&self.logo) = Some(MosaicLogo {
            hash: logo_hash(&png),
            width: img.width(),
            height: img.height(),
        });
        {
            let mut lp = lock(&self.logo_push);
            lp.tried.clear();
            lp.refused.clear();
        }
        self.kick.notify_one();
        self.announce(site);
        Ok(())
    }

    /// `DELETE /mosaic/logo`: no logo pushed any more (the boxes keep the one they have).
    pub fn delete_logo(&self, site: &Site) -> Result<(), ApiError> {
        self.require_managed()?;
        if lock(&self.logo).take().is_none() {
            return Err(api(ErrorCode::NotFound, "no logo"));
        }
        let _ = std::fs::remove_file(self.logo_path());
        self.announce(site);
        Ok(())
    }

    /// Sends the site's logo to every box online in the site that hasn't been sent it
    /// (blocking), from the top of this machine's Node's media folder, with `media_send`: under
    /// the one of `logo-a.png` / `logo-b.png` its screens don't show ([`next_logo_name`]) with
    /// `replace="true"` (IDN Beta14: the box replaces that file once the new one is whole and
    /// checked), followed to `sent`, then each of the box's screens is pointed at it as its
    /// fallback logo (`settings.fallback.logo`, relative: the box's media folder), in
    /// [`Mosaic::sync_box`]: the new path makes a screen showing the logo read it at once. A box from before Beta14 (its build, or its refusing the
    /// replacement) gets it under a name of its own (`logo-<hash>.png`: its Node never
    /// overwrites a file, it would keep a "logo (2).png") and its screens [`LOGO_SETTLE`] later,
    /// trusting the box (its sender can't tell: such a box lets the stream go early).
    fn push_logo(&self) {
        let Some(info) = lock(&self.logo).clone() else {
            lock(&self.logo_push).problem = None;
            return;
        };
        self.follow_logo_transfers(&info.hash);
        let legacy_here = self.own_node_legacy.load(Ordering::Relaxed);
        let targets: Vec<(String, String, bool, &'static str)> = {
            let s = lock(&self.store);
            let pushed = lock(&self.logo_push);
            let tried = &pushed.tried;
            lock(&self.candidates)
                .iter()
                .filter(|c| {
                    !c.vesper
                        && matches!(
                            c.device.health,
                            DeviceHealth::Online | DeviceHealth::NeedsAttention
                        )
                        && s.boxes
                            .get(&c.device.fingerprint)
                            .and_then(|b| b.logo.as_ref())
                            != Some(&info.hash)
                        && tried
                            .get(&c.device.fingerprint)
                            .is_none_or(|t| t.elapsed() >= LOGO_RETRY)
                })
                .map(|c| {
                    let shown = s
                        .boxes
                        .get(&c.device.fingerprint)
                        .and_then(|b| b.logo_shown.as_deref());
                    (
                        c.device.fingerprint.clone(),
                        c.device.name.clone(),
                        !legacy_here && takes_replace(c.device.version.as_deref()) != Some(false),
                        next_logo_name(shown),
                    )
                })
                .collect()
        };
        if targets.is_empty() {
            return;
        }
        let problem = |p: String| lock(&self.logo_push).problem = Some(p);
        if self.nodes.own().is_none() {
            problem("No IDN Node runs on this server: the logo can't be sent to the boxes".into());
            return;
        }
        let name = logo_file_name(&info.hash);
        let dir = match self.nodes.media_dir() {
            Ok(dir) => dir,
            Err(e) => return problem(format!("The logo can't be sent: {e}")),
        };
        let file = dir.join(&name);
        let Some(png) = self.logo_png() else { return };
        for f in LOGO_NAMES.iter().map(|n| dir.join(n)).chain([file.clone()]) {
            if std::fs::read(&f).ok().as_deref() == Some(&png[..]) {
                continue;
            }
            let w = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&f, &png));
            if let Err(e) = w {
                return problem(format!(
                    "The logo can't be put in this machine's media folder ({}): {e}",
                    f.display()
                ));
            }
        }
        // The logos sent before under names of their own (only Vesper's).
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n != name && is_logo_file_name(&n) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let mut failed = None;
        for (fp, box_name, modern, next) in targets {
            let sent = 'send: {
                if modern && !self.own_node_legacy.load(Ordering::Relaxed) {
                    match self.nodes.send_media(&dir.join(next), &fp, true) {
                        Ok(Some(offer)) => break 'send Ok((next.to_string(), Some(offer))),
                        Ok(None) => {
                            // This machine's Node is from before Beta14 (it sent the file
                            // without replacing): names of their own from now on.
                            tracing::info!(
                                "mosaic: this machine's IDN Node doesn't replace files (before Beta14): logos go under names of their own"
                            );
                            self.own_node_legacy.store(true, Ordering::Relaxed);
                        }
                        // A box from before Beta14 refuses a replacement.
                        Err(e) => tracing::info!(
                            "mosaic: {box_name} didn't take the logo as a replacement ({e}); sending it under a name of its own"
                        ),
                    }
                }
                self.nodes
                    .send_media(&file, &fp, false)
                    .map(|_| (name.clone(), None))
            };
            match sent {
                Ok((sent_name, offer)) => {
                    lock(&self.logo_push).refused.remove(&fp);
                    let mut s = lock(&self.store);
                    let b = s.boxes.entry(fp.clone()).or_default();
                    b.logo = Some(info.hash.clone());
                    b.logo_file = Some(sent_name);
                    b.logo_sent_at = Some(unix_now());
                    b.logo_offer = offer;
                    b.logo_arrived = false;
                    let _ = self.save(&s);
                }
                Err(e) => {
                    tracing::warn!("mosaic: the logo wasn't sent to {box_name}: {e}");
                    let text = format!("{box_name} didn't take the logo: {e}");
                    let mut lp = lock(&self.logo_push);
                    lp.tried.insert(fp.clone(), Instant::now());
                    lp.refused.insert(fp, text.clone());
                    failed.get_or_insert(text);
                }
            }
        }
        lock(&self.logo_push).problem = failed;
    }

    /// Follows the logos offered as replacements (logo `hash`) to `sent`: a box that fetched all
    /// of it has it (its screens are given it); a failed or refused transfer, or one whose
    /// fetching hasn't moved for [`LOGO_STUCK`], is sent again ([`LOGO_RETRY`] after a failure).
    /// Until then (and after one that didn't arrive) the screens keep the name that arrived last
    /// ([`Mosaic::ready_logo`]).
    fn follow_logo_transfers(&self, hash: &str) {
        let pending: Vec<(String, String, u64)> = lock(&self.store)
            .boxes
            .iter()
            .filter(|(_, b)| b.logo.as_deref() == Some(hash) && !b.logo_arrived)
            .filter_map(|(fp, b)| {
                Some((
                    fp.clone(),
                    b.logo_offer.clone()?,
                    b.logo_sent_at.unwrap_or(0),
                ))
            })
            .collect();
        lock(&self.logo_push)
            .progress
            .retain(|fp, _| pending.iter().any(|(p, _, _)| p == fp));
        for (fp, offer, at) in pending {
            let seen = self.nodes.transfer(&offer, &fp);
            let state = seen.as_ref().map(|t| t.state.as_str());
            let failed = state.and_then(transfer_failed);
            let still = Progress::observe(
                &mut lock(&self.logo_push).progress,
                &fp,
                &offer,
                seen.as_ref(),
                Instant::now(),
            );
            let stuck = unix_now() >= at + LOGO_STUCK.as_secs() && still >= LOGO_STUCK;
            if state != Some("sent") && failed.is_none() && !stuck {
                continue;
            }
            lock(&self.logo_push).progress.remove(&fp);
            if let Some(f) = failed {
                let name = lock(&self.candidates)
                    .iter()
                    .find(|c| c.device.fingerprint == fp)
                    .map_or_else(|| fp.clone(), |c| c.device.name.clone());
                tracing::warn!("mosaic: the logo didn't reach {name}: {f}");
                let mut lp = lock(&self.logo_push);
                lp.tried.insert(fp.clone(), Instant::now());
                lp.refused
                    .insert(fp.clone(), format!("{name} didn't get the logo: {f}"));
            }
            let mut s = lock(&self.store);
            if let Some(b) = s.boxes.get_mut(&fp)
                && b.logo_offer.as_deref() == Some(offer.as_str())
            {
                if state == Some("sent") {
                    b.logo_arrived = true;
                    // The screens are pointed at it now: the next logo takes the other name.
                    b.logo_shown = b.logo_file.clone();
                } else {
                    // Sent again (as a replacement: the box keeps one file), under the same
                    // name; meanwhile its screens keep the name that arrived last (H03-1).
                    b.logo = None;
                    b.logo_offer = None;
                    b.logo_file = b.logo_shown.clone();
                    b.logo_sent_at = None;
                }
                let _ = self.save(&s);
            }
        }
    }

    /// A settings command for a box's screen with the site's logo as its fallback logo, as
    /// [`Mosaic::sync_box`] sends it ([`with_logo`]).
    fn logo_into(&self, machine: &str, n: u32, body: serde_json::Value) -> serde_json::Value {
        let logo = self.ready_logo(lock(&self.store).boxes.get(machine));
        let has = lock(&self.live)
            .get(machine)
            .and_then(|l| l.screens.iter().find(|s| s.number == n))
            .and_then(|s| s.settings.clone());
        with_logo(Some(body.clone()), n, logo.as_deref(), has.as_ref()).unwrap_or(body)
    }

    /// The logo file a box's screens are given now: the one sent to it, once the box fetched
    /// all of it (a replacement; until then, or after one that didn't arrive, the name that
    /// arrived last: H03-1), else once it has had time to arrive ([`LOGO_SETTLE`]).
    fn ready_logo(&self, b: Option<&StoredBox>) -> Option<String> {
        let b = b?;
        if b.logo_offer.is_some() {
            return if b.logo_arrived {
                b.logo_file.clone()
            } else {
                b.logo_shown.clone()
            };
        }
        let Some(at) = b.logo_sent_at else {
            // A replacement that didn't arrive, being sent again.
            return b.logo_shown.clone();
        };
        (unix_now() >= at + self.logo_settle.as_secs())
            .then(|| b.logo_file.clone())
            .flatten()
    }

    /// `PUT /mosaic/boxes/{machine}/service` (blocking): "Use as a display box": switches the
    /// machine's IDN display service on (in its desktop session, or headless from boot on
    /// Linux: `apps.display` and `apps.display_boot` both on) or off, through its Node's settings.
    pub fn set_service(
        &self,
        site: &Site,
        own_name: &str,
        machine: &str,
        mode: MosaicServiceMode,
    ) -> Result<(), ApiError> {
        self.require_managed()?;
        let d = site.device(machine)?;
        self.not_vesper(site, own_name, &d)?;
        if d.health == DeviceHealth::Removed {
            return Err(api(
                ErrorCode::Conflict,
                "the machine was removed from the site",
            ));
        }
        if mode == MosaicServiceMode::Headless
            && d.machine.as_ref().and_then(|m| m.os.as_ref()).is_some()
            && !linux(&d)
        {
            return Err(api(
                ErrorCode::BadRequest,
                "headless: only on Linux (on Windows the display service runs in the desktop session, with auto-logon)",
            ));
        }
        let on = |b: bool| serde_json::Value::from(if b { "on" } else { "off" });
        let mut settings = serde_json::Map::new();
        // Headless needs both on (the service from boot); the desktop session `display` alone.
        settings.insert("apps.display".into(), on(mode != MosaicServiceMode::Off));
        settings.insert(
            "apps.display_boot".into(),
            on(mode == MosaicServiceMode::Headless),
        );
        self.nodes.apply(site, machine, settings).map_err(|e| {
            api(
                ErrorCode::Unavailable,
                format!("the machine's IDN Node didn't take it: {e}"),
            )
        })?;
        {
            let reports = d.services.iter().any(|s| s.id == "display");
            let mut s = lock(&self.store);
            let b = s.boxes.entry(machine.to_string()).or_default();
            b.service_mode = Some(mode);
            if mode != MosaicServiceMode::Off && !reports {
                b.added = true;
            }
            self.save(&s)?;
        }
        self.refresh(site, own_name);
        self.kick.notify_one();
        self.announce(site);
        Ok(())
    }

    /// `409` for a machine that runs a Vesper server.
    fn not_vesper(&self, site: &Site, own_name: &str, d: &SiteDevice) -> Result<(), ApiError> {
        if self.evidence(&site.devices()).runs_vesper(d, own_name) {
            return Err(api(
                ErrorCode::Conflict,
                "this machine runs a Vesper server: its screens are driven by Vesper's own outputs (Configure Screens), not by Mosaic",
            ));
        }
        Ok(())
    }

    /// This machine's IDN Node's fingerprint (blocking: connects to it the first time).
    pub fn own_node(&self) -> Option<String> {
        self.nodes.own()
    }

    /// Vesper box `from` (its fingerprint, checked by the Codex link) said which IDN Node is
    /// its machine's: that machine isn't a display box while the box keeps reporting
    /// ([`VESPER_GONE`]). One Node per box: a new one replaces the box's last.
    pub fn note_vesper_node(&self, from: &str, node: &str, name: &str) {
        let node = node.trim();
        if node.is_empty() || node.len() > 128 || from.is_empty() {
            return;
        }
        let now = unix_now();
        let mut s = lock(&self.store);
        let gone = VESPER_GONE.as_secs();
        let before = s.vesper_boxes.len();
        s.vesper_boxes.retain(|_, v| v.seen + gone > now);
        let was = s.vesper_boxes.get(from).cloned();
        let entry = VesperNode {
            node: node.to_string(),
            name: name.to_string(),
            seen: now,
        };
        // Saved when it changes, and once a minute for its time.
        let save = before != s.vesper_boxes.len()
            || was
                .as_ref()
                .is_none_or(|w| w.node != entry.node || w.name != entry.name || w.seen + 60 <= now);
        s.vesper_boxes.insert(from.to_string(), entry);
        if save {
            let _ = self.save(&s);
        }
    }

    /// A Launcher reported the roles of machine `machine_name`: with the display role (and no
    /// presentation server), the machine no longer runs Vesper unless it is heard from live.
    pub fn note_launcher(&self, machine_name: &str, roles: &[String]) {
        let name = machine_name.trim().to_lowercase();
        if name.is_empty() {
            return;
        }
        let display =
            roles.iter().any(|r| r == "display") && !roles.iter().any(|r| r == "presentation");
        let mut m = lock(&self.display_roles);
        m.retain(|_, t| t.elapsed() < VESPER_GONE);
        if display {
            m.insert(name, Instant::now());
        } else {
            m.remove(&name);
        }
    }

    /// What says which of `devices` run a Vesper server ([`VesperEvidence`]).
    fn evidence(&self, devices: &[SiteDevice]) -> VesperEvidence {
        let now = unix_now();
        let mut ev = VesperEvidence::default();
        for d in devices {
            if !crate::site::is_vesper_box(&d.kind, &d.name) || d.health == DeviceHealth::Removed {
                continue;
            }
            let Some(n) = d
                .name
                .strip_suffix(&format!(" ({})", protocol::PRODUCT_NAME))
                .map(|n| n.trim().to_string())
            else {
                continue;
            };
            let live = d.health != DeviceHealth::Offline;
            let recent = live
                || d.last_seen
                    .as_deref()
                    .and_then(unix_of)
                    .is_some_and(|t| t + VESPER_GONE.as_secs() > now);
            if live {
                ev.names_live.push(n.clone());
            }
            if recent {
                ev.names.push(n);
            }
        }
        for v in lock(&self.store).vesper_boxes.values() {
            if v.seen + VESPER_GONE.as_secs() > now {
                ev.nodes.push(v.node.clone());
            }
            if v.seen + VESPER_LIVE.as_secs() > now {
                ev.nodes_live.push(v.node.clone());
            }
        }
        // This server's own Node.
        if let Some(own) = self.nodes.own() {
            ev.nodes.push(own.clone());
            ev.nodes_live.push(own);
        }
        ev.display_boxes = lock(&self.display_roles)
            .iter()
            .filter(|(_, t)| t.elapsed() < VESPER_GONE)
            .map(|(n, _)| n.clone())
            .collect();
        ev
    }

    fn save(&self, s: &Store) -> Result<(), ApiError> {
        let r = (|| {
            if let Some(d) = self.path.parent() {
                std::fs::create_dir_all(d)?;
            }
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
            std::fs::rename(&tmp, &self.path)
        })();
        r.map_err(|e: std::io::Error| api(ErrorCode::Internal, format!("mosaic.json: {e}")))
    }

    /// This server manages the site's displays (it runs the Site Manager).
    pub fn managed(&self) -> bool {
        self.managed.load(Ordering::Acquire)
    }

    /// `409` unless this server manages the displays.
    pub fn require_managed(&self) -> Result<(), ApiError> {
        if self.managed() {
            Ok(())
        } else {
            Err(api(
                ErrorCode::Conflict,
                "the site's displays are managed on the site's server",
            ))
        }
    }

    /// Looks at the site's devices again (blocking: the Site Manager). Notes the address each
    /// machine asked to join from.
    pub fn refresh(&self, site: &Site, own_name: &str) {
        let managed = site.handle().is_some();
        self.managed.store(managed, Ordering::Release);
        if !managed {
            lock(&self.candidates).clear();
            lock(&self.machines).clear();
            return;
        }
        let devices = site.devices();
        let evidence = self.evidence(&devices);
        let reported = site.node_addresses();
        let mut store = lock(&self.store);
        let mut out = Vec::new();
        let mut others = Vec::new();
        for d in devices {
            if d.health == DeviceHealth::Removed {
                continue;
            }
            let service = d
                .services
                .iter()
                .find(|s| s.id == "display")
                .map(|s| s.state.clone());
            let added = store.boxes.get(&d.fingerprint).is_some_and(|b| b.added);
            let vesper = evidence.runs_vesper(&d, own_name);
            let listed = if vesper {
                service.as_deref().is_some_and(service_on)
            } else {
                service.is_some() || added
            };
            if listed {
                out.push(Candidate {
                    service: service.unwrap_or_default(),
                    vesper,
                    reported: reported.get(&d.fingerprint).map(|a| a.to_string()),
                    headless_ok: linux(&d),
                    mode_reported: self
                        .nodes
                        .report(site, &d.fingerprint)
                        .as_ref()
                        .and_then(reported_mode),
                    device: d,
                });
            } else if !vesper && d.node {
                others.push(MosaicMachine {
                    machine: d.fingerprint.clone(),
                    name: d.name.clone(),
                    health: d.health,
                    headless_ok: linux(&d),
                });
            }
        }
        // Where each asked to join from (the site's recent events).
        let mut noted = false;
        for e in site.recent_events(500) {
            if let (Some(fp), Some(addr)) = (&e.fingerprint, &e.address)
                && out.iter().any(|c| &c.device.fingerprint == fp && !c.vesper)
            {
                let b = store.boxes.entry(fp.clone()).or_default();
                if b.joined_from.is_none() {
                    b.joined_from = Some(addr.clone());
                    noted = true;
                }
            }
        }
        if noted {
            let _ = self.save(&store);
        }
        out.sort_by_key(|c| c.device.name.to_lowercase());
        *lock(&self.candidates) = out;
        others.sort_by_key(|m| m.name.to_lowercase());
        *lock(&self.machines) = others;
    }

    /// A box's token, issued or renewed through the Site Manager (blocking).
    fn token(&self, site: &Site, machine: &str) -> Result<String, String> {
        if let Some((t, until)) = lock(&self.tokens).get(machine)
            && until.saturating_duration_since(Instant::now()) > TOKEN_RENEW
        {
            return Ok(t.clone());
        }
        let h = site
            .handle()
            .ok_or("the site's Site Manager isn't running here")?;
        let aud = TokenAud {
            engine: "display".into(),
            machine: Some(machine.to_string()),
        };
        let v = h
            .issue_token(TOKEN_USER, &aud, "control", Some(TOKEN_TTL))
            .map_err(|e| format!("no display token for the box: {e}"))?;
        let t = v["token"]
            .as_str()
            .filter(|t| !t.is_empty())
            .ok_or("no display token for the box")?
            .to_string();
        lock(&self.tokens).insert(machine.to_string(), (t.clone(), Instant::now() + TOKEN_TTL));
        Ok(t)
    }

    /// Where a managed box is reached, its token and how its certificate is checked (only its
    /// own LAN certificate, `lan_client_config`), or why not (blocking).
    fn target(&self, site: &Site, machine: &str) -> Result<Target, String> {
        let c = lock(&self.candidates)
            .iter()
            .find(|c| c.device.fingerprint == machine)
            .cloned()
            .ok_or("not a display box of the site")?;
        if c.vesper {
            return Err("it runs a Vesper server: its screens are Vesper's own outputs".into());
        }
        if c.device.health == DeviceHealth::Offline {
            return Err("offline in the site".into());
        }
        let address = {
            let s = lock(&self.store);
            address_of(s.boxes.get(machine), c.reported.as_deref()).map(|(a, _)| a)
        }
        .ok_or("its address isn't known yet (its IDN Node isn't online in the site): set it (Site ▸ Displays)")?;
        let tls = site
            .handle()
            .and_then(|h| h.authority().lan_client_config(machine).ok())
            .map(Arc::new);
        Ok(Target {
            base: client::base_url(&address),
            token: self.token(site, machine)?,
            tls,
        })
    }

    /// The source a feed shows now: a channel on its router is the router's source
    /// (`HOST (Channel Main)`), else the channel's own source.
    fn source_of(&self, store: &Store, feed: &MosaicFeed) -> String {
        resolve(store, &lock(&self.routing), &*self.nodes, feed)
    }

    /// Keeps the channels' routers on this server's IDN Node (`routers.list`, through the Site
    /// Manager): sent when the channels changed or the Node is new; a refusal is tried again
    /// after [`ROUTERS_RETRY`] (blocking). Without a Node here channels stay resolved per screen.
    fn sync_routers(&self, site: &Site) {
        let want = wanted_routers(&lock(&self.store));
        let Some(host) = self.nodes.own() else {
            *lock(&self.routing) = Routing::default();
            return;
        };
        {
            let r = lock(&self.routing);
            let same =
                r.host.as_deref() == Some(host.as_str()) && r.applied.as_ref() == Some(&want);
            // Nothing to put there, and nothing put there by this run: the Node's own routers
            // are left alone.
            let nothing = want.is_empty() && r.applied.is_none();
            let waiting = r
                .tried
                .as_ref()
                .is_some_and(|(w, at)| w == &want && at.elapsed() < ROUTERS_RETRY);
            if same || nothing || waiting {
                return;
            }
        }
        // Sending the list puts every router back to the sources in it (the IDN maintainers,
        // 2026-11-26): it always carries each channel's current source (`want`, from the
        // store), and at this run's start it isn't sent when the Node's report already shows
        // the same list (routes made before are in it).
        let list = routers_list(&want);
        let fresh = lock(&self.routing).applied.is_none();
        if fresh
            && let Some(has) = self.nodes.report(site, &host).and_then(|r| {
                r.child("routers")
                    .and_then(|x| x.attr("list"))
                    .map(str::to_string)
            })
            && same_routers(&has, &list)
        {
            *lock(&self.routing) = Routing {
                host: Some(host),
                applied: Some(want),
                problem: None,
                tried: None,
            };
            return;
        }
        let mut settings = serde_json::Map::new();
        settings.insert("routers.list".into(), list.into());
        let r = self.nodes.apply(site, &host, settings);
        let mut ro = lock(&self.routing);
        match r {
            Ok(()) => {
                *ro = Routing {
                    host: Some(host),
                    applied: Some(want),
                    problem: None,
                    tried: None,
                };
            }
            Err(e) => {
                tracing::warn!("mosaic: the channels' routers weren't set: {e}");
                // The routers as they were may be gone with the Node (or point at old
                // sources): screens get each channel's source itself until the list is taken.
                ro.host = Some(host);
                ro.applied = None;
                ro.problem = Some(format!(
                    "The channels' routers on this server's IDN Node weren't set ({e}): each screen gets the channel's source itself"
                ));
                ro.tried = Some((want, Instant::now()));
            }
        }
    }

    /// Points a channel's router at `source` (`routers.route`, one change for every screen
    /// following it); `Ok(false)`: the channel isn't on a router (blocking).
    fn route(&self, site: &Site, id: &str, source: &str) -> Result<bool, String> {
        let router = {
            let r = lock(&self.routing);
            let Some(applied) = &r.applied else {
                return Ok(false);
            };
            match applied.iter().find(|(_, (ch, _))| ch == id) {
                Some((name, _)) => (name.clone(), r.host.clone().unwrap_or_default()),
                None => return Ok(false),
            }
        };
        if !routable(source) {
            // Can't be written in the setting: off its router ([`Mosaic::sync_routers`]).
            return Ok(false);
        }
        let mut settings = serde_json::Map::new();
        settings.insert(
            "routers.route".into(),
            format!("{}={source}", router.0).into(),
        );
        if let Err(e) = self.nodes.apply(site, &router.1, settings) {
            // Screens get the channels' sources themselves until the list is taken again (the
            // next round sends it, with this source).
            tracing::warn!("mosaic: the channel's router didn't take its new source: {e}");
            let mut ro = lock(&self.routing);
            ro.applied = None;
            ro.tried = None;
            ro.problem = Some(format!(
                "The channels' routers on this server's IDN Node didn't take a change ({e}): each screen gets the channel's source itself"
            ));
            return Err(e);
        }
        if let Some(a) = lock(&self.routing).applied.as_mut()
            && let Some(e) = a.get_mut(&router.0)
        {
            e.1 = source.to_string();
        }
        Ok(true)
    }

    /// Reads one box and sets what isn't as Vesper wants it (blocking). What it found is merged
    /// into the box's live entry at the end: a change made through the API meanwhile keeps its
    /// outcome.
    fn sync_box(&self, site: &Site, machine: &str) {
        let started = Instant::now();
        let mut reachable = false;
        let problem: Option<String>;
        let mut read: Option<Vec<client::ScreenState>> = None;
        // Each screen Vesper keeps: `Some`: what it refused; `None`: as Vesper wants it.
        let mut outcomes: Vec<(u32, Option<String>)> = Vec::new();
        match self.target(site, machine) {
            Err(e) => problem = Some(e),
            Ok(t) => {
                let c = t.client();
                match c.state() {
                    Err(e) => {
                        problem = Some(e.message);
                        if e.token_refused {
                            // Expired, or the site's keys changed: a new one next.
                            lock(&self.tokens).remove(machine);
                        }
                    }
                    Ok(state) => {
                        reachable = true;
                        // A service that runs no screens says why ("no desktop on this computer").
                        problem = state.error.map(|e| format!("the display service: {e}"));
                        type Wanted = (u32, Option<String>, Option<serde_json::Value>, bool);
                        let (mut wanted, logo): (Vec<Wanted>, Option<String>) = {
                            let s = lock(&self.store);
                            let b = s.boxes.get(machine);
                            let wanted = b
                                .map(|b| {
                                    b.screens
                                        .iter()
                                        .map(|(n, sc)| {
                                            (
                                                *n,
                                                sc.feed.as_ref().map(|f| self.source_of(&s, f)),
                                                s.settings_command(machine, *n, sc.unsent),
                                                sc.unsent,
                                            )
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            (wanted, self.ready_logo(b))
                        };
                        // With the site's logo on the box, every screen it lists gets it.
                        if logo.is_some() {
                            for sc in &state.screens {
                                if !wanted.iter().any(|w| w.0 == sc.number) {
                                    wanted.push((sc.number, None, None, false));
                                }
                            }
                        }
                        let mut screens = state.screens;
                        for (n, source, settings, unsent) in wanted {
                            let Some(sc) = screens.iter_mut().find(|s| s.number == n) else {
                                continue;
                            };
                            let cmd = settings.clone().unwrap_or(serde_json::Value::Null);
                            let settings =
                                with_logo(settings, n, logo.as_deref(), sc.settings.as_ref());
                            let mut refused = None;
                            // Settings first: a wall's tile is cut right from its first picture.
                            if let Some(body) = settings {
                                // The site's logo, until the screen reports it as its own.
                                let no_logo = logo.as_ref().is_some_and(|l| {
                                    sc.settings
                                        .as_ref()
                                        .is_none_or(|h| h["fallback"]["logo"] != l.as_str())
                                });
                                let lost = unsent
                                    || no_logo
                                    || sc
                                        .settings
                                        .as_ref()
                                        .is_some_and(|has| client::settings_lost(&body, has));
                                if lost {
                                    match c.post("settings", &body) {
                                        Ok(()) => {
                                            client::took_settings(&mut sc.settings, &body);
                                            self.settings_taken(machine, n, &body, &cmd);
                                        }
                                        Err(e) => refused = Some(e),
                                    }
                                }
                            }
                            // A feed Vesper never set is the box's own.
                            if let Some(source) = source
                                && sc.feed != source
                            {
                                match c.feed(n, &source) {
                                    Ok(()) => sc.feed = source,
                                    Err(e) => refused = refused.or(Some(e)),
                                }
                            }
                            outcomes.push((n, refused));
                        }
                        read = Some(screens);
                    }
                }
            }
        }
        let mut live = lock(&self.live);
        let l = live.entry(machine.to_string()).or_default();
        l.reachable = reachable;
        l.problem = problem;
        if let Some(screens) = read {
            for sc in &screens {
                if let Some(y) = &sc.sync {
                    let before = l.late_before.insert(sc.number, y.late);
                    let since = match before {
                        Some(b) if b <= y.late => y.late - b,
                        // First read, or the box started counting again.
                        _ => 0,
                    };
                    l.late_recently.insert(sc.number, since);
                } else {
                    l.late_before.remove(&sc.number);
                    l.late_recently.remove(&sc.number);
                }
            }
            l.screens = screens;
        }
        for (n, refused) in outcomes {
            match refused {
                Some(e) => {
                    l.errors.insert(n, (e, Instant::now()));
                }
                // As Vesper wants it: an error noted before this read is over (a change that
                // timed out but was taken), one noted since stays.
                None => {
                    if l.errors.get(&n).is_some_and(|(_, at)| *at <= started) {
                        l.errors.remove(&n);
                    }
                }
            }
        }
    }

    /// A screen's box took `body`, made from settings command `cmd` (the store's, before the
    /// site's logo went in: [`with_logo`] may add the box's whole fallback): unless the command
    /// changed since, it is sent; the box's settings as last read are what it has now.
    fn settings_taken(
        &self,
        machine: &str,
        n: u32,
        body: &serde_json::Value,
        cmd: &serde_json::Value,
    ) {
        if let Some(sc) = lock(&self.live)
            .get_mut(machine)
            .and_then(|l| l.screens.iter_mut().find(|s| s.number == n))
        {
            client::took_settings(&mut sc.settings, body);
        }
        let mut s = lock(&self.store);
        let unsent = s
            .boxes
            .get(machine)
            .and_then(|b| b.screens.get(&n))
            .is_some_and(|sc| sc.unsent);
        if unsent
            && s.settings_command(machine, n, true)
                .is_some_and(|c| same_command(&c, cmd))
        {
            s.screen_mut(machine, n).unsent = false;
            let _ = self.save(&s);
        }
    }

    /// One round: the devices again, then every box (blocking). Announces a change.
    pub fn tick(&self, site: &Site, own_name: &str) {
        self.refresh(site, own_name);
        if self.managed() {
            self.sync_routers(site);
        }
        let boxes: Vec<String> = lock(&self.candidates)
            .iter()
            .filter(|c| !c.vesper)
            .map(|c| c.device.fingerprint.clone())
            .collect();
        lock(&self.live).retain(|k, _| boxes.contains(k));
        for m in &boxes {
            self.sync_box(site, m);
        }
        self.push_logo();
        self.announce(site);
    }

    /// `siteChanged` (`mosaic`) when what `GET /mosaic` answers changed.
    pub fn announce(&self, site: &Site) {
        let now = self.state();
        let mut last = lock(&self.last);
        if last.as_ref() != Some(&now) {
            *last = Some(now);
            drop(last);
            site.changed([SitePart::Mosaic]);
        }
    }

    /// `GET /api/v1/mosaic`.
    pub fn state(&self) -> MosaicState {
        let managed = self.managed();
        let logo = lock(&self.logo).clone();
        let logo_failed = lock(&self.logo_push).refused.clone();
        let cands = lock(&self.candidates).clone();
        let store = lock(&self.store).clone();
        let live = lock(&self.live).clone();
        let boxes = cands
            .iter()
            .map(|c| {
                let fp = &c.device.fingerprint;
                let sb = store.boxes.get(fp).cloned().unwrap_or_default();
                let lv = live.get(fp).cloned().unwrap_or_default();
                let offline = !lv.reachable;
                let mut numbers: Vec<u32> = lv.screens.iter().map(|s| s.number).collect();
                for n in sb.screens.keys() {
                    if !numbers.contains(n) {
                        numbers.push(*n);
                    }
                }
                numbers.sort_unstable();
                let screens = if c.vesper {
                    Vec::new()
                } else {
                    numbers
                        .into_iter()
                        .map(|n| {
                            let seen = lv.screens.iter().find(|s| s.number == n);
                            let stored = sb.screens.get(&n).cloned().unwrap_or_default();
                            let error = lv.errors.get(&n).map(|(e, _)| e.clone());
                            let (status, problem) = if offline {
                                (MosaicScreenStatus::Offline, lv.problem.clone())
                            } else if let Some(e) = error {
                                (MosaicScreenStatus::Error, Some(e))
                            } else {
                                match seen {
                                    Some(s) => (s.status, s.problem.clone()),
                                    None => (
                                        MosaicScreenStatus::Error,
                                        // The service's own error says why, when it has one.
                                        Some(lv.problem.clone().unwrap_or_else(|| {
                                            "the box doesn't list this screen now".into()
                                        })),
                                    ),
                                }
                            };
                            MosaicScreen {
                                number: n,
                                name: seen.map(|s| s.name.clone()).unwrap_or_default(),
                                width: seen.and_then(|s| s.width),
                                height: seen.and_then(|s| s.height),
                                feed: stored.feed.unwrap_or_default(),
                                showing: seen.map(|s| s.feed.clone()).unwrap_or_default(),
                                test_pattern: seen.is_some_and(|s| s.test_pattern),
                                settings: store.reported_settings(
                                    fp,
                                    n,
                                    seen.and_then(|s| s.settings.as_ref()),
                                ),
                                status,
                                problem,
                                wall: store.wall_of(fp, n).map(|w| w.id.clone()),
                                sync: seen.filter(|_| !offline).and_then(|s| {
                                    s.sync.as_ref().map(|y| MosaicSync {
                                        group: y.group.clone(),
                                        mode: y.mode.clone(),
                                        // Whole milliseconds: what people read, and fewer changes.
                                        delay_ms: y.delay_ms.round(),
                                        late: y.late,
                                        late_recently: lv
                                            .late_recently
                                            .get(&n)
                                            .copied()
                                            .unwrap_or(0),
                                    })
                                }),
                            }
                        })
                        .collect()
                };
                MosaicBox {
                    machine: fp.clone(),
                    name: c.device.name.clone(),
                    health: c.device.health,
                    service: c.service.clone(),
                    address: address_of(Some(&sb), c.reported.as_deref()).map(|(a, _)| a),
                    address_from: address_of(Some(&sb), c.reported.as_deref()).map(|(_, f)| f),
                    service_mode: c.mode_reported.or(sb.service_mode),
                    service_mode_reported: c.mode_reported.is_some(),
                    headless_ok: c.headless_ok,
                    update_managed_by: c.device.update_managed_by.clone(),
                    logo_state: logo.as_ref().and_then(|l| {
                        if logo_failed.contains_key(fp) {
                            return Some(MosaicLogoState::Failed);
                        }
                        if sb.logo.as_deref() != Some(l.hash.as_str()) {
                            return None;
                        }
                        let file = sb.logo_file.as_deref()?;
                        let arrived = sb.logo_offer.is_none() || sb.logo_arrived;
                        let all = arrived
                            && !lv.screens.is_empty()
                            && lv.screens.iter().all(|s| {
                                s.settings
                                    .as_ref()
                                    .is_some_and(|x| x["fallback"]["logo"] == file)
                            });
                        Some(if all {
                            MosaicLogoState::OnBox
                        } else {
                            MosaicLogoState::Sending
                        })
                    }),
                    added: sb.added,
                    vesper: c.vesper,
                    conflict: c.vesper.then(|| {
                        "This machine runs a Vesper server and its IDN Node's display service: two programs would drive the same screens. Switch the display service off there (its screens are Vesper's outputs).".to_string()
                    }),
                    reachable: lv.reachable && !c.vesper,
                    problem: if c.vesper { None } else { lv.problem.clone() },
                    screens,
                }
            })
            .collect::<Vec<MosaicBox>>();
        let mut walls: Vec<MosaicWall> =
            store.walls.iter().map(|w| wall_state(w, &boxes)).collect();
        walls.sort_by_key(|w| w.name.to_lowercase());
        let routing = lock(&self.routing).clone();
        let mut channels: Vec<MosaicChannel> = store
            .channels
            .iter()
            .map(|ch| MosaicChannel {
                id: ch.id.clone(),
                name: ch.name.clone(),
                source: ch.source.clone(),
                router: routed(&routing, &ch.id).map(|r| self.nodes.source_name(&r)),
                screens: store
                    .boxes
                    .values()
                    .flat_map(|b| b.screens.values())
                    .filter(|s| {
                        matches!(&s.feed, Some(MosaicFeed::Channel { channel }) if *channel == ch.id)
                    })
                    .count() as u32,
            })
            .collect();
        channels.sort_by_key(|c| c.name.to_lowercase());
        MosaicState {
            managed,
            boxes,
            channels,
            walls,
            routing_problem: routing.problem.filter(|_| !store.channels.is_empty()),
            logo_problem: logo
                .as_ref()
                .and_then(|_| lock(&self.logo_push).problem.clone()),
            logo,
            machines: lock(&self.machines).clone(),
        }
    }

    /// A managed box that isn't a Vesper machine, or `404`/`409`.
    fn managed_box(&self, machine: &str) -> Result<(), ApiError> {
        self.require_managed()?;
        match lock(&self.candidates)
            .iter()
            .find(|c| c.device.fingerprint == machine)
        {
            None => Err(api(ErrorCode::NotFound, "no such display box")),
            Some(c) if c.vesper => Err(api(
                ErrorCode::Conflict,
                "this machine runs a Vesper server: its screens are driven by Vesper's own outputs (Configure Screens), not by Mosaic",
            )),
            Some(_) => Ok(()),
        }
    }

    /// A screen the box has (as last read), or any number while it can't be read or its service
    /// has an error of its own (no desktop session: it lists no screens).
    fn check_screen(&self, machine: &str, n: u32) -> Result<(), ApiError> {
        if n == 0 {
            return Err(api(ErrorCode::BadRequest, "screens count from 1"));
        }
        let live = lock(&self.live);
        match live.get(machine) {
            Some(l)
                if l.reachable
                    && l.problem.is_none()
                    && !l.screens.iter().any(|s| s.number == n) =>
            {
                Err(api(ErrorCode::NotFound, "the box has no such screen"))
            }
            _ => Ok(()),
        }
    }

    /// Runs `f` against a box's service now (blocking); its refusal or silence is `503` with
    /// `kept`'s words (what was kept is set again when the box answers).
    fn on_box<T>(
        &self,
        site: &Site,
        machine: &str,
        kept: &str,
        f: impl FnOnce(&client::Client) -> Result<T, String>,
    ) -> Result<T, ApiError> {
        let t = self
            .target(site, machine)
            .map_err(|e| api(ErrorCode::Unavailable, format!("{kept}{e}")))?;
        f(&t.client()).map_err(|e| api(ErrorCode::Unavailable, format!("{kept}{e}")))
    }

    /// Sets a screen's feed now (blocking), noting its outcome.
    fn push_feed(&self, site: &Site, machine: &str, n: u32, source: &str) -> Result<(), ApiError> {
        let r = self.on_box(site, machine, "Kept; the box didn't take it now: ", |c| {
            c.feed(n, source)
        });
        let mut live = lock(&self.live);
        let l = live.entry(machine.to_string()).or_default();
        match &r {
            Ok(()) => {
                l.errors.remove(&n);
                if let Some(s) = l.screens.iter_mut().find(|s| s.number == n) {
                    s.feed = source.to_string();
                }
            }
            Err(e) if l.reachable => {
                l.errors.insert(n, (e.message.clone(), Instant::now()));
            }
            Err(_) => {}
        }
        r
    }

    /// `PUT …/screens/{n}/feed` (blocking): kept, then set on the box.
    pub fn set_feed(
        &self,
        site: &Site,
        machine: &str,
        n: u32,
        feed: MosaicFeed,
    ) -> Result<(), ApiError> {
        self.managed_box(machine)?;
        self.check_screen(machine, n)?;
        let source = {
            let mut s = lock(&self.store);
            if let Some(w) = s.wall_of(machine, n) {
                return Err(api(
                    ErrorCode::Conflict,
                    format!(
                        "this screen is a tile of the wall \"{}\": change the wall's feed",
                        w.name
                    ),
                ));
            }
            match &feed {
                MosaicFeed::Channel { channel } if !s.channels.iter().any(|c| &c.id == channel) => {
                    return Err(api(ErrorCode::NotFound, "no such channel"));
                }
                MosaicFeed::Source { source } if !valid_source(source) => {
                    return Err(api(
                        ErrorCode::BadRequest,
                        "source: a source's name, at most 256 bytes, no control characters",
                    ));
                }
                _ => {}
            }
            s.screen_mut(machine, n).feed = Some(feed.clone());
            self.save(&s)?;
            self.source_of(&s, &feed)
        };
        let r = self.push_feed(site, machine, n, &source);
        self.announce(site);
        r
    }

    /// `PUT …/screens/{n}/settings` (blocking).
    pub fn set_settings(
        &self,
        site: &Site,
        machine: &str,
        n: u32,
        settings: MosaicScreenSettings,
    ) -> Result<(), ApiError> {
        self.managed_box(machine)?;
        self.check_screen(machine, n)?;
        let settings = {
            let mut s = lock(&self.store);
            // Tiles come only from the walls (the wall designer): a `tile` sent is ignored,
            // neither checked nor kept (a screen in a wall keeps its wall's).
            let mut settings = settings;
            settings.tile = None;
            client::check_settings(&settings).map_err(|e| api(ErrorCode::BadRequest, e))?;
            let sc = s.screen_mut(machine, n);
            sc.settings = Some(settings);
            sc.unsent = true;
            self.save(&s)?;
            s.settings_of(machine, n).unwrap_or_default()
        };
        let cmd = client::settings_body(n, &settings);
        let body = self.logo_into(machine, n, cmd.clone());
        let r = self.on_box(site, machine, "Kept; the box didn't take it now: ", |c| {
            c.post("settings", &body)
        });
        if r.is_ok() {
            self.settings_taken(machine, n, &body, &cmd);
        }
        self.announce(site);
        r
    }

    /// `identify` or `test_pattern` on one screen (`Some(n)`) or all of a box's (blocking).
    pub fn command(
        &self,
        site: &Site,
        machine: &str,
        n: Option<u32>,
        cmd: &str,
        mut body: serde_json::Value,
    ) -> Result<(), ApiError> {
        self.managed_box(machine)?;
        if let Some(n) = n {
            self.check_screen(machine, n)?;
        }
        body["screen"] = n.map_or(serde_json::json!("all"), serde_json::Value::from);
        self.on_box(site, machine, "", |c| c.post(cmd, &body))
    }

    /// `identify` or `test_pattern` on every screen of every box (blocking): the boxes that
    /// didn't take it, by name.
    pub fn command_all(
        &self,
        site: &Site,
        cmd: &str,
        body: serde_json::Value,
    ) -> Result<(), ApiError> {
        self.require_managed()?;
        let boxes: Vec<(String, String)> = lock(&self.candidates)
            .iter()
            .filter(|c| !c.vesper)
            .map(|c| (c.device.fingerprint.clone(), c.device.name.clone()))
            .collect();
        let failed: Vec<String> = boxes
            .iter()
            .filter_map(|(m, name)| {
                self.command(site, m, None, cmd, body.clone())
                    .err()
                    .map(|e| format!("{name}: {}", e.message))
            })
            .collect();
        if failed.is_empty() {
            Ok(())
        } else {
            Err(api(ErrorCode::Unavailable, failed.join("; ")))
        }
    }

    /// `GET …/screens/{n}/thumbnail` (blocking): a JPEG, or `404` when it shows nothing.
    pub fn thumbnail(
        &self,
        site: &Site,
        machine: &str,
        n: u32,
        width: u32,
    ) -> Result<Vec<u8>, ApiError> {
        self.managed_box(machine)?;
        self.on_box(site, machine, "", |c| c.thumbnail(n, width))?
            .ok_or_else(|| api(ErrorCode::NotFound, "the screen shows nothing"))
    }

    /// `PUT /mosaic/boxes/{machine}` (blocking): adds a machine of the site as a box, or sets
    /// its address.
    pub fn put_box(
        &self,
        site: &Site,
        own_name: &str,
        machine: &str,
        address: Option<String>,
    ) -> Result<(), ApiError> {
        self.require_managed()?;
        let d = site.device(machine)?;
        self.not_vesper(site, own_name, &d)?;
        if d.health == DeviceHealth::Removed {
            return Err(api(
                ErrorCode::Conflict,
                "the machine was removed from the site",
            ));
        }
        let address = address
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty());
        if let Some(a) = &address
            && (a.len() > 256 || a.chars().any(|c| c.is_control() || c.is_whitespace()))
        {
            return Err(api(
                ErrorCode::BadRequest,
                "address: a host, host:port or URL",
            ));
        }
        let reports = d.services.iter().any(|s| s.id == "display");
        {
            let mut s = lock(&self.store);
            let b = s.boxes.entry(machine.to_string()).or_default();
            b.address = address;
            b.added = b.added || !reports;
            self.save(&s)?;
        }
        lock(&self.tokens).remove(machine);
        self.refresh(site, own_name);
        self.kick.notify_one();
        Ok(())
    }

    /// `DELETE /mosaic/boxes/{machine}`: forgets what Vesper kept for a box (its address, its
    /// screens' feeds and settings; the box keeps showing what it shows).
    pub fn forget_box(&self, site: &Site, own_name: &str, machine: &str) -> Result<(), ApiError> {
        self.require_managed()?;
        let had = {
            let mut s = lock(&self.store);
            let had = s.boxes.remove(machine).is_some();
            if had {
                // Its screens leave the walls they were in.
                for w in &mut s.walls {
                    w.cells.retain(|c| c.machine != machine);
                }
                self.save(&s)?;
            }
            had
        };
        if !had {
            return Err(api(ErrorCode::NotFound, "nothing kept for that machine"));
        }
        lock(&self.live).remove(machine);
        self.refresh(site, own_name);
        self.announce(site);
        Ok(())
    }

    /// `POST /mosaic/channels` (blocking): a new channel.
    pub fn create_channel(
        &self,
        site: &Site,
        name: &str,
        source: &str,
    ) -> Result<MosaicChannel, ApiError> {
        self.require_managed()?;
        let name = channel_name(name)?;
        if !source.is_empty() && !valid_source(source) {
            return Err(api(ErrorCode::BadRequest, "source: a source's name"));
        }
        let ch = {
            let mut s = lock(&self.store);
            if s.channels
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case(&name))
            {
                return Err(api(ErrorCode::Conflict, "a channel has that name"));
            }
            let ch = StoredChannel {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                source: source.to_string(),
            };
            s.channels.push(ch.clone());
            self.save(&s)?;
            ch
        };
        // Its router on this server's Node, at once.
        self.sync_routers(site);
        self.announce(site);
        let router = routed(&lock(&self.routing), &ch.id).map(|r| self.nodes.source_name(&r));
        Ok(MosaicChannel {
            id: ch.id,
            name: ch.name,
            source: ch.source,
            screens: 0,
            router,
        })
    }

    /// `PUT /mosaic/channels/{id}` (blocking): renames it and / or points it at another source,
    /// which every screen following it shows then. The screens that didn't take it are named in
    /// a `503` (they are set again when their box answers).
    pub fn put_channel(
        &self,
        site: &Site,
        id: &str,
        name: Option<&str>,
        source: Option<&str>,
    ) -> Result<(), ApiError> {
        self.require_managed()?;
        let name = name.map(channel_name).transpose()?;
        if let Some(src) = source
            && !src.is_empty()
            && !valid_source(src)
        {
            return Err(api(ErrorCode::BadRequest, "source: a source's name"));
        }
        let feed = MosaicFeed::Channel {
            channel: id.to_string(),
        };
        let (followers, before, moved, renamed) = {
            let mut s = lock(&self.store);
            if let Some(n) = &name
                && s.channels
                    .iter()
                    .any(|c| c.id != id && c.name.eq_ignore_ascii_case(n))
            {
                return Err(api(ErrorCode::Conflict, "a channel has that name"));
            }
            let before = self.source_of(&s, &feed);
            let ch = s
                .channels
                .iter_mut()
                .find(|c| c.id == id)
                .ok_or_else(|| api(ErrorCode::NotFound, "no such channel"))?;
            let renamed = name.as_ref().is_some_and(|n| *n != ch.name);
            if let Some(n) = name {
                ch.name = n;
            }
            let moved = match source {
                Some(src) if src != ch.source => {
                    ch.source = src.to_string();
                    Some(src.to_string())
                }
                _ => None,
            };
            self.save(&s)?;
            let followers: Vec<(String, u32)> = s
                .boxes
                .iter()
                .flat_map(|(m, b)| {
                    b.screens
                        .iter()
                        .filter(|(_, sc)| sc.feed.as_ref() == Some(&feed))
                        .map(move |(n, _)| (m.clone(), *n))
                })
                .collect();
            (followers, before, moved, renamed)
        };
        let mut failed: Vec<String> = Vec::new();
        // On its router: one change on this server's Node moves every screen following it.
        // Refused, the screens are given the source themselves (below) and the routers are set
        // again on a later round.
        let mut off_router = false;
        if let Some(src) = &moved {
            match self.route(site, id, src) {
                Ok(on) => off_router = !on && !routable(src),
                Err(e) => tracing::warn!("mosaic: channel {id}: {e}"),
            }
        }
        if renamed || off_router {
            // Its router's new name, or the channel leaves its router.
            self.sync_routers(site);
        }
        // Screens move themselves only when what they are given changes (not on a router, or
        // its router renamed).
        let after = self.source_of(&lock(&self.store), &feed);
        if after != before {
            let names: HashMap<String, String> = lock(&self.candidates)
                .iter()
                .map(|c| (c.device.fingerprint.clone(), c.device.name.clone()))
                .collect();
            failed.extend(
                followers
                    .iter()
                    .filter(|(m, _)| names.contains_key(m))
                    .filter_map(|(m, n)| {
                        self.push_feed(site, m, *n, &after)
                            .err()
                            .map(|e| format!("{} screen {n}: {}", names[m], e.message))
                    }),
            );
        }
        self.announce(site);
        if failed.is_empty() {
            Ok(())
        } else {
            Err(api(ErrorCode::Unavailable, failed.join("; ")))
        }
    }

    /// `DELETE /mosaic/channels/{id}`: refused (`409`) while screens follow it.
    pub fn delete_channel(&self, site: &Site, id: &str) -> Result<(), ApiError> {
        self.require_managed()?;
        {
            let mut s = lock(&self.store);
            if !s.channels.iter().any(|c| c.id == id) {
                return Err(api(ErrorCode::NotFound, "no such channel"));
            }
            let following = s
                .boxes
                .values()
                .flat_map(|b| b.screens.values())
                .filter(
                    |sc| matches!(&sc.feed, Some(MosaicFeed::Channel { channel }) if channel == id),
                )
                .count();
            if following > 0 {
                return Err(api(
                    ErrorCode::Conflict,
                    format!(
                        "{following} screen(s) follow this channel: give them another feed first"
                    ),
                ));
            }
            if let Some(w) = s
                .walls
                .iter()
                .find(|w| matches!(&w.feed, MosaicFeed::Channel { channel } if channel == id))
            {
                return Err(api(
                    ErrorCode::Conflict,
                    format!(
                        "the wall \"{}\" follows this channel: give it another feed first",
                        w.name
                    ),
                ));
            }
            s.channels.retain(|c| c.id != id);
            self.save(&s)?;
        }
        self.sync_routers(site);
        self.announce(site);
        Ok(())
    }

    /// Checks a wall against the rules (`id`: the wall it replaces) and makes it.
    fn check_wall(
        &self,
        store: &Store,
        id: Option<&str>,
        w: &PutMosaicWall,
    ) -> Result<StoredWall, ApiError> {
        let bad = |m: &str| api(ErrorCode::BadRequest, m);
        let name = w.name.trim();
        if name.is_empty() || name.chars().count() > 48 || name.chars().any(char::is_control) {
            return Err(bad("name: 1 to 48 characters"));
        }
        // Its sync group is `wall:` and the name, at most 64 bytes on the boxes.
        if "wall:".len() + name.len() > GROUP_MAX {
            return Err(bad(
                "name: too long in this script for the wall's sync group: make it shorter",
            ));
        }
        if store
            .walls
            .iter()
            .any(|x| Some(x.id.as_str()) != id && x.name.eq_ignore_ascii_case(name))
        {
            return Err(api(ErrorCode::Conflict, "a wall has that name"));
        }
        if !(1..=16).contains(&w.rows) || !(1..=16).contains(&w.columns) {
            return Err(bad("rows and columns: 1 to 16"));
        }
        if !w
            .bezel_mm
            .iter()
            .all(|b| b.is_finite() && (0.0..=10000.0).contains(b))
        {
            return Err(bad("bezelMm: 0 to 10000 mm"));
        }
        let screen_mm = w.screen_mm.filter(|[a, b]| *a > 0.0 || *b > 0.0);
        if let Some([sw, sh]) = screen_mm
            && !(sw.is_finite()
                && sh.is_finite()
                && sw > 0.0
                && sh > 0.0
                && sw <= 100_000.0
                && sh <= 100_000.0)
        {
            return Err(bad("screenMm: the screen's visible width and height, mm"));
        }
        if w.bezel_mm != [0.0, 0.0] && screen_mm.is_none() {
            return Err(bad(
                "screenMm: with a bezel, the screen's visible width and height are needed",
            ));
        }
        match &w.feed {
            MosaicFeed::Channel { channel } if !store.channels.iter().any(|c| &c.id == channel) => {
                return Err(api(ErrorCode::NotFound, "no such channel"));
            }
            MosaicFeed::Source { source } if !valid_source(source) => {
                return Err(bad(
                    "feed: a source's name, at most 256 bytes, no control characters",
                ));
            }
            _ => {}
        }
        let old = id.and_then(|id| store.walls.iter().find(|x| x.id == id));
        let mut cells: Vec<MosaicWallCell> = Vec::new();
        for c in &w.cells {
            if c.row >= w.rows || c.column >= w.columns {
                return Err(bad(&format!(
                    "the cell at row {}, column {} is outside the wall",
                    c.row + 1,
                    c.column + 1
                )));
            }
            if cells.iter().any(|x| x.row == c.row && x.column == c.column) {
                return Err(bad(&format!(
                    "the cell at row {}, column {} has two screens",
                    c.row + 1,
                    c.column + 1
                )));
            }
            if cells
                .iter()
                .any(|x| x.machine == c.machine && x.screen == c.screen)
            {
                return Err(bad("a screen can fill only one cell"));
            }
            // A screen in the wall already stays (its box removed from the site, or not listing
            // it now): the rest of the wall can still be changed.
            if !old.is_some_and(|o| o.has(&c.machine, c.screen)) {
                self.managed_box(&c.machine)?;
                self.check_screen(&c.machine, c.screen)?;
            }
            if let Some(other) = store
                .walls
                .iter()
                .find(|x| Some(x.id.as_str()) != id && x.has(&c.machine, c.screen))
            {
                return Err(api(
                    ErrorCode::Conflict,
                    format!(
                        "screen {} of that box is in the wall \"{}\" already",
                        c.screen, other.name
                    ),
                ));
            }
            cells.push(c.clone());
        }
        cells.sort_by_key(|c| (c.row, c.column));
        Ok(StoredWall {
            id: id.map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_string),
            name: name.to_string(),
            rows: w.rows,
            columns: w.columns,
            bezel_mm: w.bezel_mm,
            screen_mm,
            feed: w.feed.clone(),
            cells,
        })
    }

    /// Keeps a wall (new, or replacing the one with its id) and what it means for its screens:
    /// each tile follows the wall's feed and gets its tile; `left` (screens no longer in it)
    /// lose their tile and keep their feed. Then sets them on the boxes: settings before the
    /// feed. The screens that didn't take it are named in a `503` (they are set again when
    /// their box answers).
    fn apply_wall(
        &self,
        site: &Site,
        wall: StoredWall,
        left: Vec<(String, u32)>,
        settings_too: bool,
    ) -> Result<(), ApiError> {
        type Push = (String, u32, Option<serde_json::Value>, Option<String>);
        let pushes: Vec<Push> = {
            let mut s = lock(&self.store);
            match s.walls.iter_mut().find(|w| w.id == wall.id) {
                Some(w) => *w = wall.clone(),
                None => s.walls.push(wall.clone()),
            }
            for c in &wall.cells {
                let sc = s.screen_mut(&c.machine, c.screen);
                sc.feed = Some(wall.feed.clone());
                sc.unsent |= settings_too;
            }
            for (m, n) in &left {
                let sc = s.screen_mut(m, *n);
                if let Some(st) = sc.settings.as_mut() {
                    st.tile = None;
                }
                // Its tile goes, also when its box is away now.
                sc.unsent = true;
            }
            self.save(&s)?;
            let source = self.source_of(&s, &wall.feed);
            let mut out: Vec<Push> = wall
                .cells
                .iter()
                .map(|c| {
                    (
                        c.machine.clone(),
                        c.screen,
                        settings_too
                            .then(|| s.settings_command(&c.machine, c.screen, false))
                            .flatten(),
                        Some(source.clone()),
                    )
                })
                .collect();
            for (m, n) in left {
                let st = s.settings_command(&m, n, true);
                out.push((m, n, st, None));
            }
            out
        };
        let names: HashMap<String, String> = lock(&self.candidates)
            .iter()
            .map(|c| (c.device.fingerprint.clone(), c.device.name.clone()))
            .collect();
        let mut failed = Vec::new();
        for (m, n, settings, source) in pushes {
            let Some(name) = names.get(&m) else {
                continue;
            };
            let mut r = Ok(());
            if let Some(cmd) = settings {
                let body = self.logo_into(&m, n, cmd.clone());
                r = self.on_box(site, &m, "", |c| c.post("settings", &body));
                match &r {
                    Ok(()) => self.settings_taken(&m, n, &body, &cmd),
                    Err(e) => {
                        let mut live = lock(&self.live);
                        let l = live.entry(m.clone()).or_default();
                        if l.reachable {
                            l.errors.insert(n, (e.message.clone(), Instant::now()));
                        }
                    }
                }
            }
            if r.is_ok()
                && let Some(src) = source
            {
                r = self.push_feed(site, &m, n, &src);
            }
            if let Err(e) = r {
                failed.push(format!("{name} screen {n}: {}", e.message));
            }
        }
        self.announce(site);
        if failed.is_empty() {
            Ok(())
        } else {
            Err(api(
                ErrorCode::Unavailable,
                format!("Kept; not every screen took it now: {}", failed.join("; ")),
            ))
        }
    }

    /// `POST /mosaic/walls` (blocking): a new wall, applied to its screens; its id. A `503`
    /// still made it (its screens are set when their boxes answer).
    pub fn create_wall(&self, site: &Site, w: &PutMosaicWall) -> Result<String, ApiError> {
        self.require_managed()?;
        let wall = {
            let s = lock(&self.store);
            self.check_wall(&s, None, w)?
        };
        let id = wall.id.clone();
        self.apply_wall(site, wall, Vec::new(), true)?;
        Ok(id)
    }

    /// `PUT /mosaic/walls/{id}` (blocking): the wall, whole. Screens moved get their new tile,
    /// screens taken out lose theirs.
    pub fn put_wall(&self, site: &Site, id: &str, w: &PutMosaicWall) -> Result<(), ApiError> {
        self.require_managed()?;
        let (wall, left) = {
            let s = lock(&self.store);
            let old = s
                .walls
                .iter()
                .find(|x| x.id == id)
                .ok_or_else(|| api(ErrorCode::NotFound, "no such wall"))?;
            let wall = self.check_wall(&s, Some(id), w)?;
            let left: Vec<(String, u32)> = old
                .cells
                .iter()
                .filter(|c| !wall.has(&c.machine, c.screen))
                .map(|c| (c.machine.clone(), c.screen))
                .collect();
            (wall, left)
        };
        self.apply_wall(site, wall, left, true)
    }

    /// `PUT /mosaic/walls/{id}/feed` (blocking): what the whole wall shows.
    pub fn set_wall_feed(&self, site: &Site, id: &str, feed: MosaicFeed) -> Result<(), ApiError> {
        self.require_managed()?;
        let wall = {
            let s = lock(&self.store);
            let mut wall = s
                .walls
                .iter()
                .find(|x| x.id == id)
                .cloned()
                .ok_or_else(|| api(ErrorCode::NotFound, "no such wall"))?;
            match &feed {
                MosaicFeed::Channel { channel } if !s.channels.iter().any(|c| &c.id == channel) => {
                    return Err(api(ErrorCode::NotFound, "no such channel"));
                }
                MosaicFeed::Source { source } if !valid_source(source) => {
                    return Err(api(
                        ErrorCode::BadRequest,
                        "source: a source's name, at most 256 bytes, no control characters",
                    ));
                }
                _ => {}
            }
            wall.feed = feed;
            wall
        };
        self.apply_wall(site, wall, Vec::new(), false)
    }

    /// `DELETE /mosaic/walls/{id}` (blocking): its screens lose their tiles and keep showing
    /// the wall's feed, each whole.
    pub fn delete_wall(&self, site: &Site, id: &str) -> Result<(), ApiError> {
        self.require_managed()?;
        let cells = {
            let mut s = lock(&self.store);
            let i = s
                .walls
                .iter()
                .position(|w| w.id == id)
                .ok_or_else(|| api(ErrorCode::NotFound, "no such wall"))?;
            let w = s.walls.remove(i);
            for c in &w.cells {
                let sc = s.screen_mut(&c.machine, c.screen);
                if let Some(st) = sc.settings.as_mut() {
                    st.tile = None;
                }
                // Its tile goes, also when its box is away now.
                sc.unsent = true;
            }
            self.save(&s)?;
            w.cells
        };
        let mut failed = Vec::new();
        for c in cells {
            let Some(cmd) = lock(&self.store).settings_command(&c.machine, c.screen, true) else {
                continue;
            };
            let body = self.logo_into(&c.machine, c.screen, cmd.clone());
            if self.managed_box(&c.machine).is_ok() {
                match self.on_box(site, &c.machine, "", |cl| cl.post("settings", &body)) {
                    Ok(()) => self.settings_taken(&c.machine, c.screen, &body, &cmd),
                    Err(e) => failed.push(format!("screen {}: {}", c.screen, e.message)),
                }
            }
        }
        self.announce(site);
        if failed.is_empty() {
            Ok(())
        } else {
            Err(api(
                ErrorCode::Unavailable,
                format!(
                    "Deleted; not every screen took it now (set again when it answers): {}",
                    failed.join("; ")
                ),
            ))
        }
    }

    /// `POST /mosaic/walls/{id}/identify` (blocking): every tile's screen shows who it is.
    pub fn identify_wall(
        &self,
        site: &Site,
        id: &str,
        body: serde_json::Value,
    ) -> Result<(), ApiError> {
        self.require_managed()?;
        let cells = lock(&self.store)
            .walls
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.cells.clone())
            .ok_or_else(|| api(ErrorCode::NotFound, "no such wall"))?;
        let failed: Vec<String> = cells
            .iter()
            .filter_map(|c| {
                self.command(site, &c.machine, Some(c.screen), "identify", body.clone())
                    .err()
                    .map(|e| format!("row {}, column {}: {}", c.row + 1, c.column + 1, e.message))
            })
            .collect();
        if failed.is_empty() {
            Ok(())
        } else {
            Err(api(ErrorCode::Unavailable, failed.join("; ")))
        }
    }
}

/// How bad a state is, for a wall's worst.
fn badness(s: MosaicScreenStatus) -> u8 {
    match s {
        MosaicScreenStatus::Showing => 0,
        MosaicScreenStatus::Idle => 1,
        MosaicScreenStatus::Connecting => 2,
        MosaicScreenStatus::Lost => 3,
        MosaicScreenStatus::Fallback => 4,
        MosaicScreenStatus::Offline => 5,
        MosaicScreenStatus::Error => 6,
    }
}

/// A wall's state from its tiles' (as `GET /mosaic` lists the boxes).
fn wall_state(w: &StoredWall, boxes: &[MosaicBox]) -> MosaicWall {
    let group = w.group();
    let mut status = if w.cells.is_empty() {
        MosaicScreenStatus::Idle
    } else {
        MosaicScreenStatus::Showing
    };
    let mut problem: Option<(u8, String)> = w
        .cells
        .is_empty()
        .then(|| (0, "No screens in the wall yet".to_string()));
    let mut showing = 0;
    let mut out_of_step: Option<String> = None;
    let mut delay: Option<f32> = None;
    let mut late = 0;
    for c in &w.cells {
        let b = boxes.iter().find(|b| b.machine == c.machine);
        let sc = b.and_then(|b| b.screens.iter().find(|s| s.number == c.screen));
        let at = format!(
            "Row {}, column {} ({} · {})",
            c.row + 1,
            c.column + 1,
            b.map_or("a box no longer in the site", |b| b.name.as_str()),
            sc.filter(|s| !s.name.is_empty())
                .map_or_else(|| format!("screen {}", c.screen), |s| s.name.clone())
        );
        let (st, why) = match sc {
            Some(s) => (s.status, s.problem.clone()),
            None => (
                MosaicScreenStatus::Offline,
                Some("not a display box of the site now".into()),
            ),
        };
        if badness(st) > badness(status) {
            status = st;
        }
        if st != MosaicScreenStatus::Showing {
            let bad = badness(st);
            if problem.as_ref().is_none_or(|(p, _)| bad > *p) {
                let what = why.unwrap_or_else(|| format!("{st:?}").to_lowercase());
                problem = Some((bad, format!("{at}: {what}")));
            }
            continue;
        }
        showing += 1;
        match sc.and_then(|s| s.sync.as_ref()) {
            Some(y) => {
                delay = Some(delay.map_or(y.delay_ms, |d: f32| d.max(y.delay_ms)));
                late += y.late_recently;
                if y.group != group && out_of_step.is_none() {
                    out_of_step = Some(format!(
                        "{at} is in another sync group ({})",
                        if y.group.is_empty() { "none" } else { &y.group }
                    ));
                }
            }
            None => {
                if out_of_step.is_none() {
                    out_of_step = Some(format!("{at} reports no frame sync"));
                }
            }
        }
    }
    let synced = showing > 0 && out_of_step.is_none() && late == 0;
    let problem = problem.map(|(_, p)| p).or(out_of_step).or_else(|| {
        (late > 0)
            .then(|| format!("{late} late picture(s) dropped lately: the tiles may be out of step"))
    });
    MosaicWall {
        id: w.id.clone(),
        name: w.name.clone(),
        rows: w.rows,
        columns: w.columns,
        bezel_mm: w.bezel_mm,
        screen_mm: w.screen_mm,
        feed: w.feed.clone(),
        cells: w.cells.clone(),
        status,
        synced,
        problem,
        delay_ms: delay,
        late_recently: late,
    }
}

/// Where a box is reached, and where that came from: the address an admin set (an override),
/// else the one the Site Manager sees its IDN Node at, else the one it asked to join from.
fn address_of(
    b: Option<&StoredBox>,
    reported: Option<&str>,
) -> Option<(String, MosaicAddressFrom)> {
    let b = b.cloned().unwrap_or_default();
    b.address
        .map(|a| (a, MosaicAddressFrom::Admin))
        .or_else(|| reported.map(|a| (a.to_string(), MosaicAddressFrom::Site)))
        .or_else(|| b.joined_from.map(|a| (a, MosaicAddressFrom::Join)))
}

fn valid_source(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}

fn channel_name(n: &str) -> Result<String, ApiError> {
    let n = n.trim();
    if n.is_empty() || n.chars().count() > 60 || n.chars().any(char::is_control) {
        return Err(api(ErrorCode::BadRequest, "name: 1 to 60 characters"));
    }
    Ok(n.to_string())
}

/// Reads every box every `poll` (and when kicked) until the server stops.
pub fn start(shared: &Arc<Shared>, mut stop: tokio::sync::watch::Receiver<bool>) {
    let sh = shared.clone();
    tokio::spawn(async move {
        loop {
            let s = sh.clone();
            let own = sh.auth.identity().name.clone();
            if tokio::task::spawn_blocking(move || s.mosaic.tick(&s.site, &own))
                .await
                .is_err()
            {
                break;
            }
            tokio::select! {
                _ = tokio::time::sleep(sh.mosaic.poll) => {}
                _ = sh.mosaic.kick.notified() => {}
                _ = stop.wait_for(|s| *s) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::DeviceService;

    fn dev(name: &str, kind: &str, services: &[&str]) -> SiteDevice {
        SiteDevice {
            fingerprint: format!("fp-{name}"),
            name: name.into(),
            kind: kind.into(),
            admin: false,
            health: DeviceHealth::Online,
            reason: None,
            problems: vec![],
            last_seen: None,
            version: None,
            restarting: vec![],
            services: services
                .iter()
                .map(|s| DeviceService {
                    id: s.to_string(),
                    name: s.to_string(),
                    state: "running".into(),
                })
                .collect(),
            updating: None,
            update_managed_by: None,
            node: true,
            node_started_at: None,
            machine: None,
            built_in: false,
            relay: false,
        }
    }

    #[test]
    fn vesper_machines_are_never_display_boxes() {
        let names = vec!["Chapel Box".to_string()];
        assert!(!vesper_machine(
            &dev("Lobby", "output", &["display"]),
            &names,
            "Booth",
            &[]
        ));
        // The Vesper app itself, a Node reporting Vesper, this server's own Node, the Node of
        // a Vesper box (by its name).
        assert!(vesper_machine(
            &dev("Chapel Box (Vesper)", "app", &[]),
            &names,
            "Booth",
            &[]
        ));
        assert!(vesper_machine(
            &dev("Side", "output", &["display", "vesper"]),
            &names,
            "Booth",
            &[]
        ));
        assert!(vesper_machine(
            &dev("booth", "output", &["display"]),
            &names,
            "Booth",
            &[]
        ));
        assert!(vesper_machine(
            &dev("Chapel Box", "output", &["display"]),
            &names,
            "Booth",
            &[]
        ));
        // M3-2: a Node a Vesper box reported as its own (by fingerprint), whatever its name.
        assert!(vesper_machine(
            &dev("mini-7", "output", &["display"]),
            &names,
            "Booth",
            &["fp-mini-7".to_string()]
        ));
    }

    /// No IDN Node here (unit tests).
    #[derive(Debug)]
    struct NoNodes;

    impl Nodes for NoNodes {
        fn own(&self) -> Option<String> {
            None
        }
        fn source_name(&self, name: &str) -> String {
            format!("SITE ({name})")
        }
        fn apply(
            &self,
            _: &Site,
            _: &str,
            _: serde_json::Map<String, serde_json::Value>,
        ) -> Result<(), String> {
            Err("no Node".into())
        }
        fn report(&self, _: &Site, _: &str) -> Option<idn_open_protocol::xml::Element> {
            None
        }
        fn media_dir(&self) -> Result<PathBuf, String> {
            Err("no Node".into())
        }
        fn send_media(&self, _: &Path, _: &str, _: bool) -> Result<Option<String>, String> {
            Err("no Node".into())
        }
        fn transfer(&self, _: &str, _: &str) -> Option<Transfer> {
            None
        }
    }

    /// X1-2 / X2-4 / H01-2 (bug run 6): a machine is a Vesper machine only while it runs one:
    /// a removed or long-offline `(Vesper)` device and a box that stopped reporting its Node no
    /// longer count, and on a machine whose Launcher has the display role only live evidence
    /// does. One reported Node per box.
    #[test]
    fn a_machine_stops_being_a_vesper_machine_when_it_no_longer_runs_vesper() {
        let dir = std::env::temp_dir().join(format!("vesper-mosaic-ev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let m = Mosaic::load(&dir, Duration::from_secs(5), Arc::new(NoNodes));
        let chapel = dev("Chapel", "output", &["display"]);
        let mini = dev("mini-7", "output", &["display"]);
        let mut app = dev("Chapel (Vesper)", "app", &[]);
        let runs = |m: &Mosaic, app: &SiteDevice, d: &SiteDevice| {
            m.evidence(std::slice::from_ref(app))
                .runs_vesper(d, "Booth")
        };
        // By name: online, recently offline, long offline, removed.
        assert!(runs(&m, &app, &chapel));
        app.health = DeviceHealth::Offline;
        let ago = |s: u64| {
            let t = std::time::SystemTime::now() - Duration::from_secs(s);
            crate::auth::rfc3339(t, true)
        };
        app.last_seen = Some(ago(120));
        assert!(runs(&m, &app, &chapel));
        app.last_seen = Some(ago(VESPER_GONE.as_secs() + 60));
        assert!(!runs(&m, &app, &chapel));
        app.last_seen = None;
        assert!(!runs(&m, &app, &chapel));
        app.health = DeviceHealth::Removed;
        assert!(!runs(&m, &app, &chapel));
        // Re-roled by its Launcher: a server that stopped a minute ago doesn't count, a running
        // one does; the presentation role again drops that.
        app.health = DeviceHealth::Offline;
        app.last_seen = Some(ago(120));
        m.note_launcher("CHAPEL", &["display".into()]);
        assert!(!runs(&m, &app, &chapel));
        app.health = DeviceHealth::Online;
        assert!(runs(&m, &app, &chapel));
        app.health = DeviceHealth::Offline;
        m.note_launcher("Chapel", &["presentation".into()]);
        assert!(runs(&m, &app, &chapel));

        // By the Node a box reports: while it reports; one Node per box.
        let none = dev("Other (Vesper)", "app", &[]);
        m.note_vesper_node("fp-box", "fp-mini-7", "Chapel");
        assert!(runs(&m, &none, &mini));
        let age = |m: &Mosaic, s: u64| {
            lock(&m.store).vesper_boxes.get_mut("fp-box").unwrap().seen -= s;
        };
        age(&m, 120);
        assert!(runs(&m, &none, &mini));
        m.note_launcher("mini-7", &["display".into()]);
        assert!(!runs(&m, &none, &mini), "re-roled, and not reporting live");
        m.note_launcher("mini-7", &[]);
        age(&m, VESPER_GONE.as_secs());
        assert!(!runs(&m, &none, &mini), "the box stopped reporting");
        m.note_vesper_node("fp-box", "fp-mini-7", "Chapel");
        assert!(runs(&m, &none, &mini));
        m.note_vesper_node("fp-box", "fp-other", "Chapel");
        assert!(!runs(&m, &none, &mini), "the box's Node is another one now");
        let kept: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("machine/mosaic.json")).unwrap())
                .unwrap();
        assert_eq!(kept["vesperBoxes"]["fp-box"]["node"], "fp-other");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// X2-5 (bug run 6): saving a screen's settings keeps what the box has and Vesper can't
    /// say (a slideshow fallback, the sound's channel map).
    #[test]
    fn a_save_keeps_the_boxs_slideshow_and_channel_map() {
        use serde_json::json;
        let has = json!({
            "overscan": 0,
            "fallback": {"show": "slideshow", "slideshow": "lobby", "slide_s": 8, "after_s": 5},
            "audio": {"device": "HDMI", "channels": [2, 3], "volume_db": 0.0, "mute": false},
        });
        let mut st = client::read_settings(&has);
        st.overscan = 2.0;
        let body = with_logo(Some(client::settings_body(1, &st)), 1, None, Some(&has)).unwrap();
        assert_eq!(body["overscan"], 2.0);
        assert_eq!(body["fallback"]["show"], "slideshow");
        assert_eq!(body["fallback"]["slideshow"], "lobby");
        assert_eq!(body["fallback"]["slide_s"], 8);
        assert_eq!(body["audio"]["channels"], json!([2, 3]));
        // Taken, it isn't lost (not sent again).
        let mut now = Some(has.clone());
        client::took_settings(&mut now, &body);
        assert!(!client::settings_lost(&body, now.as_ref().unwrap()));
        // Another sound card: its own channels; black instead of the slideshow: black.
        st.sound.as_mut().unwrap().device = "USB".into();
        st.fallback = protocol::MosaicFallback::Black;
        let body = with_logo(Some(client::settings_body(1, &st)), 1, None, Some(&has)).unwrap();
        assert!(body["audio"].get("channels").is_none(), "{body}");
        assert_eq!(body["fallback"]["show"], "black");
    }

    /// H01-5 (bug run 6): Linux is anything but Windows and macOS (the Node reports the
    /// distribution's name).
    #[test]
    fn linux_is_any_distribution() {
        let on = |os: Option<&str>| {
            let mut d = dev("Box", "output", &[]);
            d.machine = Some(protocol::SiteDeviceMachine {
                cpu: None,
                cpus: None,
                gpus: vec![],
                os: os.map(str::to_string),
                os_version: None,
                kernel: None,
                booted_at: None,
                memory_total: None,
                memory_free: None,
                reported_at: None,
            });
            linux(&d)
        };
        for os in [
            "Ubuntu",
            "Zorin OS",
            "Pop!_OS",
            "elementary OS",
            "openSUSE",
            "NixOS",
        ] {
            assert!(on(Some(os)), "{os}");
        }
        for os in ["Windows 11 Pro", "macOS", "Mac OS X"] {
            assert!(!on(Some(os)), "{os}");
        }
        assert!(!on(None));
        assert!(!on(Some(" ")));
    }

    /// A transfer's end as shown: IDN Beta21's `failed: interrupted` (one that ended without a
    /// result) is a failure like any other, tried again; still going or sent is none.
    #[test]
    fn transfer_failures_as_shown() {
        assert_eq!(
            transfer_failed("failed: interrupted").as_deref(),
            Some("the transfer was interrupted")
        );
        assert_eq!(
            transfer_failed("failed: the stream stopped").as_deref(),
            Some("the stream stopped")
        );
        assert_eq!(
            transfer_failed("failed").as_deref(),
            Some("the transfer failed")
        );
        assert_eq!(
            transfer_failed("refused").as_deref(),
            Some("the box refused it")
        );
        for going in ["offered", "sent", ""] {
            assert_eq!(transfer_failed(going), None, "{going}");
        }
    }

    /// H01-1 (bug run 6): a tile-only command (or a screen leaving a wall) sent with the site's
    /// logo and the box's whole fallback counts as sent, so it isn't posted again every poll.
    #[test]
    fn a_tile_command_with_the_logo_counts_as_sent() {
        use serde_json::json;
        let dir = std::env::temp_dir().join(format!("vesper-mosaic-sent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let m = Mosaic::load(&dir, Duration::from_secs(5), Arc::new(NoNodes));
        lock(&m.store).screen_mut("box", 1).unsent = true;
        let cmd = lock(&m.store).settings_command("box", 1, true).unwrap();
        assert_eq!(cmd, json!({"screen": 1, "wall": null}));
        // A real box reports its whole fallback.
        let has = json!({"fallback": {"show": "logo", "after_s": 5, "logo": "logo.png"}});
        let body = with_logo(
            Some(cmd.clone()),
            1,
            Some("logo-0123456789abcdef.png"),
            Some(&has),
        )
        .unwrap();
        assert_eq!(body["fallback"]["after_s"], 5);
        assert!(
            !same_command(&cmd, &body),
            "the body carries the box's fallback"
        );
        m.settings_taken("box", 1, &body, &cmd);
        assert!(!lock(&m.store).boxes["box"].screens[&1].unsent);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M3b (the IDN maintainers, 2026-11-26): the logo goes into a screen's fallback without
    /// dropping it when a fallback is sent whole; headless needs both of the Node's switches;
    /// router lists compare whatever their separators; Vesper's logo names only.
    #[test]
    fn logos_modes_and_router_lists() {
        use serde_json::json;
        let has = json!({"fallback": {"show": "last", "after_s": 5, "logo": "logo-old.png"}});
        // Ready: given to a screen never set from Vesper, with the box's own fallback.
        assert_eq!(
            with_logo(None, 2, Some("logo-new.png"), Some(&has)),
            Some(
                json!({"screen": 2, "fallback": {"show": "last", "after_s": 5, "logo": "logo-new.png"}})
            )
        );
        // Not yet: a fallback sent whole keeps the logo the box has.
        let body = json!({"screen": 1, "fallback": {"show": "logo", "after_s": 9}});
        assert_eq!(
            with_logo(Some(body.clone()), 1, None, Some(&has)).unwrap()["fallback"]["logo"],
            "logo-old.png"
        );
        assert_eq!(with_logo(None, 1, None, Some(&has)), None);
        assert!(same_command(
            &body,
            &with_logo(Some(body.clone()), 1, Some("logo-new.png"), None).unwrap()
        ));
        assert!(same_command(
            &json!({"screen": 1, "wall": null}),
            &json!({"screen": 1, "wall": null, "fallback": {"logo": "x.png"}})
        ));
        let apps = |d: &str, b: &str| {
            idn_open_protocol::xml::Element::new("idn_node_report").with_child(
                idn_open_protocol::xml::Element::new("apps")
                    .with_attr("display", d)
                    .with_attr("display_boot", b),
            )
        };
        assert_eq!(
            reported_mode(&apps("on", "on")),
            Some(MosaicServiceMode::Headless)
        );
        assert_eq!(
            reported_mode(&apps("true", "false")),
            Some(MosaicServiceMode::Desktop)
        );
        assert_eq!(
            reported_mode(&apps("off", "on")),
            Some(MosaicServiceMode::Off)
        );
        assert_eq!(
            reported_mode(&idn_open_protocol::xml::Element::new("idn_node_report")),
            None
        );
        // The display service as the box reports it (Beta14) comes first: `runs` says where.
        let svc = |on: &str, runs: &str| {
            apps("on", "on").with_child(
                idn_open_protocol::xml::Element::new("service")
                    .with_attr("id", "display")
                    .with_attr("on", on)
                    .with_attr("runs", runs),
            )
        };
        assert_eq!(
            reported_mode(&svc("true", "session")),
            Some(MosaicServiceMode::Desktop)
        );
        assert_eq!(
            reported_mode(&svc("true", "node")),
            Some(MosaicServiceMode::Headless)
        );
        assert_eq!(
            reported_mode(&svc("false", "node")),
            Some(MosaicServiceMode::Off)
        );
        assert_eq!(
            reported_mode(&svc("true", "")),
            Some(MosaicServiceMode::Headless),
            "worked out from <apps>"
        );
        assert!(same_routers(
            "Channel Main=A; Channel Side=",
            "Channel Side=\nChannel Main=A"
        ));
        assert!(!same_routers("Channel Main=A", "Channel Main=B"));
        assert!(is_logo_file_name(&logo_file_name("0123456789abcdef")));
        assert!(!LOGO_NAMES.iter().any(|n| is_logo_file_name(n)));
        // Beta14 boxes: the two names take turns, so a screen's path always changes.
        assert_eq!(next_logo_name(None), "logo-a.png");
        assert_eq!(next_logo_name(Some("logo-a.png")), "logo-b.png");
        assert_eq!(next_logo_name(Some("logo-b.png")), "logo-a.png");
        // What lane I's push used, or a pre-Beta14 name, then an upgraded box.
        assert_eq!(next_logo_name(Some("logo.png")), "logo-a.png");
        assert_eq!(
            next_logo_name(Some("logo-0123456789abcdef.png")),
            "logo-a.png"
        );
        // Which boxes' Nodes take a replacement (IDN Beta14 and later).
        for (v, want) in [
            ("0.3.4-Beta14 (805168e)", Some(true)),
            ("0.3.5-Beta15", Some(true)),
            ("v0.3.4", Some(true)),
            ("0.4.0-Alpha1", Some(true)),
            ("0.3.4-Beta13 (7f11bfb)", Some(false)),
            ("0.3.3", Some(false)),
            ("0.2.0-Alpha93 2026-11-20", Some(false)),
            ("0.3.4-Alpha2", Some(false)),
            ("unknown", None),
        ] {
            assert_eq!(takes_replace(Some(v)), want, "{v}");
        }
        assert_eq!(takes_replace(None), None);
        assert!(!is_logo_file_name("logo.png"));
        assert!(!is_logo_file_name("logo-mine.png"));
    }

    /// H03-1 / X3-1 (bug run 7): a box's screens are pointed at a logo name only once that
    /// name's replacement arrived (`sent`); while it is on its way, and after one that failed or
    /// stuck (being sent again), they keep the name that arrived last. H03-2: a replacement is
    /// taken as stuck only once its fetching stopped moving.
    #[test]
    fn logo_names_follow_what_arrived() {
        let dir = std::env::temp_dir().join(format!("vesper-mosaic-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let m = Mosaic::load(&dir, Duration::from_secs(5), Arc::new(NoNodes));
        let mut b = StoredBox {
            logo: Some("h2".into()),
            logo_file: Some("logo-b.png".into()),
            logo_sent_at: Some(unix_now()),
            logo_offer: Some("media-replace:2".into()),
            logo_shown: Some("logo-a.png".into()),
            ..StoredBox::default()
        };
        assert_eq!(m.ready_logo(Some(&b)).as_deref(), Some("logo-a.png"));
        b.logo_arrived = true;
        assert_eq!(m.ready_logo(Some(&b)).as_deref(), Some("logo-b.png"));
        // Failed or stuck, as `follow_logo_transfers` leaves it, long after it was sent.
        b.logo_arrived = false;
        b.logo = None;
        b.logo_offer = None;
        b.logo_file = b.logo_shown.clone();
        b.logo_sent_at = None;
        assert_eq!(m.ready_logo(Some(&b)).as_deref(), Some("logo-a.png"));
        // The first logo for a box: nothing to point its screens at until it arrives.
        b.logo_shown = None;
        b.logo_file = None;
        assert_eq!(m.ready_logo(Some(&b)), None);
        // A box from before Beta14: its own name, once it has had time to arrive.
        let old = StoredBox {
            logo_file: Some(logo_file_name("0123456789abcdef")),
            logo_sent_at: Some(unix_now() - 60),
            ..StoredBox::default()
        };
        assert_eq!(m.ready_logo(Some(&old)), old.logo_file);

        let t0 = Instant::now();
        let mut map = HashMap::new();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let fetching = |done| Transfer::new("offered", done);
        let observe = |map: &mut HashMap<String, Progress>, offer, seen: Option<&Transfer>, s| {
            Progress::observe(map, "fp-box", offer, seen, at(s))
        };
        assert_eq!(
            observe(&mut map, "o1", Some(&fetching(0)), 0),
            Duration::ZERO
        );
        // Still fetching at 300 s (bytes moving): never stuck.
        for s in (60..=300).step_by(60) {
            let still = observe(&mut map, "o1", Some(&fetching(s * 1000)), s);
            assert_eq!(still, Duration::ZERO, "{s}");
        }
        // Stopped moving: stuck once it hasn't moved for LOGO_STUCK.
        assert_eq!(
            observe(
                &mut map,
                "o1",
                Some(&fetching(300_000)),
                300 + LOGO_STUCK.as_secs()
            ),
            LOGO_STUCK
        );
        // No longer listed: counts from when that was first seen.
        assert_eq!(observe(&mut map, "o1", None, 500), Duration::ZERO);
        assert_eq!(observe(&mut map, "o1", None, 530), Duration::from_secs(30));
        // A new offer starts again.
        assert_eq!(observe(&mut map, "o2", None, 600), Duration::ZERO);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M3-2: an admin's address overrides the one the Site Manager sees the Node at, which
    /// comes before the one the box asked to join from.
    #[test]
    fn a_boxs_address_comes_from_the_site() {
        let mut b = StoredBox {
            joined_from: Some("192.0.2.9".into()),
            ..Default::default()
        };
        assert_eq!(
            address_of(Some(&b), None),
            Some(("192.0.2.9".into(), MosaicAddressFrom::Join))
        );
        assert_eq!(
            address_of(Some(&b), Some("192.0.2.40")),
            Some(("192.0.2.40".into(), MosaicAddressFrom::Site))
        );
        b.address = Some("lobby.local".into());
        assert_eq!(
            address_of(Some(&b), Some("192.0.2.40")),
            Some(("lobby.local".into(), MosaicAddressFrom::Admin))
        );
        assert_eq!(address_of(None, None), None);
    }

    #[test]
    fn a_channel_resolves_to_its_source() {
        let s = Store {
            channels: vec![StoredChannel {
                id: "c1".into(),
                name: "Main".into(),
                source: "BOOTH (Program)".into(),
            }],
            ..Default::default()
        };
        let ch = |c: &str| MosaicFeed::Channel { channel: c.into() };
        let nodes = IdnNodes::new(false);
        let none = Routing::default();
        assert_eq!(resolve(&s, &none, &nodes, &ch("c1")), "BOOTH (Program)");
        assert_eq!(resolve(&s, &none, &nodes, &ch("gone")), "");
        assert_eq!(resolve(&s, &none, &nodes, &MosaicFeed::None), "");
        // M3-3: on its router, a channel is the router's source on this server.
        let want = wanted_routers(&s);
        assert_eq!(routers_list(&want), "Channel Main=BOOTH (Program)");
        let on = Routing {
            host: Some("fp-site".into()),
            applied: Some(want),
            ..Default::default()
        };
        assert_eq!(
            resolve(&s, &on, &nodes, &ch("c1")),
            format!(
                "{} (Channel Main)",
                idn_open_protocol::discovery::hostname()
            )
        );
        assert_eq!(router_name("Lobby = (East); 2"), "Channel Lobby East 2");
    }

    /// H01-1: a file from before "never set" feeds: a "none" feed outside a wall is taken as
    /// never set (it was what a settings change left), a wall tile's stays.
    #[test]
    fn an_older_file_takes_none_feeds_as_never_set() {
        let mut s: Store = serde_json::from_value(serde_json::json!({
            "boxes": {"fp-a": {"added": true, "screens": {
                "1": {"feed": {"kind": "none"}, "settings": {"fit": "fill"}},
                "2": {"feed": {"kind": "none"}},
                "3": {"feed": {"kind": "source", "source": "BOOTH (Program)"}},
            }}},
            "walls": [{"id": "w", "name": "Front", "rows": 1, "columns": 1, "bezelMm": [0.0, 0.0],
                "cells": [{"row": 0, "column": 0, "machine": "fp-a", "screen": 2}]}],
        }))
        .unwrap();
        s.upgrade();
        let sc = &s.boxes["fp-a"].screens;
        assert_eq!(sc[&1].feed, None);
        assert_eq!(sc[&2].feed, Some(MosaicFeed::None), "a wall's tile");
        assert!(matches!(sc[&3].feed, Some(MosaicFeed::Source { .. })));
        // Saved as the new format: read again, nothing changes.
        let again: Store = serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(again.version, STORE_VERSION);
        assert_eq!(again.boxes["fp-a"].screens[&2].feed, Some(MosaicFeed::None));
        let mut again = again;
        again.upgrade();
        assert_eq!(again.boxes["fp-a"].screens[&2].feed, Some(MosaicFeed::None));
    }

    /// X3-1: a screen that left a wall, with no settings of its own, is told `wall: null` while
    /// that hasn't reached its box; R1-7: a wall's group fits the service's 64 bytes.
    #[test]
    fn a_tile_leaving_is_sent_until_taken_and_groups_fit() {
        let mut s = Store::default();
        s.screen_mut("fp-a", 1).feed = Some(MosaicFeed::None);
        assert_eq!(s.settings_command("fp-a", 1, false), None);
        assert_eq!(
            s.settings_command("fp-a", 1, true),
            Some(serde_json::json!({"screen": 1, "wall": null}))
        );
        let w = StoredWall {
            id: "w".into(),
            name: "Ж".repeat(48),
            rows: 1,
            columns: 1,
            bezel_mm: [0.0, 0.0],
            screen_mm: None,
            feed: MosaicFeed::None,
            cells: vec![],
        };
        let g = w.group();
        assert!(g.len() <= GROUP_MAX && g.starts_with("wall:ЖЖ"), "{g}");
    }

    fn screen(n: u32, status: MosaicScreenStatus, sync: Option<(&str, u64)>) -> MosaicScreen {
        MosaicScreen {
            number: n,
            name: format!("HDMI-{n}"),
            width: None,
            height: None,
            feed: MosaicFeed::None,
            showing: String::new(),
            test_pattern: false,
            settings: MosaicScreenSettings::default(),
            status,
            problem: None,
            wall: Some("w".into()),
            sync: sync.map(|(g, late)| MosaicSync {
                group: g.into(),
                mode: "sync/1".into(),
                delay_ms: 40.0 + n as f32,
                late: late * 10,
                late_recently: late,
            }),
        }
    }

    #[test]
    fn a_walls_state_is_its_tiles_worst_and_their_sync() {
        let wall = StoredWall {
            id: "w".into(),
            name: "Front".into(),
            rows: 1,
            columns: 2,
            bezel_mm: [0.0, 0.0],
            screen_mm: None,
            feed: MosaicFeed::None,
            cells: vec![
                MosaicWallCell {
                    row: 0,
                    column: 0,
                    machine: "fp-a".into(),
                    screen: 1,
                },
                MosaicWallCell {
                    row: 0,
                    column: 1,
                    machine: "fp-a".into(),
                    screen: 2,
                },
            ],
        };
        assert_eq!(
            wall.tile(&wall.cells[1]).group.as_deref(),
            Some("wall:Front")
        );
        let boxed = |screens: Vec<MosaicScreen>| {
            vec![MosaicBox {
                machine: "fp-a".into(),
                name: "Lobby".into(),
                health: DeviceHealth::Online,
                service: "running".into(),
                address: None,
                address_from: None,
                service_mode: None,
                service_mode_reported: false,
                headless_ok: false,
                update_managed_by: None,
                logo_state: None,
                added: false,
                vesper: false,
                conflict: None,
                reachable: true,
                problem: None,
                screens,
            }]
        };
        let ok = wall_state(
            &wall,
            &boxed(vec![
                screen(1, MosaicScreenStatus::Showing, Some(("wall:Front", 0))),
                screen(2, MosaicScreenStatus::Showing, Some(("wall:Front", 0))),
            ]),
        );
        assert_eq!(ok.status, MosaicScreenStatus::Showing);
        assert!(ok.synced);
        assert_eq!(ok.delay_ms, Some(42.0));
        assert_eq!(ok.problem, None);
        let late = wall_state(
            &wall,
            &boxed(vec![
                screen(1, MosaicScreenStatus::Showing, Some(("wall:Front", 3))),
                screen(2, MosaicScreenStatus::Showing, Some(("wall:Front", 0))),
            ]),
        );
        assert!(!late.synced);
        assert_eq!(late.late_recently, 3);
        let lost = wall_state(
            &wall,
            &boxed(vec![
                screen(1, MosaicScreenStatus::Showing, Some(("wall:Front", 0))),
                screen(2, MosaicScreenStatus::Lost, None),
            ]),
        );
        assert_eq!(lost.status, MosaicScreenStatus::Lost);
        assert!(
            lost.problem
                .unwrap()
                .starts_with("Row 1, column 2 (Lobby · HDMI-2)")
        );
        let gone = wall_state(&wall, &[]);
        assert_eq!(gone.status, MosaicScreenStatus::Offline);
        assert!(!gone.synced);
    }
}
