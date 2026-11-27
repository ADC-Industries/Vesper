//! Server identity, pairing, and who a request comes from.
//!
//! - **Identity:** `<data>/machine/server.json` = `{id, name}`, created on first run (name =
//!   the host name). Edit `name` there to rename the server.
//! - **Who needs a credential:** requests from 127.0.0.1 / ::1 are trusted (the control app on
//!   the server machine itself) unless the server runs with `--no-trust-localhost`, and not
//!   for a web page from elsewhere open in a browser there ([`local_origin`]), nor for a
//!   request addressed to any host name but the machine's own ([`local_host`]). Every
//!   other client pairs once and then sends its key (`Authorization: Bearer <key>`, or the
//!   `token` query parameter for the WebSocket, which browsers cannot add headers to).
//! - **Pairing:** the server shows a 6-digit code in its log, at `GET /api/v1/pairing/code`
//!   and in `GET /api/v1/server` (both only for trusted local requests). The code rotates when
//!   used (at once, so it pairs one client), every 10 minutes, and after
//!   [`WRONG_CODES_PER_CODE`] wrong codes from any addresses. At a machine of another server's
//!   site (a relay) the code shown is the one the site's Site Manager gives it instead
//!   ([`crate::pairing`]), and this server's own isn't used or logged
//!   ([`Auth::set_local_code_shown`]).
//!   `POST /api/v1/pair {code?, clientName}` checks the code here; the client gets a **named
//!   site API key** the site's Site Manager makes: audience `app:vesper` (every Vesper server
//!   of the site), scope `control`, named `Oriel – <client name>`, living as long as IDN allows (10 years; a browser's a year).
//!   Revoking it in the site is the control. Without a code the pairing waits for a site
//!   admin's approval; at a server of the site that isn't its server it is relayed to the site's
//!   server, which checks the code ([`crate::pairing`]). Pair attempts are limited to 5 per minute per IP address
//!   ([`PerIp`]).
//! - **Checking:** every request's key is checked locally by the site module
//!   ([`crate::Site::verify`]): the site's signature, the audience and the key's
//!   revocation, which the site pushes to every machine.
//! - **Credentials from before site keys** (`<data>/machine/clients.json`, token hashes only)
//!   still work until their client swaps them, once, for a key (`POST /api/v1/pair/upgrade`).
//!   The old credential is dropped when the new key is first used; once none is left, the
//!   file is removed and this store isn't consulted any more.

use crate::Site;
use protocol::{ApiError, ClientInfo, ErrorCode, PairingCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

/// How long a pairing code is valid.
pub const CODE_LIFETIME: Duration = Duration::from_secs(600);
/// Pair attempts allowed per IP address per minute.
pub const PAIR_ATTEMPTS_PER_MINUTE: usize = 5;
/// Wrong codes (from any addresses) after which the code changes.
pub const WRONG_CODES_PER_CODE: usize = 10;
/// Last-seen times are written to disk at most this often per client.
const LAST_SEEN_PERSIST: Duration = Duration::from_secs(300);
/// A paired client's key name: this, then the client's name (at most [`KEY_NAME_MAX`]
/// characters in all, IDN's limit).
pub const KEY_NAME_PREFIX: &str = "Oriel – ";
/// The longest key name IDN takes.
pub const KEY_NAME_MAX: usize = 60;

/// The server's persistent identity (`<data>/machine/server.json`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    /// Stable ID.
    pub id: Uuid,
    /// Display name.
    pub name: String,
}

/// A credential from before site keys.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientRecord {
    id: Uuid,
    name: String,
    /// Lowercase hex SHA-256 of the token.
    token_sha256: String,
    created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_seen: Option<String>,
    /// The site key handed over in its place (`POST /api/v1/pair/upgrade`), not used yet:
    /// this record goes when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    upgraded_to: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct ClientsFile {
    #[serde(default)]
    clients: Vec<ClientRecord>,
}

/// What a paired client showed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Credential {
    /// A site API key (or another site-signed token for `app:vesper`).
    Key,
    /// A credential from before site keys (to be swapped for a key).
    Legacy,
}

/// A paired client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    /// Its id: the key's id (a token's user), or a legacy credential's UUID.
    pub id: String,
    /// Its name (the key's: `Oriel – Booth laptop`).
    pub name: String,
    /// What it showed.
    pub credential: Credential,
}

