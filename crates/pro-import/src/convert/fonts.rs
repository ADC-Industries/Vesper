//! **Area: slides.** Fonts: source font → [`FontRef`], usage counting, the font report and the
//! suggested machine-local [`FontSubstitutionTable`].
//!
//! ## Owns
//! - [`font_ref`]: `rv.data.Font` → family + weight + italic + `postscriptName = font.name`.
//!   Family / face come from `family` / `face`; when missing, they are parsed from the
//!   PostScript name ([`font_ref_from_postscript`]: `"Calibri-BoldItalic"`, `"ArialMT"`,
//!   `"TimesNewRomanPS-BoldItalicMT"`, `"HelveticaNeue-Light"`).
//!   ProPresenter's family / face split is inconsistent (census: `("Calibri", "Bold")` next to
//!   `("Calibri-Bold", "Regular")` and `("ArialMT", "Regular")` for the same fonts), so:
//!   - a `family` that is the PostScript name, or looks like one (`-`, no space), is ignored and
//!     the PostScript name is parsed instead;
//!   - a `face` that says nothing ("Regular", empty) defers to the PostScript face part
//!     (`"Helvetica-Bold"` with face "Regular" is bold);
//!   - a family that repeats the face (`"Yu Gothic UI Light"` for `YuGothicUI-Light`) is trimmed
//!     to the PostScript family;
//!   - the `bold` / `italic` flags only ever add weight / slant.
//! - [`weight_and_italic_from_face`]: Thin 100, ExtraLight/UltraLight 200, Light 300,
//!   Regular/Roman/Book/Normal 400, Medium 500, SemiBold/DemiBold 600, Bold 700,
//!   ExtraBold/Heavy 800, Black 900; Italic/Oblique → italic.
//! - [`record_font_use`]: counts every use in `ctx.lookups.fonts` (called by `text.rs` for box
//!   defaults and RTF runs).
//! - [`advice`]: availability class + suggested open substitute (Calibri → Carlito, Arial →
//!   Liberation Sans, Helvetica* → Liberation Sans, Times New Roman → Liberation Serif, Courier
//!   New → Liberation Mono, Tahoma → DejaVu Sans, Book Antiqua → TeX Gyre Pagella, Yu Gothic →
//!   Noto Sans CJK JP, …).
//! - [`font_report`]: last phase. Fills `ctx.report.fonts` (most used first, ties by name) and
//!   sets `ctx.out.font_substitutions` (one rule per family with a substitute, sorted).
//!
//! ## Report grouping
//! Uses are counted per source identity ([`FontKey`]); RTF runs only know the PostScript name.
//! The report merges keys that resolve to the same face (family + weight + italic + PostScript
//! name), and resolves a PostScript-only key through the family / face that the same PostScript
//! name has elsewhere in the backup (so `Calibri-Bold` from RTF and `("Calibri", "Bold")` from
//! `attributes` are one entry).
//!
//! ## May call
//! `ImportContext` helpers only.

use std::collections::BTreeMap;

use model::{FontRef, FontSubstitution, FontSubstitutionTable};

use crate::context::{FontKey, ImportContext};
use crate::pb::rv::data as pp;
use crate::report::{FontAvailability, FontReportEntry};

/// Availability and substitute for a family.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontAdvice {
    /// Availability class.
    pub availability: FontAvailability,
    /// Suggested installed replacement family.
    pub substitute: Option<&'static str>,
}

/// `rv.data.Font` → [`FontRef`] (see the module docs).
pub fn font_ref(font: &pp::Font) -> FontRef {
    let mut r = font_ref_from_parts(&font.name, &font.family, &font.face);
    if font.bold && r.weight < 600 {
        r.weight = 700;
    }
    if font.italic {
        r.italic = true;
    }
    r
}

