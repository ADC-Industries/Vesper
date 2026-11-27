//! **Area: content.** `.pro` presentations → [`Presentation`] documents.
//!
//! ## Owns
//! - [`import_presentation`]: converts, de-duplicates the ID (a second file with an already used
//!   presentation UUID gets `ids::derived("presentation", [rel_path])` and keeps the source UUID
//!   in `import.originalId`, with a warning), pushes to `ctx.out.presentations` and registers
//!   `ctx.lookups.presentations[rel_path]` (with basename and arrangement IDs) for playlists.
//! - [`presentation`]: `name` from the document (not the file name); `library`;
//!   `import = ctx.document_info()` (documentPath = rel path, writer version); `canvasSize` from
//!   the slides ([`canvas_size`]); sections ([`sections`]); arrangements that resolve
//!   ([`arrangements`]); `selectedArrangement` only when it resolves, else `None` +
//!   `report.dangling_arrangements.selected_arrangement += 1`; song info ([`song_info`]),
//!   scripture ([`scripture`]), `autoAdvance` from `slideShowDuration` ([`auto_advance`]),
//!   `transition` (`common::transition`), `destination` from `contentDestination`, `category`,
//!   notes (`notes` string → `RichText::plain`), `lastDateUsed` / `lastModifiedDate`
//!   (`common::timestamp`). Unused / unmodelled fields (timeline when non-default, music key,
//!   chord chart, multiTracks, background when enabled) → `import.unmapped`.
//! - [`sections`]: cue groups in document order, each owning its cues (`slide::slide_from_cue`)
//!   in group order. Cues in no group → an extra section "Ungrouped", a
//!   `report.ungrouped_cues` entry and a warning. A cue in two groups is duplicated with a derived ID (original in
//!   `import.originalId`). Section ID = cue-group UUID; name / colour / hotkey snapshot from the
//!   group ([`section_from_group`]).
//! - [`global_group_link`]: `Link { id: applicationGroupIdentifier, name: applicationGroupName }`,
//!   also when dangling; dangling ones are counted with `report.dangling_group_link(name)`.
//! - [`song_info`]: CCLI block, verbatim strings, `songNumber` as text (0 → none); a title that
//!   differs from the presentation name → `report.ccli_title_mismatch`.
//!
//! ## Details
//! - **ID uniqueness.** Section, slide, element and action IDs must be unique inside a
//!   presentation. A repeated cue-group UUID gets a derived section ID; a cue referenced twice
//!   is converted once and the copy is re-keyed (slide, element and action IDs derived from the
//!   new section, element references in visibility conditions remapped). A presentation whose
//!   UUID is already taken is re-keyed the same way below its derived ID, so no two documents
//!   share nested IDs.
//! - **Scripture label.** ProPresenter stores no label beside the `bibleReference`; the label is
//!   the reference formatted as `"Book C:V-V"`. When the presentation name is that reference
//!   with `:` written as `_` plus a suffix (file-name safe form, e.g. `"John 3_16 (NKJV)"`), the
//!   name with `_` → `:` is the label, so the visible translation tag survives (the stored
//!   translation is unreliable and only kept as a hint). Book index: ProPresenter counts from
//!   0, the model from 1.
//! - **Auto-advance.** `slideShowDuration > 0` → `AutoAdvance { seconds, loopToStart: true }`
//!   (ProPresenter slide shows loop; the source has no separate flag).
//!
//! ## May call
//! `common::*`, `slide::slide_from_cue`, `text::rich_text_from_rtf`, `ImportContext` helpers.

use std::collections::{BTreeMap, BTreeSet};

use model::{
    ActionId, Arrangement, ArrangementId, AutoAdvance, BibleBook, ContentDestination, ElementId,
    GlobalGroupId, ImportInfo, IntRange, LibraryId, Link, Presentation, PresentationId, RichText,
    ScriptureRef, Section, SectionId, Size, Slide, SlideId, SongInfo, VisibilityCondition,
};
use serde_json::Value;
use uuid::Uuid;

use crate::context::{ImportContext, PresentationEntry, ids};
use crate::convert::{common, slide, text};
use crate::pb::rv::data as pp;
use crate::report::{Category, CcliTitleMismatch, UngroupedCues};

/// Name of the extra section that collects cues outside every cue group.
pub const UNGROUPED_SECTION: &str = "Ungrouped";

/// Cue → slide converter (the slides area's `slide::slide_from_cue`; unit tests pass a stub).
type CueConverter = fn(&mut ImportContext, &pp::Cue, Size) -> Slide;

/// Converts, registers and pushes one presentation. `None` when it could not be imported.
pub fn import_presentation(
    ctx: &mut ImportContext,
    doc: &pp::Presentation,
    rel_path: &str,
    library: LibraryId,
) -> Option<PresentationId> {
    import_presentation_with(ctx, doc, rel_path, library, slide::slide_from_cue)
}

/// A structured music key's name (`"Bb"`, `"F#m"`).
fn music_key_name(k: &pp::MusicKeyScale) -> String {
    const NAMES: [&str; 21] = [
        "Ab", "A", "A#", "Bb", "B", "B#", "Cb", "C", "C#", "Db", "D", "D#", "Eb", "E", "E#", "Fb",
        "F", "F#", "Gb", "G", "G#",
    ];
    let name = NAMES
        .get(k.music_key.max(0) as usize)
        .copied()
        .unwrap_or("C");
    if k.music_scale == 1 {
        format!("{name}m")
    } else {
        name.to_string()
    }
}

