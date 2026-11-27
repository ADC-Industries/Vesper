//! Unit tests. All text here is synthetic.

use crate::*;

/// Wraps `body` in a header shaped like ProPresenter's writer output.
fn cocoa(body: &str) -> String {
    format!(
        r"{{\rtf0\ansi\ansicpg1252{{\fonttbl\f0\fnil TestSans-Bold;\f1\fnil TestSerif;}}{{\colortbl;\red255\green255\blue255;\red0\green0\blue0;\red255\green255\blue255;\red0\green0\blue0;}}{{\*\expandedcolortbl;\csgenericrgb\c100000\c100000\c100000\c100000;\csgenericrgb\c0\c0\c0\c100000;\csgenericrgb\c100000\c100000\c100000\c0;\csgenericrgb\c0\c0\c0\c100000;}}{{\*\listtable}}{{\*\listoverridetable}}\uc1\paperw36000\margl0\margr0\margt0\margb0{body}}}"
    )
}

/// ProPresenter's per-paragraph prologue.
const PP: &str = r"\pard\li0\fi0\ri0\qc\sb0\sa0\sl240\slmult1\slleading0\f0\b\i0\ul0\strike0\fs260\expnd0\expndtw0\CocoaLigature1\cf1\strokewidth60\strokec2\nosupersub\ulc0\highlight3\cb3 ";

fn texts(doc: &Document) -> Vec<String> {
    doc.paragraphs.iter().map(Paragraph::text).collect()
}

fn clean(input: &str) -> Document {
    let (doc, diag) = parse_with_diagnostics(input);
    assert!(diag.is_clean(), "unexpected warnings: {:?}", diag.warnings);
    assert!(
        diag.unknown_words.is_empty(),
        "unknown: {:?}",
        diag.unknown_words
    );
    doc
}

const WHITE: Color = Color {
    r: 1.0,
    g: 1.0,
    b: 1.0,
    a: 1.0,
};
const BLACK: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

#[test]
fn propresenter_style_paragraph() {
    let doc = clean(&cocoa(&format!("{PP}Alpha beta")));
    assert_eq!(doc.paragraphs.len(), 1);
    let p = &doc.paragraphs[0];
    assert_eq!(p.style.alignment, Alignment::Center);
    assert_eq!(p.style.line_spacing, LineSpacing::Multiple(1.0));
    assert_eq!(p.runs.len(), 1);
    let s = &p.runs[0].style;
    assert_eq!(p.runs[0].text, "Alpha beta");
    assert_eq!(s.font.as_deref(), Some("TestSans-Bold"));
    assert_eq!(s.size_pt, 130.0);
    assert!(s.bold && !s.italic && !s.underline && !s.strikethrough);
    assert_eq!(s.color, Some(WHITE));
    assert_eq!(s.stroke_width, 3.0);
    assert_eq!(s.stroke_color, Some(BLACK));
    assert_eq!(s.highlight, Some(Color { a: 0.0, ..WHITE }));
    assert_eq!(s.background, Some(Color { a: 0.0, ..WHITE }));
    assert_eq!(s.underline_color, None);
    assert_eq!(s.script, Script::Normal);
    assert_eq!(&p.end_style, s);
}

#[test]
fn fresh_colour_entry_per_paragraph_resolves_equal() {
    // \strokec2 and \strokec4 are different indices with the same colour.
    let body = format!("{PP}One\\par\n{}Two", PP.replace("strokec2", "strokec4"));
    let doc = clean(&cocoa(&body));
    assert_eq!(texts(&doc), ["One", "Two"]);
    assert_eq!(
        doc.paragraphs[0].runs[0].style,
        doc.paragraphs[1].runs[0].style
    );
}

#[test]
fn runs_merge_when_resolved_styles_match() {
    let doc = clean(&cocoa(r"\cf2\strokec2 ab\strokec4\cf4 cd"));
    assert_eq!(doc.paragraphs[0].runs.len(), 1);
    assert_eq!(doc.paragraphs[0].runs[0].text, "abcd");
}