/// How a request is authenticated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    /// From the server machine (trusted).
    Local,
    /// A paired client.
    Paired(Client),
    /// Neither.
    Unpaired,
}

impl Access {
    /// Local or paired.
    pub fn allowed(&self) -> bool {
        !matches!(self, Access::Unpaired)
    }

    /// From the server machine itself.
    pub fn is_local(&self) -> bool {
        matches!(self, Access::Local)
    }

    /// The protocol's view.
    pub fn status(&self) -> protocol::AuthStatus {
        match self {
            Access::Local => protocol::AuthStatus::Local,
            Access::Paired(_) => protocol::AuthStatus::Paired,
            Access::Unpaired => protocol::AuthStatus::Unpaired,
        }
    }
}

struct Code {
    code: String,
    created: Instant,
    /// Wrong codes typed during this code's life (from any addresses).
    wrong: usize,
}

impl Code {
    fn new() -> Code {
        Code {
            code: new_code(),
            created: Instant::now(),
            wrong: 0,
        }
    }
}

/// The window of [`PerIp`].
const RATE_WINDOW: Duration = Duration::from_secs(60);

/// Attempts per IP address in the last minute (pair attempts here, sign-ins in the site).
/// Addresses without one in the last minute are forgotten (looked for once a minute), so the
/// map holds the addresses of the last two minutes at most, not every one ever seen.
#[derive(Default)]
pub struct PerIp {
    by_ip: HashMap<IpAddr, VecDeque<Instant>>,
    pruned: Option<Instant>,
}

impl PerIp {
    /// One more attempt from `ip`, unless it made `max` in the last minute already (`false`:
    /// not counted).
    pub fn attempt(&mut self, ip: IpAddr, max: usize) -> bool {
        let now = Instant::now();
        if self
            .pruned
            .is_none_or(|t| now.duration_since(t) >= RATE_WINDOW)
        {
            self.by_ip.retain(|_, q| {
                q.back()
                    .is_some_and(|t| now.duration_since(*t) < RATE_WINDOW)
            });
            self.pruned = Some(now);
        }
        let q = self.by_ip.entry(ip).or_default();
        while q
            .front()
            .is_some_and(|t| now.duration_since(*t) >= RATE_WINDOW)
        {
            q.pop_front();
        }
        if q.len() >= max {
            return false;
        }
        q.push_back(now);
        true
    }

    /// How many addresses are kept.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.by_ip.len()
    }

    /// Makes every attempt so far `d` older.
    #[cfg(test)]
    fn age(&mut self, d: Duration) {
        for t in self.by_ip.values_mut().flat_map(|q| q.iter_mut()) {
            *t = t.checked_sub(d).unwrap();
        }
        self.pruned = self.pruned.map(|t| t.checked_sub(d).unwrap());
    }
}

/// Identity, pairing codes and the credentials from before site keys.
pub struct Auth {
    identity: Identity,
    trust_localhost: bool,
    clients_path: PathBuf,
    /// Credentials from before site keys, until swapped.
    clients: Mutex<Vec<ClientRecord>>,
    /// Some legacy credential is still stored (else the store isn't looked at).
    any_legacy: AtomicBool,
    /// Last time each client's lastSeen was written to disk.
    persisted_seen: Mutex<HashMap<Uuid, Instant>>,
    code: Mutex<Code>,
    /// This server's own code is the one shown (not at a relay).
    shown: AtomicBool,
    attempts: Mutex<PerIp>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Cryptographically random bytes.
fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the operating system's random source failed");
    b
}

fn new_code() -> String {
    let n = u32::from_le_bytes(random_bytes::<4>()) % 1_000_000;
    format!("{n:06}")
}

fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Constant-time string comparison (equal lengths are compared in full).
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// Writes a file atomically and durably (flushed before and after the rename).
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    vesper_core::fs::write_atomic(path, bytes)
}

/// Is this address the machine itself?
pub fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => {
            v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback())
        }
    }
}

/// A browser page's `Origin` that may act as the server machine itself: the desktop app's
/// pages (`tauri://localhost`, `http(s)://tauri.localhost`) and pages of this machine
/// (`localhost`, a loopback address; any port). Any other page (`null` too) open in a browser
/// on the server machine is someone else's: it needs a key, like a remote client.
pub fn local_origin(origin: &str) -> bool {
    if origin == "tauri://localhost" {
        return true;
    }
    origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .is_some_and(local_host)
}

