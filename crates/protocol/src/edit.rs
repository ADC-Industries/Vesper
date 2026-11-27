//! Content editing: id-addressed edit operations on presentations and playlists, and the
//! bodies of the library / presentation / playlist CRUD routes.
//!
//! ## Edit operations
//! `POST /api/v1/presentations/{id}/edits` with a [`PresentationEdit`] (and
//! `POST /api/v1/playlists/{id}/edits` with a [`PlaylistEdit`]) applies a batch of operations to
//! the **current** revision of the document. Every operation names its targets by ID, so two
//! operators editing different slides never conflict. A batch is all or nothing: it fails
//! (`409 conflict`, with an [`EditConflict`] in `current`) only when a target no longer exists
//! or a per-operation precondition fails (`expectText`).
//!
//! The answer ([`PresentationEditResult`]) lists the operations as **applied** (high-level
//! operations such as `splitSlide` expanded into primitive ones, server-chosen IDs and default
//! slides filled in) and their **inverse**: the operations that undo the batch, in order. Clients
//! keep their own undo / redo stacks: undo = send `inverse`; the answer's `inverse` is the redo.
//!
//! Patch fields: an omitted field is left alone. For fields that are optional in the model
//! (`songInfo`, `color`, …) `null` clears the value.
//!
//! Positions: `after` names the sibling to insert after; omitted means "first".

use model::{
    ActionId, Arrangement, ArrangementId, BinBrowser, Build, CharStyle, Color, EdgeInsets, Element,
    ElementId, Feather, FitMode, Flip, GlobalGroup, GlobalGroupId, HotKey, Library, LibraryId,
    LineMask, Link, MediaBin, MediaBinId, MediaBinItem, MediaBinItemId, ParagraphStyle, PlaylistId,
    PlaylistItem, PlaylistItemId, PlaylistNode, Presentation, Rect, RichText, ScriptureRef,
    Section, SectionId, Shadow, Size, Slide, SlideAction, SlideId, SlideLabel, SlideTranslation,
    SongInfo, Stroke, TemplateSlideId, TextBox, TextOutline, Theme, ThemeId, Transition,
    VerticalAlign,
};
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

/// Deserializes a patch field where `null` means "clear": absent → `None`, `null` →
/// `Some(None)`, a value → `Some(Some(v))`. Use with `#[serde(default)]`.
pub fn double<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

/// A character range of a text box's plain text (paragraphs joined by `\n`, which counts as one
/// character). Offsets are Unicode scalar values (JavaScript: `Array.from(text)` indices), `end`
/// exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TextRange {
    /// First character.
    pub start: u32,
    /// One past the last character.
    pub end: u32,
}

/// Element fields `setElement` can change (omitted = unchanged).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ElementPatch {
    /// Layer-list name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Position and size, canvas px.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub bounds: Option<Rect>,
    /// Degrees clockwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rotation: Option<f64>,
    /// 0..=1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub opacity: Option<f64>,
    /// Mirroring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub flip: Option<Flip>,
    /// Not rendered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hidden: Option<bool>,
    /// Not selectable on the canvas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub locked: Option<bool>,
    /// Resizing keeps the aspect ratio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub aspect_locked: Option<bool>,
    /// Interior fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fill: Option<model::Fill>,
    /// Outline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stroke: Option<Stroke>,
    /// Drop shadow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub shadow: Option<Shadow>,
    /// Soft edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub feather: Option<Feather>,
    /// Scrolling text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scroll: Option<model::TextScroll>,
    /// The slot (role) a theme's template element fills: lyrics, title, reference, … (`null`
    /// clears: the element becomes decoration).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double"
    )]
    #[ts(optional)]
    pub placeholder: Option<Option<model::Placeholder>>,
    /// Visibility conditions (the whole list; `[]` = always visible).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub visibility: Option<Vec<model::VisibilityCondition>>,
    /// Live data link (`null` clears: the element shows its own text).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double"
    )]
    #[ts(optional)]
    pub data_link: Option<Option<model::DataLink>>,
}

