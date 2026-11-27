//! Font discovery and resolution.
//!
//! Order (first hit wins), see [`FontResolver::resolve`]:
//! 1. **PostScript name** (`"Calibri-Bold"`): the exact face, case-insensitive.
//! 2. **Family + weight + italic**: an installed family with that name; the nearest weight and
//!    style of that family (CSS matching, done by fontdb).
//! 3. **Substitution table** (machine-local [`model::FontSubstitutionTable`]): its replacement
//!    family, if installed, with the requested weight and italic.
//! 4. **Fallback family** (default `DejaVu Sans`), then any installed sans-serif face.
//!
//! Only step 1 and 2 are exact. Every other outcome is recorded as a [`FontIssue`] with a use
//! count, so the server can report "Calibri → DejaVu Sans (fallback), 120 uses".
//!
//! When the chosen face is lighter than requested (bold wanted, regular installed) or upright
//! while italic is wanted, glyphs are emboldened / slanted synthetically and the issue says so.

use model::{FontRef, FontSubstitutionTable};
use std::collections::HashMap;
use std::path::PathBuf;

/// How a requested font was satisfied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontMatch {
    /// The exact face, by PostScript name.
    PostScript,
    /// The requested family is installed (nearest weight / style).
    Family,
    /// Replaced through the machine's substitution table.
    Substituted,
    /// Neither the font nor a substitution was available: the fallback family was used.
    Fallback,
}

/// A request that did not resolve exactly (or needed synthetic styling).
#[derive(Clone, Debug, PartialEq)]
pub struct FontIssue {
    /// What the document asked for.
    pub requested: FontRef,
    /// Family actually used.
    pub used_family: String,
    /// How it was resolved.
    pub matched: FontMatch,
    /// Bold was synthesized (no heavy enough face).
    pub synthetic_bold: bool,
    /// Italic was synthesized (no italic face).
    pub synthetic_italic: bool,
    /// Number of text runs rendered with this resolution since the last report.
    pub uses: u64,
}

/// The face picked for one request.
#[derive(Clone, Debug)]
pub(crate) struct Resolved {
    pub family: String,
    pub weight: fontdb::Weight,
    pub style: fontdb::Style,
    pub stretch: fontdb::Stretch,
    pub face: fontdb::ID,
    pub matched: FontMatch,
    pub faux_bold: bool,
    pub faux_italic: bool,
}

pub(crate) struct FontResolver {
    pub substitutions: FontSubstitutionTable,
    pub fallback_family: String,
    cache: HashMap<FontRef, Option<Resolved>>,
    issues: HashMap<FontRef, FontIssue>,
}

/// Builds the font database: system fonts plus extra directories (e.g. `<data>/fonts`).
/// On Windows, the system fonts include the fonts every user installed for themselves.
pub(crate) fn load_database(extra_dirs: &[PathBuf], system: bool) -> fontdb::Database {
    let mut db = fontdb::Database::new();
    if system {
        db.load_system_fonts();
        #[cfg(windows)]
        for d in windows_user_font_dirs() {
            db.load_fonts_dir(d);
        }
    }
    for d in extra_dirs {
        if d.is_dir() {
            db.load_fonts_dir(d);
        } else if d.is_file()
            && let Err(e) = db.load_font_file(d)
        {
            log::warn!("render: cannot load font {}: {e}", d.display());
        }
    }
    db
}

/// The per-user font folders of every profile under `profiles` (`C:\Users`) except `own`
/// (fontdb loads the running user's already). Since Windows 10 1809, a font installed with a
/// double click goes to the installing user's folder only, which a server running as
/// SYSTEM (the startup task) would never see, while Oriel, in the operator's session, does.
#[cfg_attr(not(windows), allow(dead_code))] // used on Windows (and in tests)
fn user_font_dirs(profiles: &std::path::Path, own: Option<&std::path::Path>) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(profiles) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let profile = e.path();
        if own.is_some_and(|o| same_dir(o, &profile)) {
            continue;
        }
        for roam in ["Local", "Roaming"] {
            let d = ["AppData", roam, "Microsoft", "Windows", "Fonts"]
                .iter()
                .fold(profile.clone(), |p, c| p.join(c));
            if d.is_dir() {
                out.push(d);
            }
        }
    }
    out.sort();
    out
}