/// [`FontRef`] from a source PostScript name, family and face (any may be empty). The `bold` /
/// `italic` flags of `rv.data.Font` are applied by [`font_ref`].
pub fn font_ref_from_parts(postscript_name: &str, family: &str, face: &str) -> FontRef {
    let ps = postscript_name.trim();
    let family = family.trim();
    let face = face.trim();
    let from_ps = (!ps.is_empty()).then(|| font_ref_from_postscript(ps));
    let looks_like_ps = family.eq_ignore_ascii_case(ps)
        || (!family.contains(' ') && (family.contains('-') || ends_with_mt_suffix(family)));
    if family.is_empty() || looks_like_ps {
        // Family missing or itself a PostScript name: parse the PostScript name (or the family
        // when there is no PostScript name).
        let mut r = match &from_ps {
            Some(r) => r.clone(),
            None => font_ref_from_postscript(family),
        };
        let (w, i) = weight_and_italic_from_face(face);
        if w != 400 {
            r.weight = w;
        }
        r.italic |= i;
        r.postscript_name = (!ps.is_empty()).then(|| ps.to_string());
        return r;
    }
    let (mut weight, mut italic) = weight_and_italic_from_face(face);
    if weight == 400
        && !italic
        && let Some(p) = &from_ps
    {
        // Face says nothing ("Regular", empty): the PostScript face part decides.
        weight = p.weight;
        italic = p.italic;
    }
    let mut family = family.to_string();
    if let Some(p) = &from_ps
        && family != p.family
        && let Some(trimmed) = strip_style_words(&family)
        && squash(&trimmed) == squash(&p.family)
    {
        family = trimmed;
    }
    FontRef {
        family,
        weight,
        italic,
        postscript_name: (!ps.is_empty()).then(|| ps.to_string()),
    }
}

/// [`FontRef`] parsed from a PostScript name alone (RTF `\fonttbl` entries).
pub fn font_ref_from_postscript(postscript_name: &str) -> FontRef {
    let ps = postscript_name.trim();
    let (fam_part, face_part) = match ps.split_once('-') {
        Some((f, s)) => (f, s),
        None => (ps, ""),
    };
    let fam_part = strip_vendor_suffix(fam_part);
    let face_part = strip_vendor_suffix(face_part);
    let (weight, italic) = weight_and_italic_from_face(face_part);
    FontRef {
        family: family_from_postscript_part(fam_part),
        weight,
        italic,
        postscript_name: (!ps.is_empty()).then(|| ps.to_string()),
    }
}

/// `"ArialMT"` → `"Arial"`, `"TimesNewRomanPS"` → `"TimesNewRoman"`, `"BoldMT"` → `"Bold"`.
fn strip_vendor_suffix(s: &str) -> &str {
    for suffix in ["PSMT", "MT", "PS"] {
        if let Some(rest) = s.strip_suffix(suffix)
            && rest.chars().last().is_some_and(|c| c.is_ascii_lowercase())
        {
            return rest;
        }
    }
    s
}

fn ends_with_mt_suffix(s: &str) -> bool {
    strip_vendor_suffix(s).len() != s.len()
}

/// Family display names whose PostScript spelling does not split on case changes.
const FAMILY_EXCEPTIONS: &[(&str, &str)] = &[
    ("DejaVuSans", "DejaVu Sans"),
    ("DejaVuSerif", "DejaVu Serif"),
    ("DejaVuSansMono", "DejaVu Sans Mono"),
    ("MSGothic", "MS Gothic"),
    ("MSMincho", "MS Mincho"),
];

