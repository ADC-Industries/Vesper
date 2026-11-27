//! Libraries, presentations, sections (groups), arrangements and global groups.
//!
//! ## Shape
//! ```text
//! Presentation              canvasSize: the size of every slide in it
//!  ├─ sections[]            document order = default play order
//!  │   └─ slides[]          each slide belongs to exactly one section
//!  └─ arrangements[]        alternative play orders: section IDs, repeats allowed
//! ```
//! Sections *own* their slides: in the reference library cue groups partition the cues exactly
//! (405/405 files), so ownership loses nothing and makes dangling slide references impossible.
//! A cue outside every group is wrapped in an unnamed section by the importer; a cue in two
//! groups would be duplicated with a new ID (original in `import.originalId`).
//!
//! Repeated song parts are often duplicated sections with the same name (two "Tag" sections).
//! They are preserved as they are (no auto-collapse into arrangements); names are not keys.

use crate::common::{
    Color, HotKey, ImportInfo, IntRange, Link, Size, SortKey, Timestamp, is_default, is_false,
};
use crate::effects::Transition;
use crate::ids::{ArrangementId, GlobalGroupId, LibraryId, PresentationId, SectionId};
use crate::slide::{Slide, TemplateRef};
use crate::text::RichText;
use serde::{Deserialize, Serialize};

/// A library: a named shelf of presentations in the sidebar (stored document). Membership is
/// stored on the presentation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Library {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Library ID.
    pub id: LibraryId,
    /// Name ("Songs", "Readings").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Sidebar order.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Template applied to new presentations created in this library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub default_template: Option<TemplateRef>,
    /// Import provenance (original folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl Default for Library {
    fn default() -> Self {
        Library {
            schema_version: crate::SCHEMA_VERSION,
            id: LibraryId::new(),
            name: String::new(),
            sort_key: SortKey::new(),
            default_template: None,
            import: None,
        }
    }
}

/// A presentation (stored document, including its sections and slides).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Presentation {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Presentation ID.
    pub id: PresentationId,
    /// Name. Not unique: the same name exists in two libraries in the reference data.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Library this presentation lives in (`None` = none, e.g. created inside a playlist).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub library: Option<LibraryId>,
    /// Order inside the library (empty = by name).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
    /// Free-form category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub category: Option<String>,
    /// The language its text is written in: a BCP 47 code (`"en"`, `"es"`, `"pt-BR"`, checked
    /// with [`crate::language_code`]); `None` = not given. Filled by imports that know it
    /// (ChordPro `{meta: language …}`, a Scripture translation's language, a PowerPoint deck's
    /// language). The Translate dialog's default source. A slide's second language is its
    /// own ([`crate::SlideTranslation::language`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub language: Option<String>,
    /// Song metadata (CCLI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub song_info: Option<SongInfo>,
    /// Scripture passage shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub scripture: Option<ScriptureRef>,
    /// Canvas size of every slide in this presentation, px.
    #[serde(default, skip_serializing_if = "is_default")]
    pub canvas_size: Size,
    /// Sections in document order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<Section>,
    /// Alternative section orders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arrangements: Vec<Arrangement>,
    /// Arrangement used by default (`None` = document order). Must name an entry of
    /// `arrangements`; the importer drops dangling selections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub selected_arrangement: Option<ArrangementId>,
    /// Timed auto-advance (announcement loops).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub auto_advance: Option<AutoAdvance>,
    /// Default transition between slides (`None` = the layer default in show settings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub transition: Option<Transition>,
    /// Which layer the slides go to.
    #[serde(default, skip_serializing_if = "is_default")]
    pub destination: ContentDestination,
    /// Presentation-wide notes.
    #[serde(default, skip_serializing_if = "is_default")]
    pub notes: RichText,
    /// Template used for new slides and "re-apply theme" (provenance only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub template_origin: Option<TemplateRef>,
    /// Creation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub created_at: Option<Timestamp>,
    /// Last modification time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub modified_at: Option<Timestamp>,
    /// Last time it went live ("recently used" sort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub last_used_at: Option<Timestamp>,
    /// Import provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl Default for Presentation {
    fn default() -> Self {
        Presentation {
            schema_version: crate::SCHEMA_VERSION,
            id: PresentationId::new(),
            name: String::new(),
            library: None,
            sort_key: SortKey::new(),
            category: None,
            language: None,
            song_info: None,
            scripture: None,
            canvas_size: Size::HD,
            sections: Vec::new(),
            arrangements: Vec::new(),
            selected_arrangement: None,
            auto_advance: None,
            transition: None,
            destination: ContentDestination::default(),
            notes: RichText::default(),
            template_origin: None,
            created_at: None,
            modified_at: None,
            last_used_at: None,
            import: None,
        }
    }
}

