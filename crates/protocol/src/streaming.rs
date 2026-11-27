//! Recording and streaming a screen (its picture and the program audio) through IDN's
//! gateway: the box's settings (`GET /api/v1/streaming`), its targets, and the live status
//! (`GET /api/v1/streaming/status`). See `docs/RECORD-STREAM-PLAN.md`.
//!
//! Stream keys are written, never read back: a target answers `hasKey` and `key: null`.

use model::ScreenId;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A recording's file format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum RecordingContainer {
    /// Fragmented MP4: opens in every editor; a crash loses at most the last fragment.
    #[default]
    Mp4,
    /// Matroska.
    Mkv,
}

/// The video codec of a recording or stream (the box's encoder must take it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StreamCodec {
    /// H.264: every platform and editor takes it.
    #[default]
    H264,
    /// HEVC (H.265).
    Hevc,
    /// AV1 (royalty-free).
    Av1,
}

/// A stream target's protocol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum StreamKind {
    /// RTMP or RTMPS (`rtmp://` / `rtmps://` server URL plus a stream key).
    #[default]
    Rtmp,
    /// SRT (`srt://host:port?…`, sent as MPEG-TS).
    Srt,
}

fn default_record_mbps() -> f64 {
    12.0
}
fn default_record_kbps() -> u32 {
    192
}
fn default_min_free() -> f64 {
    2.0
}
fn default_stream_mbps() -> f64 {
    6.0
}
fn default_stream_kbps() -> u32 {
    160
}

/// How the box records (`PUT /api/v1/streaming/recording`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSettings {
    /// The screen recorded (`null`: the first enabled screen).
    #[serde(default)]
    pub screen: Option<ScreenId>,
    /// File format.
    #[serde(default)]
    pub container: RecordingContainer,
    /// Video codec.
    #[serde(default)]
    pub codec: StreamCodec,
    /// Video bit rate, Mbit/s (1–100).
    #[serde(default = "default_record_mbps")]
    pub video_mbps: f64,
    /// AAC bit rate, kbit/s (64–512).
    #[serde(default = "default_record_kbps")]
    pub audio_kbps: u32,
    /// A new file every this many minutes (0: one file).
    #[serde(default)]
    pub split_minutes: u32,
    /// The folder recordings go to (`null`: the data folder's `recordings`).
    #[serde(default)]
    pub folder: Option<String>,
    /// Refuse to start, and stop a recording, when the folder's disk has less free than
    /// this many GB (a running recording stops at half of it).
    #[serde(default = "default_min_free")]
    pub min_free_gb: f64,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        RecordingSettings {
            screen: None,
            container: RecordingContainer::Mp4,
            codec: StreamCodec::H264,
            video_mbps: default_record_mbps(),
            audio_kbps: default_record_kbps(),
            split_minutes: 0,
            folder: None,
            min_free_gb: default_min_free(),
        }
    }
}

/// A place the box streams to (`POST /api/v1/streaming/targets`,
/// `PUT /api/v1/streaming/targets/{id}`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StreamTarget {
    /// Made by the server.
    #[serde(default)]
    pub id: String,
    /// Shown to operators ("YouTube", "Overflow room").
    #[serde(default)]
    pub name: String,
    /// Protocol.
    #[serde(default)]
    pub kind: StreamKind,
    /// RTMP: the server URL without the key (`rtmps://a.example.com/live2`); SRT: the whole
    /// `srt://` URL (its passphrase and stream ID in the query). Shown back with SRT query
    /// values hidden.
    #[serde(default)]
    pub url: String,
    /// The stream key (RTMP), write-only: never sent by the server. Absent in a `PUT`
    /// keeps the saved key; `""` clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub key: Option<String>,
    /// A key is saved.
    #[serde(default)]
    pub has_key: bool,
    /// The screen streamed (`null`: the first enabled screen).
    #[serde(default)]
    pub screen: Option<ScreenId>,
    /// Video codec (H.264 for RTMP platforms).
    #[serde(default)]
    pub codec: StreamCodec,
    /// Video bit rate, Mbit/s (0.5–50).
    #[serde(default = "default_stream_mbps")]
    pub video_mbps: f64,
    /// AAC bit rate, kbit/s (64–320).
    #[serde(default = "default_stream_kbps")]
    pub audio_kbps: u32,
}

