//! The Launcher's configuration file (JSON): where it keeps its state, which products it
//! manages and how each is installed, its keys and the site's update channel.
//!
//! ```json
//! {
//!   "stateDir": "/var/lib/vesper-launcher",
//!   "roles": ["presentation"],
//!   "channel": { "url": "http://192.0.2.10:5555", "token": "…" },
//!   "products": [
//!     { "product": "vesper", "name": "Vesper",
//!       "install": { "kind": "deb", "units": ["vesper.service", "vesper-display.service"] },
//!       "healthUrl": "http://127.0.0.1:5555/api/v1/server" }
//!   ]
//! }
//! ```

use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;

/// The local API's default address (loopback only).
pub const DEFAULT_LISTEN: &str = "127.0.0.1:5556";

/// The configuration.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    /// Packages, the manifest, the machine ID and logs.
    #[serde(default = "default_state_dir")]
    pub state_dir: PathBuf,
    /// The local API's address (loopback).
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    /// A folder of trusted public keys (`*.pub`, minisign), writable by administrators only.
    #[serde(default = "default_key_dir")]
    pub key_dir: Option<PathBuf>,
    /// More trusted public keys (minisign text), besides [`crate::sign::RELEASE_KEYS`].
    #[serde(default)]
    pub keys: Vec<String>,
    /// The machine's roles (`presentation`, `display`, `siteManager`, …).
    #[serde(default)]
    pub roles: Vec<String>,
    /// The machine's name (default: its host name).
    #[serde(default)]
    pub machine_name: Option<String>,
    /// The platform packages must be for (default: this machine's).
    #[serde(default)]
    pub platform: Option<String>,
    /// The site's update channel (`None` = local API only).
    #[serde(default)]
    pub channel: Option<ChannelConfig>,
    /// How long a new version has to answer its health check before it is rolled back.
    #[serde(default = "default_health_timeout")]
    pub health_timeout_secs: u64,
    /// How often products are health-checked.
    #[serde(default = "default_check_interval")]
    pub check_interval_secs: u64,
    /// The products. More come from `launcher.d/*.json` next to the configuration file, one
    /// product each, which each product's own package installs (the Vesper server's
    /// `vesper.json`); a product listed here wins over a file of the same ID.
    #[serde(default)]
    pub products: Vec<ProductConfig>,
    /// The system's package manager (Linux, `deb` products): its programs and lock folder.
    /// The defaults are the system's; other values are for tests.
    #[serde(default)]
    pub dpkg: DpkgConfig,
    /// The programs the machine's roles are acted on with (the service manager, the bundled
    /// IDN Node and tools). The defaults are the system's; other values are for tests.
    #[serde(default)]
    pub system: SystemConfig,
    /// Checking Vesper's releases on GitHub when there is no church server, and for "Add app"
    /// ([`crate::github`]).
    #[serde(default)]
    pub github: crate::github::GithubConfig,
    /// The file it was read from ([`Config::load`]): the self-update's helper reads it too.
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

/// The programs [`crate::roles`] and [`crate::diagnostics`] run ([`Config::system`]).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SystemConfig {
    /// `systemctl` (Linux).
    #[serde(default = "default_systemctl")]
    pub systemctl: PathBuf,
    /// `schtasks` (Windows).
    #[serde(default = "default_schtasks")]
    pub schtasks: PathBuf,
    /// The bundled IDN programs' folder (`idn-node`, `idn-session`, `idn`, `idn-display`).
    #[serde(default = "default_idn_dir")]
    pub idn_dir: PathBuf,
    /// The system's IDN Node service unit (Linux; `idn-node install` writes it), which
    /// [`crate::idn_node`]'s guard reads.
    #[serde(default = "default_idn_node_unit")]
    pub idn_node_unit: PathBuf,
}

impl Default for SystemConfig {
    fn default() -> Self {
        SystemConfig {
            systemctl: default_systemctl(),
            schtasks: default_schtasks(),
            idn_dir: default_idn_dir(),
            idn_node_unit: default_idn_node_unit(),
        }
    }
}