impl Presentation {
    /// An empty presentation (no sections).
    pub fn new(name: impl Into<String>) -> Self {
        Presentation {
            name: name.into(),
            ..Presentation::default()
        }
    }

    /// What "New Presentation" creates: one section holding one empty slide.
    pub fn new_blank(name: impl Into<String>) -> Self {
        let mut p = Presentation::new(name);
        let mut s = Section::new("Group");
        s.slides.push(Slide::new());
        p.sections.push(s);
        p
    }

    /// All slides in document order.
    pub fn slides(&self) -> impl Iterator<Item = &Slide> {
        self.sections.iter().flat_map(|s| s.slides.iter())
    }

    /// Section by ID.
    pub fn section(&self, id: SectionId) -> Option<&Section> {
        self.sections.iter().find(|s| s.id == id)
    }

    /// Slide by ID, with the ID of its section.
    pub fn slide(&self, id: crate::ids::SlideId) -> Option<(SectionId, &Slide)> {
        self.sections
            .iter()
            .find_map(|sec| sec.slides.iter().find(|s| s.id == id).map(|s| (sec.id, s)))
    }

    /// Slides in play order for an arrangement (`None` = the selected arrangement, else document
    /// order). Repeated sections yield their slides again; disabled slides are included (show
    /// logic skips them); unknown IDs are ignored.
    pub fn play_order(&self, arrangement: Option<ArrangementId>) -> Vec<(SectionId, &Slide)> {
        let arr = arrangement
            .or(self.selected_arrangement)
            .and_then(|id| self.arrangements.iter().find(|a| a.id == id));
        let order: Vec<SectionId> = match arr {
            Some(a) => a.sections.clone(),
            None => self.sections.iter().map(|s| s.id).collect(),
        };
        order
            .into_iter()
            .filter_map(|id| self.section(id))
            .flat_map(|sec| sec.slides.iter().map(move |s| (sec.id, s)))
            .collect()
    }
}

/// What a presentation is, for lists (dots, filters): read off the document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum PresentationKind {
    /// Song metadata (CCLI) or arrangements.
    Song,
    /// A scripture passage.
    Scripture,
    /// Anything else with text.
    #[default]
    Text,
    /// Slides with media and no text (walk-in loops, picture slides).
    Media,
}

impl Presentation {
    /// The presentation's kind: scripture when it holds a passage, a song when it has song
    /// metadata or arrangements, media when no slide has text but some slide shows media (a
    /// media action or a media fill), else text.
    pub fn kind(&self) -> PresentationKind {
        if self.scripture.is_some() {
            return PresentationKind::Scripture;
        }
        if self.song_info.is_some() || !self.arrangements.is_empty() {
            return PresentationKind::Song;
        }
        let mut media = false;
        for s in self.slides() {
            if !s.plain_text().trim().is_empty() {
                return PresentationKind::Text;
            }
            media = media
                || s.actions
                    .iter()
                    .any(|a| matches!(a.kind, crate::action::ActionKind::Media(_)))
                || s.elements.iter().any(|e| {
                    e.fill.enabled && matches!(e.fill.kind, crate::element::FillKind::Media { .. })
                });
        }
        if media {
            PresentationKind::Media
        } else {
            PresentationKind::Text
        }
    }
}

