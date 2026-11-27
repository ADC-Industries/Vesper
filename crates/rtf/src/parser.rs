//! The RTF state machine: tokens in, [`Document`] out.

use std::collections::BTreeMap;

use crate::codepage::cp1252;
use crate::lexer::{Lexer, Token};
use crate::{
    Alignment, CharStyle, Color, Diagnostics, Document, LineSpacing, ListRef, Paragraph,
    ParagraphStyle, Run, Script, Warning,
};

/// Parses RTF bytes into a [`Document`]. Never fails and never panics; use
/// [`parse_with_diagnostics`] to learn about malformed input.
pub fn parse(input: impl AsRef<[u8]>) -> Document {
    parse_with_diagnostics(input).0
}

/// Parses RTF bytes into a [`Document`] and reports what was odd about them.
pub fn parse_with_diagnostics(input: impl AsRef<[u8]>) -> (Document, Diagnostics) {
    let input = input.as_ref();
    let mut p = Parser::default();
    if !input.starts_with(b"{\\rtf") {
        p.diag.warn(Warning::MissingRtfHeader);
    }
    for tok in Lexer::new(input) {
        p.token(tok);
    }
    p.finish()
}

/// Where text and table words currently go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Dest {
    #[default]
    Body,
    FontTable,
    ColorTable,
    ExpandedColorTable,
    /// `\listtext`: the rendered list marker.
    ListText,
    /// An ignorable destination: everything inside is dropped.
    Skip,
}

/// Character formatting with unresolved table indices.
#[derive(Debug, Clone, PartialEq)]
struct CharProps {
    /// `None` means the `\deff` default font.
    font: Option<i32>,
    half_points: i32,
    bold: bool,
    italic: bool,
    underline: bool,
    underline_color: Option<i32>,
    strike: bool,
    caps: bool,
    color: Option<i32>,
    stroke_twentieths: i32,
    stroke_color: Option<i32>,
    spacing_twips: i32,
    script: Script,
    offset_half_points: i32,
    highlight: Option<i32>,
    background: Option<i32>,
}

impl Default for CharProps {
    fn default() -> Self {
        Self {
            font: None,
            half_points: 24,
            bold: false,
            italic: false,
            underline: false,
            underline_color: None,
            strike: false,
            caps: false,
            color: None,
            stroke_twentieths: 0,
            stroke_color: None,
            spacing_twips: 0,
            script: Script::Normal,
            offset_half_points: 0,
            highlight: None,
            background: None,
        }
    }
}

/// Paragraph formatting in raw twips.
#[derive(Debug, Clone, PartialEq, Default)]
struct ParaProps {
    alignment: Alignment,
    li: i32,
    fi: i32,
    ri: i32,
    sb: i32,
    sa: i32,
    sl: i32,
    slmult: bool,
    slleading: i32,
    tabs: Vec<i32>,
    ls: Option<i32>,
    ilvl: i32,
}

#[derive(Debug, Clone)]
struct GroupState {
    dest: Dest,
    chr: CharProps,
    para: ParaProps,
    /// `\uc`: fallback units to skip after each `\u`.
    uc: usize,
}

impl Default for GroupState {
    fn default() -> Self {
        Self {
            dest: Dest::default(),
            chr: CharProps::default(),
            para: ParaProps::default(),
            // The RTF specification's default is `\uc1`.
            uc: 1,
        }
    }
}

#[derive(Debug, Default)]
struct RawParagraph {
    para: ParaProps,
    end: CharProps,
    runs: Vec<(CharProps, String)>,
    marker: Option<String>,
}

/// An `\*\expandedcolortbl` entry: a Cocoa colour space and its components
/// in 1/100000 units.
#[derive(Debug, Clone, Default)]
struct ExpandedColor {
    space: String,
    comps: Vec<i32>,
}

impl ExpandedColor {
    fn unit(v: i32) -> f32 {
        (v as f32 / 100_000.0).clamp(0.0, 1.0)
    }

    fn is_gray(&self) -> bool {
        self.space.to_ascii_lowercase().contains("gray")
    }

    fn alpha(&self) -> f32 {
        let i = if self.is_gray() {
            1
        } else if self.space.to_ascii_lowercase().contains("cmyk") {
            4
        } else {
            3
        };
        self.comps.get(i).map_or(1.0, |&v| Self::unit(v))
    }