/// One edit of a presentation. See the module docs for the batch semantics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PresentationOp {
    /// Presentation-level fields. Renaming a presentation is `setPresentation {name}`.
    SetPresentation {
        /// Name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        name: Option<String>,
        /// Song metadata (`null` clears).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        song_info: Option<Option<SongInfo>>,
        /// Scripture reference (`null` clears).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        scripture: Option<Option<ScriptureRef>>,
        /// Default transition (`null` = the layer default).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        transition: Option<Option<Transition>>,
        /// Canvas size (element geometry is not scaled).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        canvas_size: Option<Size>,
        /// Presentation notes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        notes: Option<RichText>,
        /// Category (`null` clears).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        category: Option<Option<String>>,
        /// The language its text is written in, a BCP 47 code (`"es"`, `"pt-BR"`; `null`
        /// clears). Stored in its usual case (`pt_br` → `pt-BR`); `400` when it is not a
        /// language code (`model::language_code`).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        language: Option<Option<String>>,
    },
    /// Inserts a section (with its slides) after `after` (omitted = first).
    AddSection {
        /// Sibling to insert after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        after: Option<SectionId>,
        /// The section. Its ID must be new.
        section: Section,
    },
    /// Deletes a section and its slides. Arrangements keep its ID (unknown IDs are skipped).
    DeleteSection {
        /// The section.
        section: SectionId,
    },
    /// Moves a section after `after` (omitted = first).
    MoveSection {
        /// The section.
        section: SectionId,
        /// Sibling to move after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        after: Option<SectionId>,
    },
    /// Section (group) fields.
    SetSection {
        /// The section.
        section: SectionId,
        /// Name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        name: Option<String>,
        /// Colour (`null` clears).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        color: Option<Option<Color>>,
        /// Hotkey (`null` clears).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        hotkey: Option<Option<HotKey>>,
        /// Global group link (`null` clears).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        global_group: Option<Option<Link<GlobalGroupId>>>,
    },
    /// Inserts a slide into `section` after `after` (omitted = first). Without `slide` the
    /// server makes one: a copy of the previous slide's styling with empty text (the slide
    /// before the insertion point in document order), else of the presentation's first text
    /// slide, else a default text slide. The applied op always carries the slide.
    AddSlide {
        /// The section.
        section: SectionId,
        /// Sibling to insert after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        after: Option<SlideId>,
        /// The slide (its ID must be new).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        slide: Option<Slide>,
    },
    /// Deletes a slide.
    DeleteSlide {
        /// The slide.
        slide: SlideId,
    },
    /// Moves a slide into `toSection` after `after` (omitted = first).
    MoveSlide {
        /// The slide.
        slide: SlideId,
        /// Destination section.
        to_section: SectionId,
        /// Sibling to move after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        after: Option<SlideId>,
    },
    /// Inserts a copy (fresh IDs) right after the slide. Applied as `addSlide`.
    DuplicateSlide {
        /// The slide.
        slide: SlideId,
    },
    /// Slide fields.
    SetSlide {
        /// The slide.
        slide: SlideId,
        /// Label (`null` clears).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        label: Option<Option<SlideLabel>>,
        /// Enabled.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        enabled: Option<bool>,
        /// Notes.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        notes: Option<RichText>,
        /// Transition (`null` = the presentation default).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        transition: Option<Option<Transition>>,
        /// Background colour (`null` = transparent).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        background: Option<Option<Color>>,
        /// Hotkey (`null` clears).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        hotkey: Option<Option<HotKey>>,
        /// Object builds, in the order they play (the whole list; `[]` removes them). Each
        /// must name an element of the slide.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        builds: Option<Vec<Build>>,
    },
    /// Replaces the plain text of a text element (Reflow). `text` has `\n` between paragraphs.
    ///
    /// - The element is `element`, else the slide's **primary text element**: the first text
    ///   element with a text placeholder slot, else the largest text box. A slide without one
    ///   gets a new text element in the presentation's prevailing style (applied as
    ///   `addElement` + `setSlideText`).
    /// - The box keeps its `defaultStyle` / `defaultParagraph`. Old and new lines are matched
    ///   by content: the unchanged lines at the start and at the end keep their paragraphs
    ///   (runs, style, chords). The changed lines between them take, in order, the style (and
    ///   `endStyle`) of the old paragraphs they replace (extra lines: the last replaced one's,
    ///   else a neighbour's) and become one run with the style of its first run.
    /// - Chords follow their characters across the changed lines: a split line keeps each
    ///   chord on the half holding its word; an inserted line has none.
    /// - `expectText`: the element's text the client last saw; a mismatch fails the batch with
    ///   `409` and the current text.
    SetSlideText {
        /// The slide.
        slide: SlideId,
        /// The text element (omitted = the primary text element).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        element: Option<ElementId>,
        /// New plain text.
        text: String,
        /// Precondition: the current text of the element.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        expect_text: Option<String>,
    },
    /// "Insert Slide Break": the primary text element's paragraphs from `atParagraph` on move
    /// to a new slide inserted right after, with the same styling and element geometry (a copy
    /// of the slide without label, hotkey, notes and actions). The text's second-language text
    /// splits with it by paragraph. Applied as `replaceTextBox` + `addSlide` (+
    /// `setSlideTranslation`).
    SplitSlide {
        /// The slide.
        slide: SlideId,
        /// First paragraph of the new slide.
        at_paragraph: u32,
    },
    /// Appends the paragraphs of `second`'s primary text element (and their second-language
    /// text) to `first`'s and deletes `second`. Applied as `replaceTextBox` (+
    /// `setSlideTranslation`) + `deleteSlide`.
    MergeSlides {
        /// The slide that stays.
        first: SlideId,
        /// The slide that goes.
        second: SlideId,
    },
    /// Inserts an element at z-index `index` (omitted = on top).
    AddElement {
        /// The slide.
        slide: SlideId,
        /// The element (its ID must be new on the slide).
        element: Element,
        /// Z position (0 = bottom).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        index: Option<u32>,
    },
    /// Deletes an element.
    DeleteElement {
        /// The slide.
        slide: SlideId,
        /// The element.
        element: ElementId,
    },
    /// Moves an element to z-index `index` (clamped).
    ReorderElement {
        /// The slide.
        slide: SlideId,
        /// The element.
        element: ElementId,
        /// New z position (0 = bottom).
        index: u32,
    },
    /// Element geometry and decoration.
    SetElement {
        /// The slide.
        slide: SlideId,
        /// The element.
        element: ElementId,
        /// Fields to change.
        patch: ElementPatch,
    },
    /// Character and paragraph style of a text element. Without `range`: the box's
    /// `defaultStyle` / `defaultParagraph`. With `range`: runs are split at its ends and the
    /// character fields set on the runs inside; paragraph fields go to every paragraph the range
    /// touches. Fields named in `clear` are unset at that level. Changing family, weight or
    /// italic without a PostScript name drops the level's PostScript name. Applied as
    /// `replaceTextBox` when a range is given.
    SetTextStyle {
        /// The slide.
        slide: SlideId,
        /// The element.
        element: ElementId,
        /// Characters to style (omitted = the whole box's defaults).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        range: Option<TextRange>,
        /// Character fields to set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        character: Option<CharStyle>,
        /// Paragraph fields to set.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        paragraph: Option<ParagraphStyle>,
        /// Field names to unset (`fontFamily`, `size`, `alignment`, …).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(as = "Option<Vec<String>>", optional)]
        clear: Vec<String>,
    },
    /// Box-level text layout.
    SetTextBox {
        /// The slide.
        slide: SlideId,
        /// The element.
        element: ElementId,
        /// Vertical placement.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        vertical_align: Option<VerticalAlign>,
        /// Fit mode.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        fit: Option<FitMode>,
        /// Inner padding.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        margins: Option<EdgeInsets>,
        /// Glyph outline.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        outline: Option<TextOutline>,
        /// Text shadow.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        text_shadow: Option<Shadow>,
        /// Line mask.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        line_mask: Option<LineMask>,
    },
    /// Replaces a text element's whole text box (the inverse of text edits; also paste).
    ReplaceTextBox {
        /// The slide.
        slide: SlideId,
        /// The element (must be a text element).
        element: ElementId,
        /// The new text box.
        text_box: TextBox,
    },
    /// Inserts an action into a slide's action list after `after` (omitted = first). Actions
    /// run in list order when the slide is triggered.
    AddSlideAction {
        /// The slide.
        slide: SlideId,
        /// Sibling to insert after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        after: Option<ActionId>,
        /// The action (its ID must be new on the slide).
        action: SlideAction,
    },
    /// Removes an action from a slide.
    RemoveSlideAction {
        /// The slide.
        slide: SlideId,
        /// The action.
        action: ActionId,
    },
    /// Moves an action after `after` (omitted = first).
    MoveSlideAction {
        /// The slide.
        slide: SlideId,
        /// The action.
        action: ActionId,
        /// Sibling to move after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        after: Option<ActionId>,
    },
    /// Replaces an action (matched by its ID) with `action`.
    SetSlideAction {
        /// The slide.
        slide: SlideId,
        /// The new action; its `id` names the action to replace.
        action: SlideAction,
    },
    /// Sets (or with `null` removes) a slide's second-language layer (`model::language`): the
    /// language code, each text element's second text, and the preferred layout. The whole
    /// layer is replaced; its inverse is another `setSlideTranslation`. At most 64 element
    /// texts; texts naming elements the slide doesn't have are kept and not drawn.
    SetSlideTranslation {
        /// The slide.
        slide: SlideId,
        /// The new layer (`null` removes it).
        #[serde(default)]
        translation: Option<SlideTranslation>,
    },
    /// Replaces a slide (found by its ID) with `slide`, in place. The primitive the theme ops
    /// are applied as; its inverse is another `replaceSlide`.
    ReplaceSlide {
        /// The new slide (its ID names the slide replaced).
        slide: Slide,
    },
    /// Makes slides follow a theme (a linked theme: they restyle when the theme changes, see
    /// `docs/THEMES-PLAN.md`). Each slide gets `template` when given, else the theme's
    /// template slide of the same name as the one it was made from, else the theme's first.
    /// Content is kept; the previous theme's elements go. Applied as `replaceSlide`.
    ApplyTheme {
        /// The theme.
        theme: ThemeId,
        /// The template slide for every slide.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        template: Option<TemplateSlideId>,
        /// The slides (omitted or empty = every slide).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(as = "Option<Vec<SlideId>>", optional)]
        slides: Vec<SlideId>,
    },
    /// "Reset to theme": clears overrides of a slide that follows its theme. Without
    /// `element`, the slide's own (background, transition, deleted theme elements) and, when
    /// `properties` is omitted too, every element's; with `element`, that element's.
    /// `properties` limits it to those names (`bounds`, `text.defaultStyle`, …). Applied as
    /// `replaceSlide`.
    ResetToTheme {
        /// The slide.
        slide: SlideId,
        /// One element.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        element: Option<ElementId>,
        /// Only these properties.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        properties: Option<Vec<String>>,
    },
    /// Stops slides following their theme; they keep their look as plain copies. Applied as
    /// `replaceSlide`.
    UnlinkTheme {
        /// The slides (omitted or empty = every slide).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(as = "Option<Vec<SlideId>>", optional)]
        slides: Vec<SlideId>,
    },
    /// Makes slides that don't follow a theme follow one **keeping their look** (imported
    /// songs): each is linked to the template slide that fits it best (its `templateOrigin`
    /// when that links, else the one with the most matching elements, then the fewest
    /// overrides; only `theme`'s when given), with every difference recorded as an override, so
    /// nothing on screen changes. Slides that can't be linked that way stay as they are.
    /// Applied as `replaceSlide`.
    LinkTheme {
        /// Only this theme.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        theme: Option<ThemeId>,
        /// The slides (omitted or empty = every slide).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[ts(as = "Option<Vec<SlideId>>", optional)]
        slides: Vec<SlideId>,
    },
    /// Replaces the arrangements (and, when given, the selected one: `null` = document order).
    SetArrangements {
        /// Every arrangement.
        arrangements: Vec<Arrangement>,
        /// Selected arrangement.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        selected: Option<Option<ArrangementId>>,
    },
    /// Puts the chords of a ChordPro text on the song's matching lyric lines (batch 16): every
    /// line of the slides' primary text whose words match a ChordPro line (ignoring case,
    /// punctuation and spacing) takes that line's chords (the first ChordPro line with those
    /// words wins); lines that match nothing keep theirs. The song's `originalKey` is set from
    /// `{key}` when it has none. Applied as `replaceTextBox` per changed element (+
    /// `setPresentation`); `400` when no line matches.
    ApplyChordPro {
        /// The ChordPro text.
        text: String,
    },
}

