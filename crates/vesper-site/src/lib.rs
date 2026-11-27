//! # `vesper-site` — the site: IDN's Site Manager as a module of the Vesper server
//!
//! Also who a request comes from ([`auth`]: the server's identity, pairing codes, paired
//! clients) and the `/api/v1/site` routes ([`api`], over any router state that gives the
//! [`Site`]). The server (`vesper-server`) re-exports this crate as `site` and [`auth`] as
//! `auth`.
//!
//! Every Vesper server is always in a site, and each site has exactly one Site Manager (the site
//! authority, its discovery server and its HTTPS API), run in-process through
//! `idn_site_manager::host` by one Vesper server. A server on its own runs its own, so a
//! standalone install is a one-machine site. A server can instead join another server's site
//! as one of its machines (an IDN `Enrollment` in `<data>/machine/site-member/`, by code and/or
//! approval: [`Site::join`]); once it is in, its own Site Manager stops and stays off until it
//! leaves ([`Site::leave`]), when it runs its own site again. A server whose own site has other
//! machines can't join another. (`vesper --no-site-manager` keeps it off, for tests.)
//!
//! - **Paired clients' keys:** pairing makes a named site API key per client with the Site
//!   Manager ([`Site::create_client_key`]: `app:vesper`, scope `control`); every server checks
//!   keys locally ([`Site::verify`]): the site's server with `Handle::verify_token`, the
//!   others with `TokenCheck::from_enrollment`, which also reports each key's use to the site.
//!   Pairing at a machine of the site that isn't its server is relayed to the site's server:
//!   with the pairing code the site gave that machine it is approved at once, otherwise it
//!   waits for an admin's approval there ([`pairing`]).
//!
//! ```text
//!  Oriel ──REST /api/v1/site/*──► site::api ──► Site ──Handle::run_from(user, caller, cmd)──► Site Manager
//!        ◄──WS siteEvent / siteChanged──┐        │   (role checked + audited by IDN)           │
//!                                       └── event thread: Handle::events_wait ◄────────────────┘
//! ```
//!
//! - **Files** (all under `<data>/machine/`, part of a backup): `site-manager/` is the Site
//!   Manager's folder (the site CA key, devices, users, its audit, join codes); `site.json` holds
//!   this server's own part (designated or not, the access level, the client sign-ins, when
//!   each device was last seen); `site-audit.jsonl` what this server decides itself (sign-ins,
//!   unlocks, the access level, requests refused before they reach the Site Manager).
//! - **Ports** (IDN's port map): discovery 5959, the Site Manager's HTTPS API 29202 (and IDN
//!   Voice on the next port). [`SiteConfig`] overrides them.
//! - **Who acts:** see [`Actor`], decided by the site's access level ([`SiteAccess`]: `open`,
//!   `pin`, `users`). Changes run as that user through the Site Manager, which checks their
//!   role (IDN's rules: joins, devices, users and settings are the admins'; operators may
//!   identify and fetch logs) and writes its audit with the client and its address
//!   ([`host::Caller`]).
//! - **The admin PIN** (`pin`): stored as an Argon2id hash in `site.json`; an unlock is kept in
//!   memory per paired client and ends after [`SiteConfig::unlock_idle`] without an admin
//!   action, on demand, or with a restart. Wrong PINs: [`PIN_TRIES`] in a row lock the client
//!   out for [`SiteConfig::pin_lockout`], doubling each time (at most [`PIN_LOCKOUT_MAX`]).
//! - **Events:** a thread per running Site Manager waits for its typed events
//!   (`Handle::events_wait`) and sends them as `siteEvent` and `siteChanged` ([`events`]).
//! - **Health:** the device list is `Handle::device_health` with the removed machines.
//! - **Tokens** for the IDN engines are issued through the host handle for the user a client
//!   acts as: its site user with `users`, else a name of its own (`vesper:<client id>`), so
//!   one client's tokens can be withdrawn alone ([`Actor::token_user`]). The scope a role may
//!   have is decided here ([`scope_role`]).
//!
//! Every method that touches the Site Manager blocks: call them from `spawn_blocking`.