/// A request's `Host` (`name[:port]`) that may be the server machine itself: `localhost`,
/// `tauri.localhost` or a loopback address (any port). A request to any other name, even
/// one that resolves to this machine (a DNS-rebinding page sends no `Origin` for its own
/// reads), is a remote client's: it needs a key.
pub fn local_host(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        // `[::1]:8080`
        Some(v6) => match v6.split_once(']') {
            Some((h, port)) if port.is_empty() || port.starts_with(':') => h,
            _ => return false,
        },
        None => match host.split_once(':') {
            Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) => h,
            Some(_) => return false,
            None => host,
        },
    };
    let name = name.strip_suffix('.').unwrap_or(name);
    name.eq_ignore_ascii_case("localhost")
        || name.eq_ignore_ascii_case("tauri.localhost")
        || name.parse::<IpAddr>().is_ok_and(is_loopback)
}

/// The site key name for a client named `client` (`Oriel – Booth laptop`), within IDN's
/// 60 characters. An empty name becomes `client at <ip>`.
pub fn key_name(client: &str, ip: IpAddr) -> String {
    let client = match client.trim() {
        "" => format!("client at {ip}"),
        n => n.chars().filter(|c| !c.is_control()).collect(),
    };
    format!("{KEY_NAME_PREFIX}{client}")
        .chars()
        .take(KEY_NAME_MAX)
        .collect::<String>()
        .trim_end()
        .to_string()
}