    /// The colour itself, for gray and RGB spaces. Only used when the plain
    /// `\colortbl` lacks the entry.
    fn color(&self) -> Option<Color> {
        let c = |i: usize| self.comps.get(i).map(|&v| Self::unit(v));
        let (r, g, b) = if self.is_gray() {
            let w = c(0)?;
            (w, w, w)
        } else if self.space.to_ascii_lowercase().contains("rgb") {
            (c(0)?, c(1)?, c(2)?)
        } else {
            return None;
        };
        Some(Color {
            r,
            g,
            b,
            a: self.alpha(),
        })
    }
}

#[derive(Default)]
struct Parser {
    state: GroupState,
    stack: Vec<GroupState>,
    diag: Diagnostics,

    default_font: Option<i32>,
    fonts: BTreeMap<i32, String>,
    font_index: Option<i32>,
    font_name: String,

    colors: Vec<Option<[u8; 3]>>,
    color_rgb: [Option<u8>; 3],
    expanded: Vec<Option<ExpandedColor>>,
    expanded_cur: ExpandedColor,

    /// Fallback units (bytes, `\'hh` escapes, control words or symbols)
    /// still to skip after a `\u`.
    uc_skip: usize,
    /// A `\u` high surrogate waiting for its low half.
    pending_high_surrogate: Option<u32>,
    /// The previous token was `{` (possibly followed by `\*`).
    group_start: bool,
    /// The previous token was `\*`.
    star: bool,
    /// The outermost group has closed.
    done: bool,
    /// The last token was a body paragraph break: nothing at all, not even
    /// a formatting word, has followed the last `\par` yet.
    bare_par: bool,

    paragraphs: Vec<RawParagraph>,
    cur: RawParagraph,
}

/// The most `\tx` tab stops kept per paragraph; further ones are dropped.
pub(crate) const MAX_TAB_STOPS: usize = 64;

/// Destinations whose content is never text.
const SKIP_DESTINATIONS: &[&str] = &[
    "listtable",
    "listoverridetable",
    "stylesheet",
    "info",
    "pict",
    "object",
    "shp",
    "shpinst",
    "nonshppict",
    "header",
    "headerl",
    "headerr",
    "headerf",
    "footer",
    "footerl",
    "footerr",
    "footerf",
    "footnote",
    "annotation",
    "fldinst",
    "pn",
    "revtbl",
    "rsidtbl",
    "xmlnstbl",
    "themedata",
    "colorschememapping",
    "latentstyles",
    "datastore",
    "generator",
    "NeXTGraphic",
    "bkmkstart",
    "bkmkend",
    "listpicture",
];

/// Words that are understood and deliberately have no effect on the AST.
const IGNORED_WORDS: &[&str] = &[
    // Document and page setup.
    "paperw",
    "paperh",
    "margl",
    "margr",
    "margt",
    "margb",
    "deftab",
    "viewkind",
    "viewscale",
    "viewh",
    "vieww",
    "landscape",
    "sectd",
    "sect",
    "page",
    "pgnstart",
    // Cocoa writer housekeeping.
    "cocoartf",
    "cocoasubrtf",
    "cocoatextscaling",
    "cocoaplatform",
    "CocoaLigature",
    "pardirnatural",
    "partightenfactor",
    "pardeftab",
    "readonlydoc",
    "AppleTypeServices",
    "AppleTypeServicesU",
    // Font table attributes.
    "fnil",
    "froman",
    "fswiss",
    "fmodern",
    "fscript",
    "fdecor",
    "ftech",
    "fbidi",
    "fcharset",
    "fprq",
    "ftnil",
    "fttruetype",
    // Fields: the result text is shown as-is.
    "field",
    "fldrslt",
    // Language, direction and widow control.
    "lang",
    "langfe",
    "langnp",
    "langfenp",
    "ltrch",
    "rtlch",
    "ltrpar",
    "rtlpar",
    "loch",
    "hich",
    "dbch",
    "widctlpar",
    "nowidctlpar",
    "kerning",
    "noproof",
    // Character effects with no model field.
    "outl",
    "shad",
    "scaps",
    "embo",
    "impr",
    "v",
    "charscalex",
    // Lists (only reachable outside the skipped list tables).
    "listid",
    "listtemplateid",
    "listhybrid",
    "listsimple",
    "levelnfc",
    "levelnfcn",
    "leveljc",
    "leveljcn",
    "levelfollow",
    "levelstartat",
    "levelnumbers",
    "leveltemplateid",
    "listlevel",
    // Tab stop kinds (the position word `\tx` is kept).
    "tqr",
    "tqc",
    "tqdec",
    "tldot",
    "tlhyph",
    "tlul",
    "tlth",
    "tleq",
];