impl PresentationOp {
    /// The op's tag (`setSlideText`, …) for logs.
    pub fn name(&self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
            .unwrap_or_default()
    }
}

/// One edit of a playlist's items.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlaylistOp {
    /// Inserts an item after `after` (omitted = first). The item's ID must be new.
    AddItem {
        /// Sibling to insert after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        after: Option<PlaylistItemId>,
        /// The item: a presentation, a header `{name, color}`, media, …
        item: PlaylistItem,
    },
    /// Removes an item.
    RemoveItem {
        /// The item.
        item: PlaylistItemId,
    },
    /// Moves an item after `after` (omitted = first).
    MoveItem {
        /// The item.
        item: PlaylistItemId,
        /// Sibling to move after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        after: Option<PlaylistItemId>,
    },
    /// Item fields. For a header, `name` and `color` are the header's own text and colour.
    SetItem {
        /// The item.
        item: PlaylistItemId,
        /// Arrangement of a presentation item (`null` = the presentation's selected one).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        arrangement: Option<Option<ArrangementId>>,
        /// Enabled.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        enabled: Option<bool>,
        /// Display name override (`null` = the presentation / media name).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        name: Option<Option<String>>,
        /// Header colour (`null` clears; headers only).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        color: Option<Option<Color>>,
    },
}

