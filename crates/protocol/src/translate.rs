//! Machine translation of second-language slides (`docs/DUAL-LANGUAGE-PLAN.md` §5): the
//! translation models of this box (`/api/v1/translate/models`) and translation previews
//! (`/api/v1/translate/preview`, `/api/v1/translate/jobs/{id}`). A preview is never saved:
//! the operator keeps it with one `setSlideTranslation` per slide.

use model::{PresentationId, SlideId, SlideTranslation};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where a translation model stands on this box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TranslationModelStatus {
    /// Not on this box (downloaded on first use, or from Settings).
    #[default]
    NotDownloaded,
    /// Being fetched (`downloaded` of `size` bytes).
    Downloading,
    /// On this box, checked.
    Ready,
    /// The last download failed (`detail`).
    Failed,
}

/// One language pair's model (`GET /api/v1/translate/models`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TranslationModel {
    /// The pair (`en-es`).
    pub id: String,
    /// Source language (BCP 47).
    pub from: String,
    /// Target languages it writes (`["pt", "pt-BR"]`).
    pub to: Vec<String>,
    /// The model (`opus-mt-en-es`).
    pub model: String,
    /// Its publisher's repository (`Helsinki-NLP/opus-mt-en-es`).
    pub repo: String,
    /// The pinned revision (commit) its files come from.
    pub revision: String,
    /// The model card's page.
    pub card: String,
    /// The model's licence (SPDX: `Apache-2.0`, `CC-BY-4.0`).
    pub licence: String,
    /// The attribution to show with it.
    pub attribution: String,
    /// Download size, bytes.
    pub size: u64,
    /// Where it stands.
    pub status: TranslationModelStatus,
    /// Bytes fetched so far (`downloading`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub downloaded: Option<u64>,
    /// Why the last download failed (`failed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
}

/// `GET /api/v1/translate/models`: the pairs offered and the engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TranslationModels {
    /// The engine (`opus-mt (candle)`).
    pub engine: String,
    /// The pairs.
    pub models: Vec<TranslationModel>,
    /// The banner shown with every machine translation.
    pub notice: String,
}

/// `POST /api/v1/translate/preview`: translate a presentation's slides (not saved).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TranslatePreview {
    /// The presentation.
    pub presentation: PresentationId,
    /// Its slides to translate (`None`: all of them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub slides: Option<Vec<SlideId>>,
    /// The target language (BCP 47: `es`, `pt-BR`).
    pub to: String,
    /// The slides' language (default `en`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from: Option<String>,
}

/// Where a translation preview stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum TranslateJobStatus {
    /// Waiting for the engine (one preview runs at a time).
    #[default]
    Queued,
    /// Fetching the model first (`downloaded` of `downloadSize` bytes).
    Downloading,
    /// Translating (`progress`).
    Translating,
    /// Finished: `slides` holds the proposals.
    Done,
    /// Failed (`detail`).
    Failed,
    /// Cancelled (`DELETE /translate/jobs/{id}`).
    Cancelled,
}

/// One slide's proposed second-language layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TranslatedSlide {
    /// The slide.
    pub slide: SlideId,
    /// The proposed layer (marked `machine`): Keep sends it as the slide's
    /// `setSlideTranslation`.
    pub translation: SlideTranslation,
}

/// A translation preview (`POST /translate/preview` answers it; `GET /translate/jobs/{id}`
/// follows it). Kept ten minutes after it ends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TranslateJob {
    /// The job.
    pub id: String,
    /// The presentation.
    pub presentation: PresentationId,
    /// Source language.
    pub from: String,
    /// Target language.
    pub to: String,
    /// The language pair's model (`opus-mt-en-es`).
    pub model: String,
    /// Where it stands.
    pub status: TranslateJobStatus,
    /// Slides translated so far, of `total`.
    pub done: u32,
    /// Slides with text to translate.
    pub total: u32,
    /// Model bytes fetched (`downloading`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub downloaded: Option<u64>,
    /// The model's download size (`downloading`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub download_size: Option<u64>,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    /// The proposals (`done`), in slide order; slides without text are left out.
    #[serde(default)]
    pub slides: Vec<TranslatedSlide>,
    /// Seconds the translating took (`done`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub seconds: Option<f64>,
    /// The banner to show with the proposals.
    pub notice: String,
}
