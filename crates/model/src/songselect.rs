//! SongSelect files: the lyrics a church's subscriber downloads from CCLI SongSelect, read into
//! a [`ChordProSong`] (the shared song shape the ChordPro import builds presentations from).
//! Vesper never contacts CCLI: these are files (or text) the user brings.
//!
//! ## Lyrics text ([`parse_lyrics`], `.txt` or pasted)
//! - The **title** is the first line.
//! - **Sections** start with a heading line: "Verse 1", "Chorus", "Pre-Chorus", "Bridge",
//!   "Tag", "Ending", "Intro", "Outro", "Interlude", … and their names in other languages
//!   ("Coro", "Estrofa 2", "Refrão", "Strophe 1", …; see [`HEADINGS`]), with an optional number.
//!   The section keeps the name as written. A blank line ends the section; lines after it
//!   without a heading are another stanza of the same section (before any heading: "Verse 1",
//!   "Verse 2", …). A section that appears again with the same words is a repeat in the play
//!   order (an arrangement), not a second section.
//! - The **footer** (the last lines): the authors separated by ` | `, `CCLI Song # <n>`,
//!   `© <year> <owners>` (owners separated by `|` or `;`), the "For use solely with the
//!   SongSelect Terms of Use…" line and `CCLI License # <n>`. The authors line comes right
//!   before the `CCLI Song #` line (files since 2023) or right after it (older files). A file
//!   without a footer is all lyrics.
//!
//! ## SongShow Plus `.usr` / `.bin` ([`parse_usr`])
//! INI-style: `[File]` (`Type=`, `Version=`), then `[S A<ccli#>]` with `Title=`, `Author=`
//! (`|`-separated), `Copyright=`, `Admin=`, `Themes=`, `Keys=`, `Fields=` (the sections'
//! names, `/t`-separated) and `Words=` (each section's lyrics, `/t`-separated, `/n` a line
//! break).
//!
//! ## ChordPro from SongSelect
//! [`crate::chordpro::parse`] reads SongSelect's directives and the same footer (as comments or
//! plain lines) with [`footer_line`].
//!
//! ## Untrusted input
//! [`decode`] takes any bytes (UTF-8 with or without a byte-order mark, UTF-16 with a mark or
//! recognisably without one, else Windows-1252) and the parsers take any text without
//! panicking; callers limit the size first ([`MAX_FILE_BYTES`]).

use crate::chordpro::{self, ChordProSection, ChordProSong};
use crate::text::Paragraph;

/// Largest file accepted (UTF-16 doubles a song's size; a long song is a few KB).
pub const MAX_FILE_BYTES: usize = 2 << 20;

/// Most sections read from one song (the rest are left out).
const MAX_SECTIONS: usize = 500;

/// Which kind of SongSelect file a text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Lyrics text (`.txt` or pasted).
    Lyrics,
    /// SongShow Plus (`.usr`, `.bin`).
    Usr,
    /// ChordPro (`.cho`, `.chordpro`, `.pro`).
    ChordPro,
}

/// The bytes of a file as text: UTF-8 (a byte-order mark dropped), UTF-16 LE/BE (with a
/// mark, or without one when every other byte is zero), else Windows-1252. Line ends become
/// `\n`; NULs and other control characters (but tabs) are dropped.
pub fn decode(bytes: &[u8]) -> String {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        utf16(rest, false)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest, true)
    } else if let Some(be) = utf16_without_mark(bytes) {
        utf16(bytes, be)
    } else {
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => bytes.iter().map(|&b| windows_1252(b)).collect(),
        }
    };
    clean(&text)
}

/// UTF-16 without a byte-order mark: most of the first characters have a zero high byte
/// (Latin text). `Some(true)` = big-endian.
fn utf16_without_mark(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let pairs = bytes.as_chunks::<2>().0.iter().take(512);
    let n = pairs.len();
    let (mut even, mut odd) = (0, 0);
    for p in pairs {
        even += usize::from(p[0] == 0);
        odd += usize::from(p[1] == 0);
    }
    if odd * 4 >= n * 3 && even * 10 <= n {
        Some(false)
    } else if even * 4 >= n * 3 && odd * 10 <= n {
        Some(true)
    } else {
        None
    }
}

