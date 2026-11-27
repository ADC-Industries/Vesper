//! DrawingML text: run and paragraph properties, list styles and their inheritance
//! (presentation default → master text styles → master placeholder → layout placeholder →
//! shape → paragraph → run), and `p:txBody` → [`TextBox`].

use crate::opc::{attr, attr_bool, attr_i64, child, children, is};
use crate::theme::Palette;
use model::{
    Baseline, Capitalization, CharStyle, Color, EdgeInsets, FitMode, LineStyle, ListKind,
    ListMarker, Paragraph, ParagraphStyle, RichText, Shadow, TextAlign, TextBox, TextRun,
    VerticalAlign,
};
use roxmltree::Node;

/// Character properties (`a:rPr`, `a:defRPr`, `a:endParaRPr`); every field inherits when unset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RPr {
    /// Size, pt.
    pub size: Option<f64>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<LineStyle>,
    pub strike: Option<LineStyle>,
    pub color: Option<Color>,
    /// The typeface as written (`+mn-lt` resolved later).
    pub font: Option<String>,
    pub cap: Option<Capitalization>,
    pub baseline: Option<Baseline>,
    /// Letter spacing, pt.
    pub spacing: Option<f64>,
    pub highlight: Option<Color>,
    /// A text shadow (`a:effectLst/a:outerShdw`).
    pub shadow: Option<ShadowSpec>,
}

/// An outer shadow, in source units (EMU, degrees clockwise from +x, y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowSpec {
    pub dist: f64,
    pub dir: f64,
    pub blur: f64,
    pub color: Color,
}

impl RPr {
    /// Reads one properties element.
    pub fn read(n: Node, p: &Palette) -> RPr {
        let line = |v: &str| match v {
            "none" => LineStyle::None,
            "dbl" => LineStyle::Double,
            "heavy" | "thick" => LineStyle::Thick,
            _ => LineStyle::Single,
        };
        RPr {
            size: attr_i64(n, "sz").map(|v| v as f64 / 100.0),
            bold: attr_bool(n, "b"),
            italic: attr_bool(n, "i"),
            underline: attr(n, "u").map(line),
            strike: attr(n, "strike").map(|v| match v {
                "noStrike" => LineStyle::None,
                "dblStrike" => LineStyle::Double,
                _ => LineStyle::Single,
            }),
            color: crate::theme::solid_fill(p, n),
            font: child(n, "latin")
                .and_then(|l| attr(l, "typeface"))
                .filter(|t| !t.is_empty())
                .map(str::to_string),
            cap: attr(n, "cap").map(|v| match v {
                "all" => Capitalization::AllCaps,
                "small" => Capitalization::SmallCaps,
                _ => Capitalization::None,
            }),
            baseline: attr_i64(n, "baseline").map(|v| match v {
                v if v > 0 => Baseline::Superscript,
                v if v < 0 => Baseline::Subscript,
                _ => Baseline::Normal,
            }),
            spacing: attr_i64(n, "spc").map(|v| v as f64 / 100.0),
            highlight: child(n, "highlight").and_then(|h| p.color_in(h)),
            shadow: child(n, "effectLst")
                .and_then(|e| child(e, "outerShdw"))
                .and_then(|s| read_shadow(s, p)),
        }
    }

    /// `self` over `base`, field by field.
    pub fn over(&self, base: &RPr) -> RPr {
        RPr {
            size: self.size.or(base.size),
            bold: self.bold.or(base.bold),
            italic: self.italic.or(base.italic),
            underline: self.underline.or(base.underline),
            strike: self.strike.or(base.strike),
            color: self.color.or(base.color),
            font: self.font.clone().or_else(|| base.font.clone()),
            cap: self.cap.or(base.cap),
            baseline: self.baseline.or(base.baseline),
            spacing: self.spacing.or(base.spacing),
            highlight: self.highlight.or(base.highlight),
            shadow: self.shadow.or(base.shadow),
        }
    }