mod access;
pub mod api;
pub mod auth;
mod devices;
pub mod events;
mod joins;
mod keys;
pub mod launchers;
mod lifecycle;
mod membership;
pub mod pairing;
mod relays;
pub use relays::is_vesper_box;
mod users;

use crate::auth::{Access, Client, Credential};
use events::{rfc3339, unix_now};
use idn_open_protocol::trust::token::Scope;
use idn_open_protocol::trust::{
    self as idn_trust, DeviceRole, Enrollment, EnrollmentConfig, JoinMode as IdnJoinMode,
    TrustState,
};
use idn_open_protocol::web::TokenCheck;
use idn_site_manager::host;
use idn_site_manager::typed::{SiteCommand, TokenAud};
use idn_site_manager::users::{self as idn_users, Role, Users};
use protocol::{
    ApiError, DeviceHealth, DeviceProblem, ErrorCode, JoinCodeInfo, JoinMode, JoinRequestInfo,
    MembershipState, ServerMessage, SiteAccess, SiteAuditEntry, SiteAuditSource, SiteDevice,
    SiteEvent, SiteMembership, SitePart, SiteRole, SiteSession, SiteSettings, SiteStatus, SiteUser,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::Write as _;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// How the Site Manager module runs (the server's `ServerConfig::site`).
#[derive(Clone, Debug)]
pub struct SiteConfig {
    /// The module may run (when the stored setting says so). `false` keeps it off whatever
    /// the setting (`vesper --no-site-manager`, tests).
    pub available: bool,
    /// The discovery server's port (IDN: 5959; 0 = any free port).
    pub discovery_port: u16,
    /// The Site Manager's HTTPS API port (IDN: 29202; 0 = any free port). IDN Voice takes
    /// the next one.
    pub api_port: u16,
    /// Announce the site on mDNS.
    pub announce: bool,
    /// A new site (no machine yet) opens its approval window this long at the start.
    pub open_new_site: Option<Duration>,
    /// How long the event thread waits for an event before it looks whether to stop (a
    /// stop takes at most this long).
    pub event_wait: Duration,
    /// How often the approval window, the join mode and the PIN unlocks are looked at (for
    /// `siteChanged`).
    pub tick: Duration,
    /// A PIN unlock ends after this long without an admin action.
    pub unlock_idle: Duration,
    /// The first lockout after [`PIN_TRIES`] wrong PINs in a row (each next one doubles).
    pub pin_lockout: Duration,
    /// How long a client's pairing waits for its answer, and an answer for its client
    /// ([`pairing::PAIRING_LIFE`]).
    pub pairing_life: Duration,
    /// At a relay, how long `POST /api/v1/pair` with a code waits for the site's answer
    /// before it answers `202` (a right code is approved at once at the site's server).
    pub relayed_code_wait: Duration,
    /// At the site's server, a key Vesper made for an Oriel pairing that is still unused this
    /// long after it was made is revoked ([`pairing::UNUSED_KEY_LIFE`]: 24 hours).
    pub unused_key_life: Duration,
    /// How often the unused keys are looked for (hourly).
    pub unused_key_sweep: Duration,
    /// A Site Manager that didn't start (a port in use) is tried again this often while it
    /// should run.
    pub start_retry: Duration,
    /// A pairing waiting for approval whose client hasn't asked for its answer this long ends
    /// ([`pairing::PAIRING_IDLE`]; a waiting client asks every second or so).
    pub pairing_idle: Duration,
}

impl Default for SiteConfig {
    fn default() -> Self {
        SiteConfig {
            available: true,
            discovery_port: idn_open_protocol::discovery::ds::DEFAULT_PORT,
            api_port: idn_site_manager::PANEL_PORT,
            announce: true,
            open_new_site: Some(Duration::from_secs(600)),
            event_wait: Duration::from_millis(250),
            tick: Duration::from_millis(500),
            unlock_idle: Duration::from_secs(30 * 60),
            pin_lockout: Duration::from_secs(60),
            pairing_life: pairing::PAIRING_LIFE,
            relayed_code_wait: Duration::from_secs(3),
            unused_key_life: pairing::UNUSED_KEY_LIFE,
            unused_key_sweep: Duration::from_secs(3600),
            start_retry: Duration::from_secs(15),
            pairing_idle: pairing::PAIRING_IDLE,
        }
    }
}

impl SiteConfig {
    /// The module never runs.
    pub fn off() -> Self {
        SiteConfig {
            available: false,
            ..SiteConfig::default()
        }
    }

    /// For tests: any free ports, no mDNS, no window at the start, quick ticks.
    pub fn loopback() -> Self {
        SiteConfig {
            available: true,
            discovery_port: 0,
            api_port: 0,
            announce: false,
            open_new_site: None,
            event_wait: Duration::from_millis(100),
            tick: Duration::from_millis(100),
            ..SiteConfig::default()
        }
    }
}

/// `<data>/machine/site.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    /// This server is the designated Site Manager.
    #[serde(default = "yes")]
    enabled: bool,
    /// The site's first start was done (its join mode set).
    #[serde(default)]
    initialized: bool,
    /// The join mode, as last read (shown while the module is off).
    #[serde(default)]
    join_mode: JoinMode,
    /// Who may do admin actions.
    #[serde(default)]
    access: SiteAccess,
    /// The admin PIN's Argon2id hash (PHC string), with `access: pin`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pin_hash: Option<String>,
    /// Paired client id → the site user it signed in as (`access: users`).
    #[serde(default)]
    sessions: BTreeMap<String, String>,
    /// Device fingerprint → when it was last seen connected (Unix seconds).
    #[serde(default)]
    last_seen: BTreeMap<String, u64>,
    /// This server joining, or in, another server's site: that site's discovery address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    member: Option<String>,
    /// The ids of the keys this server made for Oriel pairings (as the site's server),
    /// until they are revoked: revoked when still unused after a day.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pairing_keys: BTreeSet<String>,
    /// Started from a damaged file (its access unknown): until the access is set again, a
    /// paired client that hasn't signed in only reads, even with no admin user (fails closed:
    /// a site on `pin` normally has no users at all).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    damaged: bool,
    /// The machines whose pairing relay mark was decided on ([`relays`]): marked by this
    /// server (a Vesper box), or set by an admin. Never marked again by the start's pass.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    relay_decided: BTreeSet<String>,
    /// Launcher keys (their ids) → the machine each is bound to ([`launchers`]).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    launcher_keys: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

impl Stored {
    /// What a damaged `site.json` (moved aside, kept for a look) is started with: the defaults,
    /// but access `users` and `damaged`, never more open than it was: paired clients only read
    /// (sign in as a site user if there is one) until the server machine sets the access again.
    fn after_damage(file: &Path, e: &serde_json::Error) -> Stored {
        let aside = vesper_core::fs::keep_aside(file)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|e| format!("(not moved: {e})"));
        tracing::error!(
            file = %file.display(),
            kept = %aside,
            "the site settings are damaged ({e}): this server starts with access \"users\", no sign-ins and no PIN, and paired clients only read; set the access again on the server machine"
        );
        Stored {
            // A Site Manager that ran before keeps its own join mode.
            initialized: file.with_file_name("site-manager").is_dir(),
            access: SiteAccess::Users,
            damaged: true,
            ..Stored::default()
        }
    }
}