fn import_presentation_with(
    ctx: &mut ImportContext,
    doc: &pp::Presentation,
    rel_path: &str,
    library: LibraryId,
    cue: CueConverter,
) -> Option<PresentationId> {
    let rel = crate::source::normalize_rel(rel_path);
    if ctx.current.path.is_none() {
        ctx.current.path = Some(rel.clone());
    }
    let mut p = presentation_with(ctx, doc, &rel, library, cue);

    // A presentation UUID that is already taken (a copied file) gets a derived ID.
    let taken = ctx.lookups.presentations.values().any(|e| e.id == p.id);
    if taken {
        let original = p.id;
        p.id = PresentationId::from_uuid(ids::derived("presentation", &[&rel]));
        rekey_presentation(&mut p);
        p.import.get_or_insert_with(ImportInfo::default).original_id = Some(original.to_string());
        ctx.warn(
            Category::Other,
            format!("presentation UUID {original} is used by another file; imported with a new ID"),
        );
    }

    let basename = rel
        .rsplit('/')
        .next()
        .unwrap_or(&rel)
        .trim_end_matches(".pro")
        .trim_end_matches(".PRO")
        .to_string();
    ctx.lookups.presentations.insert(
        rel,
        PresentationEntry {
            id: p.id,
            library,
            name: p.name.clone(),
            basename,
            arrangements: p.arrangements.iter().map(|a| a.id).collect(),
        },
    );
    let id = p.id;
    ctx.out.presentations.push(p);
    Some(id)
}

/// Converts one presentation (pure: no push, no lookup registration).
pub fn presentation(
    ctx: &mut ImportContext,
    doc: &pp::Presentation,
    rel_path: &str,
    library: LibraryId,
) -> Presentation {
    presentation_with(ctx, doc, rel_path, library, slide::slide_from_cue)
}

fn presentation_with(
    ctx: &mut ImportContext,
    doc: &pp::Presentation,
    rel_path: &str,
    library: LibraryId,
    cue: CueConverter,
) -> Presentation {
    ctx.set_writer(doc.application_info.as_ref());
    let rel = crate::source::normalize_rel(rel_path);
    let id = PresentationId::from_uuid(ids::source_or_derived(
        doc.uuid.as_ref(),
        "presentation",
        &[&rel],
    ));
    let name = if doc.name.trim().is_empty() {
        file_stem(&rel)
    } else {
        doc.name.clone()
    };
    let mut import = Some(ctx.document_info());

    let canvas = canvas_size(ctx, doc);
    let sections = sections_with(ctx, doc, canvas, cue);
    let arrangements = arrangements(ctx, doc, &sections);

    let selected_arrangement = match ids::parse_uuid(doc.selected_arrangement.as_ref()) {
        None => None,
        Some(u) => {
            let a = ArrangementId::from_uuid(u);
            if arrangements.iter().any(|x| x.id == a) {
                Some(a)
            } else {
                ctx.report.dangling_arrangements.selected_arrangement += 1;
                None
            }
        }
    };

    let song_info = doc.ccli.as_ref().and_then(|c| song_info(ctx, c, &name));
    if let Some(c) = doc.ccli.as_ref().filter(|c| !c.artwork.is_empty()) {
        let only = pp::presentation::Ccli {
            artwork: c.artwork.clone(),
            ..Default::default()
        };
        let v = ctx.to_json(&only)["artwork"].clone();
        ctx.keep_unmapped(&mut import, "presentation.ccli.artwork", v);
    }
    let mut song_info = song_info;
    if let Some(m) = doc.music.as_ref() {
        // The written key (its name, else the structured key) and the key the user plays it in
        // (→ the song's transposition).
        let original = Some(m.original_music_key.trim().to_string())
            .filter(|k| !k.is_empty())
            .or_else(|| m.original.as_ref().map(music_key_name));
        let user = Some(m.user_music_key.trim().to_string())
            .filter(|k| !k.is_empty())
            .or_else(|| m.user.as_ref().map(music_key_name));
        if let Some(key) = &original {
            let info = song_info.get_or_insert_with(SongInfo::default);
            info.original_key = Some(key.clone());
            if let Some(d) = user
                .as_deref()
                .and_then(|u| model::chords::key_distance(key, u))
            {
                info.transpose = d;
            }
        }
    }

    let scripture = doc.bible_reference.as_ref().and_then(|br| {
        let s = scripture(br);
        if s.is_none() {
            let v = ctx.to_json(br);
            ctx.keep_unmapped(&mut import, "presentation.bibleReference", v);
        }
        s.map(|s| with_name_label(s, &name))
    });

    let transition = doc.transition.as_ref().map(|t| {
        if let Some(fav) = ids::parse_uuid(t.favorite_uuid.as_ref()) {
            ctx.keep_unmapped(
                &mut import,
                "presentation.transition.favoriteUuid",
                Value::String(fav.to_string()),
            );
        }
        common::transition(ctx, t)
    });

    let destination =
        if doc.content_destination == pp::action::ContentDestination::Announcements as i32 {
            ContentDestination::Announcements
        } else {
            ContentDestination::Slide
        };

    let notes = if doc.notes.trim().is_empty() {
        RichText::default()
    } else if doc.notes.trim_start().starts_with("{\\rtf") {
        text::rich_text_from_rtf(ctx, doc.notes.as_bytes())
    } else {
        RichText::plain(&doc.notes)
    };

    // Unmodelled document-level data.
    if let Some(bg) = doc.background.as_ref().filter(|b| b.is_enabled) {
        let v = ctx.to_json(bg);
        ctx.keep_unmapped(&mut import, "presentation.background", v);
    }
    if let Some(cc) = doc.chord_chart.as_ref() {
        let v = ctx.to_json(cc);
        ctx.keep_unmapped(&mut import, "presentation.chordChart", v);
    }
    if let Some(tl) = doc.timeline.as_ref().filter(|t| !timeline_is_default(t)) {
        let v = ctx.to_json(tl);
        ctx.keep_unmapped(&mut import, "presentation.timeline", v);
    }
    if let Some(mt) = doc.multi_tracks_licensing.as_ref() {
        let v = ctx.to_json(mt);
        ctx.keep_unmapped(&mut import, "presentation.multiTracksLicensing", v);
    }
    if !doc.music_key.is_empty() {
        ctx.keep_unmapped(
            &mut import,
            "presentation.musicKey",
            Value::String(doc.music_key.clone()),
        );
    }
    if let Some(m) = doc.music.as_ref() {
        let v = ctx.to_json(m);
        ctx.keep_unmapped(&mut import, "presentation.music", v);
    }

    Presentation {
        id,
        name,
        library: Some(library),
        category: Some(doc.category.trim().to_string()).filter(|c| !c.is_empty()),
        song_info,
        scripture,
        canvas_size: canvas,
        sections,
        arrangements,
        selected_arrangement,
        auto_advance: auto_advance(doc),
        transition,
        destination,
        notes,
        modified_at: common::timestamp(doc.last_modified_date.as_ref()),
        last_used_at: common::timestamp(doc.last_date_used.as_ref()),
        import,
        ..Presentation::default()
    }
}