/// `"TimesNewRoman"` → `"Times New Roman"`, `"YuGothicUI"` → `"Yu Gothic UI"`.
fn family_from_postscript_part(part: &str) -> String {
    if let Some((_, name)) = FAMILY_EXCEPTIONS.iter().find(|(k, _)| *k == part) {
        return (*name).to_string();
    }
    let chars: Vec<char> = part.chars().collect();
    let mut out = String::with_capacity(part.len() + 4);
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 && c.is_uppercase() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_ascii_digit() || (prev.is_uppercase() && next_lower) {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

/// Lowercase without spaces / hyphens / underscores.
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .flat_map(char::to_lowercase)
        .collect()
}

const STYLE_WORDS: &[&str] = &[
    "thin",
    "hairline",
    "extralight",
    "ultralight",
    "light",
    "regular",
    "roman",
    "book",
    "normal",
    "medium",
    "semibold",
    "demibold",
    "bold",
    "extrabold",
    "ultrabold",
    "heavy",
    "black",
    "italic",
    "oblique",
];

/// The family with trailing style words removed (`"Yu Gothic UI Light"` → `"Yu Gothic UI"`);
/// `None` when nothing was removed or nothing would remain.
fn strip_style_words(family: &str) -> Option<String> {
    let mut words: Vec<&str> = family.split_whitespace().collect();
    let n = words.len();
    while words.len() > 1
        && words
            .last()
            .is_some_and(|w| STYLE_WORDS.contains(&w.to_ascii_lowercase().as_str()))
    {
        words.pop();
    }
    (words.len() < n).then(|| words.join(" "))
}

/// CSS weight and italic flag from a face name (`"Bold Italic"` → `(700, true)`); unknown faces
/// are regular (400).
pub fn weight_and_italic_from_face(face: &str) -> (u16, bool) {
    let f = squash(face);
    let italic = f.contains("italic") || f.contains("oblique");
    let weight = if f.contains("extralight") || f.contains("ultralight") {
        200
    } else if f.contains("semibold") || f.contains("demibold") {
        600
    } else if f.contains("extrabold") || f.contains("ultrabold") || f.contains("heavy") {
        800
    } else if f.contains("black") {
        900
    } else if f.contains("thin") || f.contains("hairline") {
        100
    } else if f.contains("light") {
        300
    } else if f.contains("medium") {
        500
    } else if f.contains("bold") {
        700
    } else {
        400
    };
    (weight, italic)
}

/// Counts one use of `key`.
pub fn record_font_use(ctx: &mut ImportContext, key: FontKey) {
    *ctx.lookups.fonts.entry(key).or_default() += 1;
}

/// Known families: (lowercase family prefix, availability, substitute). Matched on the squashed
/// family name, first match wins (longer prefixes first).
const FONT_TABLE: &[(&str, FontAvailability, Option<&str>)] = {
    use FontAvailability::{CommonlyMissing, Free, Proprietary};
    &[
        ("calibri", Proprietary, Some("Carlito")),
        ("cambria", Proprietary, Some("Caladea")),
        ("arialnarrow", Proprietary, Some("Liberation Sans Narrow")),
        ("arialblack", Proprietary, Some("DejaVu Sans")),
        ("arial", Proprietary, Some("Liberation Sans")),
        ("helveticaneue", Proprietary, Some("Liberation Sans")),
        ("helvetica", Proprietary, Some("Liberation Sans")),
        ("timesnewroman", Proprietary, Some("Liberation Serif")),
        ("times", Proprietary, Some("Liberation Serif")),
        ("couriernew", Proprietary, Some("Liberation Mono")),
        ("courier", Proprietary, Some("Liberation Mono")),
        ("tahoma", Proprietary, Some("DejaVu Sans")),
        ("verdana", Proprietary, Some("DejaVu Sans")),
        ("segoeui", Proprietary, Some("Noto Sans")),
        ("georgia", Proprietary, Some("Gelasio")),
        ("bookantiqua", Proprietary, Some("TeX Gyre Pagella")),
        ("palatino", Proprietary, Some("TeX Gyre Pagella")),
        ("centurygothic", Proprietary, Some("TeX Gyre Adventor")),
        ("gillsans", Proprietary, Some("Cabin")),
        ("futura", Proprietary, Some("Jost")),
        ("avenir", Proprietary, Some("Nunito Sans")),
        ("trebuchet", Proprietary, Some("Fira Sans")),
        ("impact", Proprietary, Some("Anton")),
        ("garamond", Proprietary, Some("EB Garamond")),
        ("yugothic", Proprietary, Some("Noto Sans CJK JP")),
        ("msgothic", Proprietary, Some("Noto Sans CJK JP")),
        ("comicsans", Proprietary, Some("Comic Neue")),
        ("lucidagrande", Proprietary, Some("DejaVu Sans")),
        ("sfpro", Proprietary, Some("Inter")),
        ("carlito", CommonlyMissing, None),
        ("caladea", CommonlyMissing, None),
        ("liberation", CommonlyMissing, None),
        ("texgyre", CommonlyMissing, None),
        ("opensans", CommonlyMissing, None),
        ("montserrat", CommonlyMissing, None),
        ("lato", CommonlyMissing, None),
        ("roboto", CommonlyMissing, None),
        ("sourcesans", CommonlyMissing, None),
        ("inter", CommonlyMissing, None),
        ("noto", Free, None),
        ("dejavu", Free, None),
    ]
};

/// Availability and substitute for a family name (case-insensitive).
pub fn advice(family: &str) -> FontAdvice {
    let f = squash(family);
    if f.is_empty() {
        return FontAdvice::default();
    }
    FONT_TABLE
        .iter()
        .find(|(prefix, _, _)| f.starts_with(prefix))
        .map(|&(_, availability, substitute)| FontAdvice {
            availability,
            substitute,
        })
        .unwrap_or_default()
}

/// (family, weight, italic, PostScript name): one report entry.
type FaceKey = (String, u16, bool, String);

/// The resolved face of every used font key with its use count, merged by face (see the module
/// docs). Sorted by uses (descending), then family, weight, italic, PostScript name.
fn resolved_fonts(ctx: &ImportContext) -> Vec<(FontRef, FontKey, usize)> {
    // PostScript name → (family, face) as written by a key that has a family.
    let mut ps_info: BTreeMap<&str, (&str, &str)> = BTreeMap::new();
    for key in ctx.lookups.fonts.keys() {
        if !key.postscript_name.is_empty() && !key.family.is_empty() {
            ps_info
                .entry(key.postscript_name.as_str())
                .or_insert((key.family.as_str(), key.face.as_str()));
        }
    }
    let mut merged: BTreeMap<FaceKey, (FontRef, FontKey, usize)> = BTreeMap::new();
    for (key, &uses) in &ctx.lookups.fonts {
        let (family, face) = if key.family.is_empty() {
            ps_info
                .get(key.postscript_name.as_str())
                .copied()
                .unwrap_or(("", ""))
        } else {
            (key.family.as_str(), key.face.as_str())
        };
        if key.postscript_name.is_empty() && family.is_empty() {
            continue;
        }
        let r = font_ref_from_parts(&key.postscript_name, family, face);
        let k = (
            r.family.clone(),
            r.weight,
            r.italic,
            r.postscript_name.clone().unwrap_or_default(),
        );
        let display_key = FontKey {
            postscript_name: key.postscript_name.clone(),
            family: family.to_string(),
            face: face.to_string(),
        };
        let e = merged.entry(k).or_insert((r, display_key.clone(), 0));
        e.2 += uses;
        // Prefer the spelling that carries a family / face for display.
        if e.1.family.is_empty() && !display_key.family.is_empty() {
            e.1 = display_key;
        }
    }
    let mut out: Vec<_> = merged.into_values().collect();
    out.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.0.family.cmp(&b.0.family))
            .then_with(|| a.0.weight.cmp(&b.0.weight))
            .then_with(|| a.0.italic.cmp(&b.0.italic))
            .then_with(|| a.0.postscript_name.cmp(&b.0.postscript_name))
    });
    out
}