/// `POST /api/v1/presentations/{id}/edits`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PresentationEdit {
    /// Applied in order, all or nothing.
    pub ops: Vec<PresentationOp>,
    /// The sending client (echoed in `documentChanged`, so it can ignore its own edits).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub client_id: Option<String>,
    /// The revision the client last saw (informational: ops apply to the current revision).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub base_revision: Option<String>,
}

/// Answer to a presentation edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PresentationEditResult {
    /// The document's new revision.
    pub revision: String,
    /// The ops as applied (expanded, IDs and defaults filled in).
    pub applied: Vec<PresentationOp>,
    /// Ops that undo the batch, in order.
    pub inverse: Vec<PresentationOp>,
}

/// `POST /api/v1/playlists/{id}/edits`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEdit {
    /// Applied in order, all or nothing.
    pub ops: Vec<PlaylistOp>,
    /// The sending client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub client_id: Option<String>,
    /// The revision the client last saw (informational).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub base_revision: Option<String>,
}

/// Answer to a playlist edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEditResult {
    /// The playlist's new revision.
    pub revision: String,
    /// The ops as applied.
    pub applied: Vec<PlaylistOp>,
    /// Ops that undo the batch.
    pub inverse: Vec<PlaylistOp>,
}

/// The ops carried by `documentChanged`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum EditOps {
    /// A presentation edit.
    Presentation(Vec<PresentationOp>),
    /// A playlist edit.
    Playlist(Vec<PlaylistOp>),
}