/// Control words that end a pending `\u` fallback instead of being skipped
/// as part of it: paragraph structure and the next `\u`, which no writer
/// uses as a fallback but a writer that omits the fallback runs into.
fn ends_fallback(name: &str) -> bool {
    matches!(
        name,
        "par" | "pard" | "sect" | "sectd" | "page" | "line" | "u" | "uc"
    )
}

impl Parser {
    fn token(&mut self, tok: Token<'_>) {
        if self.done {
            let blank = matches!(tok, Token::Text(t) if t.iter().all(|b| b.is_ascii_whitespace() || *b == 0));
            if !blank {
                self.diag.warn(Warning::TrailingContent);
            }
            return;
        }
        // Anything but a closing brace after a `\par` makes the paragraph
        // that follows it real (see `close_document`).
        if !matches!(tok, Token::GroupEnd) {
            self.bare_par = false;
        }
        match tok {
            Token::GroupStart => {
                self.stack.push(self.state.clone());
                self.group_start = true;
                self.star = false;
                self.uc_skip = 0;
            }
            Token::GroupEnd => self.group_end(),
            Token::Word { name, param } => {
                let at_start = std::mem::take(&mut self.group_start);
                let starred = std::mem::take(&mut self.star);
                if self.uc_skip > 0 {
                    if ends_fallback(name) {
                        // Leniency: a writer that omits the fallback must not
                        // make us swallow a paragraph break or the next `\u`.
                        self.uc_skip = 0;
                    } else {
                        // As the specification says, a control word is one
                        // fallback unit.
                        self.uc_skip -= 1;
                        return;
                    }
                }
                self.word(name, param, at_start, starred);
            }
            Token::Symbol(b'*') => self.star = true,
            Token::Symbol(s) => {
                self.group_start = false;
                self.star = false;
                if matches!(s, b'\n' | b'\r') {
                    // `\` + newline is `\par`, with the same leniency.
                    self.uc_skip = 0;
                } else if self.uc_skip > 0 {
                    self.uc_skip -= 1;
                    return;
                }
                self.symbol(s);
            }
            Token::Hex(b) => {
                self.group_start = false;
                if self.uc_skip > 0 {
                    self.uc_skip -= 1;
                    return;
                }
                self.emit_char(cp1252(b));
            }
            Token::BadHex => {
                self.group_start = false;
                self.diag.warn(Warning::MalformedHexEscape);
            }
            Token::Text(mut bytes) => {
                self.group_start = false;
                if self.uc_skip > 0 {
                    let n = self.uc_skip.min(bytes.len());
                    bytes = &bytes[n..];
                    self.uc_skip -= n;
                }
                if !bytes.is_empty() {
                    let s: String = bytes.iter().map(|&b| cp1252(b)).collect();
                    self.emit_str(&s);
                }
            }
            Token::Bin => self.group_start = false,
        }
    }

    fn group_end(&mut self) {
        self.group_start = false;
        self.star = false;
        self.uc_skip = 0;
        if self.state.dest == Dest::FontTable {
            self.commit_font();
        }
        let Some(outer) = self.stack.pop() else {
            self.diag.warn(Warning::UnmatchedGroupEnd);
            return;
        };
        if self.stack.is_empty() {
            // The document group closed: the last paragraph takes the state
            // from inside the group, before it is popped.
            self.close_document();
            self.done = true;
        }
        self.state = outer;
    }