fn utf16(bytes: &[u8], big_endian: bool) -> String {
    let units = bytes.as_chunks::<2>().0.iter().map(|p| {
        if big_endian {
            u16::from_be_bytes(*p)
        } else {
            u16::from_le_bytes(*p)
        }
    });
    char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}

/// A Windows-1252 byte as a character (0x80–0x9F: the code page's own; the rest as Latin-1).
fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[usize::from(b - 0x80)],
        _ => char::from(b),
    }
}

/// `\r\n` and `\r` → `\n`; a byte-order mark, NULs and control characters (but tabs and line
/// ends) dropped.
fn clean(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    text.chars()
        .filter(|&c| c == '\n' || c == '\t' || !(c.is_control() || c == '\u{FEFF}'))
        .collect()
}

/// Which format a file is, from its text and (as a hint) its name.
pub fn sniff(text: &str, file_name: Option<&str>) -> Format {
    let ext = file_name
        .and_then(|n| n.rsplit_once('.'))
        .map(|(_, e)| e.to_ascii_lowercase());
    let lines = || text.lines().map(str::trim).filter(|l| !l.is_empty());
    let usr = lines().take(5).any(|l| l.eq_ignore_ascii_case("[File]"))
        || (lines().any(|l| l.starts_with("[S ")) && lines().any(|l| starts_with_ci(l, "Words=")));
    if usr || matches!(ext.as_deref(), Some("usr" | "bin")) {
        return Format::Usr;
    }
    let chordpro = lines().any(|l| {
        l.starts_with('{') && l.ends_with('}') && l[1..].starts_with(|c: char| c.is_alphabetic())
    });
    if chordpro
        || matches!(
            ext.as_deref(),
            Some("cho" | "chordpro" | "chopro" | "crd" | "pro")
        )
    {
        return Format::ChordPro;
    }
    Format::Lyrics
}

/// Whether pasted text looks like SongSelect lyrics: a `CCLI Song #` line or the SongSelect
/// terms line (for Reflow's smart paste).
pub fn looks_like_lyrics(text: &str) -> bool {
    text.lines().any(|l| {
        matches!(
            footer_line(l),
            Some(FooterLine::SongNumber(_) | FooterLine::Terms)
        )
    })
}

/// A footer line of a SongSelect file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FooterLine {
    /// `CCLI Song # 1234567` (the number, digits only; `None` when the line has none).
    SongNumber(Option<String>),
    /// `© 2019 Example Songs | Other Music` (`(c)`, `Copyright` too; also `Public Domain`).
    Copyright(Copyright),
    /// "For use solely with the SongSelect Terms of Use. All rights reserved. www.ccli.com".
    Terms,
    /// `CCLI License # 7654321` (the church's licence: not the song's metadata).
    Licence(Option<String>),
}

/// A copyright line read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Copyright {
    /// The (first) year.
    pub year: Option<u32>,
    /// The owners, joined with "; ".
    pub owners: Option<String>,
}

/// `s` starts with `prefix`, ignoring ASCII case.
fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(prefix))
}

/// `s` without `prefix` (ignoring ASCII case).
fn strip_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    starts_with_ci(s, prefix).then(|| &s[prefix.len()..])
}

/// The number after `CCLI Song` / `CCLI License`: `# 123`, `No. 123`, `: 123`, `number 123`.
fn number_after(rest: &str) -> Option<String> {
    let mut r = rest.trim_start();
    loop {
        let before = r;
        r = r.trim_start_matches(['#', ':', '.', '-', ' ', '\t']);
        for w in ["number", "nr", "no"] {
            if let Some(x) = strip_ci(r, w) {
                r = x;
            }
        }
        if r == before {
            break;
        }
    }
    let digits: String = r.chars().take_while(char::is_ascii_digit).collect();
    (!digits.is_empty() && digits.len() <= 12).then_some(digits)
}

