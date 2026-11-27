//! PowerPoint import (`/api/v1/codex/imports/pptx`): a `.pptx` (Keynote and Google Slides
//! exported as `.pptx`) becomes a presentation in a Codex collection, editable where
//! possible, with a picture of the slide where not. The import runs as a job on the Codex
//! box; its report says, slide by slide, what is editable and what was pictured.

use model::{CollectionId, LibraryId, PresentationId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where an import job stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PptxImportState {
    /// Waiting for another import to finish.
    #[default]
    Queued,
    /// Reading the slides (`done` of `total`).
    Reading,
    /// Drawing slide pictures for what isn't editable (`total` slides).
    Pictures,
    /// Taking the pictures and media into the Codex (`done` of `total` files).
    Media,
    /// Saving the presentation.
    Saving,
    /// Finished: `report` says what came in.
    Done,
    /// Stopped: `error` says why; nothing was saved (media already taken in stay in the
    /// media library, unused).
    Failed,
}

/// `GET /api/v1/codex/imports/pptx/{id}`: an import job.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PptxImportJob {
    /// The job's ID.
    pub id: String,
    /// The collection it imports into.
    pub collection: CollectionId,
    /// The uploaded file's name.
    pub file_name: String,
    /// Where it stands.
    pub state: PptxImportState,
    /// Progress within the state: this many…
    pub done: u32,
    /// …of this many.
    pub total: u32,
    /// Why it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
    /// What it made, when done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub report: Option<PptxImportReport>,
    /// When it started (RFC 3339).
    pub created_at: String,
}

/// Where the slide pictures came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum PptxPictureSource {
    /// Drawn by LibreOffice on the server.
    LibreOffice,
    /// The file's own thumbnail (its first slide only, small).
    Thumbnail,
}

/// Something that didn't come in as editable content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PptxImportNote {
    /// What it is (`chart`, `table`, `smartArt`, `line`, `a “cloud” shape`, `video` …).
    pub kind: String,
    /// The object's name in the file.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(optional, as = "Option<String>")]
    pub name: String,
    /// What became of it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(optional, as = "Option<String>")]
    pub detail: String,
}

/// One slide of the report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PptxSlideReport {
    /// 1-based.
    pub number: u32,
    /// Its title text, if it has a title.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    #[ts(optional, as = "Option<String>")]
    pub title: String,
    /// Editable elements on it.
    pub editable: u32,
    /// Objects drawn from the slide's picture.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<Vec<PptxImportNote>>")]
    pub pictured: Vec<PptxImportNote>,
    /// Objects approximated (a shape as a rectangle) or left out: no picture was available.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<Vec<PptxImportNote>>")]
    pub approximated: Vec<PptxImportNote>,
    /// Other notes (video not imported, vertical text shown horizontally …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<Vec<PptxImportNote>>")]
    pub notes: Vec<PptxImportNote>,
    /// Where its picture came from, when it uses one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub picture: Option<PptxPictureSource>,
    /// Hidden in the file: imported disabled.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(optional, as = "Option<bool>")]
    pub hidden: bool,
}

/// What an import made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PptxImportReport {
    /// The presentation made.
    pub presentation: PresentationId,
    /// Its name.
    pub name: String,
    /// The library it is in.
    pub library: LibraryId,
    /// The canvas width, px (1920).
    pub width: f64,
    /// The canvas height, px (following the slide size).
    pub height: f64,
    /// Slide by slide.
    pub slides: Vec<PptxSlideReport>,
    /// Slides with nothing pictured, approximated or left out.
    pub editable_slides: u32,
    /// Slides that use a picture of themselves for part of their content.
    pub pictured_slides: u32,
    /// Media files taken into the Codex (pictures of the file, slide pictures).
    pub media: u32,
    /// The picture program used (LibreOffice's `soffice`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub renderer: Option<String>,
    /// Why slide pictures couldn't be made, when some were needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub renderer_problem: Option<String>,
}
