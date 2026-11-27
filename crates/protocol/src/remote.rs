//! Control surfaces: the Remote API compatibility layer's settings (`/api/v1/remote-compat`).
//!
//! The compatibility layer answers the commonly used endpoints of the presentation remote REST
//! API that existing Stream Deck / Companion button profiles call, on a port of its own, with
//! the same authentication as `/api/v1` (see the README, "Remote API compatibility").

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The default port of the Remote API compatibility layer.
pub const REMOTE_COMPAT_DEFAULT_PORT: u16 = 50001;

/// The Remote API compatibility layer's settings (machine-local, kept across restarts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCompatSettings {
    /// The layer answers on its port.
    #[serde(default)]
    pub enabled: bool,
    /// Its TCP port (on the API's interface; `0` = any free port).
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_port() -> u16 {
    REMOTE_COMPAT_DEFAULT_PORT
}

impl Default for RemoteCompatSettings {
    fn default() -> Self {
        RemoteCompatSettings {
            enabled: false,
            port: REMOTE_COMPAT_DEFAULT_PORT,
        }
    }
}

/// `PUT /api/v1/remote-compat`: only the fields given change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRemoteCompat {
    /// Switch the layer on or off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub enabled: Option<bool>,
    /// A new port (`1`–`65535`; `0` = any free port).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub port: Option<u16>,
}

/// `GET /api/v1/remote-compat`: the settings and whether the layer listens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCompatStatus {
    /// Switched on.
    pub enabled: bool,
    /// The configured port.
    pub port: u16,
    /// It listens now.
    pub listening: bool,
    /// The port it listens on (the configured one, or the one taken for port `0`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub listening_port: Option<u16>,
    /// Why it doesn't listen while switched on (e.g. the port is in use).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub problem: Option<String>,
}