/// ProPresenter writes an empty timeline (`{}` or `{duration: 300}`) into every document.
fn timeline_is_default(t: &pp::presentation::Timeline) -> bool {
    t.cues.is_empty()
        && t.cues_v2.is_empty()
        && t.audio_action.is_none()
        && !t.r#loop
        && !t.timecode_enable
        && t.timecode_offset == 0.0
        && (t.duration == 0.0 || t.duration == 300.0)
}

fn file_stem(rel: &str) -> String {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    match base.rfind('.') {
        Some(i) if i > 0 => base[..i].to_string(),
        _ => base.to_string(),
    }
}

/// Canvas size of the presentation: the first slide's `baseSlide.size`, else 1920×1080. Slides
/// of another size are warned.
pub fn canvas_size(ctx: &mut ImportContext, doc: &pp::Presentation) -> Size {
    let sizes: Vec<Size> = doc
        .cues
        .iter()
        .filter_map(slide::presentation_slide)
        .filter_map(|(_, ps)| common::size(ps.base_slide.as_ref()?.size.as_ref()))
        .collect();
    let Some(first) = sizes.first().copied() else {
        return Size::HD;
    };
    let other = sizes.iter().filter(|s| **s != first).count();
    if other > 0 {
        ctx.warn(
            Category::Other,
            format!(
                "{other} slide(s) differ from the canvas size {}x{}; elements keep their coordinates",
                first.width, first.height
            ),
        );
    }
    first
}

/// Sections with their slides (see the module docs).
pub fn sections(ctx: &mut ImportContext, doc: &pp::Presentation, canvas: Size) -> Vec<Section> {
    sections_with(ctx, doc, canvas, slide::slide_from_cue)
}

fn sections_with(
    ctx: &mut ImportContext,
    doc: &pp::Presentation,
    canvas: Size,
    convert: CueConverter,
) -> Vec<Section> {
    let doc_path = ctx.current.path.clone().unwrap_or_default();
    // Cue UUID → index (first occurrence; a repeated cue UUID cannot be addressed by a group).
    let mut by_uuid: BTreeMap<Uuid, usize> = BTreeMap::new();
    for (i, c) in doc.cues.iter().enumerate() {
        if let Some(u) = ids::parse_uuid(c.uuid.as_ref()) {
            by_uuid.entry(u).or_insert(i);
        }
    }
    let mut converted: Vec<Option<Slide>> = vec![None; doc.cues.len()];
    let mut section_ids: BTreeSet<SectionId> = BTreeSet::new();
    let mut sections = Vec::with_capacity(doc.cue_groups.len() + 1);
    let mut dangling_cues = 0usize;

    for (gi, cg) in doc.cue_groups.iter().enumerate() {
        let group = cg.group.clone().unwrap_or_default();
        let mut section = section_from_group(ctx, &group);
        if ids::parse_uuid(group.uuid.as_ref()).is_none() || section_ids.contains(&section.id) {
            let derived =
                SectionId::from_uuid(ids::derived("section", &[&doc_path, &gi.to_string()]));
            if let Some(orig) = ids::parse_uuid(group.uuid.as_ref()) {
                section
                    .import
                    .get_or_insert_with(ImportInfo::default)
                    .original_id = Some(orig.to_string());
            }
            section.id = derived;
        }
        section_ids.insert(section.id);

        for cue_ref in &cg.cue_identifiers {
            let Some(&ci) = ids::parse_uuid(Some(cue_ref)).and_then(|u| by_uuid.get(&u)) else {
                dangling_cues += 1;
                continue;
            };
            match &converted[ci] {
                None => {
                    let s = convert(ctx, &doc.cues[ci], canvas);
                    converted[ci] = Some(s.clone());
                    section.slides.push(s);
                }
                Some(first) => {
                    // The same cue in a second place: an independent copy with derived IDs.
                    let mut copy = first.clone();
                    let original = copy.id;
                    rekey_slide(&mut copy, &section.id.to_string(), section.slides.len());
                    copy.import
                        .get_or_insert_with(ImportInfo::default)
                        .original_id = Some(original.to_string());
                    ctx.warn(
                        Category::Other,
                        format!(
                            "cue {original} appears more than once in the cue groups; duplicated"
                        ),
                    );
                    section.slides.push(copy);
                }
            }
        }
        sections.push(section);
    }
    if dangling_cues > 0 {
        ctx.warn(
            Category::Other,
            format!("{dangling_cues} cue reference(s) in cue groups name no cue; skipped"),
        );
    }

    // Cues outside every group.
    let ungrouped: Vec<usize> = (0..doc.cues.len())
        .filter(|&i| converted[i].is_none())
        .collect();
    if !ungrouped.is_empty() {
        let mut section = Section::new(UNGROUPED_SECTION);
        section.id = SectionId::from_uuid(ids::derived("section", &[&doc_path, "ungrouped"]));
        for i in &ungrouped {
            let s = convert(ctx, &doc.cues[*i], canvas);
            section.slides.push(s);
        }
        ctx.report.ungrouped_cues.push(UngroupedCues {
            document: doc_path.clone(),
            cues: ungrouped.len(),
        });
        ctx.warn(
            Category::UngroupedCues,
            format!(
                "{} cue(s) belong to no cue group; put into a section \"{UNGROUPED_SECTION}\"",
                ungrouped.len()
            ),
        );
        sections.push(section);
    }
    sections
}

