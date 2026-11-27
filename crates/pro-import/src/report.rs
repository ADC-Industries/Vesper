//! The import report: what was imported, what is missing and what could not be represented.
//!
//! Written as `report.json` (this struct, camelCase) and `report.md` ([`ImportReport::render_markdown`]).
//! Categories follow the importer spec:
//!
//! | category | field | filled by |
//! |---|---|---|
//! | missingMedia | [`ImportReport::missing_media`] (grouped per media root) | media area (`media::finalize_media`) |
//! | unresolvedPlaylistItems | [`ImportReport::unresolved_playlist_items`] | content area (`playlist.rs`) |
//! | danglingArrangements | [`ImportReport::dangling_arrangements`] | content area (`presentation.rs`, `playlist.rs`) |
//! | danglingGroupLinks | [`ImportReport::dangling_group_links`] | content area (`presentation.rs`) |
//! | unsupportedActions | [`ImportReport::unsupported_actions`] | slides / media areas ([`crate::ImportContext::count_unsupported_action`]) |
//! | unmappedFeatures | [`ImportReport::unmapped_features`] | every area ([`crate::ImportContext::keep_unmapped`]) |
//! | fonts | [`ImportReport::fonts`] | slides area (`fonts::font_report`) |
//! | ccliTitleMismatch | [`ImportReport::ccli_title_mismatch`] | content area (`presentation.rs`) |
//! | duplicateThemes | [`ImportReport::duplicate_themes`] | content area (`theme.rs`) |
//! | ungroupedCues | [`ImportReport::ungrouped_cues`] | content area (`presentation.rs`) |
//! | parseFailures | [`ImportReport::parse_failures`] | context (read / decode / panics) |
//!
//! Free-form notes go to [`ImportReport::warnings`] with a [`Category`] (`notImported` for
//! documents that are only reported, e.g. non-empty key mappings or calendar).
//! [`ImportReport::counts`] is computed from the bundle at the end ([`ImportReport::compute_counts`]).
//!
//! The report is output data (it names files of the imported backup); it is written only to the
//! output directory, never committed.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use model::{MediaAssetId, MediaRootId, VolumeHint};
use serde::{Deserialize, Serialize};

use crate::bundle::ImportBundle;

/// Report category of a warning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    /// A referenced media file is not available.
    MissingMedia,
    /// A playlist item that does not resolve to an imported presentation.
    UnresolvedPlaylistItems,
    /// An arrangement reference that does not resolve.
    DanglingArrangements,
    /// A section's global group link that does not resolve.
    DanglingGroupLinks,
    /// An action type the model does not represent.
    UnsupportedActions,
    /// Source data kept in `import.unmapped` or dropped.
    UnmappedFeatures,
    /// Fonts.
    Fonts,
    /// CCLI title differs from the presentation name.
    CcliTitleMismatch,
    /// Byte-identical duplicate theme skipped.
    DuplicateThemes,
    /// Cues outside every cue group.
    UngroupedCues,
    /// A file that failed to read, decode or convert.
    ParseFailures,
    /// A document that is only reported, not imported (key mappings, calendar, recording, ...).
    NotImported,
    /// Anything else.
    Other,
}

impl Category {
    /// The camelCase name used in JSON and Markdown.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::MissingMedia => "missingMedia",
            Category::UnresolvedPlaylistItems => "unresolvedPlaylistItems",
            Category::DanglingArrangements => "danglingArrangements",
            Category::DanglingGroupLinks => "danglingGroupLinks",
            Category::UnsupportedActions => "unsupportedActions",
            Category::UnmappedFeatures => "unmappedFeatures",
            Category::Fonts => "fonts",
            Category::CcliTitleMismatch => "ccliTitleMismatch",
            Category::DuplicateThemes => "duplicateThemes",
            Category::UngroupedCues => "ungroupedCues",
            Category::ParseFailures => "parseFailures",
            Category::NotImported => "notImported",
            Category::Other => "other",
        }
    }
}