/// Classifies a footer line (see [`FooterLine`]); `None` for any other line.
pub fn footer_line(line: &str) -> Option<FooterLine> {
    let t = line.trim();
    if t.is_empty() || t.len() > 400 {
        return None;
    }
    if let Some(rest) = strip_ci(t, "CCLI Song") {
        let n = number_after(rest);
        return (n.is_some() || rest.trim().trim_matches(['#', ':']).is_empty())
            .then_some(FooterLine::SongNumber(n));
    }
    for p in ["CCLI License", "CCLI Licence", "CCLI Lizenz", "CCLI-Lizenz"] {
        if let Some(rest) = strip_ci(t, p) {
            return Some(FooterLine::Licence(number_after(rest)));
        }
    }
    let lower = t.to_lowercase();
    if lower.starts_with("for use solely with")
        || (lower.contains("songselect") && lower.contains("terms of use"))
        || lower.starts_with("all rights reserved")
    {
        return Some(FooterLine::Terms);
    }
    if lower.trim_end_matches('.') == "public domain" {
        return Some(FooterLine::Copyright(Copyright {
            year: None,
            owners: Some(t.trim_end_matches('.').to_string()),
        }));
    }
    let rest = t
        .strip_prefix('©')
        .or_else(|| strip_ci(t, "(c)"))
        .or_else(|| strip_ci(t, "copyright ©"))
        .or_else(|| strip_ci(t, "copyright "))?;
    Some(FooterLine::Copyright(copyright(rest)))
}

/// `2019 Example Songs | Other Music (Admin. by Example Admin)` → year and owners ("; ").
pub fn copyright(value: &str) -> Copyright {
    let mut r = value.trim().trim_start_matches('©').trim_start();
    let mut year = None;
    // Leading years ("2007", "2007, 2010", "2007-2010"): the first one is kept.
    loop {
        let digits: String = r.chars().take_while(char::is_ascii_digit).collect();
        if digits.len() != 4 || r[4..].starts_with(|c: char| c.is_alphanumeric()) {
            break;
        }
        year = year.or(digits.parse().ok());
        r = r[4..].trim_start_matches([',', '-', '–', '/', ' ', '\t']);
    }
    let owners = r
        .split(['|', ';'])
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    Copyright {
        year,
        owners: (!owners.is_empty()).then_some(owners),
    }
}

/// `A Writer | B Writer` → "A Writer, B Writer" ("; " when a name has a comma in it).
pub fn authors(value: &str) -> Option<String> {
    let names: Vec<&str> = value
        .split('|')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .collect();
    if names.is_empty() {
        return None;
    }
    let sep = if names.iter().any(|n| n.contains(',')) {
        "; "
    } else {
        ", "
    };
    Some(names.join(sep))
}

/// Applies a footer line to the song (the licence number goes to [`ChordProSong::license`]:
/// it is the church's, not the song's).
pub fn apply_footer(song: &mut ChordProSong, line: &FooterLine) {
    match line {
        FooterLine::SongNumber(n) => {
            if n.is_some() {
                song.ccli = n.clone();
            }
        }
        FooterLine::Copyright(c) => {
            if c.owners.is_some() {
                song.copyright = c.owners.clone();
            }
            if c.year.is_some() {
                song.year = c.year;
            }
        }
        FooterLine::Licence(n) => {
            if n.is_some() {
                song.license = n.clone();
            }
        }
        FooterLine::Terms => {}
    }
}