/// A section without slides from a cue group's `Group` (ID, name, colour, hotkey, global link).
pub fn section_from_group(ctx: &mut ImportContext, group: &pp::Group) -> Section {
    let doc_path = ctx.current.path.clone().unwrap_or_default();
    let id = SectionId::from_uuid(ids::source_or_derived(
        group.uuid.as_ref(),
        "section",
        &[&doc_path, &group.name],
    ));
    Section {
        id,
        name: group.name.clone(),
        color: common::visible_color(group.color.as_ref()),
        hotkey: common::hotkey(group.hot_key.as_ref()),
        global_group: global_group_link(ctx, group),
        slides: Vec::new(),
        import: None,
    }
}

/// The global group link of a cue group (see the module docs).
pub fn global_group_link(
    ctx: &mut ImportContext,
    group: &pp::Group,
) -> Option<Link<GlobalGroupId>> {
    let uuid = ids::parse_uuid(group.application_group_identifier.as_ref())?;
    let id = GlobalGroupId::from_uuid(uuid);
    let name = group.application_group_name.trim();
    if !ctx.lookups.global_groups.contains_key(&uuid) {
        let hint = if name.is_empty() {
            group.name.as_str()
        } else {
            name
        };
        ctx.report.dangling_group_link(hint);
    }
    Some(if name.is_empty() {
        Link::to(id)
    } else {
        Link::named(id, group.application_group_name.clone())
    })
}

/// Arrangements whose group references resolve to `sections` (unresolvable references are
/// dropped from the arrangement with a warning; an arrangement left empty is dropped).
pub fn arrangements(
    ctx: &mut ImportContext,
    doc: &pp::Presentation,
    sections: &[Section],
) -> Vec<Arrangement> {
    let doc_path = ctx.current.path.clone().unwrap_or_default();
    // Source cue-group UUID → section (first section with that source UUID).
    let mut by_source: BTreeMap<Uuid, SectionId> = BTreeMap::new();
    for s in sections {
        let source = s
            .import
            .as_ref()
            .and_then(|i| i.original_id.as_deref())
            .and_then(|o| Uuid::parse_str(o).ok())
            .unwrap_or(s.id.0);
        by_source.entry(source).or_insert(s.id);
    }
    let mut out: Vec<Arrangement> = Vec::new();
    for (ai, a) in doc.arrangements.iter().enumerate() {
        let mut dropped = 0usize;
        let refs: Vec<SectionId> = a
            .group_identifiers
            .iter()
            .filter_map(|g| {
                let r = ids::parse_uuid(Some(g)).and_then(|u| by_source.get(&u).copied());
                if r.is_none() {
                    dropped += 1;
                }
                r
            })
            .collect();
        if dropped > 0 {
            ctx.warn(
                Category::DanglingArrangements,
                format!(
                    "arrangement \"{}\": {dropped} group reference(s) do not resolve; dropped",
                    a.name
                ),
            );
        }
        if refs.is_empty() {
            ctx.warn(
                Category::DanglingArrangements,
                format!(
                    "arrangement \"{}\" has no resolvable groups; not imported",
                    a.name
                ),
            );
            continue;
        }
        let mut id = ArrangementId::from_uuid(ids::source_or_derived(
            a.uuid.as_ref(),
            "arrangement",
            &[&doc_path, &ai.to_string()],
        ));
        if out.iter().any(|x| x.id == id) {
            id = ArrangementId::from_uuid(ids::derived(
                "arrangement",
                &[&doc_path, &ai.to_string()],
            ));
        }
        out.push(Arrangement {
            id,
            name: a.name.clone(),
            sections: refs,
        });
    }
    out
}

/// CCLI block → [`SongInfo`] (`None` when empty).
pub fn song_info(
    ctx: &mut ImportContext,
    ccli: &pp::presentation::Ccli,
    presentation_name: &str,
) -> Option<SongInfo> {
    let text = |s: &str| Some(s.to_string()).filter(|s| !s.trim().is_empty());
    let info = SongInfo {
        title: text(&ccli.song_title),
        ccli_number: (ccli.song_number != 0).then(|| ccli.song_number.to_string()),
        authors: text(&ccli.author),
        artist: text(&ccli.artist_credits),
        publisher: text(&ccli.publisher),
        copyright_year: (ccli.copyright_year != 0).then_some(ccli.copyright_year),
        album: text(&ccli.album),
        display: ccli.display,
        original_key: None,
        transpose: 0,
        artwork: None,
    };
    if info == SongInfo::default() {
        return None;
    }
    if let Some(title) = &info.title
        && title.trim().to_lowercase() != presentation_name.trim().to_lowercase()
    {
        ctx.report.ccli_title_mismatch.push(CcliTitleMismatch {
            document: ctx.current.path.clone().unwrap_or_default(),
            presentation: presentation_name.to_string(),
            ccli_title: title.clone(),
        });
    }
    Some(info)
}

