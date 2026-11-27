//! Desktop shell of the control app. The UI is the web frontend in `../src`; this shell only
//! adds what a browser can't do: finding servers on the LAN over mDNS.

use std::time::Duration;

use serde::Serialize;

/// A server found by mDNS, as the frontend's `discoverServers()` expects it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Discovered {
    id: String,
    name: String,
    version: String,
    protocol_version: u32,
    /// `http://<address>:<port>`, or `None` when the record carried no address.
    base_url: Option<String>,
}

/// Browses `_vesper._tcp` for `timeout_ms` (default 1.5 s) and returns what answered.
#[tauri::command]
async fn discover_servers(timeout_ms: Option<u64>) -> Result<Vec<Discovered>, String> {
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(1500).clamp(200, 10_000));
    let found = tauri::async_runtime::spawn_blocking(move || protocol::discover::discover(timeout))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(found
        .into_iter()
        .map(|s| {
            let base_url = s.base_url();
            Discovered {
                id: s.id,
                name: s.name,
                version: s.version,
                protocol_version: s.protocol_version,
                base_url,
            }
        })
        .collect())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![discover_servers])
        .run(tauri::generate_context!())
        .expect("error while running the control app");
}