    /// The model's style of fully resolved properties.
    pub fn char_style(&self, p: &Palette, px_per_pt: f64) -> CharStyle {
        CharStyle {
            font_family: Some(p.font(self.font.as_deref().unwrap_or("+mn-lt"))),
            font_weight: Some(if self.bold.unwrap_or(false) { 700 } else { 400 }),
            italic: Some(self.italic.unwrap_or(false)),
            postscript_name: None,
            size: Some(round2(self.size.unwrap_or(18.0) * px_per_pt)),
            color: Some(self.color.unwrap_or(Color::BLACK)),
            underline: Some(self.underline.unwrap_or(LineStyle::None)),
            strikethrough: Some(self.strike.unwrap_or(LineStyle::None)),
            baseline: Some(self.baseline.unwrap_or(Baseline::Normal)),
            capitalization: Some(self.cap.unwrap_or(Capitalization::None)),
            letter_spacing: Some(round2(self.spacing.unwrap_or(0.0) * px_per_pt)),
            highlight: self.highlight,
        }
    }
}

/// An outer shadow element.
pub fn read_shadow(s: Node, p: &Palette) -> Option<ShadowSpec> {
    Some(ShadowSpec {
        dist: attr_i64(s, "dist").unwrap_or(0) as f64,
        dir: attr_i64(s, "dir").unwrap_or(0) as f64 / 60_000.0,
        blur: attr_i64(s, "blurRad").unwrap_or(0) as f64,
        color: p.color_in(s).unwrap_or(Color::BLACK),
    })
}

/// The model's shadow of a source shadow (`px` per EMU).
pub fn shadow(s: ShadowSpec, px: f64) -> Shadow {
    Shadow {
        enabled: true,
        // Clockwise y-down → counter-clockwise y-up.
        angle: (360.0 - s.dir).rem_euclid(360.0),
        offset: round2(s.dist * px),
        radius: round2(s.blur * px / 2.0),
        color: Color::rgb(s.color.r, s.color.g, s.color.b),
        opacity: s.color.a as f64,
    }
}

/// A paragraph's bullet.
#[derive(Clone, Debug, PartialEq)]
pub enum Bullet {
    None,
    Char(String),
    Number(ListKind),
}

/// Paragraph spacing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Spacing {
    /// Points.
    Pts(f64),
    /// A share of the line (1 = one line).
    Lines(f64),
}

fn spacing(n: Option<Node>) -> Option<Spacing> {
    let n = n?;
    if let Some(v) = child(n, "spcPts").and_then(|c| attr_i64(c, "val")) {
        return Some(Spacing::Pts(v as f64 / 100.0));
    }
    child(n, "spcPct")
        .and_then(|c| attr_i64(c, "val"))
        .map(|v| Spacing::Lines(v as f64 / 100_000.0))
}

/// Paragraph properties (`a:pPr`, `a:lvlNpPr`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PPr {
    pub align: Option<TextAlign>,
    /// Left margin, EMU.
    pub mar_l: Option<f64>,
    /// First-line indent, EMU.
    pub indent: Option<f64>,
    pub bullet: Option<Bullet>,
    /// Line spacing as a multiple.
    pub line: Option<f64>,
    pub before: Option<Spacing>,
    pub after: Option<Spacing>,
    /// `a:defRPr`.
    pub def: RPr,
}

impl PPr {
    /// Reads one properties element.
    pub fn read(n: Node, p: &Palette) -> PPr {
        let bullet = if child(n, "buNone").is_some() {
            Some(Bullet::None)
        } else if let Some(b) = child(n, "buAutoNum") {
            Some(Bullet::Number(match attr(b, "type").unwrap_or_default() {
                t if t.starts_with("alphaLc") => ListKind::LowerAlpha,
                t if t.starts_with("alphaUc") => ListKind::UpperAlpha,
                t if t.starts_with("romanLc") => ListKind::LowerRoman,
                t if t.starts_with("romanUc") => ListKind::UpperRoman,
                _ => ListKind::Decimal,
            }))
        } else {
            child(n, "buChar")
                .and_then(|b| attr(b, "char"))
                .map(|c| Bullet::Char(c.to_string()))
        };
        PPr {
            align: attr(n, "algn").map(|a| match a {
                "ctr" => TextAlign::Center,
                "r" => TextAlign::Right,
                "just" | "dist" | "justLow" | "thaiDist" => TextAlign::Justified,
                _ => TextAlign::Left,
            }),
            mar_l: attr_i64(n, "marL").map(|v| v as f64),
            indent: attr_i64(n, "indent").map(|v| v as f64),
            bullet,
            line: child(n, "lnSpc")
                .and_then(|l| child(l, "spcPct"))
                .and_then(|c| attr_i64(c, "val"))
                .map(|v| v as f64 / 100_000.0),
            before: spacing(child(n, "spcBef")),
            after: spacing(child(n, "spcAft")),
            def: child(n, "defRPr")
                .map(|d| RPr::read(d, p))
                .unwrap_or_default(),
        }
    }