/// `current` of a `409` edit answer: why the batch failed and the document's revision.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EditConflict {
    /// Current revision (`null` when the document was deleted).
    pub revision: Option<String>,
    /// Index of the failing op in the batch.
    pub op_index: u32,
    /// For `expectText` mismatches: the slide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub slide: Option<SlideId>,
    /// For `expectText` mismatches: the element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub element: Option<ElementId>,
    /// For `expectText` mismatches: the element's text now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text: Option<String>,
}

// ---- CRUD bodies ----------------------------------------------------------------------------

/// `POST /api/v1/libraries`: a new library (`name`), or one restored from a deleted
/// `document` (undo).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateLibrary {
    /// Name (default "Library").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// A whole document to restore (keeps its ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub document: Option<Library>,
}

/// `PUT /api/v1/libraries/{id}` (`If-Match`): rename.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLibrary {
    /// New name.
    pub name: String,
}

/// `POST /api/v1/presentations`: a blank presentation (one section, one empty text slide,
/// 1920 × 1080) named `name` in `library`, or a restored `document` (undo of a delete).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreatePresentation {
    /// Name (default "Untitled").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Library.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub library: Option<LibraryId>,
    /// A whole document to restore (keeps its ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub document: Option<Presentation>,
}

/// `POST /api/v1/presentations/{id}/duplicate`: a copy with fresh IDs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DuplicatePresentation {
    /// Name of the copy (default "<name> copy").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Library of the copy (default: the original's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub library: Option<LibraryId>,
}

