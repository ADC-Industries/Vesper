//! Media preparation (`docs/MEDIA-PREP-PLAN.md`): the Codex box's background jobs that probe,
//! analyse and, where a video is hard to play live, convert uploaded media to the house format
//! (`/api/v1/codex/media-prep`, `/api/v1/codex/collections/{id}/media/{asset}/…`).

use model::{CollectionId, MediaAssetId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The video codec of the house format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum HouseVideoCodec {
    /// H.264 High, 8-bit 4:2:0 (the default: every GPU decoder takes it, every FFmpeg encodes
    /// it).
    #[default]
    H264,
    /// HEVC Main, 8-bit 4:2:0 (smaller files; needs a GPU that decodes HEVC on every box).
    Hevc,
}

/// What a conversion does while something is live on the Codex box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PrepWhileLive {
    /// Conversions wait until nothing is live (a running one stops and starts again later).
    /// Analysis (loudness, poster) still runs, at the lowest priority.
    Wait,
    /// Conversions go on at the lowest priority (the default, owner 2026-11-26: media imported
    /// late still get ready during a service): a running one is lowered once something goes
    /// live; with the encoder on "auto" one started while live takes the CPU's encoder where
    /// there is one for the codec (a GPU's encoder has no priority to give live work).
    #[default]
    LowPriority,
}

/// The priority conversions run at while nothing is live (while something is live they run at
/// the lowest, or wait: [`PrepWhileLive`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PrepPriority {
    /// Low (the default): Linux nice 19 and idle I/O, Windows below normal.
    #[default]
    Low,
    /// As other programs: faster, when the box has nothing else to do.
    Normal,
}

/// How good a conversion's picture is: its bit rate, relative to IDN's converter's own choice
/// by size and rate (1080p30 H.264 about 7.5 Mb/s; HEVC 0.6 of each).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PrepQuality {
    /// 1.6 × IDN's (1080p30 H.264: 12 Mb/s), the default.
    #[default]
    Standard,
    /// 2.67 × (1080p30 H.264: 20 Mb/s).
    High,
    /// 4 × (1080p30 H.264: 30 Mb/s).
    Maximum,
}

/// The largest picture a conversion makes (fitted inside, its shape kept; a portrait video
/// the other way round).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
pub enum PrepMaxSize {
    /// Its own size (above 4K: fitted inside 3840×2160), the default.
    #[default]
    #[serde(rename = "keep")]
    Keep,
    /// 1920×1080: a larger video is also a reason to convert it.
    #[serde(rename = "1080p")]
    P1080,
    /// 3840×2160.
    #[serde(rename = "2160p")]
    P2160,
}

impl PrepMaxSize {
    /// The landscape box a picture is fitted inside.
    pub fn bounds(self) -> (u32, u32) {
        match self {
            PrepMaxSize::P1080 => (1920, 1080),
            PrepMaxSize::Keep | PrepMaxSize::P2160 => (3840, 2160),
        }
    }
}

/// Which encoder conversions use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PrepEncoder {
    /// The GPU's where this box has one for the codec, else the CPU's (the default); while
    /// something is live, the CPU's where there is one.
    #[default]
    Auto,
    /// The GPU's only (no GPU encoder for the codec: videos are analysed, not converted).
    Gpu,
    /// The CPU's only (none for the codec: videos are analysed, not converted).
    Cpu,
}

/// Media preparation's settings (the Codex box's `codex/media-prep.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaPrepSettings {
    /// Prepare new uploads automatically (default on). Off: only "Prepare" by hand.
    pub enabled: bool,
    /// Convert videos that are hard to play live (default on). Off: analyse only.
    pub transcode: bool,
    /// The house format's video codec.
    pub video: HouseVideoCodec,
    /// The house loudness target, LUFS (default −16; −30…−5).
    pub loudness_target: f64,
    /// Jobs at the same time (default 1; 1–4).
    pub max_jobs: u32,
    /// What conversions do while something is live (default: go on at the lowest priority).
    pub while_live: PrepWhileLive,
    /// The priority conversions run at while nothing is live (default low).
    pub priority: PrepPriority,
    /// The picture's quality (default standard).
    pub quality: PrepQuality,
    /// An explicit video bit rate, kbit/s (1 000–60 000), instead of `quality`'s by size and
    /// rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub video_kbps: Option<u32>,
    /// The largest picture made (default its own size).
    pub max_size: PrepMaxSize,
    /// The encoder (default auto: the GPU's where there is one).
    pub encoder: PrepEncoder,
}

impl Default for MediaPrepSettings {
    fn default() -> Self {
        MediaPrepSettings {
            enabled: true,
            transcode: true,
            video: HouseVideoCodec::H264,
            loudness_target: -16.0,
            max_jobs: 1,
            while_live: PrepWhileLive::LowPriority,
            priority: PrepPriority::Low,
            quality: PrepQuality::Standard,
            video_kbps: None,
            max_size: PrepMaxSize::Keep,
            encoder: PrepEncoder::Auto,
        }
    }
}

