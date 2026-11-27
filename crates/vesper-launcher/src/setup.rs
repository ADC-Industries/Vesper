//! First-run setup (the wizard: choose roles, join the site, wait for approval, done) and the
//! machine's **Launcher key**.
//!
//! Joining pairs the Launcher with the site's server (`POST /api/v1/pair` with
//! `launcher: <machineId>`): with the server's pairing code the key comes back at once;
//! without one the pairing waits for an admin's approval in Oriel (Site ▸ Pairings), and the
//! Launcher asks for the answer every two seconds. The key it gets is its machine's own
//! (audience `app:vesper-launcher`, bound to the machine ID at the site's server): it reports
//! only as this machine and picks up only this machine's actions; it controls nothing else.
//!
//! State folder:
//!
//! | File | What |
//! |---|---|
//! | `setup.json` | the roles chosen, the wizard finished |
//! | `site.json` | the site's server and this machine's Launcher key (readable by the Launcher's account only) |

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The roles the wizard offers (`id`, name, available now).
pub const ROLES: &[(&str, &str, bool)] = &[
    ("presentation", "Presentation server", true),
    ("display", "Display box (Mosaic)", true),
    ("mixer", "Audio mixer (Cantor)", false),
    ("switcher", "Video switcher", false),
    ("intercom", "Intercom station", false),
];

/// How long a request to the site's server may take.
const TIMEOUT: Duration = Duration::from_secs(10);

/// `setup.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupFile {
    /// The roles chosen in the wizard (`None`: the configuration file's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    /// The wizard was finished (or skipped).
    #[serde(default)]
    pub done: bool,
}

/// `site.json`: the site this machine joined.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteKey {
    /// The site's server (`http://host:5555`).
    pub server: String,
    /// Its name, as it answered.
    #[serde(default)]
    pub server_name: String,
    /// This machine's Launcher key.
    pub token: String,
    /// The key's ID in the site (an admin revokes it by this in Site ▸ API keys).
    pub key_id: String,
    /// When it joined (RFC 3339).
    pub joined_at: String,
}

/// Where joining stands (`GET /launcher/v1/setup`'s `join`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state"
)]
pub enum JoinState {
    /// Not in a site.
    NotJoined {
        /// Why the last try ended, for people (a wrong code, refused, ended unanswered).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        problem: Option<String>,
    },
    /// Waiting for an admin's approval at the site's server.
    Waiting {
        /// The site's server.
        server: String,
        /// Its name.
        server_name: String,
        /// The pairing's secret id there.
        #[serde(skip)]
        pairing_id: String,
        /// Seconds until it ends unanswered (as of the last answer).
        expires_in_seconds: u64,
    },
    /// In the site, with this machine's Launcher key.
    Joined {
        /// The site's server.
        server: String,
        /// Its name.
        server_name: String,
        /// The key's ID.
        key_id: String,
        /// Since when.
        joined_at: String,
    },
}

/// What `POST /launcher/v1/setup/join` takes.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JoinRequest {
    /// The site's server: `host`, `host:port` or `http://host:port`.
    pub server: String,
    /// The pairing code it shows (none: wait for an admin's approval).
    #[serde(default)]
    pub code: Option<String>,
}

/// The site's server's address as a URL: `http://`, port 5555 unless given.
pub fn server_url(s: &str) -> Result<String, String> {
    let s = s.trim().trim_end_matches('/');
    if s.is_empty() {
        return Err("type the site's server's name or address".into());
    }
    if s.starts_with("https://") {
        return Err("the site's server answers on http:// on the church's network".into());
    }
    let rest = s.strip_prefix("http://").unwrap_or(s);
    if rest.contains('/') || rest.contains(char::is_whitespace) || rest.contains('@') {
        return Err(format!("{s:?} isn't a server's name or address"));
    }
    // `[v6]:port`, `[v6]`, `host:port`, `host`, a bare IPv6 address.
    let has_port = if rest.starts_with('[') {
        rest.contains("]:")
    } else {
        rest.matches(':').count() == 1
    };
    let host = if !rest.starts_with('[') && rest.matches(':').count() > 1 {
        format!("[{rest}]")
    } else {
        rest.to_string()
    };
    Ok(if has_port {
        format!("http://{host}")
    } else {
        format!("http://{host}:5555")
    })
}

