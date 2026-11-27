//! The site (`/api/v1/site/...`): the IDN Site Manager that runs as a module of one Vesper
//! server per site (the *designated* server; on by default, so a standalone install is a
//! one-machine site of its own).
//!
//! - **Joining.** One join mode per site ([`JoinMode`]). Machines join by typing a 6-digit
//!   join code issued here (single use, 10 minutes, burned after 5 wrong tries), or by asking
//!   and being approved by an operator who compares the 6-digit code both screens show (it is
//!   derived from both machines' keys).
//! - **Devices** with their health ([`DeviceHealth`]); identify, restart, logs; Remove (revoke).
//! - **Access** ([`SiteAccess`]): who may do the site's admin actions. `open` (the default):
//!   every paired client; `pin`: a paired client unlocked with the site's admin PIN; `users`:
//!   site users with roles, signed in per paired client.
//! - **Users and roles** (`admin`, `operator`, `viewer`) shared with the site's other tools, a
//!   sign-in per paired client ([`SiteSession`]), and site-signed **tokens** for the IDN engines
//!   ([`IssueToken`]).
//! - **Audit** of every change: who, from where ([`SiteAuditEntry`]).
//! - **Events** ([`SiteEvent`]) on the WebSocket (`siteEvent`), and `siteChanged` when a part
//!   of the site's state changed (refetch it).
//!
//! Join modes, roles and event names are IDN's identifiers and keep IDN's spelling
//! (`code_or_approval`, `join_code_wrong`): the same words the site's other tools use.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How machines join the site (the site option `join.mode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum JoinMode {
    /// A machine asks; an operator compares the code both screens show and approves.
    Approval,
    /// A machine that types a join code joins at once; one without a code waits for approval.
    #[default]
    CodeOrApproval,
    /// A machine must type a join code, then still waits for approval.
    CodeAndApproval,
}

impl JoinMode {
    /// IDN's value (`approval`, `code_or_approval`, `code_and_approval`).
    pub fn as_str(self) -> &'static str {
        match self {
            JoinMode::Approval => "approval",
            JoinMode::CodeOrApproval => "code_or_approval",
            JoinMode::CodeAndApproval => "code_and_approval",
        }
    }
}

/// Who may do the site's admin actions (joins, join codes, the join mode, devices, users,
/// tokens, the settings); the site setting `access`. Reading the site's state needs none of
/// it. The server machine itself is always the site's administrator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SiteAccess {
    /// Every paired client acts as the site's administrator; no PIN, no sign-in. Any paired
    /// client can approve new devices into the site: pairing is the only gate.
    #[default]
    Open,
    /// One admin PIN (4–8 digits): a paired client acts as the site's administrator once
    /// unlocked with it (`POST /api/v1/site/unlock`), until it locks again (on demand, or after
    /// 30 minutes without an admin action). Locked, it reads (as a viewer).
    Pin,
    /// Site users with roles ([`SiteRole`]): a paired client acts as the user it signed in as
    /// (`POST /api/v1/site/session`). Needs an admin user.
    Users,
}

impl SiteAccess {
    /// The value as on the wire (`open`, `pin`, `users`).
    pub fn as_str(self) -> &'static str {
        match self {
            SiteAccess::Open => "open",
            SiteAccess::Pin => "pin",
            SiteAccess::Users => "users",
        }
    }
}

/// A user's role on the site. Ordered: each may do what the ones before it may.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SiteRole {
    /// Watches: status, devices, the audit; `read` tokens.
    Viewer,
    /// Watches and operates: identify, logs; `control`, `bus:<bus>` and `iem:<mix>` tokens.
    Operator,
    /// Everything: joins, devices, users, settings.
    Admin,
}