#[test]
fn trailing_par_semantics() {
    assert_eq!(texts(&clean(&cocoa(r"a\par b"))), ["a", "b"]);
    assert_eq!(texts(&clean(&cocoa(r"a\par b\par"))), ["a", "b"]);
    assert_eq!(texts(&clean(&cocoa(r"a\par\par"))), ["a", ""]);
    assert_eq!(texts(&clean(&cocoa(r"a\par\par b"))), ["a", "", "b"]);
    assert_eq!(texts(&clean(&cocoa(r"\par"))), [""]);
    assert_eq!(texts(&clean(&cocoa(r"\par\par x"))), ["", "", "x"]);
    // Closing braces alone still count as nothing.
    assert_eq!(texts(&clean(r"{\rtf1 {\b a\par}}")), ["a"]);
    // But formatting after the last `\par` makes a real empty last line.
    assert_eq!(
        texts(&clean(r"{\rtf1 a\par b\par\pard\f0\fs24\cb3}")),
        ["a", "b", ""]
    );
    assert_eq!(texts(&clean(&cocoa(r"a\par\b"))), ["a", ""]);
    assert_eq!(texts(&clean(&cocoa(r"a\par{}"))), ["a", ""]);
}

#[test]
fn trailing_par_with_prologue_keeps_styled_empty_line() {
    // ProPresenter's shape for a box whose last line is empty: the `\par` is
    // followed by a full paragraph prologue and no text.
    let small = PP.replace(r"\qc", r"\qr").replace(r"\fs260", r"\fs120");
    let body = format!("{PP}First line\\par\n{PP}Second\\par\n{small}");
    let doc = clean(&cocoa(&body));
    assert_eq!(texts(&doc), ["First line", "Second", ""]);
    // UTF-16 length matches a range that ends after the empty last line.
    assert_eq!(doc.plain_text().encode_utf16().count(), 18);
    let last = &doc.paragraphs[2];
    assert!(last.runs.is_empty());
    assert_eq!(last.style.alignment, Alignment::Right);
    assert_eq!(last.end_style.size_pt, 60.0);
    assert_eq!(last.end_style.font.as_deref(), Some("TestSans-Bold"));
    assert_eq!(doc.paragraphs[1].end_style.size_pt, 130.0);
}

#[test]
fn paragraph_style_is_taken_at_the_par() {
    let doc = clean(&cocoa(r"\pard\ql left\par\pard\qr right"));
    assert_eq!(doc.paragraphs[0].style.alignment, Alignment::Left);
    assert_eq!(doc.paragraphs[1].style.alignment, Alignment::Right);
}

#[test]
fn empty_body_gives_one_styled_paragraph() {
    let doc = clean(&cocoa(PP));
    assert_eq!(doc.paragraphs.len(), 1);
    assert!(doc.is_empty());
    let p = &doc.paragraphs[0];
    assert!(p.runs.is_empty());
    assert_eq!(p.end_style.font.as_deref(), Some("TestSans-Bold"));
    assert_eq!(p.end_style.size_pt, 130.0);
    assert_eq!(p.style.alignment, Alignment::Center);
    assert_eq!(doc.plain_text(), "");
}

#[test]
fn unicode_and_skip_counts() {
    let doc = clean(&cocoa(r"it\u8217 ?s \uc0\u8226 x \uc2\u8220 ab q"));
    assert_eq!(doc.plain_text(), "it\u{2019}s \u{2022}x \u{201C} q");
}

#[test]
fn uc_is_scoped_to_its_group() {
    let doc = clean(&cocoa(r"{\uc0\u8226 }\u8226 ?"));
    assert_eq!(doc.plain_text(), "\u{2022}\u{2022}");
}

#[test]
fn negative_unicode_and_surrogate_pairs() {
    let doc = clean(&cocoa(r"\u-4064 ?\u-10179 ?\u-8704 ?"));
    assert_eq!(doc.plain_text(), "\u{F020}\u{1F600}");
}

#[test]
fn lone_surrogate_is_replaced_and_reported() {
    let (doc, diag) = parse_with_diagnostics(cocoa(r"\u-10179 ?x"));
    assert_eq!(doc.plain_text(), "\u{FFFD}x");
    assert_eq!(diag.warnings, [Warning::LoneSurrogate]);
}

#[test]
fn unicode_fallback_may_be_a_hex_escape() {
    let doc = clean(&cocoa(r"\u8217\'92s"));
    assert_eq!(doc.plain_text(), "\u{2019}s");
}

#[test]
fn paragraph_break_ends_unicode_fallback() {
    // A writer that forgets the fallback must not lose the paragraph break.
    let doc = clean(&cocoa(r"a\u8217\par b"));
    assert_eq!(texts(&doc), ["a\u{2019}", "b"]);
    let doc = clean(&cocoa("a\\u8217\\\nb"));
    assert_eq!(texts(&doc), ["a\u{2019}", "b"]);
    // Nor the low half of a surrogate pair.
    let doc = clean(&cocoa(r"\u-10179\u-8704 ?x"));
    assert_eq!(doc.plain_text(), "\u{1F600}x");
}