fn default_idn_node_unit() -> PathBuf {
    PathBuf::from("/etc/systemd/system/idn-node.service")
}

fn default_systemctl() -> PathBuf {
    PathBuf::from("systemctl")
}

fn default_schtasks() -> PathBuf {
    PathBuf::from("schtasks")
}

/// The bundled IDN programs: `/usr/lib/vesper-idn-node` (the `vesper-idn-node` package) on
/// Linux; the `idn-node` folder next to `vesper-launcher.exe` on Windows (its installer).
pub fn default_idn_dir() -> PathBuf {
    #[cfg(windows)]
    {
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| p.join("idn-node")))
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files\Vesper Launcher\idn-node"))
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/usr/lib/vesper-idn-node")
    }
}

impl SystemConfig {
    /// A bundled IDN program (`idn-node`, `idn`): its path in [`SystemConfig::idn_dir`].
    pub fn idn_program(&self, name: &str) -> PathBuf {
        self.idn_dir
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    }
}

/// The system's package manager, for `deb` products ([`Config::dpkg`]).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DpkgConfig {
    /// `dpkg` (installs a package).
    #[serde(default = "default_dpkg")]
    pub program: PathBuf,
    /// `dpkg-query` (the installed version of a package).
    #[serde(default = "default_dpkg_query")]
    pub query: PathBuf,
    /// `dpkg-deb` (a package's dependencies, before it is installed).
    #[serde(default = "default_dpkg_deb")]
    pub deb: PathBuf,
    /// dpkg's database folder, where its locks are (`lock-frontend`, `lock`).
    #[serde(default = "default_dpkg_admin_dir")]
    pub admin_dir: PathBuf,
    /// How long an install waits for another dpkg or apt run (an install the Launcher
    /// started before a restart, unattended upgrades) to end.
    #[serde(default = "default_dpkg_lock_timeout")]
    pub lock_timeout_secs: u64,
    /// apt's cache of the packages it downloaded: a version installed with apt from a
    /// repository is found there to go back to when the Launcher has no package of it.
    #[serde(default = "default_apt_archives")]
    pub apt_archives: PathBuf,
}

impl Default for DpkgConfig {
    fn default() -> Self {
        DpkgConfig {
            program: default_dpkg(),
            query: default_dpkg_query(),
            deb: default_dpkg_deb(),
            admin_dir: default_dpkg_admin_dir(),
            lock_timeout_secs: default_dpkg_lock_timeout(),
            apt_archives: default_apt_archives(),
        }
    }
}

fn default_dpkg() -> PathBuf {
    PathBuf::from("dpkg")
}

fn default_dpkg_query() -> PathBuf {
    PathBuf::from("dpkg-query")
}

fn default_dpkg_deb() -> PathBuf {
    PathBuf::from("dpkg-deb")
}

fn default_dpkg_admin_dir() -> PathBuf {
    PathBuf::from("/var/lib/dpkg")
}

fn default_dpkg_lock_timeout() -> u64 {
    900
}

fn default_apt_archives() -> PathBuf {
    PathBuf::from("/var/cache/apt/archives")
}

/// The site's update channel: the site's Vesper server.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelConfig {
    /// The server's address (`http://host:5555`).
    pub url: String,
    /// A site API key (not needed on the server's own machine).
    #[serde(default)]
    pub token: Option<String>,
    /// How often the Launcher reports and picks up its actions.
    #[serde(default = "default_report_interval")]
    pub interval_secs: u64,
}