    /// `self` over `base`.
    pub fn over(&self, base: &PPr) -> PPr {
        PPr {
            align: self.align.or(base.align),
            mar_l: self.mar_l.or(base.mar_l),
            indent: self.indent.or(base.indent),
            bullet: self.bullet.clone().or_else(|| base.bullet.clone()),
            line: self.line.or(base.line),
            before: self.before.or(base.before),
            after: self.after.or(base.after),
            def: self.def.over(&base.def),
        }
    }
}

/// The nine levels of a list style (`a:lstStyle`, `p:titleStyle`, `a:defaultTextStyle`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Levels(pub [PPr; 9]);

impl Levels {
    /// Reads `a:lvl1pPr` … `a:lvl9pPr` (and `a:defPPr` under level 1).
    pub fn read(n: Node, p: &Palette) -> Levels {
        let mut l = Levels::default();
        let def = child(n, "defPPr").map(|d| PPr::read(d, p));
        for (i, slot) in l.0.iter_mut().enumerate() {
            if let Some(x) = child(n, &format!("lvl{}pPr", i + 1)) {
                *slot = PPr::read(x, p);
            }
            if let Some(d) = &def {
                *slot = slot.over(d);
            }
        }
        l
    }

    /// `self` over `base`, level by level.
    pub fn over(&self, base: &Levels) -> Levels {
        let mut out = Levels::default();
        for i in 0..9 {
            out.0[i] = self.0[i].over(&base.0[i]);
        }
        out
    }
}

/// Text box properties (`a:bodyPr`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BodyPr {
    pub anchor: Option<VerticalAlign>,
    /// Insets, EMU: left, top, right, bottom.
    pub insets: [Option<f64>; 4],
    pub fit: Option<FitMode>,
    /// Vertical or rotated text (not mapped).
    pub vertical: bool,
    /// Columns (not mapped).
    pub columns: bool,
}

impl BodyPr {
    /// Reads `a:bodyPr`.
    pub fn read(n: Node) -> BodyPr {
        let fit = if child(n, "normAutofit").is_some() {
            Some(FitMode::ShrinkToFit)
        } else if child(n, "spAutoFit").is_some() || child(n, "noAutofit").is_some() {
            Some(FitMode::None)
        } else {
            None
        };
        BodyPr {
            anchor: attr(n, "anchor").map(|a| match a {
                "ctr" => VerticalAlign::Middle,
                "b" => VerticalAlign::Bottom,
                _ => VerticalAlign::Top,
            }),
            insets: ["lIns", "tIns", "rIns", "bIns"].map(|k| attr_i64(n, k).map(|v| v as f64)),
            fit,
            vertical: attr(n, "vert").is_some_and(|v| v != "horz"),
            columns: attr_i64(n, "numCol").is_some_and(|c| c > 1),
        }
    }