/// `GET /api/v1/site`: the site as this server runs it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteStatus {
    /// This server is the site's designated Site Manager (stored setting).
    pub enabled: bool,
    /// The Site Manager module is running.
    pub running: bool,
    /// Why it isn't running although enabled (a port in use, a folder not writable, turned off
    /// on the command line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// Why the running Site Manager serves no API (its admin panel), e.g. its port taken by
    /// another program; no other port is tried. Its discovery server and the site's authority
    /// run, but what needs the API (pairing a client, API keys, changes run as a user) answers
    /// errors. Only while running; gone at the next start with the API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub api_error: Option<String>,
    /// The site settings (`site.json`) were damaged at start: this server started with access
    /// `users`, and a paired client that hasn't signed in as a site user only reads (even with
    /// no admin user) until an admin sets the access again (`PUT /api/v1/site/settings`, from
    /// the server machine or signed in as an admin), which clears it.
    #[ts(optional, as = "Option<bool>")]
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub settings_damaged: bool,
    /// How machines join.
    pub join_mode: JoinMode,
    /// The approval window is open: machines without a code may ask now.
    pub pairing_open: bool,
    /// The site's id (16 hex digits of its CA fingerprint), while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub site_id: Option<String>,
    /// The site CA's SHA-256 fingerprint (64 hex digits), while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ca_fingerprint: Option<String>,
    /// The discovery server's port (machines point at `<this server>:<port>`), while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub discovery_port: Option<u16>,
    /// The Site Manager's own HTTPS API port, while running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub api_port: Option<u16>,
    /// SHA-256 of that API's certificate (to compare with what a browser shows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub api_certificate_sha256: Option<String>,
    /// Who this client acts as on the site.
    pub you: SiteSession,
    /// The settings' revision (`If-Match` for `PUT /api/v1/site/settings`).
    pub revision: String,
    /// This server's membership of another Vesper server's site (`state: none` while it runs
    /// its own).
    pub membership: SiteMembership,
}

/// `POST /api/v1/site/join` (admins): join another Vesper server's site as one of its
/// machines. This server's own Site Manager stops once it is in (one Site Manager per
/// site), and starts again when it leaves (`POST /api/v1/site/leave`). Refused (`409`)
/// while this server's own site still has other machines: remove or move them first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct JoinSite {
    /// The site server's discovery address: `host` or `host:port` (port 5959 when left out).
    /// Leave it out to type a code into a join already under way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub server: Option<String>,
    /// A join code from that site's server (`"482 913"` or `"482913"`): joins at once where the
    /// site's join mode allows it. Without one, the machine asks and waits for approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub code: Option<String>,
}

/// Where this server stands with another server's site (IDN's trust states).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum MembershipState {
    /// Not joining another site: this server runs its own.
    #[default]
    None,
    /// Contacting the site's server.
    Starting,
    /// The address answers, but isn't a site's server.
    NoSite,
    /// The site's approval window is closed: open it there. Asks again by itself.
    WaitingForWindow,
    /// Waiting for an operator to approve; `code` is the comparison code both sides show.
    WaitingForApproval,
    /// The site needs a join code typed here (`POST /api/v1/site/join {code}`).
    CodeNeeded,
    /// The code was right; waiting for approval.
    CodeAccepted,
    /// The code was wrong (`triesLeft`).
    WrongCode,
    /// The code expired, was used or burned: ask the site for a new one.
    CodeExpired,
    /// In the site.
    Enrolled,
    /// The site denied this server.
    Denied,
    /// The site removed this server.
    Removed,
    /// Something went wrong (`error`); retried.
    Error,
}

/// `GET /api/v1/site/membership` (also `SiteStatus.membership`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteMembership {
    /// Where it stands.
    pub state: MembershipState,
    /// The site server's discovery address, while joining or in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub server: Option<String>,
    /// The site's id, once in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub site_id: Option<String>,
    /// This server's fingerprint as the site lists it, while joining or in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fingerprint: Option<String>,
    /// The comparison code (`waitingForApproval`), shown `"482 913"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub code: Option<String>,
    /// The site's server is reachable now (`enrolled`; not needed to keep working).
    #[serde(default)]
    pub connected: bool,
    /// Wrong-code tries left (`wrongCode`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tries_left: Option<u32>,
    /// What went wrong (`error`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

/// `GET/PUT /api/v1/site/settings` (admins; `PUT` takes `If-Match`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteSettings {
    /// This server runs its own site's Site Manager. Every server is in a site: it is on
    /// unless the server has joined another server's site (`POST /api/v1/site/join`), and it
    /// can't be turned off otherwise, nor on while the server is in another site (`409`).
    pub enabled: bool,
    /// How machines join (applies while running).
    pub join_mode: JoinMode,
    /// Who may do admin actions. `pin` needs a PIN (set with `pin`, or one already set);
    /// `users` needs an admin user. Leaving `pin` forgets the PIN; leaving `users` ends the
    /// clients' sign-ins.
    pub access: SiteAccess,
    /// A new admin PIN, 4–8 digits (requests only, with `access: "pin"`; stored hashed, never
    /// sent back).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pin: Option<String>,
    /// An admin PIN is set (responses only; ignored in requests).
    #[serde(default)]
    pub pin_set: bool,
    /// The settings' revision (responses only; ignored in requests).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub revision: Option<String>,
}