    fn word(&mut self, name: &str, param: Option<i32>, at_start: bool, starred: bool) {
        if self.state.dest == Dest::Skip {
            return;
        }
        if at_start {
            let dest = match name {
                "fonttbl" => Some(Dest::FontTable),
                "colortbl" => Some(Dest::ColorTable),
                "expandedcolortbl" => Some(Dest::ExpandedColorTable),
                "listtext" | "pntext" => Some(Dest::ListText),
                _ if SKIP_DESTINATIONS.contains(&name) || starred => Some(Dest::Skip),
                _ => None,
            };
            if let Some(dest) = dest {
                self.enter(dest);
                return;
            }
        }
        let on = param.is_none_or(|v| v != 0);
        let p = param.unwrap_or(0);

        // Table contents.
        match (self.state.dest, name) {
            (Dest::FontTable, "f") => {
                self.commit_font();
                self.font_index = Some(p);
                return;
            }
            (Dest::ColorTable, "red" | "green" | "blue") => {
                let i = match name {
                    "red" => 0,
                    "green" => 1,
                    _ => 2,
                };
                self.color_rgb[i] = Some(p.clamp(0, 255) as u8);
                return;
            }
            (Dest::ExpandedColorTable, "c") => {
                self.expanded_cur.comps.push(p);
                return;
            }
            (Dest::ExpandedColorTable, _) if name.starts_with("cs") => {
                self.expanded_cur.space = name.to_owned();
                return;
            }
            _ => {}
        }

        let chr = &mut self.state.chr;
        let para = &mut self.state.para;
        match name {
            // Header and character set.
            "rtf" | "ansi" => {}
            "mac" | "pc" | "pca" => self.diag.warn(Warning::UnsupportedCodePage(p)),
            "ansicpg" => {
                if p != 1252 {
                    self.diag.warn(Warning::UnsupportedCodePage(p));
                }
            }
            "deff" => self.default_font = Some(p),
            "uc" => self.state.uc = usize::try_from(p).unwrap_or(0).min(64),
            "u" => {
                let unit = if p < 0 { p.wrapping_add(65536) } else { p };
                self.emit_unicode(unit);
                self.uc_skip = self.state.uc;
            }

            // Breaks and special characters.
            "par" => self.end_paragraph(),
            "line" => self.emit_char('\n'),
            "tab" => self.emit_char('\t'),
            "emdash" => self.emit_char('\u{2014}'),
            "endash" => self.emit_char('\u{2013}'),
            "emspace" => self.emit_char('\u{2003}'),
            "enspace" => self.emit_char('\u{2002}'),
            "qmspace" => self.emit_char('\u{2005}'),
            "bullet" => self.emit_char('\u{2022}'),
            "lquote" => self.emit_char('\u{2018}'),
            "rquote" => self.emit_char('\u{2019}'),
            "ldblquote" => self.emit_char('\u{201C}'),
            "rdblquote" => self.emit_char('\u{201D}'),
            "zwj" => self.emit_char('\u{200D}'),
            "zwnj" => self.emit_char('\u{200C}'),

            // Paragraph formatting.
            "pard" => *para = ParaProps::default(),
            "ql" => para.alignment = Alignment::Left,
            "qc" => para.alignment = Alignment::Center,
            "qr" => para.alignment = Alignment::Right,
            "qj" => para.alignment = Alignment::Justified,
            "li" => para.li = p,
            "fi" => para.fi = p,
            "ri" => para.ri = p,
            "sb" => para.sb = p,
            "sa" => para.sa = p,
            "sl" => para.sl = p,
            "slmult" => para.slmult = on,
            "slleading" => para.slleading = p,
            // Capped: every group and paragraph clones the tab list, so an
            // unbounded one makes hostile input cost quadratic memory.
            "tx" if para.tabs.len() < MAX_TAB_STOPS => para.tabs.push(p),
            "tx" => {}
            "ls" => para.ls = Some(p),
            "ilvl" => para.ilvl = p,

            // Character formatting.
            "plain" => *chr = CharProps::default(),
            "f" => chr.font = Some(p),
            "fs" => chr.half_points = param.unwrap_or(24),
            "b" => chr.bold = on,
            "i" => chr.italic = on,
            "ul" | "uld" | "uldash" | "uldashd" | "uldashdd" | "uldb" | "ulhwave" | "ulldash"
            | "ulth" | "ulthd" | "ulthdash" | "ulthdashd" | "ulthdashdd" | "ulthldash"
            | "ululdbwave" | "ulw" | "ulwave" => chr.underline = on,
            "ulnone" => chr.underline = false,
            "ulc" => chr.underline_color = Some(p),
            "strike" | "striked" => chr.strike = on,
            "caps" => chr.caps = on,
            "cf" => chr.color = Some(p),
            "cb" | "chcbpat" => chr.background = Some(p),
            "highlight" => chr.highlight = Some(p),
            "expnd" => chr.spacing_twips = p.saturating_mul(5),
            "expndtw" => chr.spacing_twips = p,
            "strokewidth" => chr.stroke_twentieths = p,
            "strokec" => chr.stroke_color = Some(p),
            "super" => chr.script = Script::Superscript,
            "sub" => chr.script = Script::Subscript,
            "nosupersub" => chr.script = Script::Normal,
            "up" => chr.offset_half_points = param.unwrap_or(6),
            "dn" => chr.offset_half_points = param.unwrap_or(6).saturating_neg(),

            _ if IGNORED_WORDS.contains(&name) => {
                *self.diag.ignored_words.entry(name.to_owned()).or_default() += 1;
            }
            _ => *self.diag.unknown_words.entry(name.to_owned()).or_default() += 1,
        }
    }