#[test]
fn control_word_is_one_fallback_unit() {
    let doc = clean(r"{\rtf1\uc1 a\u8220\ldblquote b}");
    assert_eq!(doc.plain_text(), "a\u{201C}b");
    let doc = clean(r"{\rtf1\uc1 a\u8212\emdash\u8211\endash b}");
    assert_eq!(doc.plain_text(), "a\u{2014}\u{2013}b");
    // A control symbol is one unit too.
    let doc = clean(r"{\rtf1\uc1 a\u160\~b}");
    assert_eq!(doc.plain_text(), "a\u{A0}b");
    // With \uc2 the word and one byte are skipped.
    let doc = clean(r"{\rtf1\uc2 a\u8220\ldblquote xb}");
    assert_eq!(doc.plain_text(), "a\u{201C}b");
}

#[test]
fn uc_defaults_to_one() {
    let doc = clean(r"{\rtf1\ansi \u8217?s}");
    assert_eq!(doc.plain_text(), "\u{2019}s");
    let doc = clean(r"{\rtf1{\listtext\tab\u8226 ?\tab}item\par}");
    assert_eq!(
        doc.paragraphs[0].list_marker.as_deref(),
        Some("\t\u{2022}\t")
    );
    assert_eq!(texts(&doc), ["item"]);
    // An explicit \uc0 still turns the skip off.
    let doc = clean(r"{\rtf1\uc0\u8217 s}");
    assert_eq!(doc.plain_text(), "\u{2019}s");
}

#[test]
fn cp1252_hex_and_raw_bytes() {
    let doc = clean(&cocoa(r"\'93Hi\'94 caf\'e9 \'80"));
    assert_eq!(doc.plain_text(), "\u{201C}Hi\u{201D} caf\u{E9} \u{20AC}");
    let mut raw = cocoa("caf_").into_bytes();
    let i = raw.iter().position(|&b| b == b'_').unwrap();
    raw[i] = 0xE9;
    assert_eq!(parse(&raw).plain_text(), "caf\u{E9}");
}

#[test]
fn escaped_braces_and_backslash() {
    let doc = clean(&cocoa(r"\{a\}\\b"));
    assert_eq!(doc.plain_text(), r"{a}\b");
}

#[test]
fn control_symbols() {
    let doc = clean(&cocoa(
        r"a\~b\-c\_d\emdash\endash\lquote\rquote\ldblquote\rdblquote\bullet",
    ));
    assert_eq!(
        doc.plain_text(),
        "a\u{A0}b\u{AD}c\u{2011}d\u{2014}\u{2013}\u{2018}\u{2019}\u{201C}\u{201D}\u{2022}"
    );
}

#[test]
fn crlf_and_whitespace_rules() {
    let doc = clean(&cocoa("a\r\nb\\b\r\nc\\par\r\n d\\i  e"));
    assert_eq!(texts(&doc), ["abc", " d e"]);
    assert!(doc.paragraphs[0].runs[1].style.bold);
    assert_eq!(doc.paragraphs[0].runs[1].text, "c");
}

#[test]
fn backslash_newline_is_a_paragraph_break() {
    let doc = clean(&cocoa("one\\\ntwo\\\r\nthree"));
    assert_eq!(texts(&doc), ["one", "two", "three"]);
}

#[test]
fn line_and_tab() {
    let doc = clean(&cocoa(r"a\line b\tab c"));
    assert_eq!(texts(&doc), ["a\nb\tc"]);
}

#[test]
fn ignorable_and_skipped_destinations() {
    let body = r"{\*\unknowndest hidden}{\info{\title hidden}}{\*\listtable{\list\listtemplateid1{\listlevel\levelnfc23{\leveltext\leveltemplateid1\'01\uc0\u8226 ;}{\levelnumbers;}}{\listname ;}\listid1}}{\stylesheet{\s0 Normal;}}shown";
    let (doc, diag) = parse_with_diagnostics(cocoa(body));
    assert_eq!(doc.plain_text(), "shown");
    assert!(diag.is_clean());
    assert!(diag.unknown_words.is_empty(), "{:?}", diag.unknown_words);
}