/// Who a client acts as on the site (`GET/DELETE /api/v1/site/session`; the answer to a
/// sign-in or an unlock).
///
/// The server machine itself acts as `admin` whatever the [`SiteAccess`]. A paired client:
/// - `open`: acts as `admin`.
/// - `pin`: acts as `admin` while unlocked (`unlocked`, `unlockSecondsLeft`); locked, it reads
///   as a viewer (no `user`).
/// - `users`: acts as the site user it signed in as. Should the site have no admin user (one
///   removed on another tool), a client that hasn't signed in acts as `admin`, so the site
///   can't be locked out; otherwise a client that hasn't signed in can only read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteSession {
    /// The site's access level.
    pub access: SiteAccess,
    /// The user (`admin`: the site's built-in administrator; none: not acting as a user).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub user: Option<String>,
    /// Their role (none: the status and lists only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub role: Option<SiteRole>,
    /// The client signed in as `user` (`users`).
    pub signed_in: bool,
    /// The client is unlocked with the admin PIN (`pin`).
    #[serde(default)]
    pub unlocked: bool,
    /// Seconds until an unlocked client locks again unless it does an admin action (each one
    /// starts the time again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unlock_seconds_left: Option<u64>,
}

/// `POST /api/v1/site/session`: sign this paired client in as a site user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteSignIn {
    /// User name.
    pub user: String,
    /// Password.
    pub password: String,
}

/// `POST /api/v1/site/unlock`: unlock this paired client with the site's admin PIN
/// (`access: "pin"`). `401` for a wrong PIN; after 5 wrong ones in a row the client is locked
/// out for a while (`429`, longer each time).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteUnlock {
    /// The PIN.
    pub pin: String,
}

/// A machine asking to join (`GET /api/v1/site/join-requests`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct JoinRequestInfo {
    /// The request's id (for approve / deny).
    pub id: u64,
    /// The name the machine gives (not proof of anything: compare the code).
    pub name: String,
    /// `app`, `output` or `input`.
    pub kind: String,
    /// The machine's key fingerprint (64 hex digits).
    pub fingerprint: String,
    /// The comparison code as shown, `"482 913"`: the machine shows the same one. Absent for a
    /// machine that typed a valid join code (`byCode`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub code: Option<String>,
    /// Where it asked from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub address: Option<String>,
    /// It typed a valid join code (`code_and_approval`): approve by name and address.
    pub by_code: bool,
    /// When it asked (RFC 3339).
    pub asked_at: String,
}

/// `POST /api/v1/site/join-requests/{id}/approve` (optional body).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApproveJoin {
    /// The machine's name in the site, instead of the one it asked with: its certificate is
    /// issued under it (the device list shows it), and a machine with an IDN Node is named so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
}

/// `POST /api/v1/site/pairing` (optional body): open the approval window.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OpenPairing {
    /// How long, 10 to 3600 seconds (default 600).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub seconds: Option<u64>,
}

/// A join code (`GET/POST /api/v1/site/join-code`, `GET /api/v1/site/join-codes`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct JoinCodeInfo {
    /// The code's id.
    pub id: u64,
    /// The code as shown, `"482 913"`: while this server holds it (codes issued through this
    /// API, until used, expired or burned). Codes issued elsewhere are listed without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub code: Option<String>,
    /// When it expires (RFC 3339).
    pub expires_at: String,
    /// Seconds until then.
    pub expires_in_seconds: u64,
    /// Wrong tries left before it is burned.
    pub tries_left: u32,
    /// Who issued it.
    pub issuer: String,
}