impl Default for Stored {
    fn default() -> Self {
        Stored {
            enabled: true,
            initialized: false,
            join_mode: JoinMode::default(),
            access: SiteAccess::default(),
            pin_hash: None,
            sessions: BTreeMap::new(),
            last_seen: BTreeMap::new(),
            member: None,
            pairing_keys: BTreeSet::new(),
            damaged: false,
            relay_decided: BTreeSet::new(),
            launcher_keys: BTreeMap::new(),
        }
    }
}

/// Who makes a request, on the site.
///
/// - The server machine itself (a trusted local request) acts as `admin`, whatever the access
///   level.
/// - `open`: a paired client acts as `admin`.
/// - `pin`: a paired client acts as `admin` while unlocked with the PIN; locked, it reads (a
///   viewer role without a user: the audit and logs, no changes, no tokens).
/// - `users`: a paired client acts as the site user it signed in as (`POST
///   /api/v1/site/session`), while that user exists. Should the site have no admin user (one
///   removed on another tool), a client that hasn't signed in acts as `admin`, so the site
///   can't be locked out; otherwise it may only read until it signs in. After a damaged
///   `site.json` (its access unknown), it only reads until the access is set again.
#[derive(Clone, Debug)]
pub struct Actor {
    /// The site's access level.
    pub access: SiteAccess,
    /// The site user (`admin`: the built-in administrator).
    pub user: Option<String>,
    /// Their role.
    pub role: Option<Role>,
    /// Signed in as `user` (`users`).
    pub signed_in: bool,
    /// Unlocked with the PIN (`pin`), and for how much longer without an admin action.
    pub unlocked: Option<Duration>,
    /// The paired client's id.
    pub client_id: Option<String>,
    /// The client, for the audit (its name, or `server machine`).
    pub client: String,
    /// Its address.
    pub peer: String,
}