/// A product the Launcher manages.
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductConfig {
    /// Its ID: the `product=` of its packages.
    pub product: String,
    /// Its name for people.
    pub name: String,
    /// How its packages are installed and how it runs.
    pub install: InstallKind,
    /// Its health check: answers `200` (with `{"version": …}` when it reports one). Empty: a
    /// package of programs rather than a service (`deb` only: the bundled IDN Node,
    /// `vesper-idn-node`), up while dpkg has it installed and healthy once dpkg has the new
    /// version; it has nothing to Launch or Stop. (Older Launchers need the field: product
    /// files write `"healthUrl": ""`.)
    #[serde(default)]
    pub health_url: String,
    /// A log file to show (default: the unit's journal, or the supervised process's log).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_file: Option<PathBuf>,
    /// This is the Launcher itself (its own package, `vesper-launcher`): an update goes
    /// through [`crate::selfupdate`], a helper that outlives the Launcher's restart.
    #[serde(default, rename = "self")]
    pub is_self: bool,
}

/// How a product's packages are installed.
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InstallKind {
    /// Linux: the package is a `.deb`, installed with `dpkg -i` (its maintainer scripts restart
    /// the enabled unit; an edited configuration file is kept); systemd restarts it after a
    /// crash. The Debian package's name is the product's ID; the version dpkg has installed is
    /// the product's (adopted when it was installed with apt).
    Deb {
        /// Its systemd units (for logs and restart counts; the first enabled one counts).
        #[serde(default)]
        units: Vec<String>,
    },
    /// Windows: the package is an NSIS installer, run silently (`/S` and `args`).
    Nsis {
        /// More installer arguments (`/DESKTOP`, …).
        #[serde(default)]
        args: Vec<String>,
        /// Its scheduled task (`Vesper`), for Launch and Stop.
        #[serde(default)]
        task: Option<String>,
    },
    /// The package is the product's executable, kept in a folder per version and run by the
    /// Launcher itself (restarted after a crash with back-off): portable installs and tests.
    Dir {
        /// Its arguments.
        #[serde(default)]
        args: Vec<String>,
    },
}

fn default_listen() -> SocketAddr {
    DEFAULT_LISTEN.parse().expect("a valid address")
}

fn default_health_timeout() -> u64 {
    60
}

fn default_check_interval() -> u64 {
    10
}

fn default_report_interval() -> u64 {
    30
}

/// The default state folder.
pub fn default_state_dir() -> PathBuf {
    #[cfg(windows)]
    {
        program_data().join("Vesper Launcher")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/var/lib/vesper-launcher")
    }
}

/// The default key folder (administrators only).
pub fn default_key_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        Some(program_data().join("Vesper").join("update-keys"))
    }
    #[cfg(not(windows))]
    {
        Some(PathBuf::from("/etc/vesper/update-keys"))
    }
}

/// The default configuration file.
pub fn default_config_file() -> PathBuf {
    #[cfg(windows)]
    {
        program_data().join("Vesper Launcher").join("launcher.json")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from("/etc/vesper/launcher.json")
    }
}

#[cfg(windows)]
fn program_data() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

impl Config {
    /// Reads a configuration file.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("can't read {}: {e}", path.display()))?;
        let mut c: Config = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("{} isn't a valid configuration: {e}", path.display()))?;
        let dir = path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("launcher.d");
        let mut extra: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == "json"))
                    .collect()
            })
            .unwrap_or_default();
        extra.sort();
        for f in extra {
            let text = std::fs::read_to_string(&f)
                .map_err(|e| anyhow::anyhow!("can't read {}: {e}", f.display()))?;
            let p: ProductConfig = serde_json::from_str(&text)
                .map_err(|e| anyhow::anyhow!("{} isn't a valid product: {e}", f.display()))?;
            if !c.products.iter().any(|q| q.product == p.product) {
                c.products.push(p);
            }
        }
        c.path = Some(path.to_path_buf());
        for p in &c.products {
            anyhow::ensure!(
                crate::package::valid_word(&p.product),
                "product ID {:?} isn't valid",
                p.product
            );
            anyhow::ensure!(
                !p.health_url.is_empty()
                    || (!p.is_self && matches!(p.install, InstallKind::Deb { .. })),
                "product {:?} needs a healthUrl (only a deb package of programs goes without)",
                p.product
            );
        }
        Ok(c)
    }
}
