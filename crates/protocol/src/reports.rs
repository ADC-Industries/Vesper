//! Reports (`/api/v1/reports/…`): CCLI song usage.
//!
//! Each time a song (a presentation with song metadata: a CCLI number, title or authors) goes
//! live, the server logs one use per service (the playlist it was shown from, else the day), with
//! whether it was projected, streamed or recorded at the time. The report counts the uses per
//! song in a date range; its CSV has the columns CCLI's reporting asks for. The reporting itself
//! is done on CCLI's site.

use model::PresentationId;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One use of a song in a service (a line of `reports/ccli-usage.jsonl`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CcliUse {
    /// The day, local time (`2026-11-29`).
    pub date: String,
    /// When it first went live (RFC 3339 UTC).
    pub at: String,
    /// The service: the playlist it was shown from, else "" (shown outside a playlist).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(as = "Option<String>", optional)]
    pub service: String,
    /// The presentation.
    pub presentation: PresentationId,
    /// The song's title (the song metadata's, else the presentation's name).
    pub title: String,
    /// The CCLI song number, as entered.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(as = "Option<String>", optional)]
    pub ccli_number: String,
    /// The authors, as entered.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(as = "Option<String>", optional)]
    pub authors: String,
    /// Shown on an audience screen.
    #[serde(default)]
    pub projected: bool,
    /// A stream was running.
    #[serde(default)]
    pub streamed: bool,
    /// A recording was running.
    #[serde(default)]
    pub recorded: bool,
}

/// One song in a usage report.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CcliReportRow {
    /// The song's title.
    pub title: String,
    /// The CCLI song number ("" when the song has none: it can't be reported).
    pub ccli_number: String,
    /// The authors.
    pub authors: String,
    /// Uses in the range.
    pub count: u32,
    /// Uses that were projected.
    pub projected: u32,
    /// Uses that were streamed.
    pub streamed: u32,
    /// Uses that were recorded.
    pub recorded: u32,
    /// The last day it was used.
    pub last_used: String,
}

/// `GET /api/v1/reports/ccli?from=YYYY-MM-DD&to=YYYY-MM-DD` (both days included; default the
/// last 6 months). `GET /api/v1/reports/ccli.csv` gives the same as CSV.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CcliReport {
    /// The first day.
    pub from: String,
    /// The last day.
    pub to: String,
    /// Songs used, most used first.
    pub rows: Vec<CcliReportRow>,
    /// Every use in the range, oldest first.
    pub uses: Vec<CcliUse>,
}

/// Settings ▸ CCLI: the church's CCLI licence and where the copyright line shows (`GET` /
/// `PUT /api/v1/settings/ccli`, the change for admins). The show settings' `ccli` part; the
/// line's layout and template are kept as they are.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CcliLicense {
    /// The church's CCLI licence number, verbatim (trimmed; at most 40 characters; none when
    /// empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub license_number: Option<String>,
    /// The copyright line under a song's words (a song with song metadata that shows it).
    #[serde(default)]
    pub enabled: bool,
    /// On which of a song's slides, counted in its play order.
    #[serde(default)]
    pub show_on: model::CcliDisplay,
}

/// `GET /api/v1/settings/ccli` (and the answer to `PUT`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CcliLicenseStatus {
    /// The settings.
    pub settings: CcliLicense,
    /// The licence number the SongSelect files read last carried (`CCLI License # …`), if any:
    /// Oriel offers it while no number is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub imported_license: Option<String>,
}
