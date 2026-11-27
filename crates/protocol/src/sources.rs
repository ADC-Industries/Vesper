//! Live network video sources (G7): IDN Open Protocol (compatible with NDI®) and OMT (Open
//! Media Transport, `omt:` names) sources this box can show on its video-input layer. `GET /api/v1/sources` lists them;
//! `sourcesChanged` sends the whole list after every change; `triggerLiveSource` puts one
//! live. See `docs/LIVE-SOURCES-PLAN.md`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// How a source is being received.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum LiveSourceStatus {
    /// Not received (nothing shows it).
    #[default]
    Idle,
    /// Looking it up or connecting; no picture yet.
    Connecting,
    /// Pictures arrive.
    Live,
    /// It went away or cannot be reached; the box keeps trying.
    Lost,
    /// It sends video this box cannot decode yet (H.264 / HEVC cameras).
    Unsupported,
}

/// A source on the network, or one received by address.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LiveSource {
    /// Full name, `HOST (name)` (or `ip:port` when triggered by address): what
    /// `triggerLiveSource` takes. OMT sources: `omt:HOST (name)` (or `omt:host:port`).
    pub name: String,
    /// `ip:port`, once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub address: Option<String>,
    /// It announced itself as an IDN device (otherwise another compatible sender).
    #[serde(default)]
    pub idn: bool,
    /// An OMT (Open Media Transport) source (its name starts with `omt:`); otherwise IDN.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub omt: bool,
    /// Discovered on the network now (`false`: received by address, or gone from the list
    /// while still in use).
    #[serde(default)]
    pub listed: bool,
    /// How it is being received.
    #[serde(default)]
    pub status: LiveSourceStatus,
    /// Picture width, once pictures arrive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub width: Option<u32>,
    /// Picture height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub height: Option<u32>,
    /// Frames per second the source states.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub frame_rate: Option<f64>,
    /// Why it is lost or unsupported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub message: Option<String>,
}

/// `GET /api/v1/sources`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LiveSourceList {
    /// Sources by name.
    pub sources: Vec<LiveSource>,
}