/// A machine's health in the device list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum DeviceHealth {
    /// Connected, nothing wrong.
    Online,
    /// Connected, but something is wrong (`reason`, `problems`).
    NeedsAttention,
    /// Not connected (`lastSeen`).
    Offline,
    /// Services restarting (`restarting`), or asked to restart one and not back yet.
    Restarting,
    /// An update is running on it (`updating`).
    Updating,
    /// Removed from the site (revoked): its joins are refused until allowed again.
    Removed,
}

/// Something that needs attention on a machine (as its IDN Node and the Site Manager see it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeviceProblem {
    /// IDN's name for it (`clock_unlocked`, `version_mismatch`, `service_failed`, …).
    pub problem: String,
    /// How much it matters.
    pub level: SiteEventLevel,
    /// Words for people.
    pub text: String,
    /// Everything else it carries.
    #[ts(type = "unknown")]
    pub fields: serde_json::Value,
}

/// One of a machine's services, as its IDN Node last reported it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeviceService {
    /// Its id: what `RestartDevice.service` takes.
    pub id: String,
    /// Its name for people (`Display`, `Switcher`, …).
    pub name: String,
    /// IDN's word for its state: `running`, `starting`, `restarting`, `failed`, `off`,
    /// `unset` (not set up yet), `unsupported` (not on this machine) or `no_session` (runs in
    /// a log-in session, and none is there); empty when not reported. Show an unknown one as
    /// it is.
    pub state: String,
}

/// What machine a device is (G9), as its IDN Node last reported it: CPU, GPUs, system, start
/// and memory. Absent from a device whose Node doesn't report it (an older Node, a device
/// without a Node); what the machine's system doesn't say is left out. An idle Node reports
/// every 30 s, so `memoryFree` can be up to 30 s old (`reportedAt`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteDeviceMachine {
    /// The CPU's model name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cpu: Option<String>,
    /// Logical CPUs (hardware threads).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cpus: Option<u32>,
    /// Its GPUs' adapter names (empty: none reported).
    #[serde(default)]
    pub gpus: Vec<String>,
    /// The operating system (`Ubuntu`, `Windows 11 Pro`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub os: Option<String>,
    /// Its version (`24.04`, `24H2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub os_version: Option<String>,
    /// The kernel's release (Linux, macOS) or the build (Windows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kernel: Option<String>,
    /// When the machine started (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub booted_at: Option<String>,
    /// Its memory, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub memory_total: Option<u64>,
    /// Its memory available when its Node last reported, in bytes (not on macOS).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub memory_free: Option<u64>,
    /// When its Node reported these (RFC 3339): up to 30 s ago while it is online.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reported_at: Option<String>,
}

/// A machine of the site (`GET /api/v1/site/devices`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteDevice {
    /// Key fingerprint (64 hex digits): the device's id in the API.
    pub fingerprint: String,
    /// Name at enrolment.
    pub name: String,
    /// `app`, `output` or `input`.
    pub kind: String,
    /// May administer the site remotely.
    pub admin: bool,
    /// Health.
    pub health: DeviceHealth,
    /// What needs attention, the first of `problems` (`needsAttention`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
    /// Everything that needs attention on it now (empty: nothing).
    pub problems: Vec<DeviceProblem>,
    /// When it was last connected or last reported (RFC 3339): now while online; absent when
    /// never seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_seen: Option<String>,
    /// The IDN build its Node runs (`0.2.0-Alpha93 …`), as last reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    /// Its services restarting now (their ids), as its Node last reported.
    pub restarting: Vec<String>,
    /// Its services, as its Node last reported them: what `…/restart` takes (empty: no Node
    /// has reported).
    #[serde(default)]
    pub services: Vec<DeviceService>,
    /// An update running on it: `downloading`, `staged`, `installing` or `checking`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub updating: Option<String>,
    /// The product that installs and updates its IDN Node instead of the Site Manager
    /// (`Vesper`: the vesper-idn-node package's marker or the Node's `update.managed_by`
    /// setting, IDN Beta14), as its Node last reported; absent: the Site Manager's updates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub update_managed_by: Option<String>,
    /// It runs an IDN Node (identify, restart and logs work).
    pub node: bool,
    /// When its Node started (RFC 3339), as last reported; absent when no Node has reported.
    /// With `machine.bootedAt`: the Node (re)started, or the machine did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub node_started_at: Option<String>,
    /// What machine it is (G9), as its Node last reported; absent when its Node doesn't say
    /// (an older Node, a device without one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub machine: Option<SiteDeviceMachine>,
    /// Part of Vesper itself: the Codex's own membership of its site, on the site's server
    /// (shown as "Codex (built-in)"). It can't be removed (`409`): it would only join again at
    /// the next start.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub built_in: bool,
    /// It relays Oriel pairings (IDN's pairing relay mark): it shows the site's pairing codes,
    /// so a client typing one there pairs without anyone's approval. Vesper marks its own
    /// boxes itself (when they join, and at the site server's start); an admin can set it
    /// on a Vesper box, or take it off any machine (`POST …/relay`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub relay: bool,
}