/// Bible reference → [`ScriptureRef`] (label kept; translation only as a hint).
pub fn scripture(br: &pp::presentation::BibleReference) -> Option<ScriptureRef> {
    let book_name = br.book_name.trim();
    let book_key = br.book_key.trim();
    if book_name.is_empty() && book_key.is_empty() {
        return None;
    }
    let range = |r: &pp::IntRange| {
        let start = u32::try_from(r.start).unwrap_or(0);
        let end = u32::try_from(r.end).unwrap_or(0);
        IntRange { start, end }
    };
    let mut chapters = br
        .chapter_range
        .as_ref()
        .map(range)
        .filter(|r| r.start > 0)?;
    chapters.end = chapters.end.max(chapters.start);
    let mut verses = br.verse_range.as_ref().map(range).filter(|r| r.start > 0);
    // Within one chapter the verse range is ordered; across chapters the end verse belongs to
    // the end chapter and may be smaller than the start verse.
    if let Some(v) = verses.as_mut()
        && chapters.start == chapters.end
    {
        v.end = v.end.max(v.start);
    }
    let book = BibleBook {
        name: book_name.to_string(),
        key: Some(book_key.to_string()).filter(|k| !k.is_empty()),
        // ProPresenter counts books from 0 (Genesis = 0); the model from 1.
        index: Some(br.book_index + 1),
    };
    let translation = [
        &br.translation_display_abbreviation,
        &br.translation_internal_abbreviation,
        &br.translation_name,
    ]
    .into_iter()
    .map(|s| s.trim())
    .find(|s| !s.is_empty())
    .map(str::to_string);
    let label = Some(reference_text(
        if book_name.is_empty() {
            book_key
        } else {
            book_name
        },
        chapters,
        verses,
    ));
    Some(ScriptureRef {
        book,
        chapters,
        verses,
        label,
        translation,
        generated: None,
    })
}

/// `"John 3:16-18"`, `"John 3:16-4:2"`, `"Psalms 23"`, `"Psalms 23-24"`.
fn reference_text(book: &str, ch: IntRange, verses: Option<IntRange>) -> String {
    match verses {
        None if ch.start == ch.end => format!("{book} {}", ch.start),
        None => format!("{book} {}-{}", ch.start, ch.end),
        Some(v) if ch.start == ch.end && v.start == v.end => {
            format!("{book} {}:{}", ch.start, v.start)
        }
        Some(v) if ch.start == ch.end => format!("{book} {}:{}-{}", ch.start, v.start, v.end),
        Some(v) => format!("{book} {}:{}-{}:{}", ch.start, v.start, ch.end, v.end),
    }
}

/// Uses the presentation name as label when it is the same reference in file-name-safe form
/// (`"John 3_16 (NKJV)"` → `"John 3:16 (NKJV)"`).
fn with_name_label(mut s: ScriptureRef, name: &str) -> ScriptureRef {
    let Some(label) = s.label.clone() else {
        return s;
    };
    let candidate = name.trim().replace('_', ":");
    let matches = candidate
        .get(..label.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(&label))
        && candidate
            .get(label.len()..)
            .is_some_and(|rest| rest.starts_with(' ') && !rest.trim().is_empty());
    if matches {
        s.label = Some(candidate);
    }
    s
}

/// `slideShowDuration` → [`AutoAdvance`].
pub fn auto_advance(doc: &pp::Presentation) -> Option<AutoAdvance> {
    match doc.slide_show {
        Some(pp::presentation::SlideShow::SlideShowDuration(d)) if d > 0.0 && d.is_finite() => {
            Some(AutoAdvance {
                seconds: d,
                loop_to_start: true,
            })
        }
        _ => None,
    }
}

// ---- re-keying --------------------------------------------------------------------------------

/// Gives a copied slide (and its elements and actions) IDs derived from `scope` and its position,
/// remapping element references in visibility conditions.
fn rekey_slide(s: &mut Slide, scope: &str, index: usize) {
    let old = s.id;
    s.id = SlideId::from_uuid(ids::derived(
        "slide",
        &[scope, &index.to_string(), &old.to_string()],
    ));
    rekey_slide_contents(s);
}

fn rekey_slide_contents(s: &mut Slide) {
    let scope = s.id.to_string();
    let mut map: Vec<(ElementId, ElementId)> = Vec::new();
    for e in &mut s.elements {
        let new = ElementId::from_uuid(ids::derived("element", &[&scope, &e.id.to_string()]));
        map.push((e.id, new));
        e.id = new;
    }
    for e in &mut s.elements {
        for c in &mut e.visibility {
            if let VisibilityCondition::ElementHasText { element, .. } = c
                && let Some((_, new)) = map.iter().find(|(o, _)| o == element)
            {
                *element = *new;
            }
        }
    }
    for a in &mut s.actions {
        a.id = ActionId::from_uuid(ids::derived("action", &[&scope, &a.id.to_string()]));
    }
}