#[test]
fn unstarred_unknown_group_is_formatting() {
    let doc = parse(cocoa(r"{\madeupword visible}"));
    assert_eq!(doc.plain_text(), "visible");
}

#[test]
fn listtext_becomes_marker() {
    let body = r"\pard\tx220\tx720\li720\fi-720\ls1\ilvl0{\listtext	\uc0\u8226 	}First\par{\listtext	\uc0\u8226 	}Second";
    let doc = clean(&cocoa(body));
    assert_eq!(texts(&doc), ["First", "Second"]);
    let p = &doc.paragraphs[0];
    assert_eq!(p.list_marker.as_deref(), Some("\t\u{2022}\t"));
    assert_eq!(
        p.style.list,
        Some(ListRef {
            override_index: 1,
            level: 0
        })
    );
    assert_eq!(p.style.left_indent_pt, 36.0);
    assert_eq!(p.style.first_line_indent_pt, -36.0);
    assert_eq!(p.style.tab_stops_pt, [11.0, 36.0]);
}

#[test]
fn superscript_runs() {
    let doc = clean(&cocoa(r"\nosupersub\super 9\nosupersub Some words"));
    let runs = &doc.paragraphs[0].runs;
    assert_eq!(runs.len(), 2);
    assert_eq!(
        (runs[0].text.as_str(), runs[0].style.script),
        ("9", Script::Superscript)
    );
    assert_eq!(
        (runs[1].text.as_str(), runs[1].style.script),
        ("Some words", Script::Normal)
    );
    let doc = clean(&cocoa(r"H\sub 2\nosupersub O"));
    assert_eq!(doc.paragraphs[0].runs[1].style.script, Script::Subscript);
}

#[test]
fn baseline_offsets() {
    let doc = clean(&cocoa(r"a{\up6 b}{\dn4 c}{\up d}"));
    let r = &doc.paragraphs[0].runs;
    assert_eq!(r[1].style.baseline_offset_pt, 3.0);
    assert_eq!(r[2].style.baseline_offset_pt, -2.0);
    assert_eq!(r[3].style.baseline_offset_pt, 3.0);
}

#[test]
fn kerning() {
    let doc = clean(&cocoa(r"\expnd52\expndtw260 a{\expnd8 b}"));
    let r = &doc.paragraphs[0].runs;
    assert_eq!(r[0].style.kerning_pt, 13.0);
    assert_eq!(r[1].style.kerning_pt, 2.0);
}

#[test]
fn paragraph_spacing_and_indents() {
    let doc = clean(&cocoa(
        r"\pard\li1440\fi-360\ri720\sb400\sa200\sl-360\slleading40 a\par\pard\sl20\slmult0 b\par\pard\sl216\slmult1 c\par\pard d",
    ));
    let s: Vec<_> = doc.paragraphs.iter().map(|p| &p.style).collect();
    assert_eq!(s[0].left_indent_pt, 72.0);
    assert_eq!(s[0].first_line_indent_pt, -18.0);
    assert_eq!(s[0].right_indent_pt, 36.0);
    assert_eq!(s[0].space_before_pt, 20.0);
    assert_eq!(s[0].space_after_pt, 10.0);
    assert_eq!(s[0].line_spacing, LineSpacing::Exactly(18.0));
    assert_eq!(s[0].line_leading_pt, 2.0);
    assert_eq!(s[1].line_spacing, LineSpacing::AtLeast(1.0));
    assert_eq!(s[2].line_spacing, LineSpacing::Multiple(0.9));
    assert_eq!(s[3], &ParagraphStyle::default());
}

#[test]
fn group_state_is_restored() {
    let doc = clean(&cocoa(r"\f1\fs40 a{\b\i\ul\strike\f0\fs80 b}c"));
    let r = &doc.paragraphs[0].runs;
    assert_eq!(r.len(), 3);
    assert!(
        r[1].style.bold && r[1].style.italic && r[1].style.underline && r[1].style.strikethrough
    );
    assert_eq!(r[1].style.font.as_deref(), Some("TestSans-Bold"));
    assert_eq!(r[2].style, r[0].style);
    assert_eq!(r[2].style.font.as_deref(), Some("TestSerif"));
    assert_eq!(r[2].style.size_pt, 20.0);
}

#[test]
fn toggles_and_resets() {
    let doc = clean(&cocoa(r"\b\ul a\b0\ulnone b\i\uldb c\plain d"));
    let r = &doc.paragraphs[0].runs;
    assert!(r[0].style.bold && r[0].style.underline);
    assert!(!r[1].style.bold && !r[1].style.underline);
    assert!(r[2].style.italic && r[2].style.underline);
    assert_eq!(r[3].style, CharStyle::default());
}