/// `POST /api/v1/site/devices/{fingerprint}/relay` (admins): mark the machine a pairing relay,
/// or not. Kept as the admin set it (Vesper doesn't mark it again at its next start).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SetDeviceRelay {
    /// Relays pairings.
    pub on: bool,
}

/// `POST /api/v1/site/devices/{fingerprint}/identify` (optional body).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdentifyDevice {
    /// How long its screens show its name (default 10 s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub seconds: Option<u64>,
}

/// `POST /api/v1/site/devices/{fingerprint}/restart`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RestartDevice {
    /// The machine's service to restart (as its IDN Node names it).
    pub service: String,
}

/// A log snapshot fetched from a machine (`GET /api/v1/site/devices/{fingerprint}/logs`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogSnapshot {
    /// Its name (`YYYYMMDD-HHMMSS`); `GET …/logs/{snapshot}/{file}` reads a file.
    pub snapshot: String,
    /// Its files.
    pub files: Vec<LogFile>,
}

/// One file of a [`LogSnapshot`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogFile {
    /// File name.
    pub name: String,
    /// Size, bytes.
    pub size: u64,
}

/// A site user (`GET /api/v1/site/users`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteUser {
    /// User name.
    pub name: String,
    /// Role.
    pub role: SiteRole,
    /// Revision (`If-Match` for `PUT/DELETE /api/v1/site/users/{name}`).
    pub revision: String,
}

/// `PUT /api/v1/site/users/{name}`: add a user or change one (admins).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PutSiteUser {
    /// Role (a new user without one is a viewer; an existing user keeps theirs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub role: Option<SiteRole>,
    /// A new password, 8 characters or more (needed for a new user).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub password: Option<String>,
}

/// `PUT /api/v1/site/password`: the site password, the built-in administrator's (admins).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SetSitePassword {
    /// The new password (8 characters or more).
    pub password: String,
}

/// `POST /api/v1/site/tokens`: a site-signed token for an IDN engine.
///
/// With access `users` it is the signed-in user's, scopes by role: viewers `read`; operators
/// and admins also `control`, `bus:<bus>` and `iem:<mix>`. With `open` and `pin` (unlocked)
/// any scope, issued to a name of the client's own (`vesper:<client id>`), so one client's
/// tokens can be withdrawn alone (`DELETE /api/v1/site/tokens/{user}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IssueToken {
    /// The engine: `desk`, `switcher`, `display`, `data` or `node`.
    pub engine: String,
    /// The machine (fingerprint); absent: any of the site's machines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub machine: Option<String>,
    /// `read`, `control`, `bus:<bus>` or `iem:<mix>`.
    pub scope: String,
    /// Lifetime, seconds (at most 12 hours, the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub ttl_seconds: Option<u64>,
}

/// A token issued (`POST /api/v1/site/tokens`): shown once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IssuedToken {
    /// The token (`idn1.…`): `Authorization: Bearer <token>` at the engine.
    pub token: String,
    /// Its id.
    pub id: String,
    /// Who it is for: a site user, `vesper:<client id>` (a client's own), or `admin`.
    pub user: String,
    /// The engine.
    pub engine: String,
    /// The machine (`*`: any).
    pub machine: String,
    /// Scope.
    pub scope: String,
    /// Issued at (RFC 3339).
    pub issued_at: String,
    /// Expires at (RFC 3339).
    pub expires_at: String,
}

/// Where an audit entry was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SiteAuditSource {
    /// This server's own: sign-ins, PIN unlocks and locks, the settings, withdrawn tokens, and
    /// requests refused before they reached the Site Manager.
    Vesper,
    /// The Site Manager's audit: the changes and tokens it carried out for this API (with the
    /// client and its address), machines (join codes typed), and changes made on its own
    /// HTTPS API.
    SiteManager,
}