#[cfg_attr(not(windows), allow(dead_code))] // used on Windows (and in tests)
fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// [`user_font_dirs`] of this Windows machine: the profiles folder is the one holding the
/// Public profile (`%PUBLIC%`), else `%SystemDrive%\Users`.
#[cfg(windows)]
fn windows_user_font_dirs() -> Vec<PathBuf> {
    let env = |v: &str| std::env::var_os(v).map(PathBuf::from);
    let profiles = env("PUBLIC")
        .and_then(|p| p.parent().map(PathBuf::from))
        .or_else(|| {
            std::env::var("SystemDrive")
                .ok()
                .map(|d| PathBuf::from(format!("{d}\\Users")))
        });
    let Some(profiles) = profiles else {
        return Vec::new();
    };
    let dirs = user_font_dirs(&profiles, env("USERPROFILE").as_deref());
    if !dirs.is_empty() {
        log::info!(
            "render: also loading the fonts of {} user font folders",
            dirs.len()
        );
    }
    dirs
}

fn family_names(face: &fontdb::FaceInfo) -> impl Iterator<Item = &str> {
    face.families.iter().map(|(n, _)| n.as_str())
}

impl FontResolver {
    pub fn new(substitutions: FontSubstitutionTable, fallback_family: String) -> Self {
        FontResolver {
            substitutions,
            fallback_family,
            cache: HashMap::new(),
            issues: HashMap::new(),
        }
    }

    pub fn set_substitutions(&mut self, table: FontSubstitutionTable) {
        self.substitutions = table;
        self.cache.clear();
    }

    /// Resolves a font and counts the use. `None` only when no font is installed at all.
    pub fn resolve(&mut self, db: &fontdb::Database, font: &FontRef) -> Option<Resolved> {
        if !self.cache.contains_key(font) {
            let r = self.resolve_uncached(db, font);
            self.cache.insert(font.clone(), r);
        }
        let r = self.cache.get(font).cloned().flatten();
        if let Some(r) = &r
            && (r.matched >= FontMatch::Substituted || r.faux_bold || r.faux_italic)
        {
            self.issues
                .entry(font.clone())
                .or_insert_with(|| FontIssue {
                    requested: font.clone(),
                    used_family: r.family.clone(),
                    matched: r.matched,
                    synthetic_bold: r.faux_bold,
                    synthetic_italic: r.faux_italic,
                    uses: 0,
                })
                .uses += 1;
        }
        r
    }

    /// Issues since the last call (cleared).
    pub fn take_issues(&mut self) -> Vec<FontIssue> {
        let mut v: Vec<FontIssue> = self.issues.drain().map(|(_, v)| v).collect();
        v.sort_by(|a, b| {
            b.uses
                .cmp(&a.uses)
                .then(a.requested.family.cmp(&b.requested.family))
        });
        v
    }

    /// Issues so far (not cleared).
    pub fn issues(&self) -> Vec<FontIssue> {
        let mut v: Vec<FontIssue> = self.issues.values().cloned().collect();
        v.sort_by(|a, b| {
            b.uses
                .cmp(&a.uses)
                .then(a.requested.family.cmp(&b.requested.family))
        });
        v
    }