/// Where a presentation's slides are shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ContentDestination {
    /// The normal slide layer.
    #[default]
    Slide,
    /// The announcements layer (runs independently of the slide layer).
    Announcements,
    /// Unknown value (slide).
    #[serde(other)]
    Unknown,
}

impl ContentDestination {
    /// The compositor layer.
    pub fn layer(self) -> crate::show::Layer {
        match self {
            ContentDestination::Announcements => crate::show::Layer::Announcements,
            _ => crate::show::Layer::Slide,
        }
    }
}

/// Timed auto-advance through a presentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct AutoAdvance {
    /// Seconds each slide stays up (a slide's own `autoAdvanceAfter` wins).
    #[serde(skip_serializing_if = "crate::common::is_zero")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub seconds: f64,
    /// After the last slide, go back to the first.
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub loop_to_start: bool,
}

/// Song metadata for CCLI reporting and the copyright line. Strings are kept verbatim as
/// entered (no splitting of author lists, CCLI number as text).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct SongInfo {
    /// Song title as licensed (may differ from the presentation name).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub title: Option<String>,
    /// CCLI song number, verbatim text.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub ccli_number: Option<String>,
    /// Author / writer credit, verbatim ("A, B and C").
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub authors: Option<String>,
    /// Performing artist.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub artist: Option<String>,
    /// Publisher / copyright holder, verbatim.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub publisher: Option<String>,
    /// Copyright year.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub copyright_year: Option<u32>,
    /// Album.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub album: Option<String>,
    /// Show the copyright line for this song (subject to the show's CCLI settings).
    #[serde(skip_serializing_if = "is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub display: bool,
    /// Original key ("G", "Bb"): the key the chords are written in.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub original_key: Option<String>,
    /// Semitones the song is played away from its written chords (the song's key control;
    /// stage chord charts and the key line follow it). See [`crate::chords`].
    #[serde(skip_serializing_if = "crate::common::is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub transpose: i32,
    /// Artwork (an image asset), shown by elements with a
    /// [`DataLink::SongArtwork`](crate::element::DataLink::SongArtwork) link.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub artwork: Option<crate::ids::MediaAssetId>,
}

/// A scripture passage reference.
///
/// The visible `label` is authoritative. Imported `translation` values are unreliable
/// (ProPresenter wrote "KJV" on 15 decks labelled "(NKJV)") and are only a hint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ScriptureRef {
    /// Book.
    pub book: BibleBook,
    /// Chapter range.
    pub chapters: IntRange,
    /// Verse range (`None` = whole chapters). For a cross-chapter range the start verse belongs
    /// to `chapters.start` and the end verse to `chapters.end`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub verses: Option<IntRange>,
    /// Display label as the audience sees it ("Psalm 23:1-6 (KJV)").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub label: Option<String>,
    /// Translation hint (may be wrong on imports; prefer the label).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub translation: Option<String>,
    /// How a passage made on the Bible page was generated, so the server can split it again
    /// when its theme changes. Absent on imports and older passages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub generated: Option<ScriptureGenerated>,
}

