//! Updates (G10, `docs/LAUNCHER-PLAN.md`): the site's update channel on the Vesper server
//! (`/api/v1/updates/…`) and what each machine's Vesper Launcher reports to it.
//!
//! - **Packages** ([`UpdatePackage`]): signed product packages an admin uploaded, which the
//!   site's Launchers download.
//! - **Launchers** ([`LauncherReport`], [`LauncherEntry`]): each machine's Launcher reports its
//!   products, versions and health, and gets back the [`LauncherAction`]s queued for it.
//!
//! The Launcher's own local API (`127.0.0.1:5556/launcher/v1/…`) uses the same types.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A signed package in the site's update channel (`GET /api/v1/updates/packages`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePackage {
    /// Its ID (the first 16 hex digits of its SHA-256).
    pub id: String,
    /// The product (`vesper`, `oriel`, …), from the signed trusted comment.
    pub product: String,
    /// Its version, from the signed trusted comment.
    pub version: String,
    /// The platform it installs on (`linux-x86_64`, `windows-x86_64`), from the signed trusted
    /// comment.
    pub platform: String,
    /// The uploaded file's name.
    pub file_name: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256 (hex).
    pub sha256: String,
    /// The signing key's ID (minisign's 16 hex digits).
    pub key_id: String,
    /// The signature's trusted comment.
    pub trusted_comment: String,
    /// When it was uploaded (RFC 3339 UTC).
    pub uploaded_at: String,
    /// Where it came from: `github` (the server's GitHub release checker, [`GithubReleases`]);
    /// absent: uploaded by an admin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub source: Option<String>,
}

/// What a product is doing on its machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ProductState {
    /// Up and answering its health check.
    Running,
    /// Not running (stopped by an admin, or not started).
    Stopped,
    /// Started, not answering yet.
    Starting,
    /// An update is being installed.
    Updating,
    /// The update failed its health check; the previous version is going back in.
    RollingBack,
    /// Not answering and not coming back by itself.
    Failed,
}

/// How an update (or a rollback) ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum UpdateResult {
    /// The new version is installed and healthy.
    Installed,
    /// The new version wasn't healthy: the previous one is back.
    RolledBack,
    /// It failed before anything changed (bad signature, wrong platform, download), or the
    /// rollback failed too.
    Failed,
}

/// The last update of a product on a machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOutcome {
    /// The version before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<String>,
    /// The version asked for.
    pub to: String,
    /// How it ended.
    pub result: UpdateResult,
    /// What happened, in words (empty when it simply worked).
    #[serde(default)]
    pub message: String,
    /// When it ended (RFC 3339 UTC).
    pub at: String,
}

/// A product a Launcher manages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LauncherProduct {
    /// Its ID (`vesper`, `oriel`, …): the `product=` of its packages.
    pub product: String,
    /// Its name for people.
    pub name: String,
    /// The installed version (`None` = not installed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    /// The kept previous version, what "Roll back" goes back to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub previous: Option<String>,
    /// What it is doing.
    pub state: ProductState,
    /// How often it was restarted after a crash since the Launcher started.
    #[serde(default)]
    pub restarts: u32,
    /// Its last update or rollback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_update: Option<UpdateOutcome>,
}

/// A machine's health numbers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MachineHealth {
    /// Logical CPUs.
    pub cpus: u32,
    /// CPU load over the last minute, in percent of all CPUs (`None` = not known here).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cpu_percent: Option<f32>,
    /// Memory in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub memory_total: Option<u64>,
    /// Memory available in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub memory_available: Option<u64>,
    /// The disk holding the Launcher's packages, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub disk_total: Option<u64>,
    /// Its free bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub disk_free: Option<u64>,
    /// The machine's GPUs (driver or model names).
    #[serde(default)]
    pub gpus: Vec<String>,
}

/// What a Launcher reports (`PUT /api/v1/updates/launchers/{machineId}`; its own
/// `GET /launcher/v1/status`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LauncherReport {
    /// The machine's stable ID (the Launcher makes one on first start).
    pub machine_id: String,
    /// The machine's name (its host name).
    pub machine_name: String,
    /// The Launcher's version.
    pub launcher_version: String,
    /// The platform its packages must be for (`linux-x86_64`, `windows-x86_64`).
    pub platform: String,
    /// The machine's roles (`presentation`, `display`, `siteManager`, …).
    #[serde(default)]
    pub roles: Vec<String>,
    /// The products it manages.
    pub products: Vec<LauncherProduct>,
    /// "Add app" installs that failed (the product isn't managed here then), newest first;
    /// one per product, until it is added again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<FailedInstall>>", optional)]
    pub failed_installs: Vec<FailedInstall>,
    /// Health numbers.
    pub health: MachineHealth,
}