/// The audience of the keys and tokens this server takes: `app:vesper`, for every Vesper
/// server of the site.
pub const APP: &str = protocol::PRODUCT_ID;
/// The audience as IDN writes it (`app:vesper`).
pub fn app_audience() -> String {
    format!("{}{APP}", idn_open_protocol::trust::token::APP_PREFIX)
}

/// This server's membership of another server's site: the IDN enrollment, and the check of
/// that site's keys and tokens (reporting each named key's use to the site).
struct Member {
    server: String,
    enrollment: Arc<Enrollment>,
    check: Arc<TokenCheck>,
}

/// The client named in the audit for the server machine itself.
pub const SERVER_MACHINE: &str = "server machine";
/// Tokens issued to a paired client that doesn't act as a site user are this followed by its
/// id.
pub const CLIENT_TOKEN_PREFIX: &str = "vesper:";

impl Actor {
    /// Who the Site Manager's audit names for a change: the address and the client.
    pub fn caller(&self) -> host::Caller {
        host::Caller {
            address: self.peer.clone(),
            client: self.client.clone(),
        }
    }

    /// The user engine tokens are issued to and withdrawn from for this client: the site user
    /// it signed in as (`users`); for another paired client a name of its own,
    /// `vesper:<client id>`, so its tokens can be withdrawn without anyone else's; the
    /// server machine itself: `admin`.
    pub fn token_user(&self) -> Option<String> {
        if self.signed_in {
            return self.user.clone();
        }
        match &self.client_id {
            Some(id) => Some(format!("{CLIENT_TOKEN_PREFIX}{id}")),
            None => self.user.clone(),
        }
    }

    /// The protocol's view.
    pub fn session(&self) -> SiteSession {
        SiteSession {
            access: self.access,
            user: self.user.clone(),
            role: self.role.map(site_role),
            signed_in: self.signed_in,
            unlocked: self.unlocked.is_some(),
            unlock_seconds_left: self.unlocked.map(|d| d.as_secs_f64().ceil() as u64),
        }
    }
}

/// IDN's role as the protocol's.
pub fn site_role(r: Role) -> SiteRole {
    match r {
        Role::Viewer => SiteRole::Viewer,
        Role::Operator => SiteRole::Operator,
        Role::Admin => SiteRole::Admin,
    }
}

fn idn_role(r: SiteRole) -> Role {
    match r {
        SiteRole::Viewer => Role::Viewer,
        SiteRole::Operator => Role::Operator,
        SiteRole::Admin => Role::Admin,
    }
}

fn our_mode(m: IdnJoinMode) -> JoinMode {
    match m {
        IdnJoinMode::Approval => JoinMode::Approval,
        IdnJoinMode::CodeOrApproval => JoinMode::CodeOrApproval,
        IdnJoinMode::CodeAndApproval => JoinMode::CodeAndApproval,
    }
}

/// The least role that may have a token with `scope`: viewers `read`; operators `control`,
/// `bus:<bus>` and `iem:<mix>` (as the Site Manager's own token command allows). `None`: not
/// a scope.
pub fn scope_role(scope: &str) -> Option<Role> {
    let named = |p: &str| {
        scope
            .strip_prefix(p)
            .is_some_and(|n| !n.is_empty() && n.len() <= 64 && !n.contains(char::is_whitespace))
    };
    match scope {
        "read" => Some(Role::Viewer),
        "control" => Some(Role::Operator),
        _ if named("bus:") || named("iem:") => Some(Role::Operator),
        _ => None,
    }
}