/// One free-form warning.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    /// Category.
    pub category: Category,
    /// Source document (relative to the backup root), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<String>,
    /// What happened.
    pub detail: String,
}

/// A file that could not be read, decoded or converted (the import continued).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseFailure {
    /// File relative to the backup root.
    pub file: String,
    /// Error.
    pub error: String,
}

/// Missing media files under one media root (or unresolved, `root: None`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingMediaRoot {
    /// The root (`None` = unresolved references without a root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<MediaRootId>,
    /// Root display name.
    pub name: String,
    /// What source machines called the root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<VolumeHint>,
    /// Missing files, sorted by path.
    pub files: Vec<MissingFile>,
}

/// One missing file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingFile {
    /// Path relative to the root (or the best source reference when unresolved).
    pub path: String,
    /// The asset.
    pub asset: MediaAssetId,
    /// Number of use sites referencing it.
    pub references: usize,
}

/// A playlist item that does not resolve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedPlaylistItem {
    /// Playlist path in the tree (`"Folder/Playlist"`).
    pub playlist: String,
    /// Item name.
    pub item: String,
    /// The referenced document path as written.
    pub document_path: String,
    /// Why it did not resolve.
    pub reason: String,
}

/// Dangling arrangement references.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DanglingArrangements {
    /// Presentations whose `selectedArrangement` does not resolve (became `None`).
    pub selected_arrangement: usize,
    /// Playlist items whose arrangement does not resolve (became `None`, original in provenance).
    pub playlist_items: usize,
}

/// Dangling global group links (kept as `Link { id, name }`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DanglingGroupLinks {
    /// Number of sections with a link that does not resolve.
    pub count: usize,
    /// Distinct linked group names (`applicationGroupName`) → occurrences.
    pub names: BTreeMap<String, usize>,
}

/// How available a font is likely to be on a Linux render server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FontAvailability {
    /// Proprietary (Microsoft / Apple) font, not installable on Linux by default.
    Proprietary,
    /// Free font that is commonly missing from minimal installs.
    CommonlyMissing,
    /// Free and widely available.
    Free,
    /// Not classified.
    #[default]
    Unknown,
}

/// One font used by the imported content.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FontReportEntry {
    /// Family.
    pub family: String,
    /// Face as the source names it (may be empty).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub face: String,
    /// PostScript name (may be empty).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub postscript_name: String,
    /// CSS weight.
    pub weight: u16,
    /// Italic.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    /// Number of uses.
    pub uses: usize,
    /// Availability class.
    pub availability: FontAvailability,
    /// Suggested open substitute family (also emitted in the suggested substitution table).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub substitute: Option<String>,
}

/// CCLI title differs from the presentation name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CcliTitleMismatch {
    /// Document.
    pub document: String,
    /// Presentation name.
    pub presentation: String,
    /// CCLI song title.
    pub ccli_title: String,
}

/// A byte-identical theme that was skipped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateTheme {
    /// Theme folder that was kept.
    pub kept: String,
    /// Theme folder that was skipped.
    pub duplicate: String,
}

/// Cues outside every cue group in one presentation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UngroupedCues {
    /// Document.
    pub document: String,
    /// Number of cues put into the extra "Ungrouped" section.
    pub cues: usize,
}

/// Document counts of the bundle (computed at the end).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(missing_docs)]
pub struct ImportCounts {
    pub libraries: usize,
    pub presentations: usize,
    pub sections: usize,
    pub slides: usize,
    pub elements: usize,
    pub slide_actions: usize,
    pub global_groups: usize,
    pub playlist_folders: usize,
    pub playlists: usize,
    pub playlist_items: usize,
    pub media_bins: usize,
    pub media_assets: usize,
    pub media_assets_missing: usize,
    pub media_roots: usize,
    pub themes: usize,
    pub template_slides: usize,
    pub screens: usize,
    pub looks: usize,
    pub masks: usize,
    pub video_inputs: usize,
    pub audio_inputs: usize,
    pub timers: usize,
    pub stage_layouts: usize,
    pub clear_groups: usize,
    pub props: usize,
    pub messages: usize,
    pub macros: usize,
    pub output_bindings: usize,
    pub input_bindings: usize,
}