/// Re-keys every nested ID of a presentation below its (new) ID: sections, arrangements (and
/// their section references), the selected arrangement, slides, elements and actions.
fn rekey_presentation(p: &mut Presentation) {
    let scope = p.id.to_string();
    let mut section_map: BTreeMap<SectionId, SectionId> = BTreeMap::new();
    for sec in &mut p.sections {
        let new = SectionId::from_uuid(ids::derived("section", &[&scope, &sec.id.to_string()]));
        section_map.entry(sec.id).or_insert(new);
        sec.id = new;
        for s in &mut sec.slides {
            s.id = SlideId::from_uuid(ids::derived("slide", &[&scope, &s.id.to_string()]));
            rekey_slide_contents(s);
        }
    }
    let mut arr_map: BTreeMap<ArrangementId, ArrangementId> = BTreeMap::new();
    for a in &mut p.arrangements {
        let new =
            ArrangementId::from_uuid(ids::derived("arrangement", &[&scope, &a.id.to_string()]));
        arr_map.insert(a.id, new);
        a.id = new;
        for s in &mut a.sections {
            if let Some(n) = section_map.get(s) {
                *s = *n;
            }
        }
    }
    if let Some(sel) = p.selected_arrangement {
        p.selected_arrangement = arr_map.get(&sel).copied();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::ImportOptions;
    use crate::context::NamedId;
    use model::{Element, SlideAction};
    use std::path::Path;

    pub(crate) fn uuid(n: u128) -> pp::Uuid {
        pp::Uuid {
            string: Uuid::from_u128(n).to_string().to_uppercase(),
        }
    }

    fn ctx() -> ImportContext {
        let mut ctx = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        ctx.current.path = Some("Libraries/Songs/Song.pro".into());
        ctx
    }

    /// Stand-in for `slide::slide_from_cue`: ID from the cue, one element per base-slide element.
    pub(crate) fn stub_cue(_: &mut ImportContext, cue: &pp::Cue, _: Size) -> Slide {
        let mut s = Slide::new();
        s.id = SlideId::from_uuid(ids::parse_uuid(cue.uuid.as_ref()).unwrap_or_default());
        s.enabled = cue.is_enabled;
        let n = slide::presentation_slide(cue)
            .and_then(|(_, ps)| ps.base_slide.as_ref())
            .map(|b| b.elements.len())
            .unwrap_or(0);
        for i in 0..n {
            s.elements.push(Element {
                id: ElementId::from_uuid(Uuid::from_u128(0xE000 + i as u128)),
                ..Element::default()
            });
        }
        let mut a = SlideAction::new(model::ActionKind::Unknown);
        a.id = ActionId::from_uuid(Uuid::from_u128(0xA000));
        s.actions.push(a);
        s
    }

    pub(crate) fn cue(n: u128, elements: usize, size: Option<(f64, f64)>) -> pp::Cue {
        let base = pp::Slide {
            elements: vec![pp::slide::Element::default(); elements],
            size: size.map(|(w, h)| pp::graphics::Size {
                width: w,
                height: h,
            }),
            uuid: Some(uuid(n + 0x1000)),
            ..Default::default()
        };
        pp::Cue {
            uuid: Some(uuid(n)),
            name: format!(" {n}"),
            is_enabled: true,
            actions: vec![pp::Action {
                action_type_data: Some(pp::action::ActionTypeData::Slide(pp::action::SlideType {
                    slide: Some(pp::action::slide_type::Slide::Presentation(
                        pp::PresentationSlide {
                            base_slide: Some(base),
                            ..Default::default()
                        },
                    )),
                })),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    pub(crate) fn group(
        n: u128,
        name: &str,
        global: Option<(u128, &str)>,
        cues: &[u128],
    ) -> pp::presentation::CueGroup {
        pp::presentation::CueGroup {
            group: Some(pp::Group {
                uuid: Some(uuid(n)),
                name: name.into(),
                color: Some(pp::Color {
                    red: 0.0,
                    green: 0.0,
                    blue: 1.0,
                    alpha: 1.0,
                }),
                hot_key: Some(pp::HotKey {
                    code: pp::KeyCode::AnsiA as i32,
                    ..Default::default()
                }),
                application_group_identifier: global.map(|(g, _)| uuid(g)),
                application_group_name: global.map(|(_, n)| n.to_string()).unwrap_or_default(),
            }),
            cue_identifiers: cues.iter().map(|c| uuid(*c)).collect(),
        }
    }

    fn song() -> pp::Presentation {
        pp::Presentation {
            uuid: Some(uuid(1)),
            name: "Example Song".into(),
            cues: vec![
                cue(10, 2, Some((1920.0, 1080.0))),
                cue(11, 1, None),
                cue(12, 0, None),
            ],
            cue_groups: vec![
                group(20, "Verse 1", Some((30, "Verse 1")), &[10, 11]),
                group(21, "Chorus", Some((31, "Chorus")), &[12]),
            ],
            selected_arrangement: Some(uuid(99)),
            arrangements: vec![pp::presentation::Arrangement {
                uuid: Some(uuid(40)),
                name: "Short".into(),
                group_identifiers: vec![uuid(21), uuid(20), uuid(21), uuid(77)],
            }],
            ccli: Some(pp::presentation::Ccli {
                author: "A, B and C".into(),
                song_title: "Example Song (Live)".into(),
                song_number: 1234567,
                copyright_year: 2001,
                publisher: "Example Music".into(),
                display: true,
                ..Default::default()
            }),
            timeline: Some(pp::presentation::Timeline {
                duration: 300.0,
                ..Default::default()
            }),
            background: Some(pp::Background {
                is_enabled: false,
                fill: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn song_sections_slides_and_links() {
        let mut ctx = ctx();
        ctx.lookups.global_groups.insert(
            Uuid::from_u128(30),
            NamedId {
                id: GlobalGroupId::from_uuid(Uuid::from_u128(30)),
                name: "Verse 1".into(),
            },
        );
        let lib = ids::library_id("Songs");
        let id =
            import_presentation_with(&mut ctx, &song(), "Libraries/Songs/Song.pro", lib, stub_cue)
                .unwrap();
        let p = &ctx.out.presentations[0];
        assert_eq!(p.id, id);
        assert_eq!(p.id.0, Uuid::from_u128(1));
        assert_eq!(p.name, "Example Song");
        assert_eq!(p.library, Some(lib));
        assert_eq!(p.canvas_size, Size::HD);
        assert_eq!(p.sections.len(), 2);
        assert_eq!(p.sections[0].id.0, Uuid::from_u128(20));
        assert_eq!(p.sections[0].name, "Verse 1");
        assert_eq!(p.sections[0].hotkey.as_ref().unwrap().code, "KeyA");
        assert!(p.sections[0].color.is_some());
        let ids: Vec<_> = p.slides().map(|s| s.id.0).collect();
        assert_eq!(
            ids,
            [
                Uuid::from_u128(10),
                Uuid::from_u128(11),
                Uuid::from_u128(12)
            ]
        );
        // Group links: one resolves, one dangles (kept with its name).
        assert_eq!(
            p.sections[0].global_group.as_ref().unwrap().id.0,
            Uuid::from_u128(30)
        );
        let dangling = p.sections[1].global_group.as_ref().unwrap();
        assert_eq!(dangling.name.as_deref(), Some("Chorus"));
        assert_eq!(ctx.report.dangling_group_links.count, 1);
        assert_eq!(ctx.report.dangling_group_links.names["Chorus"], 1);
        // Arrangement: the dangling reference is dropped, repeats kept.
        assert_eq!(p.arrangements.len(), 1);
        assert_eq!(
            p.arrangements[0]
                .sections
                .iter()
                .map(|s| s.0)
                .collect::<Vec<_>>(),
            [
                Uuid::from_u128(21),
                Uuid::from_u128(20),
                Uuid::from_u128(21)
            ]
        );
        // The selected arrangement dangles.
        assert_eq!(p.selected_arrangement, None);
        assert_eq!(ctx.report.dangling_arrangements.selected_arrangement, 1);
        // Song info verbatim + title mismatch.
        let si = p.song_info.as_ref().unwrap();
        assert_eq!(si.ccli_number.as_deref(), Some("1234567"));
        assert_eq!(si.authors.as_deref(), Some("A, B and C"));
        assert_eq!(si.copyright_year, Some(2001));
        assert!(si.display);
        assert_eq!(ctx.report.ccli_title_mismatch.len(), 1);
        assert_eq!(
            ctx.report.ccli_title_mismatch[0].ccli_title,
            "Example Song (Live)"
        );
        // Provenance; default timeline / disabled background are not kept.
        let imp = p.import.as_ref().unwrap();
        assert_eq!(
            imp.document_path.as_deref(),
            Some("Libraries/Songs/Song.pro")
        );
        assert!(imp.unmapped.is_null());
        // Registered for playlists.
        let e = ctx
            .lookups
            .presentation_by_path("Libraries/Songs/Song.pro")
            .unwrap();
        assert_eq!(e.basename, "Song");
        assert_eq!(e.arrangements, vec![p.arrangements[0].id]);
        assert!(ctx.report.ungrouped_cues.is_empty());
    }

    #[test]
    fn selected_arrangement_kept_when_it_resolves() {
        let mut ctx = ctx();
        let mut doc = song();
        doc.selected_arrangement = Some(uuid(40));
        let p = presentation_with(
            &mut ctx,
            &doc,
            "Libraries/Songs/Song.pro",
            ids::library_id("Songs"),
            stub_cue,
        );
        assert_eq!(
            p.selected_arrangement.map(|a| a.0),
            Some(Uuid::from_u128(40))
        );
        assert_eq!(ctx.report.dangling_arrangements.selected_arrangement, 0);
    }

    #[test]
    fn music_keys_give_the_key_and_transposition() {
        let mut ctx = ctx();
        let mut doc = song();
        doc.music = Some(pp::presentation::Music {
            original_music_key: String::new(),
            user_music_key: String::new(),
            original: Some(pp::MusicKeyScale {
                music_key: 19, // G
                music_scale: 0,
            }),
            user: Some(pp::MusicKeyScale {
                music_key: 1, // A
                music_scale: 0,
            }),
        });
        let p = presentation_with(
            &mut ctx,
            &doc,
            "Libraries/Songs/Song.pro",
            ids::library_id("Songs"),
            stub_cue,
        );
        let info = p.song_info.unwrap();
        assert_eq!(info.original_key.as_deref(), Some("G"));
        assert_eq!(info.transpose, 2);
        assert_eq!(
            music_key_name(&pp::MusicKeyScale {
                music_key: 3,
                music_scale: 1
            }),
            "Bbm"
        );
    }

    #[test]
    fn ungrouped_and_duplicated_cues() {
        let mut ctx = ctx();
        let mut doc = song();
        doc.cues.push(cue(13, 0, None)); // in no group
        doc.cue_groups.push(group(22, "Tag", None, &[10])); // cue 10 again
        let p = presentation_with(
            &mut ctx,
            &doc,
            "Libraries/Songs/Song.pro",
            ids::library_id("Songs"),
            stub_cue,
        );
        assert_eq!(p.sections.len(), 4);
        let tag = &p.sections[2];
        assert_eq!(tag.global_group, None);
        let copy = &tag.slides[0];
        assert_ne!(copy.id.0, Uuid::from_u128(10));
        assert_eq!(
            copy.import.as_ref().unwrap().original_id.as_deref(),
            Some(Uuid::from_u128(10).to_string().as_str())
        );
        let first = &p.sections[0].slides[0];
        assert_eq!(copy.elements.len(), first.elements.len());
        assert_ne!(copy.elements[0].id, first.elements[0].id);
        assert_ne!(copy.actions[0].id, first.actions[0].id);
        let un = &p.sections[3];
        assert_eq!(un.name, UNGROUPED_SECTION);
        assert_eq!(un.slides[0].id.0, Uuid::from_u128(13));
        assert_eq!(ctx.report.ungrouped_cues[0].cues, 1);
        // All slide IDs unique.
        let all: BTreeSet<_> = p.slides().map(|s| s.id).collect();
        assert_eq!(all.len(), p.slides().count());
    }

    #[test]
    fn duplicate_presentation_uuid_is_rekeyed() {
        let mut ctx = ctx();
        let lib = ids::library_id("Songs");
        let a = import_presentation_with(&mut ctx, &song(), "Libraries/Songs/A.pro", lib, stub_cue)
            .unwrap();
        let b = import_presentation_with(&mut ctx, &song(), "Libraries/Songs/B.pro", lib, stub_cue)
            .unwrap();
        assert_ne!(a, b);
        let (pa, pb) = (&ctx.out.presentations[0], &ctx.out.presentations[1]);
        assert_eq!(
            pb.import.as_ref().unwrap().original_id.as_deref(),
            Some(a.to_string().as_str())
        );
        let sa: BTreeSet<_> = pa.slides().map(|s| s.id).collect();
        assert!(pb.slides().all(|s| !sa.contains(&s.id)));
        assert!(pb.sections.iter().all(|s| pa.section(s.id).is_none()));
        // Arrangement references follow the new section IDs.
        for sec in &pb.arrangements[0].sections {
            assert!(pb.section(*sec).is_some());
        }
        assert_eq!(ctx.lookups.presentations["Libraries/Songs/B.pro"].id, b);
    }

    #[test]
    fn scripture_auto_advance_destination_transition() {
        let mut ctx = ctx();
        let doc = pp::Presentation {
            uuid: Some(uuid(2)),
            name: "John 3_16-18 (XYZ)".into(),
            bible_reference: Some(pp::presentation::BibleReference {
                book_index: 42,
                book_name: "John".into(),
                book_key: "JHN".into(),
                chapter_range: Some(pp::IntRange { start: 3, end: 3 }),
                verse_range: Some(pp::IntRange { start: 16, end: 18 }),
                translation_display_abbreviation: "ABC".into(),
                ..Default::default()
            }),
            slide_show: Some(pp::presentation::SlideShow::SlideShowDuration(8.0)),
            content_destination: pp::action::ContentDestination::Announcements as i32,
            transition: Some(pp::Transition {
                duration: 0.3,
                favorite_uuid: Some(uuid(5)),
                effect: Some(pp::Effect {
                    name: "Example Warp".into(),
                    render_id: "VENDOR-ID".into(),
                    ..Default::default()
                }),
            }),
            background: Some(pp::Background {
                is_enabled: true,
                fill: Some(pp::background::Fill::Color(pp::Color {
                    red: 1.0,
                    green: 1.0,
                    blue: 1.0,
                    alpha: 1.0,
                })),
            }),
            notes: "line one\nline two".into(),
            category: "Readings".into(),
            ..Default::default()
        };
        let p = presentation_with(
            &mut ctx,
            &doc,
            "Libraries/Verses/x.pro",
            ids::library_id("Verses"),
            stub_cue,
        );
        let s = p.scripture.as_ref().unwrap();
        assert_eq!(s.book.index, Some(43));
        assert_eq!(s.book.key.as_deref(), Some("JHN"));
        assert_eq!(s.chapters, IntRange::single(3));
        assert_eq!(s.verses, Some(IntRange { start: 16, end: 18 }));
        assert_eq!(s.label.as_deref(), Some("John 3:16-18 (XYZ)"));
        assert_eq!(s.translation.as_deref(), Some("ABC"));
        assert_eq!(
            p.auto_advance,
            Some(AutoAdvance {
                seconds: 8.0,
                loop_to_start: true
            })
        );
        assert_eq!(p.destination, ContentDestination::Announcements);
        let t = p.transition.as_ref().unwrap();
        assert!(matches!(t.kind, model::TransitionKind::Vendor(_)));
        assert_eq!(t.duration, 0.3);
        assert_eq!(p.notes.paragraphs.len(), 2);
        assert_eq!(p.category.as_deref(), Some("Readings"));
        let un = &p.import.as_ref().unwrap().unmapped;
        assert!(un["background"].is_object());
        assert!(un["favoriteUuid"].is_string());
        assert!(p.sections.is_empty());
        assert_eq!(p.song_info, None);
    }

    #[test]
    fn scripture_labels() {
        let br = |c: (i32, i32), v: Option<(i32, i32)>| pp::presentation::BibleReference {
            book_name: "Psalms".into(),
            chapter_range: Some(pp::IntRange {
                start: c.0,
                end: c.1,
            }),
            verse_range: v.map(|(s, e)| pp::IntRange { start: s, end: e }),
            ..Default::default()
        };
        let l = |b| scripture(&b).unwrap().label.unwrap();
        assert_eq!(l(br((23, 23), None)), "Psalms 23");
        assert_eq!(l(br((23, 24), None)), "Psalms 23-24");
        assert_eq!(l(br((23, 23), Some((1, 1)))), "Psalms 23:1");
        assert_eq!(l(br((23, 24), Some((5, 2)))), "Psalms 23:5-24:2");
        assert!(scripture(&pp::presentation::BibleReference::default()).is_none());
        // Name not matching the reference: the generated label stays.
        let s = with_name_label(scripture(&br((23, 23), None)).unwrap(), "Sunday reading");
        assert_eq!(s.label.as_deref(), Some("Psalms 23"));
    }

    #[test]
    fn empty_ccli_and_matching_title() {
        let mut ctx = ctx();
        assert_eq!(
            song_info(&mut ctx, &pp::presentation::Ccli::default(), "X"),
            None
        );
        let c = pp::presentation::Ccli {
            song_title: "example song".into(),
            ..Default::default()
        };
        assert!(song_info(&mut ctx, &c, "Example Song").is_some());
        assert!(ctx.report.ccli_title_mismatch.is_empty());
    }

    #[test]
    fn timeline_with_content_is_kept() {
        let mut ctx = ctx();
        let mut doc = song();
        doc.timeline = Some(pp::presentation::Timeline {
            duration: 12.5,
            r#loop: true,
            ..Default::default()
        });
        doc.music_key = "G".into();
        let p = presentation_with(
            &mut ctx,
            &doc,
            "Libraries/Songs/Song.pro",
            ids::library_id("Songs"),
            stub_cue,
        );
        let un = &p.import.as_ref().unwrap().unmapped;
        assert_eq!(un["timeline"]["loop"], true);
        assert_eq!(un["musicKey"], "G");
        assert_eq!(ctx.report.unmapped_features["presentation.timeline"], 1);
    }

    #[test]
    fn mixed_canvas_sizes_warn() {
        let mut ctx = ctx();
        let doc = pp::Presentation {
            cues: vec![
                cue(1, 0, Some((1280.0, 720.0))),
                cue(2, 0, Some((1920.0, 1080.0))),
            ],
            ..Default::default()
        };
        assert_eq!(canvas_size(&mut ctx, &doc), Size::new(1280.0, 720.0));
        assert_eq!(ctx.report.warnings.len(), 1);
        assert_eq!(
            canvas_size(&mut ctx, &pp::Presentation::default()),
            Size::HD
        );
    }
}