/// `POST /api/v1/themes`: a new theme (three starter template slides: Lyrics, Title,
/// Scripture), or `document` (an undo or a restore: kept as it is, with its ID).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateTheme {
    /// Name (default "Theme", numbered when taken).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// A whole theme to recreate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub document: Option<Theme>,
}

/// `PUT /api/v1/themes/{id}` (`If-Match`): rename, and (batch 16) set the second-language
/// style. Omitted fields are kept.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTheme {
    /// New name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// How second-language text looks on slides styled from the theme (numbers clamped:
    /// `sizeScale` 0.2–2, `gap` 0–400).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub second_language: Option<model::SecondLanguageStyle>,
}

/// `POST /api/v1/themes/{id}/duplicate`: a copy with fresh IDs (nothing follows it yet).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateTheme {
    /// Name of the copy (default "<name> copy").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
}

/// `POST /api/v1/presentations/{id}/move` (`If-Match`): to another library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MovePresentation {
    /// Destination library.
    pub library: LibraryId,
}

/// `POST /api/v1/playlists`: a new folder or playlist, or a restored `document`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreatePlaylistNode {
    /// Name (default "Playlist" / "Folder").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Folder or playlist (default playlist).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kind: Option<crate::PlaylistNodeType>,
    /// What a new playlist is for (default `service`; `audio` playlists appear in the audio
    /// panel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub purpose: Option<model::PlaylistPurpose>,
    /// Parent folder (omitted = top level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parent: Option<PlaylistId>,
    /// Sibling to place it after (omitted = last).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub after: Option<PlaylistId>,
    /// A whole document to restore (keeps its ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub document: Option<PlaylistNode>,
}

/// `PUT /api/v1/playlists/{id}` (`If-Match`): rename and / or move.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePlaylistNode {
    /// New name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// New parent folder (`null` = top level).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double"
    )]
    #[ts(optional)]
    pub parent: Option<Option<PlaylistId>>,
    /// Sibling to place it after among the (new) siblings (`null` = first).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double"
    )]
    #[ts(optional)]
    pub after: Option<Option<PlaylistId>>,
}