#[test]
fn auto_colour_and_out_of_range_index() {
    let doc = clean(&cocoa(r"\cf0 a\cf99 b\cf2 c"));
    let r = &doc.paragraphs[0].runs;
    assert_eq!(r[0].style.color, None);
    assert_eq!(r[0].text, "ab");
    assert_eq!(r[1].style.color, Some(BLACK));
}

#[test]
fn colour_spaces_in_expanded_table() {
    let rtf = r"{\rtf1\ansi{\colortbl;\red255\green0\blue0;\red128\green128\blue128;\red0\green0\blue255;}{\*\expandedcolortbl;\cssrgb\c100000\c0\c0;\csgray\c50000\c25000;\csgenericcmyk\c0\c0\c0\c0\c50000;}\cf1 a\cf2 b\cf3 c}";
    let doc = clean(rtf);
    let r = &doc.paragraphs[0].runs;
    assert_eq!(r[0].style.color.unwrap().a, 1.0);
    assert_eq!(r[1].style.color.unwrap().a, 0.25);
    assert_eq!(r[2].style.color.unwrap().a, 0.5);
    assert_eq!(r[2].style.color.unwrap().b, 1.0);
}

#[test]
fn expanded_table_fills_missing_colortbl_entries() {
    let doc = clean(r"{\rtf1{\*\expandedcolortbl;\csgenericrgb\c0\c50000\c100000\c80000;}\cf1 x}");
    let c = doc.paragraphs[0].runs[0].style.color.unwrap();
    assert_eq!((c.r, c.g, c.b, c.a), (0.0, 0.5, 1.0, 0.8));
}

#[test]
fn font_table_with_subgroups_and_deff() {
    let doc = clean(
        r"{\rtf1\ansi\deff1{\fonttbl{\f0\fswiss\fcharset0 Test Sans;}{\f1\froman Test Serif;}}a\f0 b}",
    );
    let r = &doc.paragraphs[0].runs;
    assert_eq!(r[0].style.font.as_deref(), Some("Test Serif"));
    assert_eq!(r[1].style.font.as_deref(), Some("Test Sans"));
}

#[test]
fn bin_payload_is_skipped() {
    let doc = clean(r"{\rtf1 a{\*\blob\bin4 {}\}b}c}");
    assert_eq!(doc.plain_text(), "ac");
    let doc = clean(r"{\rtf1 a\bin2 xyb}");
    assert_eq!(doc.plain_text(), "ab");
}

#[test]
fn diagnostics_report_problems() {
    let (_, d) = parse_with_diagnostics(r"{\rtf1 \frobnicate3 a\paperw100 b");
    assert_eq!(d.warnings, [Warning::UnexpectedEof { open_groups: 1 }]);
    assert_eq!(d.unknown_words.get("frobnicate"), Some(&1));
    assert_eq!(d.ignored_words.get("paperw"), Some(&1));

    let (doc, d) = parse_with_diagnostics("plain text");
    assert_eq!(doc.plain_text(), "plain text");
    assert_eq!(d.warnings, [Warning::MissingRtfHeader]);

    let (doc, d) = parse_with_diagnostics(r"{\rtf1 a} b}");
    assert_eq!(doc.plain_text(), "a");
    assert_eq!(d.warnings, [Warning::TrailingContent]);

    let (_, d) = parse_with_diagnostics("{\\rtf1 a}\r\n\0");
    assert!(d.is_clean());

    let (_, d) = parse_with_diagnostics(r"{\rtf1\ansicpg1251\'zz\# x}");
    assert_eq!(
        d.warnings,
        [
            Warning::UnsupportedCodePage(1251),
            Warning::MalformedHexEscape
        ]
    );
    assert_eq!(d.unknown_symbols.get(&'#'), Some(&1));
}

#[test]
fn truncated_document_keeps_its_text() {
    let (doc, d) = parse_with_diagnostics(cocoa(&format!("{PP}One\\par Tw")).trim_end_matches('}'));
    assert_eq!(texts(&doc), ["One", "Tw"]);
    assert!(matches!(d.warnings[..], [Warning::UnexpectedEof { .. }]));
}