/// The import report. See the module docs for who fills what.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportReport {
    /// Document counts.
    pub counts: ImportCounts,
    /// Missing media grouped by root, sorted by root name.
    pub missing_media: Vec<MissingMediaRoot>,
    /// Unresolved playlist items.
    pub unresolved_playlist_items: Vec<UnresolvedPlaylistItem>,
    /// Dangling arrangements.
    pub dangling_arrangements: DanglingArrangements,
    /// Dangling group links.
    pub dangling_group_links: DanglingGroupLinks,
    /// Unsupported action type → occurrences.
    pub unsupported_actions: BTreeMap<String, usize>,
    /// Unmapped feature (dotted source path) → occurrences.
    pub unmapped_features: BTreeMap<String, usize>,
    /// Fonts used, most used first.
    pub fonts: Vec<FontReportEntry>,
    /// CCLI titles that differ from the presentation name.
    pub ccli_title_mismatch: Vec<CcliTitleMismatch>,
    /// Skipped duplicate themes.
    pub duplicate_themes: Vec<DuplicateTheme>,
    /// Presentations with ungrouped cues.
    pub ungrouped_cues: Vec<UngroupedCues>,
    /// Files that failed.
    pub parse_failures: Vec<ParseFailure>,
    /// Free-form warnings in import order.
    pub warnings: Vec<Warning>,
}

impl ImportReport {
    /// Appends a warning.
    pub fn warn(&mut self, category: Category, document: Option<String>, detail: String) {
        self.warnings.push(Warning {
            category,
            document,
            detail,
        });
    }

    /// Records one dangling group link with its source name hint.
    pub fn dangling_group_link(&mut self, name: &str) {
        self.dangling_group_links.count += 1;
        *self
            .dangling_group_links
            .names
            .entry(name.to_string())
            .or_default() += 1;
    }

    /// Fills [`ImportReport::counts`] from the bundle.
    pub fn compute_counts(&mut self, b: &ImportBundle) {
        use model::{MediaStatus, PlaylistNodeKind};
        let slides = || b.presentations.iter().flat_map(|p| p.slides());
        self.counts = ImportCounts {
            libraries: b.libraries.len(),
            presentations: b.presentations.len(),
            sections: b.presentations.iter().map(|p| p.sections.len()).sum(),
            slides: slides().count(),
            elements: slides().map(|s| s.elements.len()).sum(),
            slide_actions: slides().map(|s| s.actions.len()).sum(),
            global_groups: b.global_groups.len(),
            playlist_folders: b
                .playlist_nodes
                .iter()
                .filter(|n| matches!(n.kind, PlaylistNodeKind::Folder))
                .count(),
            playlists: b
                .playlist_nodes
                .iter()
                .filter(|n| matches!(n.kind, PlaylistNodeKind::Playlist { .. }))
                .count(),
            playlist_items: b
                .playlist_nodes
                .iter()
                .map(|n| match &n.kind {
                    PlaylistNodeKind::Playlist { items, .. } => items.len(),
                    _ => 0,
                })
                .sum(),
            media_bins: b.media_bins.len(),
            media_assets: b.media_assets.len(),
            media_assets_missing: b
                .media_assets
                .iter()
                .filter(|a| a.status == MediaStatus::Missing)
                .count(),
            media_roots: b.media_roots.len(),
            themes: b.themes.len(),
            template_slides: b.themes.iter().map(|t| t.slides.len()).sum(),
            screens: b.screens.len(),
            looks: b.looks.len(),
            masks: b.masks.len(),
            video_inputs: b.video_inputs.len(),
            audio_inputs: b.audio_inputs.len(),
            timers: b.timers.len(),
            stage_layouts: b.stage_layouts.len(),
            clear_groups: b.clear_groups.len(),
            props: b.props.len(),
            messages: b.messages.len(),
            macros: b.macros.len(),
            output_bindings: b.output_bindings.len(),
            input_bindings: b.input_bindings.len(),
        };
    }