/// Where a preparation job stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PrepStep {
    /// Waiting for a free job slot.
    #[default]
    Queued,
    /// Reading the file: facts, loudness, poster frame.
    Analysing,
    /// Converting to the house format (`progress`).
    Converting,
    /// A conversion waiting until nothing is live.
    WaitingForLive,
    /// Finished (kept in the list for a minute).
    Done,
    /// Failed (`error`; kept until the asset is prepared again).
    Failed,
}

/// One preparation job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaPrepJob {
    /// The asset.
    pub asset: MediaAssetId,
    /// Its collection.
    pub collection: CollectionId,
    /// Its name.
    pub name: String,
    /// Where it stands.
    pub step: PrepStep,
    /// How far the conversion is, 0–1.
    #[serde(default)]
    pub progress: f64,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

/// `GET /api/v1/codex/media-prep`: the settings, the jobs and what this box can do.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaPrepStatus {
    /// The settings.
    pub settings: MediaPrepSettings,
    /// Jobs queued, running, and recently finished or failed.
    pub jobs: Vec<MediaPrepJob>,
    /// What converts here: IDN's converter and the FFmpeg it runs on (`IDN media conversion,
    /// FFmpeg 8.0.1 (…)`); `None`: nothing (then videos are analysed but not converted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub converter: Option<String>,
    /// The encoder the house format uses here, for people (`VA-API`, `OpenH264 (CPU)`); `None`:
    /// none encodes it here (media are analysed, not converted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub encoder: Option<String>,
    /// This box's GPU encoder for the house codec, for people (`VA-API`); `None`: none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gpu_encoder: Option<String>,
    /// This box's CPU encoder for the house codec, for people (`OpenH264 (CPU)`); `None`: none
    /// (HEVC on Ubuntu's FFmpeg).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cpu_encoder: Option<String>,
    /// What this box has for each house codec, so a codec being chosen shows its own
    /// encoders before it is saved (`gpuEncoder` / `cpuEncoder` are the saved codec's).
    /// Absent from servers before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub codec_encoders: Option<CodecEncoders>,
    /// Something is live on this box now.
    pub live: bool,
}

/// [`MediaPrepStatus::codec_encoders`]: this box's encoders for H.264 and for HEVC.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CodecEncoders {
    /// H.264's.
    pub h264: BoxEncoders,
    /// HEVC's.
    pub hevc: BoxEncoders,
}

/// A codec's encoders on this box, for people (`VA-API`, `OpenH264 (CPU)`); `None`: none.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct BoxEncoders {
    /// The GPU's encoder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gpu: Option<String>,
    /// The CPU's encoder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cpu: Option<String>,
}

/// `POST /api/v1/codex/collections/{id}/media/{asset}/prepare`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
pub struct PrepareMedia {
    /// Prepare it again even when it was prepared before (analysed again; converted again
    /// from the original).
    pub again: bool,
}

/// `PUT /api/v1/codex/collections/{id}/media/{asset}/trim`: the trim, seconds from the
/// file's start (`null`: none).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
pub struct SetMediaTrim {
    /// The in point.
    #[ts(optional)]
    pub trim_in: Option<f64>,
    /// The out point.
    #[ts(optional)]
    pub trim_out: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_saved_before_the_new_groups_load_with_their_defaults() {
        // As batch 16 saved them: no priority, quality, size or encoder.
        let old: MediaPrepSettings = serde_json::from_str(
            r#"{"enabled": false, "transcode": true, "video": "hevc", "loudnessTarget": -18.0,
                "maxJobs": 2, "whileLive": "wait"}"#,
        )
        .unwrap();
        assert_eq!(
            old,
            MediaPrepSettings {
                enabled: false,
                video: HouseVideoCodec::Hevc,
                loudness_target: -18.0,
                max_jobs: 2,
                // A choice saved before is kept.
                while_live: PrepWhileLive::Wait,
                ..MediaPrepSettings::default()
            }
        );
        let d = MediaPrepSettings::default();
        assert_eq!(
            (
                d.enabled,
                d.while_live,
                d.priority,
                d.quality,
                d.max_size,
                d.encoder,
                d.video
            ),
            (
                true,
                PrepWhileLive::LowPriority,
                PrepPriority::Low,
                PrepQuality::Standard,
                PrepMaxSize::Keep,
                PrepEncoder::Auto,
                HouseVideoCodec::H264
            )
        );
        assert_eq!(d.video_kbps, None);
    }

    #[test]
    fn settings_round_trip() {
        let s = MediaPrepSettings {
            priority: PrepPriority::Normal,
            quality: PrepQuality::Maximum,
            video_kbps: Some(25_000),
            max_size: PrepMaxSize::P1080,
            encoder: PrepEncoder::Cpu,
            ..MediaPrepSettings::default()
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["maxSize"], "1080p");
        assert_eq!(v["videoKbps"], 25_000);
        assert_eq!(v["whileLive"], "lowPriority");
        assert_eq!(serde_json::from_value::<MediaPrepSettings>(v).unwrap(), s);
        let none = serde_json::to_value(MediaPrepSettings::default()).unwrap();
        assert!(none.get("videoKbps").is_none(), "{none}");
        assert_eq!(PrepMaxSize::P2160.bounds(), (3840, 2160));
    }
}