    /// `self` over `base`.
    pub fn over(&self, base: &BodyPr) -> BodyPr {
        let mut insets = self.insets;
        for (i, v) in insets.iter_mut().enumerate() {
            *v = v.or(base.insets[i]);
        }
        BodyPr {
            anchor: self.anchor.or(base.anchor),
            insets,
            fit: self.fit.or(base.fit),
            vertical: self.vertical || base.vertical,
            columns: self.columns || base.columns,
        }
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Scale of a conversion: canvas px per EMU.
#[derive(Clone, Copy, Debug)]
pub struct Scale {
    pub px: f64,
}

impl Scale {
    /// px per point.
    pub fn pt(&self) -> f64 {
        self.px * 12_700.0
    }
}

/// The marker of a bullet.
fn marker(b: &Bullet, level: u8) -> Option<ListMarker> {
    let kind = match b {
        Bullet::None => return None,
        Bullet::Number(k) => *k,
        Bullet::Char(c) => match c.chars().next().unwrap_or('•') {
            'o' | '◦' | '○' => ListKind::Circle,
            '▪' | '■' | '§' | 'q' | 'n' | '□' => ListKind::Square,
            '–' | '-' | '—' => ListKind::Hyphen,
            _ => ListKind::Disc,
        },
    };
    Some(ListMarker { kind, level })
}

fn spacing_px(s: Option<Spacing>, size_pt: f64, sc: Scale) -> Option<f64> {
    s.map(|s| match s {
        Spacing::Pts(v) => round2(v * sc.pt()),
        Spacing::Lines(v) => round2(v * size_pt * 1.2 * sc.pt()),
    })
}

/// What a text body became.
pub struct Converted {
    pub text: TextBox,
    /// Something in the text the model doesn't hold (vertical text, columns).
    pub unmapped: Vec<String>,
}

/// `p:txBody` → [`TextBox`], with `levels` the inherited list style (everything up to the
/// shape's placeholder chain, without the shape's own `a:lstStyle`), `body` the inherited
/// body properties and `base` extra run properties (a shape style's font colour).
pub fn convert(
    tx: Node,
    levels: &Levels,
    body: &BodyPr,
    base: &RPr,
    p: &Palette,
    sc: Scale,
) -> Converted {
    let own = child(tx, "lstStyle")
        .map(|l| Levels::read(l, p))
        .unwrap_or_default();
    let levels = own.over(levels);
    let body = child(tx, "bodyPr")
        .map(BodyPr::read)
        .unwrap_or_default()
        .over(body);
    let mut unmapped = Vec::new();
    if body.vertical {
        unmapped.push("vertical text".to_string());
    }
    if body.columns {
        unmapped.push("text columns".to_string());
    }
    let mut paragraphs = Vec::new();
    let mut first: Option<(CharStyle, ParagraphStyle)> = None;
    let mut shadow = None;
    for para in children(tx, "p") {
        let ppr_node = child(para, "pPr");
        let level = ppr_node
            .and_then(|n| attr_i64(n, "lvl"))
            .unwrap_or(0)
            .clamp(0, 8) as usize;
        let own = ppr_node.map(|n| PPr::read(n, p)).unwrap_or_default();
        let ppr = own.over(&levels.0[level]);
        let pdef = ppr.def.over(base);
        let mut runs = Vec::new();
        for r in para.children().filter(|n| n.is_element()) {
            let (text, rpr) = match r.tag_name().name() {
                "r" | "fld" => {
                    let t: String = child(r, "t")
                        .map(|t| t.text().unwrap_or_default().to_string())
                        .unwrap_or_default();
                    let rp = child(r, "rPr").map(|n| RPr::read(n, p)).unwrap_or_default();
                    (t, rp)
                }
                "br" => (
                    "\u{2028}".to_string(),
                    child(r, "rPr").map(|n| RPr::read(n, p)).unwrap_or_default(),
                ),
                _ => continue,
            };
            if text.is_empty() {
                continue;
            }
            let full = rpr.over(&pdef);
            if shadow.is_none() {
                shadow = full.shadow;
            }
            runs.push((text, full));
        }
        let end = child(para, "endParaRPr")
            .map(|n| RPr::read(n, p))
            .unwrap_or_default()
            .over(&pdef);
        let size_pt = runs
            .first()
            .map(|(_, r)| r.size.unwrap_or(18.0))
            .unwrap_or(end.size.unwrap_or(18.0));
        let bullet = if runs.is_empty() {
            None
        } else {
            ppr.bullet.as_ref().and_then(|b| marker(b, level as u8))
        };
        let pstyle = ParagraphStyle {
            alignment: Some(ppr.align.unwrap_or(TextAlign::Left)),
            line_height_multiple: Some(ppr.line.unwrap_or(1.0)),
            line_spacing: None,
            space_before: spacing_px(ppr.before, size_pt, sc),
            space_after: spacing_px(ppr.after, size_pt, sc),
            indent_start: Some(round2(ppr.mar_l.unwrap_or(0.0) * sc.px)),
            indent_end: None,
            indent_first_line: Some(round2(ppr.indent.unwrap_or(0.0) * sc.px)),
            list: bullet,
        };
        let end_style = end.char_style(p, sc.pt());
        if first.is_none() {
            let cs = runs
                .first()
                .map(|(_, r)| r.char_style(p, sc.pt()))
                .unwrap_or_else(|| end_style.clone());
            first = Some((cs, pstyle.clone()));
        }
        let (def_c, def_p) = first.clone().unwrap_or_default();
        paragraphs.push(Paragraph {
            style: diff_para(&pstyle, &def_p),
            runs: runs
                .into_iter()
                .map(|(t, r)| TextRun::styled(t, diff_char(&r.char_style(p, sc.pt()), &def_c)))
                .collect(),
            end_style: diff_char(&end_style, &def_c),
            chords: Vec::new(),
        });
    }
    // Drop trailing empty paragraphs only when every paragraph is empty (an empty box).
    if paragraphs.iter().all(|p| p.runs.is_empty()) {
        paragraphs.clear();
    }
    let (default_style, default_paragraph) = first.unwrap_or_else(|| {
        let lvl0 = levels.0[0].def.over(base);
        (
            lvl0.char_style(p, sc.pt()),
            ParagraphStyle {
                alignment: Some(levels.0[0].align.unwrap_or(TextAlign::Left)),
                ..ParagraphStyle::default()
            },
        )
    });
    let ins = |i: usize, d: f64| round2(body.insets[i].unwrap_or(d) * sc.px);
    let text = TextBox {
        paragraphs,
        default_style,
        default_paragraph,
        vertical_align: body.anchor.unwrap_or(VerticalAlign::Top),
        fit: body.fit.unwrap_or(FitMode::None),
        margins: EdgeInsets {
            left: ins(0, 91_440.0),
            top: ins(1, 45_720.0),
            right: ins(2, 91_440.0),
            bottom: ins(3, 45_720.0),
        },
        text_shadow: shadow.map(|s| self::shadow(s, sc.px)).unwrap_or_default(),
        ..TextBox::default()
    };
    Converted { text, unmapped }
}

/// The fields of `s` that differ from `base` (sparse run overrides).
fn diff_char(s: &CharStyle, base: &CharStyle) -> CharStyle {
    fn d<T: PartialEq + Clone>(a: &Option<T>, b: &Option<T>) -> Option<T> {
        if a == b { None } else { a.clone() }
    }
    CharStyle {
        font_family: d(&s.font_family, &base.font_family),
        font_weight: d(&s.font_weight, &base.font_weight),
        italic: d(&s.italic, &base.italic),
        postscript_name: None,
        size: d(&s.size, &base.size),
        color: d(&s.color, &base.color),
        underline: d(&s.underline, &base.underline),
        strikethrough: d(&s.strikethrough, &base.strikethrough),
        baseline: d(&s.baseline, &base.baseline),
        capitalization: d(&s.capitalization, &base.capitalization),
        letter_spacing: d(&s.letter_spacing, &base.letter_spacing),
        highlight: d(&s.highlight, &base.highlight),
    }
}

fn diff_para(s: &ParagraphStyle, base: &ParagraphStyle) -> ParagraphStyle {
    fn d<T: PartialEq + Clone>(a: &Option<T>, b: &Option<T>) -> Option<T> {
        if a == b { None } else { a.clone() }
    }
    ParagraphStyle {
        alignment: d(&s.alignment, &base.alignment),
        line_height_multiple: d(&s.line_height_multiple, &base.line_height_multiple),
        line_spacing: d(&s.line_spacing, &base.line_spacing),
        space_before: d(&s.space_before, &base.space_before),
        space_after: d(&s.space_after, &base.space_after),
        indent_start: d(&s.indent_start, &base.indent_start),
        indent_end: d(&s.indent_end, &base.indent_end),
        indent_first_line: d(&s.indent_first_line, &base.indent_first_line),
        // A list ends explicitly where the first paragraph had one.
        list: match (&s.list, &base.list) {
            (a, b) if a == b => None,
            (None, Some(_)) => Some(ListMarker {
                kind: ListKind::None,
                level: 0,
            }),
            (a, _) => a.clone(),
        },
    }
}

/// The plain text of a notes body (`p:txBody`) as notes.
pub fn notes(tx: Node) -> RichText {
    let lines: Vec<String> = children(tx, "p")
        .map(|para| {
            para.children()
                .filter(|n| is(n, "r") || is(n, "fld") || is(n, "br"))
                .map(|r| {
                    if is(&r, "br") {
                        "\u{2028}".to_string()
                    } else {
                        child(r, "t")
                            .and_then(|t| t.text())
                            .unwrap_or_default()
                            .to_string()
                    }
                })
                .collect()
        })
        .collect();
    let text = lines.join("\n");
    if text.trim().is_empty() {
        RichText::default()
    } else {
        RichText::plain(text.trim_end())
    }
}