/// The suggested substitution table for the fonts used (rules sorted by family).
pub fn substitution_table(ctx: &ImportContext) -> FontSubstitutionTable {
    let mut rules: BTreeMap<String, &'static str> = BTreeMap::new();
    for (r, _, _) in resolved_fonts(ctx) {
        if let Some(sub) = advice(&r.family).substitute
            && !r.family.eq_ignore_ascii_case(sub)
        {
            rules.entry(r.family.clone()).or_insert(sub);
        }
    }
    FontSubstitutionTable {
        rules: rules
            .into_iter()
            .map(|(family, sub)| FontSubstitution {
                postscript_name: None,
                family: Some(family),
                replacement_family: sub.to_string(),
            })
            .collect(),
        ..FontSubstitutionTable::default()
    }
}

/// Final phase: fills `ctx.report.fonts` and `ctx.out.font_substitutions`.
pub fn font_report(ctx: &mut ImportContext) {
    let fonts = resolved_fonts(ctx);
    ctx.report.fonts = fonts
        .into_iter()
        .map(|(r, key, uses)| {
            let a = advice(&r.family);
            FontReportEntry {
                family: r.family,
                face: key.face,
                postscript_name: key.postscript_name,
                weight: r.weight,
                italic: r.italic,
                uses,
                availability: a.availability,
                substitute: a.substitute.map(str::to_string),
            }
        })
        .collect();
    let table = substitution_table(ctx);
    ctx.out.font_substitutions = (!table.rules.is_empty()).then_some(table);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(name: &str, family: &str, face: &str, bold: bool, italic: bool) -> pp::Font {
        pp::Font {
            name: name.into(),
            family: family.into(),
            face: face.into(),
            bold,
            italic,
            size: 72.0,
        }
    }

    fn fr(family: &str, weight: u16, italic: bool, ps: &str) -> FontRef {
        FontRef {
            family: family.into(),
            weight,
            italic,
            postscript_name: Some(ps.into()),
        }
    }

    #[test]
    fn faces_to_weights() {
        assert_eq!(weight_and_italic_from_face("Bold Italic"), (700, true));
        assert_eq!(weight_and_italic_from_face("SemiBold"), (600, false));
        assert_eq!(weight_and_italic_from_face("Demi Bold"), (600, false));
        assert_eq!(weight_and_italic_from_face("ExtraLight"), (200, false));
        assert_eq!(weight_and_italic_from_face("UltraLight"), (200, false));
        assert_eq!(weight_and_italic_from_face("Light Oblique"), (300, true));
        assert_eq!(weight_and_italic_from_face("Thin"), (100, false));
        assert_eq!(weight_and_italic_from_face("Medium"), (500, false));
        assert_eq!(weight_and_italic_from_face("Heavy"), (800, false));
        assert_eq!(weight_and_italic_from_face("ExtraBold"), (800, false));
        assert_eq!(weight_and_italic_from_face("Black"), (900, false));
        assert_eq!(weight_and_italic_from_face("Roman"), (400, false));
        assert_eq!(weight_and_italic_from_face("Book"), (400, false));
        assert_eq!(weight_and_italic_from_face(""), (400, false));
    }

    #[test]
    fn postscript_names() {
        assert_eq!(
            font_ref_from_postscript("Calibri-BoldItalic"),
            fr("Calibri", 700, true, "Calibri-BoldItalic")
        );
        assert_eq!(
            font_ref_from_postscript("ArialMT"),
            fr("Arial", 400, false, "ArialMT")
        );
        assert_eq!(
            font_ref_from_postscript("TimesNewRomanPS-BoldItalicMT"),
            fr("Times New Roman", 700, true, "TimesNewRomanPS-BoldItalicMT")
        );
        assert_eq!(
            font_ref_from_postscript("TimesNewRomanPSMT"),
            fr("Times New Roman", 400, false, "TimesNewRomanPSMT")
        );
        assert_eq!(
            font_ref_from_postscript("HelveticaNeue-Light"),
            fr("Helvetica Neue", 300, false, "HelveticaNeue-Light")
        );
        assert_eq!(
            font_ref_from_postscript("YuGothicUI-Regular"),
            fr("Yu Gothic UI", 400, false, "YuGothicUI-Regular")
        );
        assert_eq!(
            font_ref_from_postscript("CourierNewPSMT").family,
            "Courier New"
        );
        assert_eq!(
            font_ref_from_postscript("BookAntiqua").family,
            "Book Antiqua"
        );
        assert_eq!(
            font_ref_from_postscript("Helvetica-BoldOblique"),
            fr("Helvetica", 700, true, "Helvetica-BoldOblique")
        );
    }

    #[test]
    fn source_fonts_with_inconsistent_family_face() {
        let want = fr("Calibri", 700, false, "Calibri-Bold");
        assert_eq!(
            font_ref(&font("Calibri-Bold", "Calibri", "Bold", true, false)),
            want
        );
        assert_eq!(
            font_ref(&font(
                "Calibri-Bold",
                "Calibri-Bold",
                "Regular",
                true,
                false
            )),
            want
        );
        assert_eq!(
            font_ref(&font("ArialMT", "ArialMT", "Regular", false, false)),
            fr("Arial", 400, false, "ArialMT")
        );
        assert_eq!(
            font_ref(&font("HelveticaNeue", "", "", false, false)),
            fr("Helvetica Neue", 400, false, "HelveticaNeue")
        );
        // Family substituted by the writer; the face comes from the PostScript name.
        assert_eq!(
            font_ref(&font("Helvetica-Bold", "Arial", "Regular", true, false)),
            fr("Arial", 700, false, "Helvetica-Bold")
        );
        assert_eq!(
            font_ref(&font(
                "YuGothicUI-Light",
                "Yu Gothic UI Light",
                "Regular",
                false,
                false
            )),
            fr("Yu Gothic UI", 300, false, "YuGothicUI-Light")
        );
        assert_eq!(
            font_ref(&font(
                "TimesNewRomanPS-ItalicMT",
                "Times New Roman",
                "",
                false,
                true
            )),
            fr("Times New Roman", 400, true, "TimesNewRomanPS-ItalicMT")
        );
        assert_eq!(
            font_ref(&font(
                "Arial-BoldItalicMT",
                "Arial",
                "Bold Italic",
                true,
                true
            )),
            fr("Arial", 700, true, "Arial-BoldItalicMT")
        );
        // No PostScript name at all.
        let r = font_ref(&font("", "Open Sans", "SemiBold", false, false));
        assert_eq!(
            (r.family.as_str(), r.weight, r.postscript_name),
            ("Open Sans", 600, None)
        );
        let empty = font_ref(&pp::Font::default());
        assert_eq!(empty.family, "");
        assert_eq!(empty.postscript_name, None);
    }

    #[test]
    fn advice_table() {
        assert_eq!(advice("Calibri").substitute, Some("Carlito"));
        assert_eq!(advice("calibri light").substitute, Some("Carlito"));
        assert_eq!(advice("Arial").substitute, Some("Liberation Sans"));
        assert_eq!(advice("Helvetica Neue").substitute, Some("Liberation Sans"));
        assert_eq!(
            advice("Times New Roman").substitute,
            Some("Liberation Serif")
        );
        assert_eq!(advice("Book Antiqua").substitute, Some("TeX Gyre Pagella"));
        assert_eq!(advice("Yu Gothic UI").substitute, Some("Noto Sans CJK JP"));
        assert_eq!(advice("Tahoma").availability, FontAvailability::Proprietary);
        assert_eq!(advice("DejaVu Sans").availability, FontAvailability::Free);
        assert_eq!(advice("Some Font").availability, FontAvailability::Unknown);
        assert_eq!(advice("").substitute, None);
    }

    #[test]
    fn report_merges_rtf_and_attribute_keys() {
        let mut ctx = ImportContext::new(
            std::path::Path::new("/nonexistent"),
            crate::ImportOptions::default(),
        );
        let key = |ps: &str, fam: &str, face: &str| FontKey {
            postscript_name: ps.into(),
            family: fam.into(),
            face: face.into(),
        };
        record_font_use(&mut ctx, key("Calibri-Bold", "Calibri", "Bold"));
        record_font_use(&mut ctx, key("Calibri-Bold", "", ""));
        record_font_use(&mut ctx, key("Calibri-Bold", "", ""));
        record_font_use(&mut ctx, key("ArialMT", "", ""));
        record_font_use(&mut ctx, key("", "", ""));
        font_report(&mut ctx);
        let f = &ctx.report.fonts;
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].family, "Calibri");
        assert_eq!(f[0].face, "Bold");
        assert_eq!(f[0].uses, 3);
        assert_eq!(f[0].weight, 700);
        assert_eq!(f[0].substitute.as_deref(), Some("Carlito"));
        assert_eq!(f[1].family, "Arial");
        let t = ctx.out.font_substitutions.as_ref().unwrap();
        let fams: Vec<_> = t.rules.iter().map(|r| r.family.clone().unwrap()).collect();
        assert_eq!(fams, ["Arial", "Calibri"]);
        assert_eq!(
            t.substitute(&fr("Calibri", 700, false, "Calibri-Bold")),
            Some("Carlito")
        );
    }

    #[test]
    fn empty_report_has_no_table() {
        let mut ctx = ImportContext::new(
            std::path::Path::new("/nonexistent"),
            crate::ImportOptions::default(),
        );
        font_report(&mut ctx);
        assert!(ctx.report.fonts.is_empty());
        assert!(ctx.out.font_substitutions.is_none());
    }
}