    /// Warnings per category.
    pub fn warning_counts(&self) -> BTreeMap<Category, usize> {
        let mut m = BTreeMap::new();
        for w in &self.warnings {
            *m.entry(w.category).or_default() += 1;
        }
        m
    }

    /// Human summary (`report.md`).
    pub fn render_markdown(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "# ProPresenter import report\n");

        let _ = writeln!(s, "## Imported\n");
        let _ = writeln!(s, "| documents | count |\n|---|---:|");
        let counts = serde_json::to_value(&self.counts).unwrap_or_default();
        if let Some(obj) = counts.as_object() {
            for (k, v) in obj {
                let _ = writeln!(s, "| {k} | {v} |");
            }
        }

        let _ = writeln!(s, "\n## Summary by category\n");
        let _ = writeln!(s, "| category | count |\n|---|---:|");
        let missing: usize = self.missing_media.iter().map(|r| r.files.len()).sum();
        let rows = [
            ("missingMedia (files)", missing),
            (
                "unresolvedPlaylistItems",
                self.unresolved_playlist_items.len(),
            ),
            (
                "danglingArrangements (selected)",
                self.dangling_arrangements.selected_arrangement,
            ),
            (
                "danglingArrangements (playlist items)",
                self.dangling_arrangements.playlist_items,
            ),
            ("danglingGroupLinks", self.dangling_group_links.count),
            (
                "unsupportedActions",
                self.unsupported_actions.values().sum(),
            ),
            ("unmappedFeatures", self.unmapped_features.values().sum()),
            ("fonts", self.fonts.len()),
            ("ccliTitleMismatch", self.ccli_title_mismatch.len()),
            ("duplicateThemes", self.duplicate_themes.len()),
            (
                "ungroupedCues",
                self.ungrouped_cues.iter().map(|u| u.cues).sum(),
            ),
            ("parseFailures", self.parse_failures.len()),
            ("warnings", self.warnings.len()),
        ];
        for (k, v) in rows {
            let _ = writeln!(s, "| {k} | {v} |");
        }

        if !self.parse_failures.is_empty() {
            let _ = writeln!(s, "\n## Parse failures\n");
            for f in &self.parse_failures {
                let _ = writeln!(s, "- `{}`: {}", f.file, f.error);
            }
        }