/// An "Add app" install that failed on a machine, with the Launcher's reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FailedInstall {
    /// The product.
    pub product: String,
    /// Its name for people.
    pub name: String,
    /// How it ended (`failed` or `rolledBack`, the version asked for, the reason, when).
    pub outcome: UpdateOutcome,
}

/// An action for a Launcher, queued in Oriel (`POST /api/v1/updates/launchers/{id}/actions`) or
/// sent to its local API (`POST /launcher/v1/actions`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LauncherAction {
    /// Install `version` of `product` from the site's update channel.
    Update {
        /// The product.
        product: String,
        /// The version.
        version: String,
    },
    /// Go back to the kept previous version of `product`.
    Rollback {
        /// The product.
        product: String,
    },
    /// "Add app": install `product`, which the machine doesn't have yet, at `version` (from the
    /// site's update channel, else from GitHub when the Launcher checks GitHub itself).
    Install {
        /// The product.
        product: String,
        /// The version.
        version: String,
    },
}

/// The answer to a Launcher's report: the actions queued for it (each is given out once).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LauncherActions {
    /// What to do, in order.
    pub actions: Vec<LauncherAction>,
}

/// An IDN join code minted for a Launcher's machine (`POST
/// /api/v1/updates/launchers/{machineId}/idn-join`): a display box's IDN Node joins the site
/// with it (`idn-node join CODE`), without a second code for anyone to type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LauncherJoinCode {
    /// The code (six digits; single use).
    pub code: String,
    /// Seconds until it expires (10 minutes when made).
    pub expires_in_seconds: u64,
}

/// A newer version in the channel for one of a machine's products.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    /// The product.
    pub product: String,
    /// The newest version for the machine's platform.
    pub version: String,
    /// Its package.
    pub package: String,
}

/// A machine's Launcher as the site's server knows it (`GET /api/v1/updates/launchers`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LauncherEntry {
    /// Its last report.
    pub report: LauncherReport,
    /// Seconds since that report.
    pub last_seen_seconds: u64,
    /// It reported within the last 90 seconds.
    pub online: bool,
    /// Actions queued and not yet picked up.
    #[serde(default)]
    pub pending: Vec<LauncherAction>,
    /// Newer versions in the channel for its products.
    #[serde(default)]
    pub updates: Vec<AvailableUpdate>,
}

/// A release channel on GitHub: `stable` takes normal releases only, `beta` pre-releases too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ReleaseChannel {
    /// Normal releases.
    #[default]
    Stable,
    /// Pre-releases as well.
    Beta,
}

/// Checking Vesper's releases on GitHub (the server's `GET`/`PUT
/// /api/v1/updates/github`; the Launcher's own when it has no church server).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GithubSettings {
    /// Check at all (the off switch).
    pub enabled: bool,
    /// The repository (`owner/name`).
    pub repository: String,
    /// Which releases.
    pub channel: ReleaseChannel,
    /// Hours between checks (1–168).
    pub interval_hours: u32,
}

impl Default for GithubSettings {
    fn default() -> Self {
        GithubSettings {
            enabled: true,
            repository: "ADC-Industries/Vesper".into(),
            channel: ReleaseChannel::Stable,
            interval_hours: 6,
        }
    }
}

/// What checking GitHub found last (`GET /api/v1/updates/github`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct GithubReleases {
    /// The settings.
    pub settings: GithubSettings,
    /// When it last checked (RFC 3339 UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_check: Option<String>,
    /// When the last check worked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_success: Option<String>,
    /// What went wrong with the last check (offline, rate-limited, a refused package).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// When it checks next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub next_check: Option<String>,
    /// The packages the last checks brought in (`<product> <version> <platform>`, newest first,
    /// at most 20).
    #[serde(default)]
    pub added: Vec<String>,
    /// A check is running now.
    #[serde(default)]
    pub checking: bool,
}

/// A product that can be added to a machine ("Add app"; `GET /api/v1/updates/catalogue`, the
/// Launcher's `GET /launcher/v1/catalogue`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CatalogueProduct {
    /// Its ID (`vesper`, `vesper-oriel`, …).
    pub product: String,
    /// Its name for people.
    pub name: String,
    /// The newest stable version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stable: Option<String>,
    /// The newest beta, when newer than the newest stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub beta: Option<String>,
    /// The platforms it has packages for.
    pub platforms: Vec<String>,
    /// Where it comes from: `channel` (the site's update channel has it), `github`, or both.
    pub sources: Vec<String>,
}

/// The products that can be added.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Catalogue {
    /// By name.
    pub products: Vec<CatalogueProduct>,
}