/// One edit of a manual media bin's items (`POST /api/v1/media-bins/{id}/edits`). Undo works
/// as for playlists: the answer's `inverse` undoes the batch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MediaBinOp {
    /// Inserts an item after `after` (omitted = first). The item's ID must be new.
    AddItem {
        /// Sibling to insert after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        after: Option<MediaBinItemId>,
        /// The item (its media and trigger settings).
        item: MediaBinItem,
    },
    /// Removes an item (its media stays in the collection).
    RemoveItem {
        /// The item.
        item: MediaBinItemId,
    },
    /// Moves an item after `after` (omitted = first).
    MoveItem {
        /// The item.
        item: MediaBinItemId,
        /// Sibling to move after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        after: Option<MediaBinItemId>,
    },
    /// Item fields.
    SetItem {
        /// The item.
        item: MediaBinItemId,
        /// Enabled.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        enabled: Option<bool>,
        /// Display name (`null` = the media's name).
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "double"
        )]
        #[ts(optional)]
        name: Option<Option<String>>,
    },
}

/// `POST /api/v1/media-bins/{id}/edits`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaBinEdit {
    /// Applied in order, all or nothing.
    pub ops: Vec<MediaBinOp>,
    /// The sending client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub client_id: Option<String>,
}

/// Answer to a media bin edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MediaBinEditResult {
    /// The bin's new revision.
    pub revision: String,
    /// The ops as applied.
    pub applied: Vec<MediaBinOp>,
    /// Ops that undo the batch.
    pub inverse: Vec<MediaBinOp>,
}

/// `POST /api/v1/media-bins`: a new empty manual bin, or a restored `document`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateMediaBin {
    /// Name (default "Bin").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Folder path, outermost first (omitted = top level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub folder: Option<Vec<String>>,
    /// Media or audio (default media).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub browser: Option<BinBrowser>,
    /// Bin of the same folder to place it after (omitted = last).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub after: Option<MediaBinId>,
    /// A whole document to restore (keeps its ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub document: Option<MediaBin>,
}

/// `PUT /api/v1/media-bins/{id}` (`If-Match`): rename, move to a folder, reorder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMediaBin {
    /// New name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// New folder path (`[]` = top level).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub folder: Option<Vec<String>>,
    /// Bin of the (new) folder to place it after (`null` = first).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double"
    )]
    #[ts(optional)]
    pub after: Option<Option<MediaBinId>>,
}

/// `POST /api/v1/groups`: a new global group, or a restored `document`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateGlobalGroup {
    /// Name (default "Group").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color: Option<Color>,
    /// Hotkey.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub hotkey: Option<HotKey>,
    /// Group to place it after (omitted = last).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub after: Option<GlobalGroupId>,
    /// A whole document to restore (keeps its ID).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub document: Option<GlobalGroup>,
}

/// `PUT /api/v1/groups/{id}` (`If-Match`): rename, recolour, hotkey, reorder. Sections linked
/// to the group keep their own name and colour (their snapshot).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGlobalGroup {
    /// New name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// Colour (`null` clears).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double"
    )]
    #[ts(optional)]
    pub color: Option<Option<Color>>,
    /// Hotkey (`null` clears).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double"
    )]
    #[ts(optional)]
    pub hotkey: Option<Option<HotKey>>,
    /// Group to place it after (`null` = first).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double"
    )]
    #[ts(optional)]
    pub after: Option<Option<GlobalGroupId>>,
}

/// `POST /api/v1/playlists/{id}/duplicate`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DuplicatePlaylist {
    /// Name of the copy (default "<name> copy").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
}

/// Answer of the create / update / duplicate / move routes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DocumentRevision {
    /// Document kind.
    pub kind: model::DocumentKind,
    /// Document ID.
    pub id: String,
    /// New revision (also the `ETag`).
    pub revision: String,
}

/// Answer of the delete routes: the deleted documents as they were, so a client can undo by
/// sending each back as `{"document": …}` to the create route.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DeletedDocuments {
    /// Document kind.
    pub kind: model::DocumentKind,
    /// The documents (a folder deletes its subtree: parents first).
    #[ts(type = "Array<unknown>")]
    pub documents: Vec<serde_json::Value>,
}

/// `GET /api/v1/groups`: the global groups (name, colour, hotkey) the group pickers offer.
pub type GlobalGroups = Vec<model::GlobalGroup>;