/// Section headings SongSelect writes (lower case), in English and other languages, with
/// their kind.
pub const HEADINGS: &[(&str, &str)] = &[
    ("verse", "verse"),
    ("chorus", "chorus"),
    ("pre-chorus", "part"),
    ("prechorus", "part"),
    ("pre chorus", "part"),
    ("bridge", "bridge"),
    ("tag", "part"),
    ("intro", "part"),
    ("outro", "part"),
    ("ending", "part"),
    ("interlude", "part"),
    ("instrumental", "part"),
    ("refrain", "chorus"),
    ("vamp", "part"),
    ("coda", "part"),
    ("turnaround", "part"),
    ("misc", "part"),
    ("spoken", "part"),
    // Spanish
    ("verso", "verse"),
    ("estrofa", "verse"),
    ("coro", "chorus"),
    ("pre-coro", "part"),
    ("precoro", "part"),
    ("puente", "bridge"),
    ("final", "part"),
    ("introducción", "part"),
    ("interludio", "part"),
    // Portuguese
    ("refrão", "chorus"),
    ("pré-refrão", "part"),
    ("ponte", "bridge"),
    // French
    ("couplet", "verse"),
    ("pré-refrain", "part"),
    ("pont", "bridge"),
    ("fin", "part"),
    // German
    ("strophe", "verse"),
    ("vers", "verse"),
    ("schluss", "part"),
    // Dutch, Italian
    ("refrein", "chorus"),
    ("brug", "bridge"),
    ("strofa", "verse"),
    ("ritornello", "chorus"),
];

/// A heading line ("Verse 1", "Chorus", "Coro 2:", "[Bridge]") → (kind, name as written).
pub fn heading(line: &str) -> Option<(&'static str, String)> {
    let t = line
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches(':')
        .trim();
    if t.is_empty() || t.chars().count() > 32 {
        return None;
    }
    let lower = t.to_lowercase();
    let (word, kind) = HEADINGS
        .iter()
        .filter(|(w, _)| lower.starts_with(w))
        .max_by_key(|(w, _)| w.len())?;
    let rest = lower[word.len()..].trim_start();
    // An optional number, maybe with one letter ("Verse 1", "Verse 2b", "Chorus 2").
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    let tail: String = rest.chars().skip(digits).collect();
    let tail = tail.trim();
    let ok = rest.is_empty()
        || (digits > 0
            && (tail.is_empty()
                || (tail.chars().count() == 1 && tail.chars().all(char::is_alphabetic))));
    ok.then(|| (*kind, t.split_whitespace().collect::<Vec<_>>().join(" ")))
}

fn paragraph(text: &str) -> Paragraph {
    chordpro::paragraph(text)
}

/// Builds a song's sections: a section with the same name and words as an earlier one is a
/// repeat in the play order.
#[derive(Default)]
struct Sections {
    song: ChordProSong,
    verses: usize,
}

impl Sections {
    /// Adds a section of stanzas (dropped when it has no lines).
    fn push(&mut self, kind: &str, name: Option<String>, stanzas: Vec<Vec<Paragraph>>) {
        let stanzas: Vec<Vec<Paragraph>> = stanzas.into_iter().filter(|s| !s.is_empty()).collect();
        if stanzas.is_empty() || self.song.sections.len() >= MAX_SECTIONS {
            return;
        }
        let name = name.unwrap_or_else(|| {
            self.verses += 1;
            format!("Verse {}", self.verses)
        });
        if let Some(i) = self
            .song
            .sections
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(&name) && s.stanzas == stanzas)
        {
            self.song.order.push(i);
            return;
        }
        let n = stanzas.len();
        self.song.order.push(self.song.sections.len());
        self.song.sections.push(ChordProSection {
            name,
            kind: kind.to_string(),
            stanzas,
            seconds: vec![Vec::new(); n],
            comments: vec![Vec::new(); n],
        });
    }
}