    fn resolve_uncached(&self, db: &fontdb::Database, font: &FontRef) -> Option<Resolved> {
        let want_weight = fontdb::Weight(font.weight.clamp(1, 1000));
        let want_style = if font.italic {
            fontdb::Style::Italic
        } else {
            fontdb::Style::Normal
        };
        // 1. PostScript name.
        if let Some(ps) = font.postscript_name.as_deref().filter(|s| !s.is_empty())
            && let Some(face) = db
                .faces()
                .find(|f| f.post_script_name.eq_ignore_ascii_case(ps))
        {
            return Some(self.finish(face, FontMatch::PostScript, want_weight, want_style, false));
        }
        // 2. Family.
        if let Some(r) = self.query(db, &font.family, want_weight, want_style, FontMatch::Family) {
            return Some(r);
        }
        // 3. Substitution table.
        if let Some(rep) = self.substitutions.substitute(font)
            && let Some(r) = self.query(db, rep, want_weight, want_style, FontMatch::Substituted)
        {
            return Some(r);
        }
        // 4. Fallback family, then any sans-serif, then anything.
        if let Some(r) = self.query(
            db,
            &self.fallback_family,
            want_weight,
            want_style,
            FontMatch::Fallback,
        ) {
            return Some(r);
        }
        let id = db.query(&fontdb::Query {
            families: &[fontdb::Family::SansSerif],
            weight: want_weight,
            stretch: fontdb::Stretch::Normal,
            style: want_style,
        });
        let face = id
            .and_then(|id| db.face(id))
            .or_else(|| db.faces().next())?;
        Some(self.finish(face, FontMatch::Fallback, want_weight, want_style, true))
    }

    fn query(
        &self,
        db: &fontdb::Database,
        family: &str,
        weight: fontdb::Weight,
        style: fontdb::Style,
        matched: FontMatch,
    ) -> Option<Resolved> {
        // fontdb matches family names exactly; find the installed spelling case-insensitively.
        let name = db
            .faces()
            .flat_map(family_names)
            .find(|n| n.eq_ignore_ascii_case(family))?
            .to_string();
        let id = db.query(&fontdb::Query {
            families: &[fontdb::Family::Name(&name)],
            weight,
            stretch: fontdb::Stretch::Normal,
            style,
        })?;
        let face = db.face(id)?;
        Some(self.finish(face, matched, weight, style, true))
    }

    fn finish(
        &self,
        face: &fontdb::FaceInfo,
        matched: FontMatch,
        want_weight: fontdb::Weight,
        want_style: fontdb::Style,
        allow_synthetic: bool,
    ) -> Resolved {
        let family = family_names(face).next().unwrap_or("").to_string();
        Resolved {
            family,
            weight: face.weight,
            style: face.style,
            stretch: face.stretch,
            face: face.id,
            matched,
            faux_bold: allow_synthetic
                && want_weight.0 >= 600
                && face.weight.0 + 150 < want_weight.0,
            faux_italic: allow_synthetic
                && want_style != fontdb::Style::Normal
                && face.style == fontdb::Style::Normal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// X4-5: every other profile's per-user font folders, not the running user's own (fontdb
    /// loads those) and not profiles without any.
    #[test]
    fn every_users_font_folder_is_found() {
        let root = std::env::temp_dir().join(format!("render-userfonts-{}", std::process::id()));
        let fonts = |user: &str, roam: &str| {
            ["AppData", roam, "Microsoft", "Windows", "Fonts"]
                .iter()
                .fold(root.join(user), |p, c| p.join(c))
        };
        for d in [
            fonts("anna", "Local"),
            fonts("anna", "Roaming"),
            fonts("ben", "Local"),
            fonts("me", "Local"),
        ] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::create_dir_all(root.join("nofonts").join("AppData")).unwrap();
        assert_eq!(
            user_font_dirs(&root, Some(&root.join("me"))),
            vec![
                fonts("anna", "Local"),
                fonts("anna", "Roaming"),
                fonts("ben", "Local")
            ]
        );
        assert_eq!(user_font_dirs(&root, None).len(), 4);
        assert!(user_font_dirs(&root.join("missing"), None).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