/// An admin PIN this API accepts: 4–8 digits.
pub fn valid_pin(pin: &str) -> bool {
    (4..=8).contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_digit())
}

/// An Argon2id hash of `secret` (PHC string).
fn hash_secret(secret: &str) -> Result<String, ApiError> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| api_err(ErrorCode::Internal, e.to_string()))?;
    let salt =
        SaltString::encode_b64(&salt).map_err(|e| api_err(ErrorCode::Internal, e.to_string()))?;
    argon2::Argon2::default()
        .hash_password(secret.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| api_err(ErrorCode::Internal, e.to_string()))
}

/// `secret` matches `hash` (from [`hash_secret`]).
fn verify_secret(hash: &str, secret: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    PasswordHash::new(hash).is_ok_and(|h| {
        argon2::Argon2::default()
            .verify_password(secret.as_bytes(), &h)
            .is_ok()
    })
}

fn api_err(code: ErrorCode, msg: impl Into<String>) -> ApiError {
    ApiError::new(code, msg)
}

/// A Site Manager answer's error as an API error (the status from what it says).
fn from_idn(e: String) -> ApiError {
    let code = if e.contains("may not") || e.contains("not logged in") {
        ErrorCode::Forbidden
    } else if e.starts_with("no pending request")
        || e.starts_with("no join code")
        || e.starts_with("no such")
        || e.starts_with("no user")
        || e.starts_with("no API key")
        || e.starts_with("no pairing")
        || e.starts_with("no key request")
    {
        ErrorCode::NotFound
    } else if e.contains("isn't online") {
        ErrorCode::Conflict
    } else {
        ErrorCode::BadRequest
    };
    ApiError::new(code, e)
}

fn not_running() -> ApiError {
    api_err(
        ErrorCode::Unavailable,
        "this server isn't running the site's Site Manager: turn the site on in its site settings first",
    )
}

/// Events kept for `GET /api/v1/site/events`.
const RECENT_EVENTS: usize = 200;
/// Sign-in attempts allowed per address per minute.
const SIGN_INS_PER_MINUTE: usize = 5;
/// Wrong PINs in a row before a client is locked out.
pub const PIN_TRIES: u32 = 5;
/// The longest PIN lockout.
pub const PIN_LOCKOUT_MAX: Duration = Duration::from_secs(15 * 60);
/// A device asked to restart that isn't back after this long is shown as it is.
const RESTART_GRACE: Duration = Duration::from_secs(120);
/// The audit file is trimmed to the newest [`AUDIT_KEEP`] lines past this many.
const AUDIT_MAX: usize = 50_000;
const AUDIT_KEEP: usize = 40_000;

struct State {
    stored: Stored,
    /// Why the module isn't running although enabled.
    error: Option<String>,
    /// Why the running module serves no API (its port taken by another program).
    api_error: Option<String>,
    /// Codes issued through this API, by id: shown until used, expired or burned.
    held_codes: BTreeMap<u64, String>,
    /// Join codes made for Launchers' display boxes (`launchers`), by machine ID: the one
    /// still waiting is taken back when the machine asks again.
    launcher_codes: HashMap<String, u64>,
    /// Devices asked to restart: since when, and whether they were seen offline since.
    restarting: HashMap<String, (Instant, bool)>,
    /// The approval window as last told (`siteChanged` `status` when it opens or closes).
    pairing_open: bool,
    recent: VecDeque<SiteEvent>,
    /// Paired clients unlocked with the PIN → their last admin action (or the unlock).
    unlocks: HashMap<String, Instant>,
    /// Wrong PINs per paired client.
    pin_tries: HashMap<String, PinTries>,
    /// The membership's state as last told (`siteChanged` `membership`).
    membership_seen: MembershipState,
    /// When the unused pairing keys were last looked for.
    key_sweep: Option<Instant>,
    /// When starting the Site Manager last failed (tried again after
    /// [`SiteConfig::start_retry`]).
    start_failed: Option<Instant>,
}