impl Default for StreamTarget {
    fn default() -> Self {
        StreamTarget {
            id: String::new(),
            name: String::new(),
            kind: StreamKind::Rtmp,
            url: String::new(),
            key: None,
            has_key: false,
            screen: None,
            codec: StreamCodec::H264,
            video_mbps: default_stream_mbps(),
            audio_kbps: default_stream_kbps(),
        }
    }
}

/// An encoder this box has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StreamEncoder {
    /// The codec.
    pub codec: StreamCodec,
    /// What encodes it ("VA-API", "NVENC", "OpenH264 (CPU)").
    pub label: String,
}

/// `GET /api/v1/streaming`: the box's recording and streaming settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StreamingSettings {
    /// Recording.
    pub recording: RecordingSettings,
    /// Stream targets (keys hidden).
    pub targets: Vec<StreamTarget>,
    /// The folder recordings go to now (the setting, else the default).
    pub recordings_folder: String,
    /// The encoders this box has (empty: it can't record or stream).
    pub encoders: Vec<StreamEncoder>,
}

/// What a recording or stream is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum GatewayState {
    /// Not running.
    #[default]
    Idle,
    /// Opening the file or connection.
    Connecting,
    /// Recording or sending.
    Live,
    /// The connection failed; trying again shortly (`detail` says why).
    Retrying,
    /// It stopped with an error (`detail`).
    Failed,
}

/// The recording in progress.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatus {
    /// The screen recorded.
    pub screen: ScreenId,
    /// Its name.
    pub screen_name: String,
    /// The file being written (its name in the recordings folder).
    pub file: String,
    /// State.
    pub state: GatewayState,
    /// Why it retries or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    /// When it started, ms since the Unix epoch.
    pub started_at: u64,
    /// Seconds since it started.
    pub seconds: f64,
    /// Bytes written (all parts).
    pub bytes: u64,
    /// The part being written (1 the first; a split starts the next).
    pub part: u32,
    /// Video bit rate encoded, kbit/s.
    pub video_kbps: u32,
    /// Pictures the encoder skipped because it couldn't keep up.
    pub skipped_pictures: u64,
    /// Packets dropped because writing was too slow.
    pub dropped: u64,
    /// Free space left in the recordings folder, bytes.
    pub free_bytes: u64,
}

/// One target's stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StreamStatus {
    /// The target.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The screen streamed.
    pub screen: ScreenId,
    /// State.
    pub state: GatewayState,
    /// Why it retries or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    /// When it started, ms since the Unix epoch.
    pub started_at: u64,
    /// Seconds since it started.
    pub seconds: f64,
    /// Bytes sent.
    pub bytes: u64,
    /// What it sent over the last seconds, kbit/s.
    pub sent_kbps: u32,
    /// Video bit rate encoded, kbit/s.
    pub video_kbps: u32,
    /// Times it connected again.
    pub reconnects: u32,
    /// Packets dropped because the connection was too slow.
    pub dropped: u64,
    /// Pictures the encoder skipped because it couldn't keep up.
    pub skipped_pictures: u64,
    /// How long the oldest packet has waited, ms (over a second: the upload can't keep up).
    pub lag_ms: u32,
}

/// `GET /api/v1/streaming/status`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StreamingStatus {
    /// The recording in progress.
    #[serde(default)]
    pub recording: Option<RecordingStatus>,
    /// The streams running.
    #[serde(default)]
    pub streams: Vec<StreamStatus>,
}

/// `POST /api/v1/recording/start` (and a target's `…/start`): which screen (`null`: the
/// setting's).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StartGateway {
    /// The screen (`null`: the setting's).
    #[serde(default)]
    pub screen: Option<ScreenId>,
}