        if !self.missing_media.is_empty() {
            let _ = writeln!(s, "\n## Missing media (relink report)\n");
            for r in &self.missing_media {
                let refs: usize = r.files.iter().map(|f| f.references).sum();
                let _ = writeln!(
                    s,
                    "### {} ({} files, {} references)\n",
                    r.name,
                    r.files.len(),
                    refs
                );
                for h in &r.hints {
                    let parts: Vec<String> = [
                        h.drive_letter.as_ref().map(|d| format!("drive {d}:")),
                        h.volume_name.as_ref().map(|v| format!("volume \"{v}\"")),
                        h.unc_prefix.as_ref().map(|u| format!("UNC {u}")),
                        h.path_prefix.as_ref().map(|p| format!("prefix {p}")),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    if !parts.is_empty() {
                        let _ = writeln!(s, "- hint: {}", parts.join(", "));
                    }
                }
                for f in &r.files {
                    let _ = writeln!(s, "- `{}` ({}×)", f.path, f.references);
                }
                s.push('\n');
            }
        }

        if !self.unresolved_playlist_items.is_empty() {
            let _ = writeln!(s, "\n## Unresolved playlist items\n");
            for u in &self.unresolved_playlist_items {
                let _ = writeln!(
                    s,
                    "- {} / {}: `{}` ({})",
                    u.playlist, u.item, u.document_path, u.reason
                );
            }
        }

        if self.dangling_group_links.count > 0 {
            let _ = writeln!(
                s,
                "\n## Dangling group links ({})\n",
                self.dangling_group_links.count
            );
            for (name, n) in &self.dangling_group_links.names {
                let _ = writeln!(s, "- {name}: {n}");
            }
        }

        let map_section = |s: &mut String, title: &str, m: &BTreeMap<String, usize>| {
            if !m.is_empty() {
                let _ = writeln!(s, "\n## {title}\n");
                for (k, v) in m {
                    let _ = writeln!(s, "- `{k}`: {v}");
                }
            }
        };
        map_section(&mut s, "Unsupported actions", &self.unsupported_actions);
        map_section(
            &mut s,
            "Unmapped features (kept in import.unmapped or dropped)",
            &self.unmapped_features,
        );

        if !self.fonts.is_empty() {
            let _ = writeln!(s, "\n## Fonts\n");
            let _ = writeln!(
                s,
                "| family | face | PostScript | weight | italic | uses | availability | substitute |\n|---|---|---|---:|---|---:|---|---|"
            );
            for f in &self.fonts {
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {} | {} | {} | {:?} | {} |",
                    f.family,
                    f.face,
                    f.postscript_name,
                    f.weight,
                    if f.italic { "yes" } else { "" },
                    f.uses,
                    f.availability,
                    f.substitute.as_deref().unwrap_or("")
                );
            }
        }

        if !self.ccli_title_mismatch.is_empty() {
            let _ = writeln!(s, "\n## CCLI title differs from presentation name\n");
            for c in &self.ccli_title_mismatch {
                let _ = writeln!(
                    s,
                    "- `{}`: \"{}\" vs CCLI \"{}\"",
                    c.document, c.presentation, c.ccli_title
                );
            }
        }

        if !self.duplicate_themes.is_empty() {
            let _ = writeln!(s, "\n## Duplicate themes (skipped)\n");
            for d in &self.duplicate_themes {
                let _ = writeln!(s, "- `{}` = `{}` (kept)", d.duplicate, d.kept);
            }
        }

        if !self.ungrouped_cues.is_empty() {
            let _ = writeln!(s, "\n## Ungrouped cues\n");
            for u in &self.ungrouped_cues {
                let _ = writeln!(s, "- `{}`: {}", u.document, u.cues);
            }
        }

        if !self.warnings.is_empty() {
            let _ = writeln!(s, "\n## Warnings\n");
            let mut by_cat: BTreeMap<Category, Vec<&Warning>> = BTreeMap::new();
            for w in &self.warnings {
                by_cat.entry(w.category).or_default().push(w);
            }
            for (cat, ws) in by_cat {
                let _ = writeln!(s, "### {} ({})\n", cat.as_str(), ws.len());
                for w in ws {
                    match &w.document {
                        Some(d) => {
                            let _ = writeln!(s, "- `{d}`: {}", w.detail);
                        }
                        None => {
                            let _ = writeln!(s, "- {}", w.detail);
                        }
                    }
                }
                s.push('\n');
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_lists_categories() {
        let mut r = ImportReport::default();
        r.warn(Category::Other, Some("a/b".into()), "note".into());
        r.dangling_group_link("Verse 1");
        r.parse_failures.push(ParseFailure {
            file: "x".into(),
            error: "bad".into(),
        });
        let md = r.render_markdown();
        assert!(md.contains("danglingGroupLinks | 1"));
        assert!(md.contains("### other (1)"));
        assert!(md.contains("`x`: bad"));
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["danglingGroupLinks"]["names"]["Verse 1"], 1);
    }
}