impl Auth {
    /// Loads (or creates) the identity and the legacy credentials from `<data>/machine/`.
    pub fn load(data_dir: &Path, trust_localhost: bool) -> anyhow::Result<Auth> {
        let machine = data_dir.join("machine");
        let id_path = machine.join("server.json");
        let identity = match std::fs::read(&id_path) {
            Ok(b) => serde_json::from_slice::<Identity>(&b).map_err(|e| {
                anyhow::anyhow!("{} is not a valid server identity: {e}", id_path.display())
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let host = gethostname::gethostname()
                    .to_string_lossy()
                    .trim()
                    .to_string();
                let identity = Identity {
                    id: Uuid::new_v4(),
                    name: if host.is_empty() {
                        protocol::PRODUCT_NAME.to_string()
                    } else {
                        host
                    },
                };
                write_atomic(&id_path, &serde_json::to_vec_pretty(&identity)?)?;
                tracing::info!(id = %identity.id, name = %identity.name, "created server identity {}", id_path.display());
                identity
            }
            Err(e) => return Err(anyhow::anyhow!("reading {}: {e}", id_path.display())),
        };
        let clients_path = machine.join("clients.json");
        let clients = match std::fs::read(&clients_path) {
            Ok(b) => match serde_json::from_slice::<ClientsFile>(&b) {
                Ok(f) => f.clients,
                Err(e) => {
                    // Damaged: the server still starts; those clients pair again.
                    let aside = vesper_core::fs::keep_aside(&clients_path)
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|e| format!("(not moved: {e})"));
                    tracing::error!(file = %clients_path.display(), kept = %aside, "the paired clients' credentials from before site keys are damaged ({e}): those clients pair again");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        if !clients.is_empty() {
            tracing::info!(
                "{} paired clients still use their credential from before site keys; each swaps it for a site key when it next connects",
                clients.len()
            );
        }
        Ok(Auth {
            identity,
            trust_localhost,
            clients_path,
            any_legacy: AtomicBool::new(!clients.is_empty()),
            clients: Mutex::new(clients),
            persisted_seen: Mutex::new(HashMap::new()),
            code: Mutex::new(Code::new()),
            shown: AtomicBool::new(true),
            attempts: Mutex::new(PerIp::default()),
        })
    }

    /// Server identity.
    pub fn identity(&self) -> Identity {
        self.identity.clone()
    }

    /// Whether localhost is trusted.
    pub fn trusts_localhost(&self) -> bool {
        self.trust_localhost
    }

    /// The current pairing code (rotated first when expired).
    pub fn pairing_code(&self) -> PairingCode {
        self.rotate_if_expired();
        let c = lock(&self.code);
        PairingCode {
            code: c.code.clone(),
            expires_in_seconds: CODE_LIFETIME.saturating_sub(c.created.elapsed()).as_secs(),
        }
    }

    /// Rotates the code when it has expired (and logs the new one).
    pub fn rotate_if_expired(&self) {
        let expired = lock(&self.code).created.elapsed() >= CODE_LIFETIME;
        if expired {
            self.rotate();
        }
    }

    fn rotate(&self) {
        *lock(&self.code) = Code::new();
        self.log_pairing_code();
    }

    /// Whether this server's own code is the one shown: not while it relays pairings to its
    /// site's server, which gives it a code of the site's ([`crate::Site::relay_code`]);
    /// then it isn't logged. Shown again, a new one is made and logged.
    pub fn set_local_code_shown(&self, shown: bool) {
        let was = self.shown.swap(shown, Ordering::SeqCst);
        if shown && !was {
            self.rotate();
        }
    }

    /// Logs the current pairing code (when it is the one shown).
    pub fn log_pairing_code(&self) {
        if !self.shown.load(Ordering::SeqCst) {
            return;
        }
        let c = lock(&self.code).code.clone();
        tracing::info!("pairing code: {c} (valid 10 minutes or until used)");
    }

    /// Who is making a request: a client with a site key (checked by the site, here), one
    /// with a legacy credential, the server machine itself, or no one paired.
    pub fn classify(&self, site: &Site, ip: IpAddr, token: Option<&str>) -> Access {
        if let Some(t) = token.filter(|t| !t.is_empty()) {
            if t.starts_with(idn_open_protocol::trust::token::PREFIX) {
                if let Some(c) = site.verify(t) {
                    if self.any_legacy.load(Ordering::Relaxed)
                        && let Some(old) = self.key_used(&c.id)
                    {
                        site.rebind_client(&old, &c.id);
                    }
                    return Access::Paired(c);
                }
            } else if self.any_legacy.load(Ordering::Relaxed)
                && let Some(c) = self.legacy(t)
            {
                return Access::Paired(c);
            }
        }
        if self.trust_localhost && is_loopback(ip) {
            Access::Local
        } else {
            Access::Unpaired
        }
    }

    /// The legacy credential `token`'s client.
    fn legacy(&self, token: &str) -> Option<Client> {
        let h = sha256_hex(token);
        let mut clients = lock(&self.clients);
        let c = clients.iter_mut().find(|c| same(&c.token_sha256, &h))?;
        let (id, name) = (c.id, c.name.clone());
        let now = rfc3339(SystemTime::now(), false);
        let changed = c.last_seen.as_deref() != Some(now.as_str());
        c.last_seen = Some(now);
        drop(clients);
        if changed {
            self.persist_seen(id);
        }
        Some(Client {
            id: id.to_string(),
            name,
            credential: Credential::Legacy,
        })
    }

    /// The key `key` was used: the legacy credential it replaced (if any) is dropped now.
    /// Its id.
    fn key_used(&self, key: &str) -> Option<String> {
        let old = {
            let mut clients = lock(&self.clients);
            let i = clients
                .iter()
                .position(|c| c.upgraded_to.as_deref() == Some(key))?;
            clients.remove(i)
        };
        tracing::info!(client = %old.name, key, "the client swapped its old credential for its site key; the old one is dropped");
        self.save();
        Some(old.id.to_string())
    }

    /// Pairing: checks the rate limit and the code, if one was typed (`Ok(true)`: right;
    /// `Ok(false)`: none, the pairing waits for approval). A right code is used up here, at
    /// once, so it pairs one client even when sent twice at the same time (and stays used up
    /// if the pairing then fails: the new one is shown). After [`WRONG_CODES_PER_CODE`] wrong
    /// codes from any addresses, the code changes.
    pub fn check_code(&self, ip: IpAddr, code: Option<&str>) -> Result<bool, ApiError> {
        self.pair_attempt(ip)?;
        let Some(code) = code.map(str::trim).filter(|c| !c.is_empty()) else {
            return Ok(false);
        };
        self.rotate_if_expired();
        let mut c = lock(&self.code);
        if same(code, &c.code) {
            *c = Code::new();
            drop(c);
            tracing::info!(%ip, "pairing code used");
            self.log_pairing_code();
            return Ok(true);
        }
        c.wrong += 1;
        let changed = c.wrong >= WRONG_CODES_PER_CODE;
        if changed {
            *c = Code::new();
        }
        drop(c);
        tracing::warn!(%ip, "pairing attempt with a wrong code");
        if changed {
            tracing::warn!(
                "{WRONG_CODES_PER_CODE} wrong pairing codes were tried: the pairing code changed"
            );
            self.log_pairing_code();
        }
        Err(ApiError::new(
            ErrorCode::Unauthorized,
            "wrong or expired pairing code",
        ))
    }

    /// One pair attempt from `ip`: refused past [`PAIR_ATTEMPTS_PER_MINUTE`] in a minute (also
    /// at a relay, which passes the code on unchecked).
    pub fn pair_attempt(&self, ip: IpAddr) -> Result<(), ApiError> {
        if !lock(&self.attempts).attempt(ip, PAIR_ATTEMPTS_PER_MINUTE) {
            tracing::warn!(%ip, "pairing rate limit hit");
            return Err(ApiError::new(
                ErrorCode::RateLimited,
                "too many pairing attempts; wait a minute",
            ));
        }
        Ok(())
    }

    /// A legacy credential `id` was swapped for the key `key` (not used yet). The key handed
    /// over before for it, if any (never used: revoke it).
    pub fn upgraded(&self, id: &str, key: &str) -> Option<String> {
        let prev = {
            let mut clients = lock(&self.clients);
            let c = clients.iter_mut().find(|c| c.id.to_string() == id)?;
            c.upgraded_to.replace(key.to_string())
        };
        self.save();
        prev
    }

    fn persist_seen(&self, id: Uuid) {
        let due = {
            let mut p = lock(&self.persisted_seen);
            let due = p.get(&id).is_none_or(|t| t.elapsed() >= LAST_SEEN_PERSIST);
            if due {
                p.insert(id, Instant::now());
            }
            due
        };
        if due {
            self.save();
        }
    }

    /// Writes the legacy credentials (the file goes with the last one).
    fn save(&self) {
        let clients = lock(&self.clients).clone();
        self.any_legacy
            .store(!clients.is_empty(), Ordering::Relaxed);
        if clients.is_empty() {
            match std::fs::remove_file(&self.clients_path) {
                Ok(()) => tracing::info!(
                    "every client uses a site key now: {} removed",
                    self.clients_path.display()
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => tracing::error!("removing {}: {e}", self.clients_path.display()),
            }
            return;
        }
        let res = serde_json::to_vec_pretty(&ClientsFile { clients })
            .map_err(std::io::Error::other)
            .and_then(|b| write_atomic(&self.clients_path, &b));
        if let Err(e) = res {
            tracing::error!("writing {}: {e}", self.clients_path.display());
        }
    }

    /// The clients still on a legacy credential.
    pub fn legacy_clients(&self) -> Vec<ClientInfo> {
        lock(&self.clients)
            .iter()
            .map(|c| ClientInfo {
                id: c.id.to_string(),
                name: c.name.clone(),
                created_at: c.created_at.clone(),
                last_seen: c.last_seen.clone(),
            })
            .collect()
    }

    /// Drops a legacy credential. `None` for unknown IDs; else the site key handed over for
    /// it and not used yet, if any (the client has it: revoke it too).
    pub fn revoke_legacy(&self, id: &str) -> Option<Option<String>> {
        let removed = {
            let mut c = lock(&self.clients);
            let i = c.iter().position(|r| r.id.to_string() == id)?;
            c.remove(i)
        };
        tracing::info!(%id, "legacy client credential revoked");
        self.save();
        Some(removed.upgraded_to)
    }
}

#[cfg(test)]
impl Auth {
    /// How many addresses the pair rate limit keeps.
    fn attempts_tracked(&self) -> usize {
        lock(&self.attempts).len()
    }

    /// Makes every pair attempt so far `d` older.
    fn age_attempts(&self, d: Duration) {
        lock(&self.attempts).age(d);
    }
}

/// RFC 3339 UTC timestamp (`2026-10-19T09:30:00Z`); `seconds: false` truncates to minutes.
pub fn rfc3339(t: SystemTime, seconds: bool) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, mi, s) = (
        rem / 3600,
        rem % 3600 / 60,
        if seconds { rem % 60 } else { 0 },
    );
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_792_402_245); // date -u -d @1792402245
        assert_eq!(rfc3339(t, true), "2026-10-19T09:30:45Z");
        assert_eq!(rfc3339(t, false), "2026-10-19T09:30:00Z");
        assert_eq!(
            rfc3339(SystemTime::UNIX_EPOCH, true),
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn codes_and_tokens() {
        let c = new_code();
        assert_eq!(c.len(), 6);
        assert!(c.chars().all(|c| c.is_ascii_digit()));
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
        assert_eq!(sha256_hex("abc").len(), 64);
        assert!(is_loopback("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_loopback("192.0.2.2".parse().unwrap()));
        for o in [
            "tauri://localhost",
            "http://tauri.localhost",
            "https://tauri.localhost",
            "http://localhost:5173",
            "http://127.0.0.1",
            "http://[::1]:8080",
        ] {
            assert!(local_origin(o), "{o}");
        }
        for o in [
            "null",
            "http://attacker.example",
            "http://localhost.attacker.example",
            "http://127.0.0.1.attacker.example",
            "http://[::1].attacker.example",
            "http://192.0.2.2:5555",
            "file://",
            "tauri://localhost.attacker.example",
        ] {
            assert!(!local_origin(o), "{o}");
        }
        for h in [
            "localhost",
            "LOCALHOST:5555",
            "localhost.:5555",
            "127.0.0.1:5555",
            "127.0.0.2",
            "[::1]:5555",
            "[::1]",
            "tauri.localhost",
        ] {
            assert!(local_host(h), "{h}");
        }
        for h in [
            "oriel.test:5621",
            "oriel.test",
            "localhost.attacker.example",
            "127.0.0.1.attacker.example:5555",
            "192.0.2.2:5555",
            "[::1].attacker.example",
            "localhost:x",
            "",
        ] {
            assert!(!local_host(h), "{h}");
        }
        let ip = "192.0.2.7".parse().unwrap();
        assert_eq!(key_name(" Booth laptop ", ip), "Oriel – Booth laptop");
        assert_eq!(key_name("", ip), "Oriel – client at 192.0.2.7");
        let long = key_name(&"x".repeat(100), ip);
        assert_eq!(long.chars().count(), KEY_NAME_MAX);
    }

    /// A scratch data folder for an `Auth`.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vesper-auth-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The `n`th address of 2001:db8::/32 (documentation addresses).
    fn v6(n: u32) -> IpAddr {
        IpAddr::V6(std::net::Ipv6Addr::new(
            0x2001,
            0xdb8,
            0,
            0,
            0,
            0,
            (n >> 16) as u16,
            n as u16,
        ))
    }

    /// Guessing from many addresses (one try each, under the per-address limit): after
    /// [`WRONG_CODES_PER_CODE`] wrong codes the code changes, so a guesser can't try much of
    /// the 10^6 codes during one code's life.
    #[test]
    fn many_wrong_codes_from_many_addresses_change_the_code() {
        let dir = scratch("guess");
        let auth = Auth::load(&dir, true).unwrap();
        let code = auth.pairing_code().code;
        let wrong = if code == "000000" { "000001" } else { "000000" };
        for n in 0..WRONG_CODES_PER_CODE {
            assert!(auth.check_code(v6(n as u32), Some(wrong)).is_err());
        }
        assert_ne!(auth.pairing_code().code, code, "the code changed");
        // The per-address limit stays.
        let ip = v6(999);
        let wrong = "x";
        for _ in 0..PAIR_ATTEMPTS_PER_MINUTE {
            let e = auth.check_code(ip, Some(wrong)).unwrap_err();
            assert_eq!(e.code, ErrorCode::Unauthorized);
        }
        let e = auth.check_code(ip, Some(wrong)).unwrap_err();
        assert_eq!(e.code, ErrorCode::RateLimited);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A right code is used up at once: the same code a second time is wrong.
    #[test]
    fn a_code_works_once() {
        let dir = scratch("once");
        let auth = Auth::load(&dir, true).unwrap();
        let code = auth.pairing_code().code;
        assert!(matches!(auth.check_code(v6(1), Some(&code)), Ok(true)));
        assert_ne!(auth.pairing_code().code, code);
        assert!(auth.check_code(v6(2), Some(&code)).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The addresses that tried to pair are forgotten once their minute is over: the map
    /// doesn't grow with every address ever seen.
    #[test]
    fn the_pair_rate_limit_forgets_old_addresses() {
        let dir = scratch("prune");
        let auth = Auth::load(&dir, true).unwrap();
        for n in 0..1000 {
            auth.check_code(v6(n), None).unwrap();
        }
        assert_eq!(auth.attempts_tracked(), 1000);
        auth.age_attempts(Duration::from_secs(61));
        auth.check_code(v6(5000), None).unwrap();
        assert_eq!(
            auth.attempts_tracked(),
            1,
            "only the address of the last minute"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