/// A client's wrong PINs.
#[derive(Default)]
struct PinTries {
    /// In a row, since the last lockout.
    wrong: u32,
    /// Lockouts so far (each doubles the next).
    lockouts: u32,
    /// Locked out until then.
    until: Option<Instant>,
}

/// The site module (see the module docs).
pub struct Site {
    me: Weak<Site>,
    cfg: SiteConfig,
    file: PathBuf,
    idn_dir: PathBuf,
    audit_file: PathBuf,
    events: broadcast::Sender<Arc<ServerMessage>>,
    handle: RwLock<Option<Arc<host::Handle>>>,
    state: Mutex<State>,
    /// Serializes start and stop.
    lifecycle: Mutex<()>,
    /// The event thread of the running Site Manager: its stop flag and itself.
    pump: Mutex<Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>>,
    audit_lock: Mutex<()>,
    sign_ins: Mutex<crate::auth::PerIp>,
    /// Serializes PIN checks, so PINs sent at once are counted before the next is tried.
    pin_check: Mutex<()>,
    /// This server's name (its enrollment's name in another site: `<name> (Vesper)`).
    name: String,
    /// `<data>/machine/site-member/`: this server's identity as a machine of another site.
    member_dir: PathBuf,
    member: RwLock<Option<Arc<Member>>>,
    /// Serializes joins and leaves.
    membership_lock: Mutex<()>,
    /// The server is stopping: the tick does nothing more.
    closing: AtomicBool,
    /// Clients' pairings waiting for approval ([`pairing`]).
    pairings: Mutex<pairing::Pairings>,
    /// At a relay: the pairing code the site gave it ([`pairing::RelayCode`]).
    relay_code: Mutex<pairing::RelayCode>,
    /// Serializes asking the site for it.
    relay_fetch: Mutex<()>,
    /// Serializes approving relayed pairings typed with the right code.
    coded_approvals: Mutex<()>,
    /// The fingerprints of this box's own services in its site (the Codex,
    /// the Codex's `codex::member`): not "other machines" of the site.
    own_services: Mutex<BTreeSet<String>>,
    /// Serializes marking pairing relays ([`relays`]); holds the Vesper boxes approved here
    /// and not enrolled yet (by fingerprint).
    relay_marking: Mutex<BTreeSet<String>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Writes a file atomically and durably (flushed before and after the rename): a power cut
/// leaves the old file or the new one.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    vesper_core::fs::write_atomic(path, bytes)
}