/// One audit entry (`GET /api/v1/site/audit?n=`, newest first).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteAuditEntry {
    /// When (RFC 3339).
    pub at: String,
    /// Who (a site user, `admin`; empty for a machine).
    pub user: String,
    /// From where (an IP address).
    pub peer: String,
    /// The client (a paired client's name, `server machine`), when known.
    pub client: String,
    /// What (`approve`, `revoke`, `join_code`, `user`, `token (host)`, `sign_in`, `unlock`, …).
    pub action: String,
    /// It was done.
    pub ok: bool,
    /// Why not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// Details, secrets left out.
    #[ts(type = "unknown")]
    pub detail: serde_json::Value,
    /// Which audit it comes from.
    pub source: SiteAuditSource,
}

/// The site's typed event names: IDN's (the Site Manager's and the machines' typed failure
/// events) and the ones this server derives from the site's state. A newer server or IDN
/// release may send names not listed here: show their `text`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SiteEventName {
    /// A machine asks to join (`requestId`; refetch the join requests).
    JoinRequested,
    /// A join request left the list (`requestId`): withdrawn (the machine stopped asking, the
    /// window closed) or answered (a `device_joined` or `join_denied` says which).
    JoinCancelled,
    /// A machine joined the site (approved, or by code).
    DeviceJoined,
    /// A machine connected.
    DeviceOnline,
    /// A machine disconnected.
    DeviceOffline,
    /// A join request was denied.
    JoinDenied,
    /// A denied or removed machine asked again and was refused.
    JoinRefused,
    /// A machine approved by code withdrew: its person didn't confirm the code on it.
    JoinWithdrawn,
    /// A machine was removed from the site.
    DeviceRevoked,
    /// A join code was issued.
    JoinCodeIssued,
    /// A join code was used (`codeId`).
    JoinCodeUsed,
    /// A wrong join code was typed (`triesLeft`).
    JoinCodeWrong,
    /// A join code can't be used any more: 5 wrong tries, or a newer code replaced it
    /// (`detail.replaced`).
    JoinCodeBurned,
    /// A join code expired unused.
    JoinCodeExpired,
    /// A join code was taken back.
    JoinCodeCancelled,
    /// (A machine) it is already in a site.
    AlreadyInSite,
    /// (A machine) the Site Manager can't be reached.
    SiteManagerUnreachable,
    /// (A machine) it was removed from its site.
    Revoked,
    /// (A machine) two Site Managers claim the network.
    TwoSites,
    /// A client asks to pair and waits for approval (`detail.pairing`: its id in
    /// `GET /api/v1/site/pairings`; `machine`: the server it asked at).
    KeyRequested,
    /// A pairing waiting for approval ended unanswered: its server disconnected, or 10 minutes
    /// passed (`detail.pairing`).
    KeyRequestCancelled,
    /// A machine left the site by itself (it may join again).
    DeviceLeft,
}

/// How much a site event matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SiteEventLevel {
    /// For information.
    Info,
    /// Something to look at.
    Warning,
    /// Something failed.
    Error,
    /// A problem is over.
    Ok,
}

/// A typed site event (`{"type": "siteEvent", "event": {…}}` on the WebSocket; the newest at
/// `GET /api/v1/site/events`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteEvent {
    /// Its name: a [`SiteEventName`], or a newer one.
    #[ts(
        type = "\"join_requested\" | \"join_cancelled\" | \"device_joined\" | \"device_online\" | \"device_offline\" | \"join_denied\" | \"join_refused\" | \"join_withdrawn\" | \"device_revoked\" | \"join_code_issued\" | \"join_code_used\" | \"join_code_wrong\" | \"join_code_burned\" | \"join_code_expired\" | \"join_code_cancelled\" | \"already_in_site\" | \"site_manager_unreachable\" | \"revoked\" | \"two_sites\" | \"key_requested\" | \"key_request_cancelled\" | \"device_left\" | (string & {})"
    )]
    pub name: String,
    /// Level.
    pub level: SiteEventLevel,
    /// Words for people.
    pub text: String,
    /// When (RFC 3339).
    pub at: String,
    /// The machine it is about (fingerprint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fingerprint: Option<String>,
    /// The machine's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub machine: Option<String>,
    /// Where it came from (an IP address).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub address: Option<String>,
    /// The join request (`join_requested`, `join_cancelled`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub request_id: Option<u64>,
    /// The join code (`join_code_*`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub code_id: Option<u64>,
    /// Wrong tries left (`join_code_wrong`; 0: used up).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tries_left: Option<u32>,
    /// Everything else the event carries.
    #[ts(type = "unknown")]
    pub detail: serde_json::Value,
}

