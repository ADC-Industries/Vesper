//! Scripture: bundled and local Bible translations, references, passages, search and the
//! presentations generated from a passage (`/api/v1/scripture/...`).

use model::{PlaylistId, PlaylistItemId, PresentationId, TemplateSlideId, ThemeId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// `GET /api/v1/scripture/translations`: a translation this server has (no verse text).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureTranslation {
    /// ID used in requests (`KJV`).
    pub id: String,
    /// Abbreviation shown after references (`KJV`).
    pub abbreviation: String,
    /// Full name ("King James Version").
    pub name: String,
    /// Language (BCP 47, `en`).
    pub language: String,
    /// Licence ("Public domain").
    pub license: String,
    /// The line generated slides must carry (a publisher's required notice), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub attribution: Option<String>,
    /// Where it comes from: `bundled` (installed with the server) or `local` (the data
    /// folder's `scripture/`).
    pub origin: ScriptureOrigin,
    /// Number of books.
    pub books: u32,
}

/// Where a translation comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ScriptureOrigin {
    /// Installed with the server (read-only).
    #[default]
    Bundled,
    /// The data folder's `scripture/` (the church's own files).
    Local,
}

/// `GET /api/v1/scripture/translations/{id}/books`: a book of a translation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureBook {
    /// USFM key (`JHN`).
    pub key: String,
    /// Name ("John").
    pub name: String,
    /// Old or New Testament (other books: `other`).
    pub testament: Testament,
    /// Verse count of each chapter (chapter 1 first).
    pub chapters: Vec<u32>,
}

/// A book's testament.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum Testament {
    /// Old Testament.
    Old,
    /// New Testament.
    New,
    /// Anything else (deuterocanonical books).
    Other,
}

/// One range of a reference: from `chapter:verse` to `endChapter:endVerse`, inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct VerseRange {
    /// First chapter.
    pub chapter: u32,
    /// First verse.
    pub verse: u32,
    /// Last chapter.
    pub end_chapter: u32,
    /// Last verse.
    pub end_verse: u32,
}

/// `GET /api/v1/scripture/parse?translation=&q=`: what a typed reference means.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureParse {
    /// `true` when `q` is a reference this translation has.
    pub ok: bool,
    /// Book key (`JHN`) when a book was recognised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub book: Option<String>,
    /// Book name ("John") when a book was recognised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub book_name: Option<String>,
    /// The ranges, checked against the translation (empty unless `ok`).
    pub ranges: Vec<VerseRange>,
    /// Normalised label ("John 3:16–18") when `ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub label: Option<String>,
    /// Why it isn't a reference ("John has 21 chapters"), when not `ok`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<String>,
}

/// A verse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureVerse {
    /// Book key.
    pub book: String,
    /// Chapter.
    pub chapter: u32,
    /// Verse.
    pub verse: u32,
    /// Text.
    pub text: String,
}

/// `GET /api/v1/scripture/passage?translation=&ref=`: the verses of a reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScripturePassage {
    /// Translation ID.
    pub translation: String,
    /// Book key.
    pub book: String,
    /// Normalised label without the translation ("Psalm 23:1–4, 6").
    pub label: String,
    /// The ranges.
    pub ranges: Vec<VerseRange>,
    /// The translation's attribution line, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub attribution: Option<String>,
    /// The verses in order.
    pub verses: Vec<ScriptureVerse>,
}

/// `GET /api/v1/scripture/search?translation=&q=&limit=`: verses holding every word of `q`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureSearch {
    /// Translation ID.
    pub translation: String,
    /// The hits in canonical order (at most `limit`, default 50, at most 500).
    pub hits: Vec<ScriptureVerse>,
    /// How many verses match in all.
    pub total: u32,
}

/// Where a generated passage shows its reference (a model type since batch 9: generated
/// passages record it).
pub use model::ReferencePlacement;

/// `POST /api/v1/scripture/slides`: how to turn a passage into slides.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureSlideRequest {
    /// Translation ID.
    pub translation: String,
    /// The reference ("Psalm 23:1-4, 6").
    pub reference: String,
    /// Theme to style the slides with (read-only; omitted = the built-in plain layout).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub theme: Option<ThemeId>,
    /// Template slide of the theme (omitted = the one named "Scripture", else the first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub template: Option<TemplateSlideId>,
    /// At most this many verses on a slide (omitted = as many as fit). Fewer when they don't
    /// fit the text box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub verses_per_slide: Option<u32>,
    /// At most this many characters on a slide (60–2000), on top of what fits (omitted = only
    /// what fits). Slides are split by laying their text out in the theme's text box (see
    /// the protocol README).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub max_chars: Option<u32>,
    /// Show verse numbers (superscript).
    #[serde(default)]
    pub verse_numbers: bool,
    /// Where the reference goes.
    #[serde(default)]
    pub reference_on: ReferencePlacement,
    /// A second translation's ID (parallel Bible, batch 16): each slide also carries the same
    /// verses in it as its second-language text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub second_translation: Option<String>,
    /// How the parallel slides prefer to be shown where a screen's look says `auto` (omitted =
    /// `sideBySide`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub second_layout: Option<model::LanguageDisplay>,
}

/// A generated slide's text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureSlideText {
    /// The verse text (verse numbers inline as "16 ", when asked for).
    pub text: String,
    /// The same verses in the second translation, when one was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub second: Option<String>,
    /// The reference line on this slide ("John 3:16–17 KJV"), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reference: Option<String>,
    /// The attribution line on this slide, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub attribution: Option<String>,
}

/// `POST /api/v1/scripture/slides` → the split, without writing anything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureSlides {
    /// The presentation's name ("John 3:16–18 (KJV)").
    pub name: String,
    /// The slides.
    pub slides: Vec<ScriptureSlideText>,
}

/// What `POST /api/v1/scripture/presentations` does after creating the presentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ScriptureThen {
    /// Nothing more.
    #[default]
    None,
    /// Append it to `playlist`.
    Playlist,
    /// Show its first slide now (no playlist).
    Show,
}

/// `POST /api/v1/scripture/presentations`: a presentation from a passage, in the active
/// collection's **Scripture** library (created when missing).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureCreate {
    /// The passage and slide options.
    #[serde(flatten)]
    pub slides: ScriptureSlideRequest,
    /// What to do next.
    #[serde(default)]
    pub then: ScriptureThen,
    /// The playlist for `then: playlist`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub playlist: Option<PlaylistId>,
    /// Name (default: the label, "John 3:16–18 (KJV)").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
}

/// `POST /api/v1/scripture/presentations/{id}/resplit`: split a generated passage again with
/// its theme as it is now.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureResplit {
    /// Also when the operator edited the passage (the edits are replaced; without it such a
    /// passage answers `409`).
    #[serde(default)]
    pub force: bool,
}

/// The answer of `POST /api/v1/scripture/presentations/{id}/resplit`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureResplitResult {
    /// The passage.
    pub presentation: PresentationId,
    /// Its slide count now.
    pub slide_count: u32,
    /// False when the split came out the same (nothing written).
    pub changed: bool,
}

/// `201` from `POST /api/v1/scripture/presentations`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScriptureCreated {
    /// The new presentation.
    pub presentation: PresentationId,
    /// Its name.
    pub name: String,
    /// Its slide count.
    pub slide_count: u32,
    /// The playlist item, for `then: playlist`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub playlist_item: Option<PlaylistItemId>,
}