/// Where the footer starts (an index into `lines`), reading it into `song`.
fn read_footer(lines: &[&str], song: &mut ChordProSong) -> usize {
    let mut start = lines.len();
    let mut found = Vec::new();
    let mut authors_at = None;
    let mut i = lines.len();
    while i > 0 {
        let l = lines[i - 1].trim();
        if l.is_empty() {
            i -= 1;
            continue;
        }
        if let Some(f) = footer_line(l) {
            found.push(f);
            start = i - 1;
            i -= 1;
            continue;
        }
        // The authors line: right before the `CCLI Song #` line (new files) or right after it
        // (old ones), and only one.
        let next_is_number = lines
            .get(i)
            .is_some_and(|n| matches!(footer_line(n), Some(FooterLine::SongNumber(_))));
        let prev_is_number = lines[..i - 1]
            .iter()
            .rev()
            .find(|p| !p.trim().is_empty())
            .is_some_and(|p| matches!(footer_line(p), Some(FooterLine::SongNumber(_))));
        if authors_at.is_none() && (next_is_number || prev_is_number) {
            authors_at = Some(i - 1);
            start = i - 1;
            i -= 1;
            continue;
        }
        break;
    }
    // Earlier lines first, so the last copyright / number wins as in the file.
    for f in found.iter().rev() {
        apply_footer(song, f);
    }
    if let Some(a) = authors_at {
        song.authors = authors(lines[a]);
    }
    start
}