/// Reads a JSON state file (`None` when there is none).
pub fn read<T: for<'de> Deserialize<'de>>(path: &Path) -> anyhow::Result<Option<T>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(serde_json::from_slice(&b).map_err(|e| {
            anyhow::anyhow!("{} isn't valid: {e}", path.display())
        })?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Writes a JSON state file atomically; `secret`: readable by this account only (Unix).
pub fn write<T: Serialize>(path: &Path, v: &T, secret: bool) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        if secret {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = secret;
        let mut f = o.open(&tmp)?;
        std::io::Write::write_all(&mut f, &serde_json::to_vec_pretty(v)?)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// The state folder's `setup.json`.
pub fn setup_path(state: &Path) -> PathBuf {
    state.join("setup.json")
}

/// The state folder's `site.json`.
pub fn site_path(state: &Path) -> PathBuf {
    state.join("site.json")
}

/// The answer of the site's server to a pairing.
pub enum PairAnswer {
    /// The key: `(token, keyId)`.
    Key(String, String),
    /// Waiting for approval: `(pairingId, expiresInSeconds)`.
    Waiting(String, u64),
    /// Refused, ended, or the server can't be reached: why, for people.
    No(String),
}

fn message(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string())
}

fn pair_answer(status: u16, body: &[u8]) -> PairAnswer {
    let v: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    match status {
        200 => match (v["token"].as_str(), v["clientId"].as_str()) {
            (Some(t), Some(id)) => PairAnswer::Key(t.into(), id.into()),
            _ => PairAnswer::No("the site's server answered something unexpected".into()),
        },
        202 => match v["pairingId"].as_str() {
            Some(id) => PairAnswer::Waiting(id.into(), v["expiresInSeconds"].as_u64().unwrap_or(0)),
            None => PairAnswer::No("the site's server answered something unexpected".into()),
        },
        401 => PairAnswer::No(
            "that code isn't right (or it changed meanwhile): type the code the site's server shows now"
                .into(),
        ),
        403 => PairAnswer::No(format!("refused: {}", message(body))),
        404 => PairAnswer::No(
            "the request ended without an answer (after 10 minutes): join again".into(),
        ),
        429 => PairAnswer::No(format!("{} ", message(body)).trim().to_string()),
        _ => PairAnswer::No(format!(
            "the site's server answered {status}: {}",
            message(body)
        )),
    }
}

/// Asks the site's server at `url` what it is: its name (a Vesper server answers
/// `GET /api/v1/server`).
pub fn server_name(url: &str) -> Result<String, String> {
    let r = crate::http::request("GET", &format!("{url}/api/v1/server"), &[], None, TIMEOUT)
        .map_err(|e| format!("no Vesper server answers at {url}: {e}"))?;
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap_or_default();
    match (r.status, v["name"].as_str()) {
        (200, Some(n)) => Ok(n.to_string()),
        _ => Err(format!("{url} isn't a Vesper server")),
    }
}

/// `POST /api/v1/pair` for this machine's Launcher.
pub fn pair(url: &str, machine_name: &str, machine_id: &str, code: Option<&str>) -> PairAnswer {
    let mut body = serde_json::json!({"clientName": machine_name, "launcher": machine_id});
    if let Some(c) = code {
        body["code"] = c.into();
    }
    let body = body.to_string();
    match crate::http::request(
        "POST",
        &format!("{url}/api/v1/pair"),
        &[("Content-Type", "application/json")],
        Some(body.as_bytes()),
        TIMEOUT,
    ) {
        Ok(r) => pair_answer(r.status, &r.body),
        Err(e) => PairAnswer::No(format!("the site's server can't be reached: {e}")),
    }
}

/// `GET /api/v1/pair/{id}`: the waiting pairing's answer. `None`: the server can't be
/// reached now (asked again).
pub fn pairing_status(url: &str, id: &str) -> Option<PairAnswer> {
    crate::http::request(
        "GET",
        &format!("{url}/api/v1/pair/{id}"),
        &[],
        None,
        TIMEOUT,
    )
    .ok()
    .map(|r| pair_answer(r.status, &r.body))
}

/// `DELETE /api/v1/pair/{id}`: takes the waiting pairing back (best effort).
pub fn withdraw(url: &str, id: &str) {
    let _ = crate::http::request(
        "DELETE",
        &format!("{url}/api/v1/pair/{id}"),
        &[],
        None,
        TIMEOUT,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_addresses_become_urls() {
        assert_eq!(server_url("192.0.2.10").unwrap(), "http://192.0.2.10:5555");
        assert_eq!(
            server_url(" http://192.0.2.10:8080/ ").unwrap(),
            "http://192.0.2.10:8080"
        );
        assert_eq!(server_url("main.local").unwrap(), "http://main.local:5555");
        assert_eq!(
            server_url("2001:db8::1").unwrap(),
            "http://[2001:db8::1]:5555"
        );
        assert_eq!(
            server_url("[2001:db8::1]:80").unwrap(),
            "http://[2001:db8::1]:80"
        );
        assert!(server_url("").is_err());
        assert!(server_url("https://x").is_err());
        assert!(server_url("x/y").is_err());
    }
}