    fn enter(&mut self, dest: Dest) {
        self.state.dest = dest;
        match dest {
            Dest::FontTable => {
                self.font_index = None;
                self.font_name.clear();
            }
            Dest::ColorTable => {
                self.colors.clear();
                self.color_rgb = [None; 3];
            }
            Dest::ExpandedColorTable => {
                self.expanded.clear();
                self.expanded_cur = ExpandedColor::default();
            }
            Dest::ListText => {
                self.cur.marker.get_or_insert_with(String::new);
            }
            Dest::Body | Dest::Skip => {}
        }
    }

    fn symbol(&mut self, s: u8) {
        match s {
            b'\\' | b'{' | b'}' => self.emit_char(char::from(s)),
            b'~' => self.emit_char('\u{A0}'),
            b'-' => self.emit_char('\u{AD}'),
            b'_' => self.emit_char('\u{2011}'),
            b'\n' | b'\r' => self.end_paragraph(),
            b'\t' => self.emit_char('\t'),
            // Index sub-entry and formula markers carry no text.
            b':' | b'|' => {}
            _ => {
                if self.state.dest != Dest::Skip {
                    *self.diag.unknown_symbols.entry(char::from(s)).or_default() += 1;
                }
            }
        }
    }

    fn emit_unicode(&mut self, unit: i32) {
        match unit {
            0xD800..=0xDBFF => {
                self.flush_surrogate();
                self.pending_high_surrogate = Some(unit as u32);
            }
            0xDC00..=0xDFFF => {
                if let Some(high) = self.pending_high_surrogate.take() {
                    let c = 0x10000 + ((high - 0xD800) << 10) + (unit as u32 - 0xDC00);
                    let c = char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER);
                    self.emit_str_raw(c.encode_utf8(&mut [0; 4]));
                } else {
                    self.diag.warn(Warning::LoneSurrogate);
                    self.emit_char(char::REPLACEMENT_CHARACTER);
                }
            }
            _ => {
                let c = u32::try_from(unit)
                    .ok()
                    .and_then(char::from_u32)
                    .unwrap_or(char::REPLACEMENT_CHARACTER);
                self.emit_char(c);
            }
        }
    }

    /// Emits U+FFFD for a high surrogate that was not followed by a low one.
    fn flush_surrogate(&mut self) {
        if self.pending_high_surrogate.take().is_some() {
            self.diag.warn(Warning::LoneSurrogate);
            self.emit_str_raw("\u{FFFD}");
        }
    }

    fn emit_char(&mut self, c: char) {
        self.emit_str(c.encode_utf8(&mut [0; 4]));
    }

    fn emit_str(&mut self, s: &str) {
        self.flush_surrogate();
        self.emit_str_raw(s);
    }

    fn emit_str_raw(&mut self, s: &str) {
        match self.state.dest {
            Dest::Body => match self.cur.runs.last_mut() {
                Some((props, text)) if *props == self.state.chr => text.push_str(s),
                _ => self.cur.runs.push((self.state.chr.clone(), s.to_owned())),
            },
            Dest::ListText => self.cur.marker.get_or_insert_with(String::new).push_str(s),
            Dest::FontTable => {
                for c in s.chars() {
                    if c == ';' {
                        self.commit_font();
                    } else {
                        self.font_name.push(c);
                    }
                }
            }
            Dest::ColorTable => {
                for _ in s.matches(';') {
                    let rgb = self.color_rgb;
                    self.color_rgb = [None; 3];
                    self.colors.push(if rgb.iter().any(Option::is_some) {
                        Some(rgb.map(|c| c.unwrap_or(0)))
                    } else {
                        None
                    });
                }
            }
            Dest::ExpandedColorTable => {
                for _ in s.matches(';') {
                    let e = std::mem::take(&mut self.expanded_cur);
                    let empty = e.space.is_empty() && e.comps.is_empty();
                    self.expanded.push((!empty).then_some(e));
                }
            }
            Dest::Skip => {}
        }
    }

    fn commit_font(&mut self) {
        let name = self.font_name.trim();
        if let Some(i) = self.font_index.take()
            && !name.is_empty()
        {
            self.fonts.insert(i, name.to_owned());
        }
        self.font_name.clear();
    }

    fn end_paragraph(&mut self) {
        if self.state.dest != Dest::Body {
            return;
        }
        self.flush_surrogate();
        let mut p = std::mem::take(&mut self.cur);
        p.para = self.state.para.clone();
        p.end = self.state.chr.clone();
        self.paragraphs.push(p);
        self.bare_par = true;
    }

    /// Ends the document and guarantees one paragraph. The last paragraph is
    /// kept unless nothing at all followed the last `\par` (see the crate
    /// docs on trailing `\par`); it takes the state at the closing brace.
    fn close_document(&mut self) {
        let dest = std::mem::replace(&mut self.state.dest, Dest::Body);
        self.flush_surrogate();
        let bare_trailing_par = self.bare_par
            && self.cur.runs.is_empty()
            && self.cur.marker.is_none()
            && !self.paragraphs.is_empty();
        if !bare_trailing_par {
            self.end_paragraph();
        }
        self.state.dest = dest;
    }

    fn finish(mut self) -> (Document, Diagnostics) {
        if !self.done {
            if !self.stack.is_empty() {
                self.diag.warn(Warning::UnexpectedEof {
                    open_groups: self.stack.len(),
                });
            }
            self.close_document();
        }
        let paragraphs = std::mem::take(&mut self.paragraphs)
            .into_iter()
            .map(|p| self.resolve_paragraph(p))
            .collect();
        (Document { paragraphs }, self.diag)
    }

    fn resolve_paragraph(&self, p: RawParagraph) -> Paragraph {
        let mut runs: Vec<Run> = Vec::with_capacity(p.runs.len());
        for (props, text) in p.runs {
            if text.is_empty() {
                continue;
            }
            let style = self.resolve_char(&props);
            // Distinct raw props can resolve to the same style (ProPresenter
            // writes a fresh colour-table entry per paragraph).
            match runs.last_mut() {
                Some(last) if last.style == style => last.text.push_str(&text),
                _ => runs.push(Run { text, style }),
            }
        }
        let pp = &p.para;
        let twips = |v: i32| v as f32 / 20.0;
        let line_spacing = match pp.sl {
            0 => LineSpacing::Auto,
            n if n < 0 => LineSpacing::Exactly(-(n as f32) / 20.0),
            n if pp.slmult => LineSpacing::Multiple(n as f32 / 240.0),
            n => LineSpacing::AtLeast(twips(n)),
        };
        Paragraph {
            style: ParagraphStyle {
                alignment: pp.alignment,
                line_spacing,
                line_leading_pt: twips(pp.slleading),
                space_before_pt: twips(pp.sb),
                space_after_pt: twips(pp.sa),
                left_indent_pt: twips(pp.li),
                first_line_indent_pt: twips(pp.fi),
                right_indent_pt: twips(pp.ri),
                tab_stops_pt: pp.tabs.iter().map(|&t| twips(t)).collect(),
                list: pp.ls.map(|ls| ListRef {
                    override_index: ls,
                    level: pp.ilvl,
                }),
            },
            runs,
            end_style: self.resolve_char(&p.end),
            list_marker: p.marker,
        }
    }

    fn resolve_char(&self, c: &CharProps) -> CharStyle {
        CharStyle {
            font: c
                .font
                .or(self.default_font)
                .and_then(|i| self.fonts.get(&i).cloned()),
            size_pt: c.half_points.max(0) as f32 / 2.0,
            bold: c.bold,
            italic: c.italic,
            underline: c.underline,
            underline_color: self.color(c.underline_color),
            strikethrough: c.strike,
            caps: c.caps,
            color: self.color(c.color),
            stroke_width: c.stroke_twentieths as f32 / 20.0,
            stroke_color: self.color(c.stroke_color),
            kerning_pt: c.spacing_twips as f32 / 20.0,
            script: c.script,
            baseline_offset_pt: c.offset_half_points as f32 / 2.0,
            highlight: self.color(c.highlight),
            background: self.color(c.background),
        }
    }

    /// Resolves a colour index: RGB from `\colortbl`, alpha from
    /// `\*\expandedcolortbl`. Index 0 of a Cocoa table is "auto" (`None`).
    fn color(&self, index: Option<i32>) -> Option<Color> {
        let i = usize::try_from(index?).ok()?;
        let expanded = self.expanded.get(i).and_then(Option::as_ref);
        match self.colors.get(i) {
            Some(Some([r, g, b])) => {
                let mut c = Color::rgb8(*r, *g, *b);
                if let Some(e) = expanded {
                    c.a = e.alpha();
                }
                Some(c)
            }
            Some(None) => None,
            None => expanded.and_then(ExpandedColor::color),
        }
    }
}
