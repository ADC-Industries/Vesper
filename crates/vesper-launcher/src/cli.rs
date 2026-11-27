//! `vesper-launcher join`: joins this machine to the site from the command line (a headless
//! box, a script), through the running Launcher service's local API, as the wizard does.
//!
//! ```text
//! vesper-launcher join 482913 --server main-server.local --role display
//! vesper-launcher join                      # the only server found on the network; waits for an admin
//! ```

use std::io::Write;
use std::time::{Duration, Instant};

/// What `join` is asked.
#[derive(Clone, Debug, Default)]
pub struct JoinArgs {
    /// The Launcher service's local API (`127.0.0.1:5556`).
    pub api: String,
    /// The site's server (`None`: the only Vesper server found on the network).
    pub server: Option<String>,
    /// The site's server's pairing code (`None`: wait for an admin's approval).
    pub code: Option<String>,
    /// The machine's role, set before joining (`presentation`, `display`).
    pub role: Option<String>,
    /// Wait for an admin's approval (otherwise return once the request is made).
    pub wait: bool,
    /// How long to wait for the approval (the pairing ends after 10 minutes anyway).
    pub wait_for: Duration,
}

fn call(
    api: &str,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> anyhow::Result<serde_json::Value> {
    let body = body.map(|b| b.to_string());
    let r = crate::http::request(
        method,
        &format!("http://{api}/launcher/v1{path}"),
        &[("Content-Type", "application/json")],
        body.as_deref().map(str::as_bytes),
        Duration::from_secs(30),
    )
    .map_err(|e| {
        anyhow::anyhow!("the Vesper Launcher service doesn't answer at {api} ({e}): is it running? (systemctl status vesper-launcher)")
    })?;
    let v: serde_json::Value = serde_json::from_slice(&r.body).unwrap_or_default();
    if !(200..300).contains(&r.status) {
        let m = v["error"]["message"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| String::from_utf8_lossy(&r.body).trim().to_string());
        anyhow::bail!("{m}");
    }
    Ok(v)
}

/// The site's server to join: the one given, else the only Vesper server on the network.
fn pick_server(a: &JoinArgs, out: &mut dyn Write) -> anyhow::Result<String> {
    if let Some(s) = &a.server {
        return Ok(s.clone());
    }
    writeln!(out, "Looking for Vesper servers on the network…")?;
    let found = protocol::discover::discover(Duration::from_secs(2)).map_err(|e| {
        anyhow::anyhow!("can't look for servers on the network: {e}; give --server")
    })?;
    let urls: Vec<(String, String)> = found
        .into_iter()
        .filter_map(|s| Some((s.name.clone(), s.base_url()?)))
        .collect();
    match urls.as_slice() {
        [] => anyhow::bail!(
            "no Vesper server answered on the network: give the site's server with --server NAME-OR-ADDRESS"
        ),
        [(name, url)] => {
            writeln!(out, "Found {name} at {url}.")?;
            Ok(url.clone())
        }
        many => anyhow::bail!(
            "several Vesper servers are on the network; say which is the site's server with --server:\n{}",
            many.iter()
                .map(|(n, u)| format!("  {n}  {u}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

/// `vesper-launcher join`: sets the role (if given), joins, waits for the answer (if asked),
/// and marks the first-run setup done. Messages for people go to `out`.
pub fn join(a: &JoinArgs, out: &mut dyn Write) -> anyhow::Result<()> {
    if let Some(role) = &a.role {
        call(
            &a.api,
            "PUT",
            "/setup",
            Some(serde_json::json!({"roles": [role]})),
        )?;
        writeln!(out, "This machine's role: {role}.")?;
    }
    let state = call(&a.api, "GET", "/setup", None)?;
    if state["join"]["state"] == "joined" {
        writeln!(
            out,
            "This machine is in the site already ({}). To join another, leave it first: curl -X DELETE -H 'Content-Type: application/json' http://{}/launcher/v1/setup/join",
            state["join"]["serverName"].as_str().unwrap_or_default(),
            a.api
        )?;
        call(
            &a.api,
            "PUT",
            "/setup",
            Some(serde_json::json!({"done": true})),
        )?;
        return Ok(());
    }
    let server = pick_server(a, out)?;
    let code = a
        .code
        .as_deref()
        .map(|c| c.chars().filter(|c| !c.is_whitespace()).collect::<String>());
    let mut v = call(
        &a.api,
        "POST",
        "/setup/join",
        Some(serde_json::json!({"server": server, "code": code})),
    )?;
    let deadline = Instant::now() + a.wait_for;
    let mut said = false;
    loop {
        let j = &v["join"];
        match j["state"].as_str() {
            Some("joined") => {
                call(
                    &a.api,
                    "PUT",
                    "/setup",
                    Some(serde_json::json!({"done": true})),
                )?;
                writeln!(
                    out,
                    "Joined the site at {} ({}): this machine reports there and gets its updates from it.",
                    j["serverName"].as_str().unwrap_or_default(),
                    j["server"].as_str().unwrap_or_default()
                )?;
                if a.role.as_deref() == Some("display") {
                    writeln!(
                        out,
                        "The Launcher now sets this machine up as a display box (its IDN Node joins the site too); see the progress with: vesper-launcher join, or its window."
                    )?;
                }
                return Ok(());
            }
            Some("waiting") => {
                if !said {
                    writeln!(
                        out,
                        "Waiting for an admin to approve \"Vesper Launcher – {}\" in Oriel ▸ Site ▸ Pairings at {}…",
                        v["machineName"].as_str().unwrap_or_default(),
                        j["serverName"].as_str().unwrap_or_default()
                    )?;
                    said = true;
                }
                if !a.wait {
                    writeln!(
                        out,
                        "(Not waiting: see where it stands with curl http://{}/launcher/v1/setup)",
                        a.api
                    )?;
                    return Ok(());
                }
                anyhow::ensure!(
                    Instant::now() < deadline,
                    "no answer from the site yet; it is still waiting (cancel with curl -X DELETE -H 'Content-Type: application/json' http://{}/launcher/v1/setup/join)",
                    a.api
                );
                std::thread::sleep(Duration::from_secs(2));
                v = call(&a.api, "GET", "/setup", None)?;
            }
            _ => anyhow::bail!(
                "{}",
                j["problem"]
                    .as_str()
                    .unwrap_or("the site's server didn't take this machine")
            ),
        }
    }
}
