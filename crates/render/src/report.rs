//! What the renderer could not render faithfully: font fallbacks, missing media, unsupported
//! features. Collected while rendering and handed to the server with
//! [`Renderer::take_report`](crate::Renderer::take_report).

use crate::fonts::FontIssue;
use crate::gpu::AdapterSummary;
use std::collections::BTreeMap;

/// A media file that could not be drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaIssue {
    /// File path, decoded-image key or asset ID.
    pub source: String,
    /// What went wrong ("file not found", "video playback is not supported yet", ...).
    pub problem: String,
    /// How many times it was requested since the last report.
    pub uses: u64,
}

/// Rendering diagnostics since the last [`Renderer::take_report`](crate::Renderer::take_report).
#[derive(Clone, Debug)]
pub struct RenderReport {
    /// The GPU adapter in use.
    pub adapter: AdapterSummary,
    /// Fonts that were substituted, fell back or needed synthetic bold / italic.
    pub fonts: Vec<FontIssue>,
    /// Media that could not be drawn.
    pub media: Vec<MediaIssue>,
    /// Features skipped by this renderer version, with counts.
    pub unsupported: Vec<(String, u64)>,
}

impl RenderReport {
    /// True when everything rendered as authored.
    pub fn is_clean(&self) -> bool {
        self.fonts.is_empty() && self.media.is_empty() && self.unsupported.is_empty()
    }
}

impl std::fmt::Display for RenderReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "adapter: {}", self.adapter)?;
        for i in &self.fonts {
            let mut extra = Vec::new();
            if i.synthetic_bold {
                extra.push("synthetic bold");
            }
            if i.synthetic_italic {
                extra.push("synthetic italic");
            }
            writeln!(
                f,
                "font: {} {}{}{} -> {} ({:?}{}{}), {} uses",
                i.requested.family,
                i.requested.weight,
                if i.requested.italic { " italic" } else { "" },
                i.requested
                    .postscript_name
                    .as_deref()
                    .map(|p| format!(" [{p}]"))
                    .unwrap_or_default(),
                i.used_family,
                i.matched,
                if extra.is_empty() { "" } else { ", " },
                extra.join(", "),
                i.uses
            )?;
        }
        for m in &self.media {
            writeln!(f, "media: {}: {} ({} uses)", m.source, m.problem, m.uses)?;
        }
        for (u, n) in &self.unsupported {
            writeln!(f, "unsupported: {u} ({n}x)")?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct ReportState {
    media: BTreeMap<(String, String), u64>,
    unsupported: BTreeMap<String, u64>,
}

impl ReportState {
    pub fn media_issue(&mut self, source: &str, problem: &str) {
        *self
            .media
            .entry((source.to_string(), problem.to_string()))
            .or_default() += 1;
    }

    pub fn unsupported(&mut self, what: &str) {
        if !what.is_empty() {
            *self.unsupported.entry(what.to_string()).or_default() += 1;
        }
    }

    pub fn take(&mut self) -> (Vec<MediaIssue>, Vec<(String, u64)>) {
        let media = std::mem::take(&mut self.media)
            .into_iter()
            .map(|((source, problem), uses)| MediaIssue {
                source,
                problem,
                uses,
            })
            .collect();
        let unsupported = std::mem::take(&mut self.unsupported).into_iter().collect();
        (media, unsupported)
    }

    pub fn snapshot(&self) -> (Vec<MediaIssue>, Vec<(String, u64)>) {
        let media = self
            .media
            .iter()
            .map(|((source, problem), uses)| MediaIssue {
                source: source.clone(),
                problem: problem.clone(),
                uses: *uses,
            })
            .collect();
        let unsupported = self
            .unsupported
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        (media, unsupported)
    }
}