/// A part of the site's state that changed (`siteChanged`): refetch it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum SitePart {
    /// `GET /api/v1/site` and `/site/settings` (running, join mode, approval window).
    Status,
    /// `GET /api/v1/site/join-requests`.
    JoinRequests,
    /// `GET /api/v1/site/join-codes`.
    JoinCodes,
    /// `GET /api/v1/site/devices`.
    Devices,
    /// `GET /api/v1/site/users`.
    Users,
    /// A client's own session (`GET /api/v1/site/session`): an unlock ended.
    Session,
    /// This server's membership of another site (`GET /api/v1/site/membership`).
    Membership,
    /// `GET /api/v1/site/pairings`: clients waiting for approval.
    Pairings,
    /// `GET /api/v1/site/api-keys`.
    ApiKeys,
    /// `GET /api/v1/mosaic`: the display boxes, their screens and the channels.
    Mosaic,
    /// `GET /api/v1/signage`: signage channels, playlists, feeds, settings, the override.
    Signage,
}

/// A client's pairing waiting for a site admin's approval (`GET /api/v1/site/pairings`):
/// asked without a code at the site's server, or at another server of the site (relayed).
/// Approve or deny it: `POST /api/v1/site/pairings/{id}/approve`, `…/deny`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PairingRequest {
    /// Its id here (`p<n>`: at this server; `k<n>`: relayed by another server of the site).
    pub id: String,
    /// The client's name (as its key will be named: `Oriel – Booth laptop`).
    pub client: String,
    /// The server it asked at.
    pub via: String,
    /// That server's fingerprint in the site (relayed requests).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub via_fingerprint: Option<String>,
    /// The client's address (requests at this server).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub address: Option<String>,
    /// Relayed by another server of the site.
    pub relayed: bool,
    /// When it asked (RFC 3339).
    pub asked_at: String,
    /// A Vesper Launcher asking to join (its machine ID): approved, it gets that machine's
    /// Launcher key (update reports and packages only), not a client's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub launcher: Option<String>,
}

/// `POST /api/v1/site/pairings/{id}/deny` body (optional).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DenyPairing {
    /// Said to the client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reason: Option<String>,
}

/// A named site API key, as the Site Manager lists it (`GET /api/v1/site/api-keys`): the
/// paired clients' keys and any other (Stream Deck, Companion). Never the key itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SiteApiKey {
    /// Its id (`DELETE /api/v1/site/api-keys/{id}`).
    pub id: String,
    /// Its name (`Oriel – Booth laptop`).
    pub name: String,
    /// Its audience's engine (`app:vesper`, `desk`, …).
    pub engine: String,
    /// Its audience's machine (`*`: any of the site's).
    pub machine: String,
    /// `control`, `read`, …
    pub scope: String,
    /// Made (RFC 3339).
    pub created_at: String,
    /// By whom: a user, or `user (via machine)` for a relayed pairing.
    pub by: String,
    /// When it ends (RFC 3339).
    pub expires_at: String,
    /// Revoked (listed until it would have ended).
    pub revoked: bool,
    /// When it was revoked (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub revoked_at: Option<String>,
    /// When it was last used (RFC 3339), where told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_used: Option<String>,
    /// The machine it was last used on (its fingerprint), where told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_machine: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_in_the_typescript_type() {
        let cfg = ts_rs::Config::default();
        let decl = <SiteEvent as TS>::decl(&cfg);
        let names = <SiteEventName as TS>::decl(&cfg);
        let listed: Vec<&str> = names.split('"').skip(1).step_by(2).collect();
        assert_eq!(listed.len(), 22, "{names}");
        for n in listed {
            assert!(
                decl.contains(&format!("\"{n}\"")),
                "{n} missing from SiteEvent.name"
            );
        }
    }
}
