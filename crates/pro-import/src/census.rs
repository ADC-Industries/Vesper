//! Census invariants of the reference backup (the importer's quality bar, IMPORTER.md).
//!
//! `pp-import --check` (and any test run with `PP_BACKUP` set) imports the real backup and
//! compares these aggregate counts, derived from a census of the backup (with the
//! critic's corrections), against the bundle. Only counts live here, never backup content.

use std::collections::BTreeSet;

use model::{ActionKind, MediaBinKind, PlaylistItemKind, PlaylistNodeKind, PlaylistPurpose};

use crate::bundle::ImportBundle;

/// One expected count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expectation {
    /// What is counted.
    pub name: &'static str,
    /// Census value.
    pub expected: usize,
    /// Value in the bundle.
    pub actual: usize,
}

impl Expectation {
    /// True when the bundle matches the census.
    pub fn holds(&self) -> bool {
        self.expected == self.actual
    }
}

/// Every census expectation evaluated against `b`.
pub fn expectations(b: &ImportBundle) -> Vec<Expectation> {
    let slides: Vec<_> = b.presentations.iter().flat_map(|p| p.slides()).collect();
    let sections: Vec<_> = b.presentations.iter().flat_map(|p| &p.sections).collect();
    let groups: BTreeSet<_> = b.global_groups.iter().map(|g| g.id).collect();
    let links: Vec<_> = sections
        .iter()
        .filter_map(|s| s.global_group.as_ref())
        .collect();

    let playlists = |purpose: PlaylistPurpose| {
        b.playlist_nodes.iter().filter_map(move |n| match &n.kind {
            PlaylistNodeKind::Playlist { purpose: p, items } if *p == purpose => Some(items),
            _ => None,
        })
    };
    let service_items: Vec<_> = playlists(PlaylistPurpose::Service).flatten().collect();
    let pres_items: Vec<_> = service_items
        .iter()
        .filter_map(|i| match &i.kind {
            PlaylistItemKind::Presentation {
                presentation,
                arrangement,
                unresolved,
            } => Some((presentation, arrangement, unresolved)),
            _ => None,
        })
        .collect();
    let presentation_ids: BTreeSet<_> = b.presentations.iter().map(|p| p.id).collect();

    let e = |name, expected, actual| Expectation {
        name,
        expected,
        actual,
    };
    vec![
        e("libraries", 6, b.libraries.len()),
        e("presentations", 407, b.presentations.len()),
        e("sections (cue groups)", 1579, sections.len()),
        e("slides (cues)", 3980, slides.len()),
        e(
            "elements on presentation slides",
            4448,
            slides.iter().map(|s| s.elements.len()).sum(),
        ),
        e(
            "media actions on presentation slides",
            314,
            slides
                .iter()
                .flat_map(|s| &s.actions)
                .filter(|a| matches!(a.kind, ActionKind::Media(_)))
                .count(),
        ),
        e(
            "unsupported actions on presentation slides",
            0,
            slides
                .iter()
                .flat_map(|s| &s.actions)
                .filter(|a| matches!(a.kind, ActionKind::Unsupported { .. }))
                .count(),
        ),
        e(
            "disabled slides",
            4,
            slides.iter().filter(|s| !s.enabled).count(),
        ),
        e(
            "arrangements",
            1,
            b.presentations.iter().map(|p| p.arrangements.len()).sum(),
        ),
        e(
            "selected arrangements kept",
            0,
            b.presentations
                .iter()
                .filter(|p| p.selected_arrangement.is_some())
                .count(),
        ),
        e("global groups", 27, b.global_groups.len()),
        e(
            "section group links that resolve",
            1069,
            links.iter().filter(|l| groups.contains(&l.id)).count(),
        ),
        e(
            "section group links that dangle",
            254,
            links.iter().filter(|l| !groups.contains(&l.id)).count(),
        ),
        e(
            "sections without group link",
            256,
            sections.iter().filter(|s| s.global_group.is_none()).count(),
        ),
        e(
            "playlist folders",
            2,
            b.playlist_nodes
                .iter()
                .filter(|n| matches!(n.kind, PlaylistNodeKind::Folder))
                .count(),
        ),
        e(
            "service playlists",
            179,
            playlists(PlaylistPurpose::Service).count(),
        ),
        e(
            "audio playlists",
            2,
            playlists(PlaylistPurpose::Audio).count(),
        ),
        e("service playlist items", 1088, service_items.len()),
        e("playlist presentation items", 958, pres_items.len()),
        e(
            "playlist presentation items that resolve",
            958,
            pres_items
                .iter()
                .filter(|(p, _, u)| u.is_none() && presentation_ids.contains(&p.id))
                .count(),
        ),
        e(
            "playlist presentation items with an arrangement",
            1,
            pres_items.iter().filter(|(_, a, _)| a.is_some()).count(),
        ),
        e(
            "playlist media items",
            127,
            service_items
                .iter()
                .filter(|i| matches!(i.kind, PlaylistItemKind::Media(_)))
                .count(),
        ),
        e(
            "playlist video input items",
            3,
            service_items
                .iter()
                .filter(|i| matches!(i.kind, PlaylistItemKind::VideoInput(_)))
                .count(),
        ),
        e("media bins (smart folders)", 23, b.media_bins.len()),
        e(
            "media bin cached items",
            397,
            b.media_bins
                .iter()
                .map(|m| match &m.kind {
                    MediaBinKind::Watched { last_scan, .. } => last_scan.len(),
                    MediaBinKind::Manual { items } => items.len(),
                    MediaBinKind::Unknown => 0,
                })
                .sum(),
        ),
        e(
            "themes before de-duplication",
            17,
            b.themes.len() + b.report.duplicate_themes.len(),
        ),
        e("themes after de-duplication", 16, b.themes.len()),
        e(
            "template slides (after de-duplication)",
            144,
            b.themes.iter().map(|t| t.slides.len()).sum(),
        ),
        e("timers", 4, b.timers.len()),
        e("screens", 4, b.screens.len()),
        e("output binding suggestions", 3, b.output_bindings.len()),
        e("looks", 1, b.looks.len()),
        e("masks", 1, b.masks.len()),
        e("video inputs", 3, b.video_inputs.len()),
        e("audio inputs", 1, b.audio_inputs.len()),
        e("stage layouts", 1, b.stage_layouts.len()),
        e("clear groups", 1, b.clear_groups.len()),
        e("props", 1, b.props.len()),
        e("messages", 0, b.messages.len()),
        e("macros", 0, b.macros.len()),
        e(
            "ungrouped cues",
            0,
            b.report.ungrouped_cues.iter().map(|u| u.cues).sum(),
        ),
        e("parse failures", 0, b.report.parse_failures.len()),
    ]
}

/// The expectations that do not hold (empty = the bundle matches the census).
pub fn check(b: &ImportBundle) -> Vec<Expectation> {
    expectations(b).into_iter().filter(|e| !e.holds()).collect()
}