#[test]
fn empty_and_tiny_inputs() {
    for input in [
        "", "{", "}", "\\", "{\\", "{\\rtf", "{\\rtf1}", "{}", "\\'", "\\u", "\\u-",
    ] {
        let doc = parse(input);
        assert_eq!(doc.paragraphs.len(), 1, "{input:?}");
    }
}

/// A tiny deterministic PRNG so the fuzz test needs no dependencies.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn never_panics_on_truncated_input() {
    let sample = cocoa(&format!(
        r"{PP}Ab\u8217 ?c\par{{\listtext \u8226 ?}}\super 9\nosupersub d\'e9\{{\}}\\{{\*\x y}}\u-10179 ?\u-8704 ?\bin3 abc\line\tab e"
    ));
    let bytes = sample.as_bytes();
    for end in 0..=bytes.len() {
        let _ = parse_with_diagnostics(&bytes[..end]);
        let _ = parse_with_diagnostics(&bytes[end..]);
    }
}

#[test]
fn never_panics_on_garbage() {
    const PIECES: &[&[u8]] = &[
        b"{",
        b"}",
        b"\\",
        b"\\*",
        b"\\'",
        b"\\'9",
        b"\\'zz",
        b"\\u",
        b"\\u-",
        b"\\u-10179",
        b"\\u56000",
        b"\\uc",
        b"\\uc5",
        b"\\uc-3",
        b"\\par",
        b"\\pard",
        b"\\fonttbl",
        b"\\colortbl",
        b"\\expandedcolortbl",
        b"\\listtext",
        b"\\f",
        b"\\f-1",
        b"\\cf",
        b"\\cf2147483647",
        b"\\red300",
        b"\\c",
        b"\\csgray",
        b"\\bin",
        b"\\bin99",
        b"\\bin-5",
        b"\\fs-99999999999",
        b"\\sl-2147483648",
        b"\\expnd2147483647",
        b"\\dn",
        b"\\up-2147483648",
        b";",
        b" ",
        b"\r\n",
        b"\0",
        b"x",
        b"\xff",
        b"\x80",
        b"9",
        b"-",
        b"\\\n",
        b"\\~",
        b"\\plain",
    ];
    let mut rng = Lcg(0x5eed);
    for _ in 0..4000 {
        let mut input = Vec::new();
        for _ in 0..rng.below(64) {
            if rng.below(8) == 0 {
                input.push(rng.next() as u8);
            } else {
                input.extend_from_slice(PIECES[rng.below(PIECES.len())]);
            }
        }
        let (doc, _) = parse_with_diagnostics(&input);
        assert!(!doc.paragraphs.is_empty());
    }

    // Mutations of a well-formed document.
    let base = cocoa(&format!(r"{PP}Ab\u8217 ?c\par d\super 9")).into_bytes();
    for _ in 0..4000 {
        let mut input = base.clone();
        for _ in 0..1 + rng.below(6) {
            let i = rng.below(input.len());
            match rng.below(3) {
                0 => input[i] = rng.next() as u8,
                1 => {
                    input.remove(i);
                }
                _ => input.insert(i, PIECES[rng.below(PIECES.len())][0]),
            }
        }
        let _ = parse_with_diagnostics(&input);
    }
}

#[test]
fn never_panics_on_deep_nesting() {
    let deep = "{".repeat(200_000) + "x" + &"}".repeat(200_000);
    assert_eq!(parse(&deep).plain_text(), "x");
    let open = "{\\rtf1".to_owned() + &"{\\b".repeat(100_000);
    assert_eq!(parse(&open).paragraphs.len(), 1);
}

#[test]
fn tab_stops_are_capped_against_quadratic_blowup() {
    // Every `\par` and every `{` copies the tab list. Uncapped, 20 000 tab
    // stops followed by 20 000 paragraphs (160 KB of input) produced 1.6 GB
    // of tab stops.
    let n = 20_000;
    let input = "{\\rtf1 ".to_owned() + &"\\tx20".repeat(n) + &"\\par".repeat(n) + "}";
    let doc = parse(&input);
    assert_eq!(doc.paragraphs.len(), n);
    for p in &doc.paragraphs {
        assert_eq!(p.style.tab_stops_pt.len(), crate::parser::MAX_TAB_STOPS);
        assert_eq!(p.style.tab_stops_pt[0], 1.0);
    }
    // A realistic tab list is kept whole.
    let doc = parse(r"{\rtf1\tx560\tx1120\tx1680 a\tab b}");
    assert_eq!(doc.paragraphs[0].style.tab_stops_pt, vec![28.0, 56.0, 84.0]);
}