/// Reads SongSelect lyrics text (see the module docs). A text with no lyrics gives a song with
/// no sections.
pub fn parse_lyrics(text: &str) -> ChordProSong {
    let text = clean(text);
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let mut b = Sections::default();
    let end = read_footer(&lines, &mut b.song);
    let mut i = lines
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(end);
    if i < end && heading(lines[i]).is_none() {
        b.song.title = Some(lines[i].trim().to_string());
        i += 1;
    }
    let mut cur: Option<(&'static str, Option<String>)> = None;
    let mut stanzas: Vec<Vec<Paragraph>> = Vec::new();
    let mut stanza: Vec<Paragraph> = Vec::new();
    for l in &lines[i.min(end)..end] {
        let l = l.trim();
        if l.is_empty() {
            if !stanza.is_empty() {
                stanzas.push(std::mem::take(&mut stanza));
            }
            continue;
        }
        if let Some((kind, name)) = heading(l) {
            if !stanza.is_empty() {
                stanzas.push(std::mem::take(&mut stanza));
            }
            let (k, n) = cur.take().unwrap_or(("verse", None));
            b.push(k, n, std::mem::take(&mut stanzas));
            cur = Some((kind, Some(name)));
            continue;
        }
        // Lyrics before any heading: a "Verse N" per stanza.
        if cur.is_none() && stanza.is_empty() && !stanzas.is_empty() {
            b.push("verse", None, std::mem::take(&mut stanzas));
        }
        stanza.push(paragraph(l));
    }
    if !stanza.is_empty() {
        stanzas.push(stanza);
    }
    let (k, n) = cur.unwrap_or(("verse", None));
    b.push(k, n, stanzas);
    b.song
}

/// A `/t`-separated USR value.
fn usr_list(v: &str) -> Vec<&str> {
    v.split("/t").collect()
}

/// Reads a SongShow Plus `.usr` / `.bin` file (see the module docs).
pub fn parse_usr(text: &str) -> ChordProSong {
    let text = clean(text);
    let mut b = Sections::default();
    let (mut fields, mut words, mut admin) = (None, None, None);
    for line in text.lines() {
        let l = line.trim();
        if let Some(h) = l.strip_prefix('[') {
            // `[S A1234567]`: the song's CCLI number.
            let h = h.trim_end_matches(']').trim();
            if let Some(id) = h.strip_prefix('S').filter(|r| r.starts_with(' ')) {
                let digits: String = id
                    .trim()
                    .trim_start_matches(|c: char| c.is_ascii_alphabetic())
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect();
                if !digits.is_empty() && digits.len() <= 12 {
                    b.song.ccli = Some(digits);
                }
            }
            continue;
        }
        let Some((k, v)) = l.split_once('=') else {
            continue;
        };
        let v = v.trim();
        let some = || Some(v.to_string()).filter(|v| !v.is_empty());
        match k.trim().to_ascii_lowercase().as_str() {
            "title" => b.song.title = some(),
            "author" => b.song.authors = authors(v),
            "copyright" => {
                let c = copyright(v);
                b.song.copyright = c.owners;
                b.song.year = c.year;
            }
            "admin" => admin = some(),
            "keys" => {
                b.song.key = usr_list(v)
                    .into_iter()
                    .map(str::trim)
                    .find(|k| !k.is_empty())
                    .map(str::to_string)
            }
            "fields" => fields = Some(v.to_string()),
            "words" => words = Some(v.to_string()),
            _ => {}
        }
    }
    if let Some(a) = admin {
        let owners = b.song.copyright.take().unwrap_or_default();
        b.song.copyright = Some(if owners.is_empty() {
            format!("Admin. by {a}")
        } else if owners.contains(&a) {
            owners
        } else {
            format!("{owners} (Admin. by {a})")
        });
    }
    let fields = fields.unwrap_or_default();
    let names = usr_list(&fields);
    let words = words.unwrap_or_default();
    for (i, w) in usr_list(&words).into_iter().enumerate() {
        let mut stanzas = vec![Vec::new()];
        for l in w.split("/n") {
            let l = l.trim();
            if l.is_empty() {
                stanzas.push(Vec::new());
            } else if let Some(s) = stanzas.last_mut() {
                s.push(paragraph(l));
            }
        }
        let name = names
            .get(i)
            .map(|n| n.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|n| !n.is_empty());
        let kind = name
            .as_deref()
            .and_then(heading)
            .map_or("verse", |(k, _)| k);
        b.push(kind, name, stanzas);
    }
    b.song
}

/// Reads a SongSelect file of any [`Format`] (ChordPro through [`chordpro::parse`]).
pub fn parse(text: &str, format: Format) -> ChordProSong {
    match format {
        Format::Lyrics => parse_lyrics(text),
        Format::Usr => parse_usr(text),
        Format::ChordPro => chordpro::parse(&clean(text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(s: &ChordProSong) -> Vec<&str> {
        s.sections.iter().map(|x| x.name.as_str()).collect()
    }

    fn words(s: &ChordProSong, i: usize) -> Vec<Vec<String>> {
        s.sections[i]
            .stanzas
            .iter()
            .map(|st| st.iter().map(|p| p.plain_text()).collect())
            .collect()
    }

    // Everything here is made up: titles, names, words, numbers.
    const NEW: &str = "Lanterns Over Kettle Bay

Verse 1
Lanterns over Kettle Bay
Calling every sleeper home

Chorus
Rise and sing the harbour song
Rise and sing it all night long

Verse 2
Morning comes on silver wings

Chorus
Rise and sing the harbour song
Rise and sing it all night long

Ending
All night long

Alex Example | Jamie Sample
CCLI Song # 1234567
© 2019 Example Songs Publishing | Sample Music Co. (Admin. by Example Admin)
For use solely with the SongSelect® Terms of Use. All rights reserved. www.ccli.com
CCLI License # 7654321
";

    #[test]
    fn lyrics_with_the_new_footer() {
        let s = parse_lyrics(NEW);
        assert_eq!(s.title.as_deref(), Some("Lanterns Over Kettle Bay"));
        assert_eq!(names(&s), ["Verse 1", "Chorus", "Verse 2", "Ending"]);
        // The second chorus has the same words: a repeat in the order.
        assert_eq!(s.order, [0, 1, 2, 1, 3]);
        assert_eq!(
            words(&s, 1),
            [[
                "Rise and sing the harbour song",
                "Rise and sing it all night long"
            ]]
        );
        assert_eq!(s.authors.as_deref(), Some("Alex Example, Jamie Sample"));
        assert_eq!(s.ccli.as_deref(), Some("1234567"));
        assert_eq!(s.year, Some(2019));
        assert_eq!(
            s.copyright.as_deref(),
            Some("Example Songs Publishing; Sample Music Co. (Admin. by Example Admin)")
        );
        let info = s.song_info().unwrap();
        assert!(
            info.display,
            "a song with a CCLI number shows its copyright line"
        );
        assert_eq!(info.ccli_number.as_deref(), Some("1234567"));
        assert_eq!(info.copyright_year, Some(2019));
    }

    #[test]
    fn lyrics_with_the_old_footer() {
        let text = "Paper Boats\n\nVerse 1\nPaper boats upon the stream\n\nChorus\nFloat away\n\nCCLI Song # 7000001\nRiley Placeholder | Casey Invented\n© 2004 Imaginary Hymns; Fictional Tunes\nFor use solely with the SongSelect Terms of Use.  All rights reserved. www.ccli.com\nCCLI Licence No. 1112223\n";
        let s = parse_lyrics(text);
        assert_eq!(s.title.as_deref(), Some("Paper Boats"));
        assert_eq!(names(&s), ["Verse 1", "Chorus"]);
        assert_eq!(
            s.authors.as_deref(),
            Some("Riley Placeholder, Casey Invented")
        );
        assert_eq!(s.ccli.as_deref(), Some("7000001"));
        assert_eq!(s.year, Some(2004));
        assert_eq!(
            s.copyright.as_deref(),
            Some("Imaginary Hymns; Fictional Tunes")
        );
        assert_eq!(words(&s, 1), [["Float away"]]);
    }

    #[test]
    fn lyrics_without_a_footer_or_headings() {
        let s =
            parse_lyrics("Hum Of The Kettle\n\nFirst line here\nSecond line here\n\nThird line\n");
        assert_eq!(s.title.as_deref(), Some("Hum Of The Kettle"));
        assert_eq!(names(&s), ["Verse 1", "Verse 2"]);
        assert!(s.ccli.is_none() && s.authors.is_none());
        assert!(s.song_info().is_some_and(|i| !i.display));
        // A stanza after a blank line inside a section stays in that section.
        let s = parse_lyrics("T\nChorus\nOne\n\nTwo\n");
        assert_eq!(names(&s), ["Chorus"]);
        assert_eq!(words(&s, 0), [["One"], ["Two"]]);
    }

    #[test]
    fn localised_headings_and_odd_lines() {
        let s = parse_lyrics(
            "Canción De Prueba\n\nEstrofa 1\nUna línea\n\nCoro\nOtra línea\n\nPuente:\nY otra más\n\nFinal\nFin de la canción\n",
        );
        assert_eq!(names(&s), ["Estrofa 1", "Coro", "Puente", "Final"]);
        assert_eq!(s.sections[1].kind, "chorus");
        // A lyric line that only starts like a heading is a lyric line.
        assert!(heading("Final hour has come").is_none());
        assert!(heading("Verse 2b").is_some());
        assert!(heading("Verse two").is_none());
        // Nothing but a title, an empty text, junk: no panic, no sections.
        assert!(parse_lyrics("Just A Title").sections.is_empty());
        assert!(parse_lyrics("").sections.is_empty());
        assert!(
            parse_lyrics("\n\n\u{0}\u{1}ÿ\n CCLI Song #\n")
                .sections
                .is_empty()
        );
        let _ = parse_lyrics("©\n©9999x\nCCLI Song # 99999999999999999999\n(c)");
    }

    #[test]
    fn encodings() {
        let text = "Ünïcödé Sóng\n\nVerse 1\nCafé ‘quoted’ – dash\n";
        // UTF-8 with a BOM, CRLF line ends.
        let mut bom = vec![0xEF, 0xBB, 0xBF];
        bom.extend(text.replace('\n', "\r\n").as_bytes());
        assert_eq!(decode(&bom), text);
        // UTF-16 LE and BE with a mark, LE without one.
        let le: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let be: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
        assert_eq!(decode(&[&[0xFF, 0xFE][..], &le].concat()), text);
        assert_eq!(decode(&[&[0xFE, 0xFF][..], &be].concat()), text);
        assert_eq!(decode(&le), text);
        // Windows-1252: curly quotes and the en dash are 0x91, 0x92, 0x96; é is 0xE9.
        let cp: Vec<u8> = b"Caf\xE9 \x91quoted\x92 \x96 dash".to_vec();
        assert_eq!(decode(&cp), "Café ‘quoted’ – dash");
        // Any bytes: no panic.
        let junk: Vec<u8> = (0..=255u8).cycle().take(3001).collect();
        let _ = parse_lyrics(&decode(&junk));
        let _ = parse_usr(&decode(&junk));
    }

    const USR: &str = "[File]
Type=SongSelect Import File
Version=3.0
[S A1234567]
Title=Lanterns Over Kettle Bay
Author=Alex Example | Jamie Sample
Copyright=2019 Example Songs Publishing | Sample Music Co.
Admin=Example Admin
Themes=Hope/tLight
Keys=G
Fields=Verse 1/tChorus/tVerse 2/tChorus
Words=Lanterns over Kettle Bay/nCalling every sleeper home/tRise and sing the harbour song/nRise and sing it all night long/tMorning comes on silver wings/n/nSecond stanza here/tRise and sing the harbour song/nRise and sing it all night long
";

    #[test]
    fn usr_files() {
        assert_eq!(sniff(USR, None), Format::Usr);
        let s = parse_usr(USR);
        assert_eq!(s.title.as_deref(), Some("Lanterns Over Kettle Bay"));
        assert_eq!(s.ccli.as_deref(), Some("1234567"));
        assert_eq!(s.authors.as_deref(), Some("Alex Example, Jamie Sample"));
        assert_eq!(s.year, Some(2019));
        assert_eq!(
            s.copyright.as_deref(),
            Some("Example Songs Publishing; Sample Music Co. (Admin. by Example Admin)")
        );
        assert_eq!(s.key.as_deref(), Some("G"));
        assert_eq!(names(&s), ["Verse 1", "Chorus", "Verse 2"]);
        assert_eq!(s.order, [0, 1, 2, 1]);
        assert_eq!(
            words(&s, 2),
            [
                vec!["Morning comes on silver wings"],
                vec!["Second stanza here"]
            ]
        );
        // Words without names, names without words, malformed lines.
        let s = parse_usr("[S]\nWords=One/tTwo\nFields=/t\n=\nnoequals\n[S A99x]");
        assert_eq!(names(&s), ["Verse 1", "Verse 2"]);
        assert_eq!(s.ccli.as_deref(), Some("99"));
        assert!(parse_usr("Fields=Verse 1/tChorus").sections.is_empty());
    }

    #[test]
    fn sniffing_and_footer_lines() {
        assert_eq!(sniff(NEW, Some("song.txt")), Format::Lyrics);
        assert_eq!(sniff("{title: X}\n[G]Words", None), Format::ChordPro);
        assert_eq!(sniff("words", Some("x.CHO")), Format::ChordPro);
        assert_eq!(sniff("words", Some("x.usr")), Format::Usr);
        assert!(looks_like_lyrics(NEW));
        assert!(!looks_like_lyrics("Verse 1\nJust words\n"));
        assert_eq!(
            footer_line("CCLI Song No. 1234567"),
            Some(FooterLine::SongNumber(Some("1234567".into())))
        );
        assert_eq!(
            footer_line("Copyright © 2001, 2010 Example Music"),
            Some(FooterLine::Copyright(Copyright {
                year: Some(2001),
                owners: Some("Example Music".into())
            }))
        );
        assert_eq!(
            footer_line("Public Domain"),
            Some(FooterLine::Copyright(Copyright {
                year: None,
                owners: Some("Public Domain".into())
            }))
        );
        assert_eq!(footer_line("Copycat lines"), None);
        assert_eq!(footer_line("Ccli song lyrics"), None);
        assert_eq!(
            authors("Smith, Alex | Doe, Jamie").as_deref(),
            Some("Smith, Alex; Doe, Jamie")
        );
    }
}