/// The request a generated passage was made from, and a digest of the slides it got.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ScriptureGenerated {
    /// Translation ID.
    pub translation: String,
    /// The reference as typed ("Psalm 23:1-4, 6").
    pub reference: String,
    /// The theme its slides were styled with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub theme: Option<crate::ids::ThemeId>,
    /// The template slide asked for (omitted: the theme's "Scripture", else its first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub template: Option<crate::ids::TemplateSlideId>,
    /// At most this many verses on a slide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub verses_per_slide: Option<u32>,
    /// At most this many characters on a slide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub max_chars: Option<u32>,
    /// Verse numbers.
    #[serde(default, skip_serializing_if = "crate::common::is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub verse_numbers: bool,
    /// Where the reference goes.
    #[serde(default, skip_serializing_if = "crate::common::is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub reference_on: ReferencePlacement,
    /// The second translation of a parallel passage (batch 16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub second_translation: Option<String>,
    /// Its preferred layout (omitted = side by side).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub second_layout: Option<crate::language::LanguageDisplay>,
    /// Digest of the sections as generated: when they differ, the operator edited the passage
    /// and a theme change doesn't split it again on its own.
    pub slides: String,
    /// Its theme changed after the operator edited it: the passage wasn't split again (Oriel
    /// offers to).
    #[serde(default, skip_serializing_if = "crate::common::is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub stale: bool,
}

/// Where a generated passage shows its reference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ReferencePlacement {
    /// On every slide.
    #[default]
    Each,
    /// On the last slide only.
    Last,
    /// Nowhere (the attribution line, when required, still goes on the last slide).
    None,
}

/// A Bible book.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct BibleBook {
    /// Display name ("Psalms").
    #[serde(skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub name: String,
    /// Canonical key (USFM code like `"MAT"`), if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub key: Option<String>,
    /// 1-based canonical index (Genesis = 1), if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub index: Option<u32>,
}

/// A section of a presentation (ProPresenter "group": Verse 1, Chorus).
///
/// `name`, `color` and `hotkey` are the section's own snapshot. `globalGroup` links to a
/// [`GlobalGroup`] (with the source's group name as hint, since many links dangle); the server
/// may propagate edits of the definition to linked sections.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Section {
    /// Section ID.
    pub id: SectionId,
    /// Name (may be empty or a default like "Group").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Colour in the slide grid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub color: Option<Color>,
    /// Shortcut that jumps to this section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub hotkey: Option<HotKey>,
    /// Link to a global group (may dangle).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub global_group: Option<Link<GlobalGroupId>>,
    /// Slides in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slides: Vec<Slide>,
    /// Import provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub import: Option<ImportInfo>,
}

impl Default for Section {
    fn default() -> Self {
        Section::new("")
    }
}

impl Section {
    /// An empty, uncoloured section.
    pub fn new(name: impl Into<String>) -> Self {
        Section {
            id: SectionId::new(),
            name: name.into(),
            color: None,
            hotkey: None,
            global_group: None,
            slides: Vec::new(),
            import: None,
        }
    }

    /// A section snapshotting a global group's name, colour and hotkey.
    pub fn from_global(group: &GlobalGroup) -> Self {
        Section {
            color: group.color,
            hotkey: group.hotkey.clone(),
            global_group: Some(Link::named(group.id, group.name.clone())),
            ..Section::new(group.name.clone())
        }
    }
}

/// A named play order of sections; sections may repeat ("V1 C V2 C B C C").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct Arrangement {
    /// Arrangement ID.
    pub id: ArrangementId,
    /// Name ("Sunday", "Short").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Section IDs in play order, repeats allowed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<SectionId>,
}

/// A global group definition, e.g. "Verse 1" = blue, hotkey A (stored document).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct GlobalGroup {
    /// Schema version.
    #[serde(default = "crate::migrate::schema_v1")]
    pub schema_version: u32,
    /// Group ID (imported `applicationGroupIdentifier`).
    pub id: GlobalGroupId,
    /// Name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub color: Option<Color>,
    /// Hotkey (several groups may share one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub hotkey: Option<HotKey>,
    /// Display order.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sort_key: SortKey,
}

impl Default for GlobalGroup {
    fn default() -> Self {
        GlobalGroup {
            schema_version: crate::SCHEMA_VERSION,
            id: GlobalGroupId::new(),
            name: String::new(),
            color: None,
            hotkey: None,
            sort_key: SortKey::new(),
        }
    }
}