impl Site {
    /// Loads this server's part of the site from `<data>/machine/` (nothing is started).
    /// `name`: this server's name.
    pub fn load(
        data_dir: &Path,
        name: &str,
        cfg: SiteConfig,
        events: broadcast::Sender<Arc<ServerMessage>>,
    ) -> anyhow::Result<Arc<Site>> {
        let machine = data_dir.join("machine");
        let file = machine.join("site.json");
        let mut stored = match std::fs::read(&file) {
            Ok(b) => match serde_json::from_slice::<Stored>(&b) {
                Ok(s) => s,
                // Damaged (a power cut, a disk fault): the server still starts. The site
                // itself is the Site Manager's (its own folder); what is lost here is this
                // server's part: sign-ins, the PIN, a membership of another site.
                Err(e) => {
                    let s = Stored::after_damage(&file, &e);
                    // Saved now: with no file at the next start, it would start open.
                    if let Err(e) = serde_json::to_vec_pretty(&s)
                        .map_err(std::io::Error::other)
                        .and_then(|b| write_atomic(&file, &b))
                    {
                        tracing::error!("writing {}: {e}", file.display());
                    }
                    s
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Stored::default(),
            Err(e) => return Err(anyhow::anyhow!("reading {}: {e}", file.display())),
        };
        if !stored.enabled && stored.member.is_none() {
            // Every server is in a site: one not in another's runs its own.
            tracing::warn!(
                "this server's Site Manager was off without another site to be in: it runs its own site again"
            );
            stored.enabled = true;
        }
        let audit_file = machine.join("site-audit.jsonl");
        trim_audit(&audit_file);
        Ok(Arc::new_cyclic(|me| Site {
            me: me.clone(),
            cfg,
            file,
            idn_dir: machine.join("site-manager"),
            audit_file,
            events,
            handle: RwLock::new(None),
            state: Mutex::new(State {
                stored,
                error: None,
                api_error: None,
                held_codes: BTreeMap::new(),
                launcher_codes: HashMap::new(),
                restarting: HashMap::new(),
                pairing_open: false,
                recent: VecDeque::new(),
                unlocks: HashMap::new(),
                pin_tries: HashMap::new(),
                membership_seen: MembershipState::None,
                key_sweep: None,
                start_failed: None,
            }),
            lifecycle: Mutex::new(()),
            pump: Mutex::new(None),
            audit_lock: Mutex::new(()),
            sign_ins: Mutex::new(crate::auth::PerIp::default()),
            pin_check: Mutex::new(()),
            name: name.to_string(),
            member_dir: machine.join("site-member"),
            member: RwLock::new(None),
            membership_lock: Mutex::new(()),
            closing: AtomicBool::new(false),
            pairings: Mutex::new(pairing::Pairings::default()),
            relay_code: Mutex::new(pairing::RelayCode::default()),
            relay_fetch: Mutex::new(()),
            coded_approvals: Mutex::new(()),
            own_services: Mutex::new(BTreeSet::new()),
            relay_marking: Mutex::new(BTreeSet::new()),
        }))
    }

    /// The module's configuration.
    pub fn config(&self) -> &SiteConfig {
        &self.cfg
    }

    /// The Site Manager's folder.
    pub fn idn_dir(&self) -> &Path {
        &self.idn_dir
    }

    fn save(&self, st: &State) {
        let res = serde_json::to_vec_pretty(&st.stored)
            .map_err(std::io::Error::other)
            .and_then(|b| write_atomic(&self.file, &b));
        if let Err(e) = res {
            tracing::error!("writing {}: {e}", self.file.display());
        }
    }

    /// The running Site Manager.
    pub fn handle(&self) -> Option<Arc<host::Handle>> {
        self.handle
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn running(&self) -> Result<Arc<host::Handle>, ApiError> {
        self.handle().ok_or_else(not_running)
    }

    fn send(&self, m: ServerMessage) {
        let _ = self.events.send(Arc::new(m));
    }

    /// Tells clients that parts of the site's state changed.
    pub fn changed(&self, parts: impl IntoIterator<Item = SitePart>) {
        let parts: Vec<SitePart> = parts
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if !parts.is_empty() {
            self.send(ServerMessage::SiteChanged { parts });
        }
    }
}

/// The event thread of a running Site Manager: waits for its events and hands them to the
/// site until told to stop (it notices within `wait`) or the site is gone.
fn event_thread(
    site: Weak<Site>,
    h: Arc<host::Handle>,
    stop: Arc<AtomicBool>,
    mut since: u64,
    wait: Duration,
) {
    while !stop.load(Ordering::Relaxed) {
        let batch = h.events_wait(since, wait);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Some(site) = site.upgrade() else { break };
        since = batch.next;
        if batch.missed || !batch.events.is_empty() {
            site.take_events(batch);
        }
    }
}

/// The site has an admin user.
fn has_admin_user(users: &Users) -> bool {
    users.list().iter().any(|(_, r)| *r == Role::Admin)
}

/// `429` for a client locked out of PIN tries for `left`.
fn locked_out(left: Duration) -> ApiError {
    api_err(
        ErrorCode::RateLimited,
        format!(
            "too many wrong PINs; try again in {} s",
            left.as_secs_f64().ceil().max(1.0) as u64
        ),
    )
}

/// A user with its revision.
fn user_entry(name: &str, role: Role) -> SiteUser {
    let role = site_role(role);
    SiteUser {
        name: name.to_string(),
        role,
        revision: vesper_core::change::revision(&(name, role)),
    }
}

fn audit_entry(v: &Value, source: SiteAuditSource) -> (u64, SiteAuditEntry) {
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let at = v.get("at").and_then(Value::as_u64).unwrap_or(0);
    (
        at,
        SiteAuditEntry {
            at: rfc3339(at),
            user: s("user"),
            peer: s("peer"),
            client: s("client"),
            action: s("action"),
            ok: v.get("ok").and_then(Value::as_bool).unwrap_or(true),
            error: v.get("error").and_then(Value::as_str).map(str::to_owned),
            detail: v.get("detail").cloned().unwrap_or(Value::Null),
            source,
        },
    )
}

/// Keeps the audit file to the newest [`AUDIT_KEEP`] lines once it passes [`AUDIT_MAX`].
fn trim_audit(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= AUDIT_MAX {
        return;
    }
    let mut keep = lines[lines.len() - AUDIT_KEEP..].join("\n");
    keep.push('\n');
    let tmp = path.with_extension("jsonl.tmp");
    if std::fs::write(&tmp, keep)
        .and_then(|_| std::fs::rename(&tmp, path))
        .is_err()
    {
        tracing::warn!("trimming {} failed", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_files_dont_stop_the_server() {
        let data =
            std::env::temp_dir().join(format!("vesper-site-damaged-{}", uuid::Uuid::new_v4()));
        let machine = data.join("machine");
        std::fs::create_dir_all(&machine).unwrap();
        // As a power cut right after a save leaves them: zeros.
        std::fs::write(machine.join("site.json"), [0u8; 64]).unwrap();
        std::fs::write(machine.join("clients.json"), [0u8; 16]).unwrap();
        let (tx, _) = broadcast::channel(4);
        let site = Site::load(&data, "Booth", SiteConfig::off(), tx).expect("starts");
        let s = site.settings();
        assert_eq!(s.access, SiteAccess::Users, "never more open than it was");
        assert!(s.enabled);
        // A paired client only reads, though the site has no admin user (D05-3), and a restart
        // keeps it so.
        let client = Access::Paired(auth::Client {
            id: "k1".into(),
            name: "Tablet".into(),
            credential: auth::Credential::Key,
        });
        let ip: IpAddr = [192, 0, 2, 7].into();
        assert_eq!(site.actor(&client, ip).role, Some(Role::Viewer));
        assert_eq!(site.actor(&Access::Local, ip).role, Some(Role::Admin));
        drop(site);
        let (tx, _) = broadcast::channel(4);
        let site = Site::load(&data, "Booth", SiteConfig::off(), tx).expect("starts again");
        assert_eq!(site.settings().access, SiteAccess::Users);
        assert_eq!(site.actor(&client, ip).role, Some(Role::Viewer));
        let auth = auth::Auth::load(&data, true).expect("starts");
        drop(auth);
        let names: Vec<String> = std::fs::read_dir(&machine)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        for f in ["site.json", "clients.json"] {
            assert!(
                names
                    .iter()
                    .any(|n| n.starts_with(&format!("{f}.damaged-"))),
                "{f} kept aside: {names:?}"
            );
        }
        std::fs::remove_dir_all(&data).ok();
    }

    #[test]
    fn scopes_by_role() {
        assert_eq!(scope_role("read"), Some(Role::Viewer));
        assert_eq!(scope_role("control"), Some(Role::Operator));
        assert_eq!(scope_role("bus:main"), Some(Role::Operator));
        assert_eq!(scope_role("iem:3"), Some(Role::Operator));
        for bad in ["", "full", "bus:", "iem:", "bus:a b", "READ"] {
            assert_eq!(scope_role(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn idn_errors_map_to_statuses() {
        let c = |s: &str| from_idn(s.into()).code;
        assert_eq!(c("an operator may not do this"), ErrorCode::Forbidden);
        assert_eq!(c("no pending request 7"), ErrorCode::NotFound);
        assert_eq!(c("no join code 6"), ErrorCode::NotFound);
        assert_eq!(c("no API key \"ab12\""), ErrorCode::NotFound);
        assert_eq!(
            c("that machine isn't online, or doesn't run an IDN Node"),
            ErrorCode::Conflict
        );
        assert_eq!(c("ttl_s: 1 to 43200 (12 hours)"), ErrorCode::BadRequest);
    }
}
