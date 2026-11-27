//! ChordPro: songs as text with inline chords (`[C]Morning [G/B]light`), to and from the model.
//!
//! ## Reading ([`parse`])
//! - **Directives** `{name: value}` / `{name}` (case-insensitive, short forms too): `title` /
//!   `t`, `subtitle` / `st`, `artist`, `composer`, `lyricist`, `copyright`, `year`, `album`,
//!   `key`, `capo`, `tempo`, `ccli`, and `meta: <name> <value>` for the same names.
//! - **Sections**: `start_of_verse` / `sov`, `start_of_chorus` / `soc`, `start_of_bridge` /
//!   `sob`, `start_of_part` and any other `start_of_<kind>` (with an optional label: `{sov:
//!   Verse 2}`), ended by the matching `end_of_…` (or the next section). `{chorus}` plays the last
//!   chorus again (an arrangement repeat). Unlabelled sections are named like other imports:
//!   "Verse 1", "Verse 2", …, "Chorus", "Bridge" (a second unlabelled chorus is "Chorus 2").
//!   A `{comment: …}` (`c`, `ci`, `cb`, `highlight`) or a line on its own (`Chorus`, `Verse 2:`,
//!   `[Bridge]`) that names a section kind starts that section; other comments are skipped.
//!   Lines before any section form "Verse 1", …, one per stanza.
//! - **Lines**: chords in brackets sit over the character that follows them. A plain-text
//!   chord line (every word a chord) above a lyric line is read the same way, by column. A
//!   blank line ends a stanza: slides never span stanzas. `#` lines, tabs and grids are
//!   skipped; `{new_song}` ends the first song.
//!
//! ## Two languages ([`parse_two`], docs/DUAL-LANGUAGE-PLAN.md §2)
//! ChordPro has no translation feature of its own, so a two-language file uses one of these
//! conventions ([`SecondSplit`]); the first language keeps the chords, the second is plain text
//! (chords written on its lines are dropped):
//! - **Alternating lines** in each stanza: first, second, first, second… (a chord line above a
//!   lyric line belongs to that line; a line of chords alone doesn't count).
//! - **A separator line** in each stanza (`---`): the first language above, the second below.
//! - **A separator inside each line**: `[G]Morning light / Luz de la mañana` (`/`, `|`, …),
//!   outside the chords (`[G/B]` is a chord, not a separator).
//! - **Italic comments**: `{comment_italic: …}` / `{ci: …}` lines inside a stanza are its second
//!   language (how printed bilingual chord sheets often set a translation under the lyrics; other
//!   ChordPro programs show them in italics). Vesper's export writes this one.
//!
//! These are the splits of Reflow's "Paste two languages" (`apps/control/src/lib/edit/
//! language.ts` is the reference) plus the comments style. `{meta: language en}` names the
//! language of the lyrics (the presentation's own language), `{meta: second_language es}` names
//! the second language; `{meta: second_language_split comments}` (also `alternating`, `line`,
//! `inline /`) says which convention the file uses, for [`SecondSplit::Auto`].
//!
//! ## Writing ([`export`])
//! Metadata directives (and `{meta: language …}` when the presentation has a language), then
//! each section in document order as `{start_of_verse: Verse 1}` /
//! `{start_of_chorus: …}` / `{start_of_bridge: …}` / `{start_of_part: …}`, a blank line between
//! slides, the primary text of each slide with its chords inline. Slides with a second text get
//! it as `{ci: …}` lines after their own lines, and the header names the language and the
//! convention (`{meta: second_language es}`, `{meta: second_language_split comments}`), so the
//! file reads back the same way.

use crate::chords::{Chord, ChordMark};
use crate::text::{Paragraph, TextRun};
use crate::{Presentation, SongInfo};
use serde::{Deserialize, Serialize};

/// How a two-language ChordPro file carries its second language (see the module docs).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SecondSplit {
    /// One language: every lyric line is the first language (and `---` is a lyric line).
    #[default]
    None,
    /// What the file says (`{meta: second_language_split …}`), else a `---` line in a stanza,
    /// else one language.
    Auto,
    /// Lines alternate in each stanza: first, second, first, second…
    Alternating,
    /// A `---` line in each stanza: the first language above, the second below.
    Line,
    /// Inside each line: `first <separator> second` (outside the chords).
    Inline {
        /// The separator (`/`, `|`).
        separator: String,
    },
    /// `{comment_italic: …}` / `{ci: …}` lines inside a stanza are its second language.
    Comments,
}

impl SecondSplit {
    /// `alternating`, `line`, `inline /`, `comments` (the `second_language_split` meta value).
    fn from_meta(v: &str) -> Option<SecondSplit> {
        let v = v.trim();
        let (word, rest) = match v.find(char::is_whitespace) {
            Some(i) => (&v[..i], v[i..].trim()),
            None => (v, ""),
        };
        Some(match word.to_ascii_lowercase().as_str() {
            "alternating" => SecondSplit::Alternating,
            "line" => SecondSplit::Line,
            "inline" if !rest.is_empty() => SecondSplit::Inline {
                separator: rest.to_string(),
            },
            "comments" => SecondSplit::Comments,
            "none" => SecondSplit::None,
            _ => return None,
        })
    }
}

/// A parsed ChordPro song.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChordProSong {
    /// `{title}`.
    pub title: Option<String>,
    /// `{subtitle}`.
    pub subtitle: Option<String>,
    /// `{artist}`.
    pub artist: Option<String>,
    /// `{composer}` and `{lyricist}`, joined with ", ".
    pub authors: Option<String>,
    /// `{copyright}`.
    pub copyright: Option<String>,
    /// `{year}`.
    pub year: Option<u32>,
    /// `{album}`.
    pub album: Option<String>,
    /// `{key}`.
    pub key: Option<String>,
    /// `{capo}`.
    pub capo: Option<u8>,
    /// `{tempo}`.
    pub tempo: Option<String>,
    /// `{ccli}`.
    pub ccli: Option<String>,
    /// Sections in document order.
    pub sections: Vec<ChordProSection>,
    /// Play order (indexes into `sections`, repeats from `{chorus}`).
    pub order: Vec<usize>,
    /// `{meta: language …}`: the language of the lyrics (a BCP 47 code, in its usual case;
    /// a value that is not a code is ignored).
    pub language: Option<String>,
    /// `{meta: second_language …}`: the second language's code (normalized like
    /// `language`; a value that is not a code is ignored).
    pub second_language: Option<String>,
    /// `{meta: second_language_split …}`: the file's two-language convention.
    pub second_split: Option<SecondSplit>,
}

/// A section: its name and its stanzas (lines separated by blank lines).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChordProSection {
    /// "Verse 1", "Chorus", …
    pub name: String,
    /// The kind (`verse`, `chorus`, `bridge`, `part`, …).
    pub kind: String,
    /// Stanzas of lines (paragraphs with chords).
    pub stanzas: Vec<Vec<Paragraph>>,
    /// Each stanza's second-language lines (parallel to `stanzas`; filled by [`parse_two`],
    /// empty for one language).
    pub seconds: Vec<Vec<String>>,
    /// Each stanza's italic comment lines (`{ci: …}` / `{comment_italic: …}` that name no
    /// section; parallel to `stanzas`), the second language in the comments convention. Other
    /// comments (`{c: …}`, `{cb: …}`, `{highlight: …}`) are not kept.
    pub comments: Vec<Vec<String>>,
}

impl ChordProSong {
    /// Song metadata for the presentation (`None` when there is none).
    pub fn song_info(&self) -> Option<SongInfo> {
        let info = SongInfo {
            title: self.title.clone(),
            ccli_number: self.ccli.clone(),
            authors: self.authors.clone(),
            artist: self.artist.clone(),
            publisher: self.copyright.clone(),
            copyright_year: self.year,
            album: self.album.clone(),
            original_key: self.key.clone(),
            ..SongInfo::default()
        };
        (info != SongInfo::default()).then_some(info)
    }

    /// The order differs from document order (a repeated chorus).
    pub fn has_repeats(&self) -> bool {
        self.order != (0..self.sections.len()).collect::<Vec<_>>()
    }

    /// Each section's slides: its stanzas, split into slides of at most `lines_per_slide` lines
    /// (0 = a whole stanza per slide).
    pub fn slides(&self, lines_per_slide: usize) -> Vec<Vec<Vec<Paragraph>>> {
        self.slides_two(lines_per_slide)
            .into_iter()
            .map(|s| s.into_iter().map(|(p, _)| p).collect())
            .collect()
    }

    /// [`slides`](Self::slides) with each slide's second-language lines. A stanza split into
    /// several slides splits its second lines with its lyric lines when they pair line by line
    /// (as many second lines as lyric lines; a line of chords alone takes none); when the
    /// counts differ, all of the stanza's second lines stay with its first slide (see
    /// [`unpaired_stanzas`](Self::unpaired_stanzas)) rather than guessing. A stanza whose first
    /// language is empty after the split (only second lines) still gets its slide.
    pub fn slides_two(&self, lines_per_slide: usize) -> Vec<Vec<(Vec<Paragraph>, Vec<String>)>> {
        self.sections
            .iter()
            .map(|s| {
                s.stanzas
                    .iter()
                    .enumerate()
                    .flat_map(|(i, st)| {
                        let second: &[String] = s.seconds.get(i).map_or(&[], Vec::as_slice);
                        let per = if lines_per_slide == 0 {
                            st.len().max(1)
                        } else {
                            lines_per_slide
                        };
                        let k = st.len().div_ceil(per).max(usize::from(!second.is_empty()));
                        let paired = second.len() == lyric_lines(st);
                        (0..k)
                            .map(|j| {
                                let (from, to) =
                                    ((j * per).min(st.len()), ((j + 1) * per).min(st.len()));
                                let lines = if paired {
                                    let a = lyric_lines(&st[..from]);
                                    second[a..a + lyric_lines(&st[from..to])].to_vec()
                                } else if j == 0 {
                                    second.to_vec()
                                } else {
                                    Vec::new()
                                };
                                (st[from..to].to_vec(), lines)
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect()
            })
            .collect()
    }

    /// Stanzas split into several slides whose second lines don't pair line by line with
    /// their lyric lines, so [`slides_two`](Self::slides_two) keeps them all on the stanza's
    /// first slide (for the import's report).
    pub fn unpaired_stanzas(&self, lines_per_slide: usize) -> usize {
        self.sections
            .iter()
            .flat_map(|s| s.stanzas.iter().zip(&s.seconds))
            .filter(|(st, second)| {
                let per = if lines_per_slide == 0 {
                    st.len().max(1)
                } else {
                    lines_per_slide
                };
                !second.is_empty() && st.len() > per && second.len() != lyric_lines(st)
            })
            .count()
    }

    /// Lines of the second language.
    pub fn second_lines(&self) -> usize {
        self.sections
            .iter()
            .flat_map(|s| s.seconds.iter())
            .map(Vec::len)
            .sum()
    }
}

/// Lines with words (a line of chords alone has none, and pairs with no second line).
fn lyric_lines(lines: &[Paragraph]) -> usize {
    lines
        .iter()
        .filter(|p| !p.plain_text().trim().is_empty())
        .count()
}

/// Section kinds a heading or comment may name, with their display names.
const KINDS: &[(&str, &str)] = &[
    ("verse", "Verse"),
    ("chorus", "Chorus"),
    ("refrain", "Chorus"),
    ("pre-chorus", "Pre-Chorus"),
    ("prechorus", "Pre-Chorus"),
    ("bridge", "Bridge"),
    ("tag", "Tag"),
    ("intro", "Intro"),
    ("outro", "Outro"),
    ("ending", "Ending"),
    ("interlude", "Interlude"),
    ("instrumental", "Instrumental"),
    ("turnaround", "Turnaround"),
    ("vamp", "Vamp"),
    ("coda", "Coda"),
];

/// `"Chorus"`, `"Verse 2:"`, `"[Pre-Chorus]"`, `"BRIDGE"` → (kind, section name).
fn heading(line: &str) -> Option<(String, String)> {
    let t = line
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches(':')
        .trim();
    if t.is_empty() || t.len() > 24 {
        return None;
    }
    let lower = t.to_lowercase();
    let (word, rest) = KINDS
        .iter()
        .filter(|(k, _)| lower.starts_with(k))
        .max_by_key(|(k, _)| k.len())?;
    let suffix = lower[word.len()..].trim();
    if !(suffix.is_empty()
        || suffix
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace() || c == 'x'))
    {
        return None;
    }
    let kind = match *rest {
        "Chorus" => "chorus",
        "Bridge" => "bridge",
        "Verse" => "verse",
        _ => "part",
    };
    let num = t[word.len()..].trim();
    let name = if num.is_empty() {
        rest.to_string()
    } else {
        format!("{rest} {num}")
    };
    Some((kind.to_string(), name))
}

/// Reads a line with inline chords: its text and chords.
pub fn parse_inline(line: &str) -> Paragraph {
    let mut text = String::new();
    let mut n = 0u32;
    let mut chords = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']').map(|c| open + c) else {
            break;
        };
        let before = &rest[..open];
        n += before.chars().count() as u32;
        text.push_str(before);
        let chord = rest[open + 1..close].trim();
        if !chord.is_empty() {
            chords.push(ChordMark::new(n, chord));
        }
        rest = &rest[close + 1..];
    }
    text.push_str(rest);
    let mut p = paragraph(text.trim_end());
    let len = p.plain_text().chars().count() as u32;
    for c in &mut chords {
        c.at = c.at.min(len);
    }
    p.chords = chords;
    p
}

fn paragraph(text: &str) -> Paragraph {
    Paragraph {
        runs: if text.is_empty() {
            Vec::new()
        } else {
            vec![TextRun::plain(text)]
        },
        ..Paragraph::default()
    }
}

/// A plain chord line ("G      D/F#   Em"): each chord with its column, or `None` when a word is
/// not a chord.
fn chord_line(line: &str) -> Option<Vec<(u32, String)>> {
    if line.contains('[') || line.trim().is_empty() {
        return None;
    }
    let mut out = Vec::new();
    let mut word = String::new();
    let mut start = 0u32;
    let flush = |word: &mut String, start: u32, out: &mut Vec<(u32, String)>| -> bool {
        if word.is_empty() {
            return true;
        }
        let ok = Chord::parse(word).is_some() || matches!(word.as_str(), "|" | "N.C." | "NC");
        out.push((start, std::mem::take(word)));
        ok
    };
    for (col, c) in line.chars().enumerate() {
        if c.is_whitespace() {
            if !flush(&mut word, start, &mut out) {
                return None;
            }
        } else {
            if word.is_empty() {
                start = col as u32;
            }
            word.push(c);
        }
    }
    if !flush(&mut word, start, &mut out) {
        return None;
    }
    out.retain(|(_, w)| w != "|");
    (!out.is_empty()).then_some(out)
}

/// A chord line over a lyric line → one paragraph.
fn merge_chord_line(chords: &[(u32, String)], lyric: &str) -> Paragraph {
    let mut p = paragraph(lyric.trim_end());
    let len = p.plain_text().chars().count() as u32;
    p.chords = chords
        .iter()
        .map(|(c, ch)| ChordMark::new((*c).min(len), ch.clone()))
        .collect();
    p
}

/// A chord line alone (an instrumental line): the chords over an empty line.
fn chords_only(chords: &[(u32, String)]) -> Paragraph {
    let mut p = paragraph("");
    p.chords = chords
        .iter()
        .map(|(_, c)| ChordMark::new(0, c.clone()))
        .collect();
    p
}

struct Builder {
    song: ChordProSong,
    /// The section being filled (`None` = outside any: each stanza becomes "Verse N").
    current: Option<usize>,
    stanza: Vec<Paragraph>,
    /// The open stanza's italic comment lines.
    notes: Vec<String>,
    counts: Vec<(String, usize)>,
    last_chorus: Option<usize>,
}

impl Builder {
    /// Ends the open stanza into the open section (a new "Verse N" outside any section).
    fn end_stanza(&mut self) {
        if self.stanza.is_empty() {
            return;
        }
        let st = std::mem::take(&mut self.stanza);
        let notes = std::mem::take(&mut self.notes);
        let i = match self.current {
            Some(i) => i,
            None => self.new_section("verse", None),
        };
        self.song.sections[i].stanzas.push(st);
        self.song.sections[i].comments.push(notes);
        self.song.sections[i].seconds.push(Vec::new());
    }

    /// Ends the open stanza and starts a section.
    fn open(&mut self, kind: &str, label: Option<String>) {
        self.end_stanza();
        self.current = Some(self.new_section(kind, label));
    }

    fn new_section(&mut self, kind: &str, label: Option<String>) -> usize {
        let display = KINDS
            .iter()
            .find(|(k, _)| *k == kind)
            .map(|(_, d)| d.to_string())
            .unwrap_or_else(|| "Part".into());
        let n = match self.counts.iter_mut().find(|(k, _)| *k == display) {
            Some((_, n)) => {
                *n += 1;
                *n
            }
            None => {
                self.counts.push((display.clone(), 1));
                1
            }
        };
        let name = label.filter(|l| !l.trim().is_empty()).unwrap_or_else(|| {
            if display == "Verse" || n > 1 {
                format!("{display} {n}")
            } else {
                display.clone()
            }
        });
        let i = self.song.sections.len();
        self.song.sections.push(ChordProSection {
            name,
            kind: kind.to_string(),
            ..ChordProSection::default()
        });
        self.song.order.push(i);
        if kind == "chorus" {
            self.last_chorus = Some(i);
        }
        i
    }
}

fn directive(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    let inner = t.strip_prefix('{')?.strip_suffix('}')?;
    let (name, value) = match inner.find([':', ' ']) {
        Some(i) => (&inner[..i], inner[i + 1..].trim()),
        None => (inner, ""),
    };
    Some((name.trim().to_ascii_lowercase(), value.to_string()))
}

/// Parses a ChordPro song (see the module docs).
pub fn parse(text: &str) -> ChordProSong {
    let mut b = Builder {
        song: ChordProSong::default(),
        current: None,
        stanza: Vec::new(),
        notes: Vec::new(),
        counts: Vec::new(),
        last_chorus: None,
    };
    let mut skipping: Option<&'static str> = None;
    let mut pending_chords: Option<Vec<(u32, String)>> = None;
    let some = |v: &str| Some(v.trim().to_string()).filter(|v| !v.is_empty());
    let lines: Vec<&str> = text.lines().collect();
    for raw in lines {
        let line = raw.trim_end_matches('\r');
        if let Some(end) = skipping {
            if directive(line).is_some_and(|(n, _)| n == end || n == short_end(end)) {
                skipping = None;
            }
            continue;
        }
        if line.trim_start().starts_with('#') {
            continue;
        }
        if let Some((name, value)) = directive(line) {
            if let Some(c) = pending_chords.take() {
                b.stanza.push(chords_only(&c));
            }
            let (name, value) = if name == "meta" {
                let v = value.trim();
                match v.find(char::is_whitespace) {
                    Some(i) => (v[..i].to_ascii_lowercase(), v[i..].trim().to_string()),
                    None => (v.to_ascii_lowercase(), String::new()),
                }
            } else {
                (name, value)
            };
            match name.as_str() {
                "title" | "t" => b.song.title = some(&value),
                "subtitle" | "st" => b.song.subtitle = some(&value),
                "artist" => b.song.artist = some(&value),
                "composer" | "lyricist" => {
                    if let Some(v) = some(&value) {
                        b.song.authors = Some(match b.song.authors.take() {
                            Some(a) if !a.contains(&v) => format!("{a}, {v}"),
                            Some(a) => a,
                            None => v,
                        });
                    }
                }
                "copyright" => b.song.copyright = some(&value),
                "year" => b.song.year = value.trim().parse().ok(),
                "album" => b.song.album = some(&value),
                "key" => b.song.key = some(&value),
                "capo" => b.song.capo = value.trim().parse().ok(),
                "tempo" => b.song.tempo = some(&value),
                "ccli" | "ccli_song_number" | "ccli_number" => b.song.ccli = some(&value),
                "language" => b.song.language = crate::language_code(&value),
                "second_language" => b.song.second_language = crate::language_code(&value),
                "second_language_split" => b.song.second_split = SecondSplit::from_meta(&value),
                "new_song" | "ns" => break,
                "start_of_tab" | "sot" => skipping = Some("end_of_tab"),
                "start_of_grid" | "sog" => skipping = Some("end_of_grid"),
                "chorus" => {
                    b.end_stanza();
                    b.current = None;
                    if let Some(i) = b.last_chorus {
                        b.song.order.push(i);
                    }
                }
                "comment" | "c" | "comment_italic" | "ci" | "comment_box" | "cb" | "highlight" => {
                    if let Some((kind, name)) = heading(&value) {
                        b.open(&kind, Some(name));
                    } else if matches!(name.as_str(), "comment_italic" | "ci")
                        && !b.stanza.is_empty()
                        && let Some(v) = some(&value)
                    {
                        // Only italic comments are a second language; other comments
                        // (performance notes like "repeat 2x") are skipped.
                        b.notes.push(v);
                    }
                }
                n if n.starts_with("end_of_") || n.starts_with("eo") && n.len() == 3 => {
                    b.end_stanza();
                    b.current = None;
                }
                n => {
                    let kind = match n {
                        "soc" => Some("chorus"),
                        "sov" => Some("verse"),
                        "sob" => Some("bridge"),
                        _ => n.strip_prefix("start_of_"),
                    };
                    if let Some(kind) = kind {
                        let kind = match kind {
                            "chorus" | "verse" | "bridge" => kind,
                            _ => "part",
                        };
                        b.open(kind, some(&value));
                    }
                }
            }
            continue;
        }
        if line.trim().is_empty() {
            if let Some(c) = pending_chords.take() {
                b.stanza.push(chords_only(&c));
            }
            b.end_stanza();
            continue;
        }
        if let Some(c) = pending_chords.take() {
            if chord_line(line).is_none() && heading(line).is_none() {
                b.stanza.push(merge_chord_line(&c, line));
                continue;
            }
            b.stanza.push(chords_only(&c));
        }
        if let Some((kind, name)) = heading(line)
            && !line.contains("][")
            && (line.trim().starts_with('[') && line.trim().ends_with(']')
                || chord_line(line).is_none())
        {
            b.open(&kind, Some(name));
            continue;
        }
        if let Some(c) = chord_line(line) {
            pending_chords = Some(c);
            continue;
        }
        b.stanza.push(parse_inline(line));
    }
    if let Some(c) = pending_chords.take() {
        b.stanza.push(chords_only(&c));
    }
    b.end_stanza();
    let mut song = b.song;
    // Drop sections that got no lines (a heading followed by nothing), keeping the order valid.
    let keep: Vec<bool> = song
        .sections
        .iter()
        .map(|s| !s.stanzas.is_empty())
        .collect();
    let mut remap = Vec::with_capacity(keep.len());
    let mut n = 0;
    for k in &keep {
        remap.push(n);
        if *k {
            n += 1;
        }
    }
    song.order = song
        .order
        .iter()
        .filter(|i| keep[**i])
        .map(|i| remap[*i])
        .collect();
    song.sections.retain(|s| !s.stanzas.is_empty());
    song
}

/// Parses a two-language ChordPro song: [`parse`], then each stanza's second language taken
/// out by `split` (see the module docs) into [`ChordProSection::seconds`].
pub fn parse_two(text: &str, split: &SecondSplit) -> ChordProSong {
    let mut song = parse(text);
    let split = match split {
        SecondSplit::Auto => song.second_split.clone().unwrap_or_else(|| {
            let has_line = song
                .sections
                .iter()
                .flat_map(|s| s.stanzas.iter().flatten())
                .any(|p| p.chords.is_empty() && p.plain_text().trim() == "---");
            if has_line {
                SecondSplit::Line
            } else {
                SecondSplit::None
            }
        }),
        other => other.clone(),
    };
    for sec in &mut song.sections {
        for i in 0..sec.stanzas.len() {
            let stanza = std::mem::take(&mut sec.stanzas[i]);
            let notes = sec.comments.get(i).cloned().unwrap_or_default();
            let (first, second) = split_stanza(stanza, notes, &split);
            sec.stanzas[i] = first;
            sec.seconds[i] = second;
        }
    }
    song
}

/// One stanza → its first-language lines (with their chords) and its second-language lines.
fn split_stanza(
    stanza: Vec<Paragraph>,
    notes: Vec<String>,
    split: &SecondSplit,
) -> (Vec<Paragraph>, Vec<String>) {
    let sep = match split {
        SecondSplit::None | SecondSplit::Auto => return (stanza, Vec::new()),
        SecondSplit::Comments => return (stanza, notes),
        SecondSplit::Alternating => {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            let mut n = 0;
            for p in stanza {
                let text = p.plain_text();
                if text.trim().is_empty() {
                    // A line of chords alone (an instrumental line) stays with the first.
                    if !p.chords.is_empty() {
                        a.push(p);
                    }
                    continue;
                }
                if n % 2 == 0 {
                    a.push(p);
                } else {
                    b.push(text.trim().to_string());
                }
                n += 1;
            }
            return (a, b);
        }
        SecondSplit::Line => "---",
        SecondSplit::Inline { separator } => separator.trim(),
    };
    if sep.is_empty() {
        return (stanza, Vec::new());
    }
    // As Reflow's paste: a line that is the separator puts the lines after it in the second
    // language; a line with the separator inside is split there.
    let (mut a, mut b) = (Vec::new(), Vec::new());
    let mut below = false;
    for p in stanza {
        let text = p.plain_text();
        let t = text.trim();
        if t == sep {
            below = true;
        } else if let Some(at) = text.find(sep) {
            let first = text[..at].trim_end();
            let len = first.chars().count() as u32;
            let mut q = paragraph(first);
            q.chords = p.chords.iter().filter(|c| c.at <= len).cloned().collect();
            a.push(q);
            let second = text[at + sep.len()..].trim();
            if !second.is_empty() {
                b.push(second.to_string());
            }
        } else if below {
            if !t.is_empty() {
                b.push(t.to_string());
            }
        } else {
            a.push(p);
        }
    }
    (a, b)
}

fn short_end(end: &str) -> &'static str {
    match end {
        "end_of_tab" => "eot",
        "end_of_grid" => "eog",
        _ => "",
    }
}

/// A line with its chords inline (`[G]Morning [D]light`).
pub fn inline(p: &Paragraph) -> String {
    let text: Vec<char> = p.plain_text().replace('\u{2028}', " ").chars().collect();
    let mut chords: Vec<&ChordMark> = p.chords.iter().collect();
    chords.sort_by_key(|c| c.at);
    let mut out = String::new();
    let mut k = 0;
    for (i, ch) in text.iter().enumerate() {
        while k < chords.len() && chords[k].at as usize <= i {
            out.push_str(&format!("[{}]", chords[k].chord));
            k += 1;
        }
        out.push(*ch);
    }
    for c in &chords[k..] {
        out.push_str(&format!("[{}]", c.chord));
    }
    out
}

/// Writes a presentation as ChordPro (see the module docs). `slide_text` gives a slide's lyric
/// paragraphs and its second-language lines (the server passes its primary text box and that
/// box's second text).
pub fn export(
    p: &Presentation,
    slide_text: impl Fn(&crate::Slide) -> (Vec<Paragraph>, Vec<String>),
) -> String {
    let mut out = String::new();
    let info = p.song_info.clone().unwrap_or_default();
    let title = info.title.clone().unwrap_or_else(|| p.name.clone());
    let mut meta = |k: &str, v: Option<String>| {
        if let Some(v) = v.filter(|v| !v.trim().is_empty()) {
            out.push_str(&format!("{{{k}: {}}}\n", v.trim()));
        }
    };
    meta("title", Some(title));
    meta("artist", info.artist.clone());
    meta("composer", info.authors.clone());
    meta("album", info.album.clone());
    meta("copyright", info.publisher.clone());
    meta("year", info.copyright_year.map(|y| y.to_string()));
    meta("key", info.original_key.clone());
    meta("ccli", info.ccli_number.clone());
    if let Some(l) = &p.language {
        out.push_str(&format!("{{meta: language {l}}}\n"));
    }
    let texts: Vec<_> = p
        .sections
        .iter()
        .map(|s| s.slides.iter().map(&slide_text).collect::<Vec<_>>())
        .collect();
    if texts.iter().flatten().any(|(_, second)| !second.is_empty()) {
        // The language most slides with a second text name.
        let mut langs: Vec<(String, usize)> = Vec::new();
        for s in p.slides() {
            if let Some(l) = s
                .translation
                .as_ref()
                .and_then(|t| crate::language_code(&t.language))
            {
                match langs.iter_mut().find(|(x, _)| *x == l) {
                    Some((_, n)) => *n += 1,
                    None => langs.push((l, 1)),
                }
            }
        }
        let lang = langs
            .iter()
            .max_by_key(|(_, n)| *n)
            .map(|(l, _)| l.as_str());
        if let Some(l) = lang {
            out.push_str(&format!("{{meta: second_language {l}}}\n"));
        }
        out.push_str("{meta: second_language_split comments}\n");
    }
    for (si, s) in p.sections.iter().enumerate() {
        let lower = s.name.to_lowercase();
        let kind = if lower.starts_with("chorus") || lower.starts_with("refrain") {
            "chorus"
        } else if lower.starts_with("verse") {
            "verse"
        } else if lower.starts_with("bridge") {
            "bridge"
        } else {
            "part"
        };
        out.push('\n');
        if s.name.is_empty() {
            out.push_str(&format!("{{start_of_{kind}}}\n"));
        } else {
            out.push_str(&format!("{{start_of_{kind}: {}}}\n", s.name));
        }
        let mut first = true;
        for (paras, second) in &texts[si] {
            // A blank line ends a stanza, so a slide's empty paragraphs can't be written as
            // they are: leading and trailing ones are dropped, and inner ones split the slide
            // into stanzas (as a one-language file reads back). Each piece gets its own `{ci:}`
            // lines when they pair line by line with the lyric lines, else the first piece
            // gets them all (never after a blank line, where they would be lost).
            let blank = |p: &Paragraph| p.chords.is_empty() && p.plain_text().trim().is_empty();
            let pieces: Vec<&[Paragraph]> = paras
                .split(|p| blank(p))
                .filter(|piece| !piece.is_empty())
                .collect();
            if pieces.is_empty() {
                continue;
            }
            let second: Vec<&str> = second
                .iter()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .collect();
            let paired = second.len() == pieces.iter().map(|p| lyric_lines(p)).sum::<usize>();
            let mut next = 0;
            for (k, piece) in pieces.iter().enumerate() {
                if !first {
                    out.push('\n');
                }
                first = false;
                for para in *piece {
                    out.push_str(inline(para).trim_end());
                    out.push('\n');
                }
                let lines = if paired {
                    let n = lyric_lines(piece);
                    next += n;
                    &second[next - n..next]
                } else if k == 0 {
                    &second[..]
                } else {
                    &[]
                };
                for line in lines {
                    out.push_str(&format!("{{ci: {}}}\n", line.replace(['{', '}'], "")));
                }
            }
        }
        out.push_str(&format!("{{end_of_{kind}}}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(p: &Paragraph) -> (String, Vec<(u32, &str)>) {
        (
            p.plain_text(),
            p.chords.iter().map(|c| (c.at, c.chord.as_str())).collect(),
        )
    }

    #[test]
    fn inline_chords() {
        let p = parse_inline("[C]Morning [G/B]light, [Am7]new and [F]bright");
        assert_eq!(
            line(&p),
            (
                "Morning light, new and bright".to_string(),
                vec![(0, "C"), (8, "G/B"), (15, "Am7"), (23, "F")]
            )
        );
        assert_eq!(inline(&p), "[C]Morning [G/B]light, [Am7]new and [F]bright");
        // A chord at the end of the line.
        let p = parse_inline("Hold on [D]");
        assert_eq!(line(&p), ("Hold on".to_string(), vec![(7, "D")]));
        assert_eq!(inline(&p), "Hold on[D]");
    }

    const SONG: &str = "\
{title: Lanterns On The Shore}
{artist: The Harbour Choir}
{composer: A. Writer}
{key: G}
{capo: 2}
{ccli: 1234567}
# a comment line

{start_of_verse}
[G]Lanterns on the [C]shore
[Em]Calling us [D]home
{end_of_verse}

{soc}
[C]Sing, [G/B]sing
[Am7]Sing it [Dsus4]once [D]more
{eoc}

{sov: Verse 2}
[G]Morning on the [C]sea

[Em]Light upon the [D]foam
{eov}

{chorus}
";

    #[test]
    fn a_song_with_directives() {
        let s = parse(SONG);
        assert_eq!(s.title.as_deref(), Some("Lanterns On The Shore"));
        assert_eq!(s.artist.as_deref(), Some("The Harbour Choir"));
        assert_eq!(s.authors.as_deref(), Some("A. Writer"));
        assert_eq!(s.key.as_deref(), Some("G"));
        assert_eq!(s.capo, Some(2));
        assert_eq!(s.ccli.as_deref(), Some("1234567"));
        let names: Vec<&str> = s.sections.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Verse 1", "Chorus", "Verse 2"]);
        assert_eq!(s.order, [0, 1, 2, 1]);
        assert!(s.has_repeats());
        assert_eq!(s.sections[2].stanzas.len(), 2);
        let ch = &s.sections[1].stanzas[0][1];
        assert_eq!(
            line(ch),
            (
                "Sing it once more".to_string(),
                vec![(0, "Am7"), (8, "Dsus4"), (13, "D")]
            )
        );
        let info = s.song_info().unwrap();
        assert_eq!(info.original_key.as_deref(), Some("G"));
        assert_eq!(info.ccli_number.as_deref(), Some("1234567"));
        // Two lines per slide: the verse is one slide; stanzas never share a slide.
        let slides = s.slides(2);
        assert_eq!(slides[0].len(), 1);
        assert_eq!(slides[2].len(), 2);
        assert_eq!(s.slides(1)[0].len(), 2);
    }

    #[test]
    fn headings_comments_and_chord_lines() {
        let text = "\
Verse 1
G         C
Lanterns on the shore
Em        D
Calling us home

{c: Chorus}
[C]Sing, sing

[Bridge]
Am   F
Oh oh oh
G  D  Em
";
        let s = parse(text);
        let names: Vec<&str> = s.sections.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Verse 1", "Chorus", "Bridge"]);
        let v = &s.sections[0].stanzas[0];
        assert_eq!(
            line(&v[0]),
            (
                "Lanterns on the shore".to_string(),
                vec![(0, "G"), (10, "C")]
            )
        );
        let b = &s.sections[2].stanzas[0];
        assert_eq!(
            line(&b[0]),
            ("Oh oh oh".to_string(), vec![(0, "Am"), (5, "F")])
        );
        // A trailing chord line alone: an instrumental line.
        assert_eq!(
            line(&b[1]),
            (String::new(), vec![(0, "G"), (0, "D"), (0, "Em")])
        );
    }

    #[test]
    fn loose_stanzas_become_verses() {
        let s = parse("[G]One line\n\nTwo [D]line\n");
        let names: Vec<&str> = s.sections.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["Verse 1", "Verse 2"]);
        assert!(!s.has_repeats());
        assert!(s.song_info().is_none());
    }

    #[test]
    fn tabs_are_skipped_and_new_song_ends() {
        let s = parse("{sot}\ne|---3---|\n{eot}\n[G]Words\n{new_song}\n[C]Other\n");
        assert_eq!(s.sections.len(), 1);
        assert_eq!(s.sections[0].stanzas[0].len(), 1);
    }

    #[test]
    fn export_round_trips() {
        let s = parse(SONG);
        let mut p = Presentation::new("Lanterns");
        p.song_info = s.song_info();
        for (sec, slides) in s.sections.iter().zip(s.slides(0)) {
            let mut section = crate::Section::new(sec.name.clone());
            for paras in slides {
                let mut slide = crate::Slide::new();
                slide.elements.push(crate::Element::text(
                    crate::Rect::new(0.0, 0.0, 10.0, 10.0),
                    crate::TextBox {
                        paragraphs: paras,
                        ..crate::TextBox::default()
                    },
                ));
                section.slides.push(slide);
            }
            p.sections.push(section);
        }
        let text = export(&p, |s| {
            let paras = s
                .elements
                .iter()
                .find_map(|e| e.text_box())
                .map(|t| t.paragraphs.clone())
                .unwrap_or_default();
            (paras, Vec::new())
        });
        assert!(text.contains("{key: G}"));
        assert!(text.contains("{start_of_chorus: Chorus}\n[C]Sing, [G/B]sing\n"));
        let again = parse(&text);
        assert_eq!(again.sections.len(), 3);
        assert_eq!(again.sections[1].stanzas, s.sections[1].stanzas);
        assert_eq!(again.sections[2].stanzas, s.sections[2].stanzas);
        assert_eq!(again.title.as_deref(), Some("Lanterns On The Shore"));
    }

    /// The made-up second-language lines of a stanza, per section.
    fn seconds(s: &ChordProSong) -> Vec<Vec<Vec<String>>> {
        s.sections.iter().map(|x| x.seconds.clone()).collect()
    }

    /// A line's text and chords (as [`line`] gives them).
    type Line<'a> = (String, Vec<(u32, &'a str)>);

    /// Each stanza's first-language lines.
    fn firsts(s: &ChordProSong) -> Vec<Vec<Line<'_>>> {
        s.sections
            .iter()
            .flat_map(|x| x.stanzas.iter())
            .map(|st| st.iter().map(line).collect())
            .collect()
    }

    #[test]
    fn two_languages_alternating_lines() {
        let text = "\
{sov}
[G]Lanterns on the [C]shore
Faroles en la orilla
G         D
Calling us home
Llamándonos a casa
{eov}
";
        let s = parse_two(text, &SecondSplit::Alternating);
        assert_eq!(
            firsts(&s),
            [vec![
                (
                    "Lanterns on the shore".to_string(),
                    vec![(0, "G"), (16, "C")]
                ),
                ("Calling us home".to_string(), vec![(0, "G"), (10, "D")]),
            ]]
        );
        assert_eq!(
            seconds(&s),
            [vec![vec![
                "Faroles en la orilla".to_string(),
                "Llamándonos a casa".to_string()
            ]]]
        );
        // One language: the same file reads as four lyric lines.
        assert_eq!(parse(text).sections[0].stanzas[0].len(), 4);
        assert_eq!(parse(text).sections[0].seconds, [Vec::<String>::new()]);
    }

    #[test]
    fn two_languages_separator_line_and_auto() {
        let text = "\
{meta: second_language es}
[G]Lanterns on the [C]shore
[Em]Calling us [D]home
---
Faroles en la orilla
Llamándonos a casa

[C]Sing, sing
---
Canta, canta
";
        for split in [SecondSplit::Line, SecondSplit::Auto] {
            let s = parse_two(text, &split);
            assert_eq!(s.second_language.as_deref(), Some("es"));
            assert_eq!(s.sections.len(), 2);
            assert_eq!(s.sections[0].stanzas[0].len(), 2);
            assert_eq!(s.sections[0].stanzas[0][1].chords.len(), 2);
            assert_eq!(
                s.sections[0].seconds,
                [vec![
                    "Faroles en la orilla".to_string(),
                    "Llamándonos a casa".to_string()
                ]]
            );
            assert_eq!(s.sections[1].seconds, [vec!["Canta, canta".to_string()]]);
            assert_eq!(s.second_lines(), 3);
        }
        // Without a split, `---` is a lyric line.
        assert_eq!(parse_two(text, &SecondSplit::None).second_lines(), 0);
    }

    #[test]
    fn two_languages_inside_each_line() {
        let text = "\
[G/B]Lanterns on the [C]shore / Faroles en la [D]orilla
Calling us home / Llamándonos a casa
Only the first
";
        let s = parse_two(
            text,
            &SecondSplit::Inline {
                separator: "/".into(),
            },
        );
        // `[G/B]` is a chord, not a separator; chords after the separator are dropped.
        assert_eq!(
            firsts(&s),
            [vec![
                (
                    "Lanterns on the shore".to_string(),
                    vec![(0, "G/B"), (16, "C")]
                ),
                ("Calling us home".to_string(), vec![]),
                ("Only the first".to_string(), vec![]),
            ]]
        );
        assert_eq!(
            seconds(&s),
            [vec![vec![
                "Faroles en la orilla".to_string(),
                "Llamándonos a casa".to_string()
            ]]]
        );
        let s = parse_two(
            "One | Uno\nTwo | Dos\n",
            &SecondSplit::Inline {
                separator: "|".into(),
            },
        );
        assert_eq!(
            s.sections[0].seconds,
            [vec!["Uno".to_string(), "Dos".to_string()]]
        );
    }

    #[test]
    fn two_languages_in_italic_comments() {
        let text = "\
{meta: second_language_split comments}
{c: Chorus}
[C]Sing, [G/B]sing
{ci: Canta, canta}
{c: repeat 2x}
[Am7]Sing it once [D]more
{comment_italic: Cántalo otra vez}
{cb: softly}
{highlight: all together}
";
        let s = parse_two(text, &SecondSplit::Auto);
        assert_eq!(s.sections[0].name, "Chorus");
        assert_eq!(s.sections[0].stanzas[0].len(), 2);
        assert_eq!(
            s.sections[0].seconds,
            [vec![
                "Canta, canta".to_string(),
                "Cántalo otra vez".to_string()
            ]]
        );
        // Plain, boxed and highlighted comments are performance notes, not the second
        // language. One language: the comments are skipped, as before.
        assert_eq!(parse(text).second_lines(), 0);
    }

    #[test]
    fn second_lines_follow_the_slides() {
        let text = "One\nUno\nTwo\nDos\nThree\nTres\n";
        let s = parse_two(text, &SecondSplit::Alternating);
        let slides = s.slides_two(2);
        assert_eq!(slides[0].len(), 2);
        assert_eq!(slides[0][0].1, ["Uno", "Dos"]);
        assert_eq!(slides[0][1].1, ["Tres"]);
        assert_eq!(s.unpaired_stanzas(2), 0);
        // Not line by line (three first lines, one second): all on the stanza's first slide,
        // and the report counts the stanza.
        let s = parse_two("One\nTwo\nThree\n---\nUno dos tres\n", &SecondSplit::Line);
        let slides = s.slides_two(2);
        assert_eq!(slides[0][0].1, ["Uno dos tres"]);
        assert_eq!(slides[0][1].1, Vec::<String>::new());
        assert_eq!(s.unpaired_stanzas(2), 1);
        assert_eq!(
            s.unpaired_stanzas(0),
            0,
            "one slide per stanza: nothing to split"
        );
        assert_eq!(s.slides(0)[0][0].len(), 3);
        // Three lines, two translations (the last line has none): no guessing either.
        let s = parse_two("One\nUno\nTwo\nDos\nThree\n", &SecondSplit::Alternating);
        let slides = s.slides_two(2);
        assert_eq!(slides[0][0].1, ["Uno", "Dos"]);
        assert_eq!(slides[0][1].1, Vec::<String>::new());
    }

    /// A line of chords alone (an instrumental line) stays with the first language and takes
    /// no second line, so the translations still sit on the slides of their own lines.
    #[test]
    fn second_lines_pair_past_a_chords_only_line() {
        let text = "[G] [D]\nOne\nUno\nTwo\nDos\nThree\nTres\nFour\nCuatro\n";
        let s = parse_two(text, &SecondSplit::Alternating);
        assert_eq!(s.sections[0].stanzas[0].len(), 5);
        let slides: Vec<(Vec<String>, Vec<String>)> = s.slides_two(2)[0]
            .iter()
            .map(|(p, b)| (p.iter().map(Paragraph::plain_text).collect(), b.clone()))
            .collect();
        assert_eq!(
            slides,
            [
                (vec!["".to_string(), "One".into()], vec!["Uno".to_string()]),
                (
                    vec!["Two".into(), "Three".into()],
                    vec!["Dos".into(), "Tres".into()]
                ),
                (vec!["Four".into()], vec!["Cuatro".into()]),
            ]
        );
        assert_eq!(s.unpaired_stanzas(2), 0);
    }

    /// A stanza that is all second language (nothing above its `---`) keeps its lines on a
    /// slide of its own rather than losing them.
    #[test]
    fn a_stanza_of_second_lines_only_keeps_them() {
        let s = parse_two("One\n---\nUno\n\n---\nDos\n", &SecondSplit::Line);
        let slides = s.slides_two(0);
        let all: Vec<_> = slides.iter().flatten().collect();
        assert_eq!(all.len(), 2);
        assert!(all[1].0.is_empty());
        assert_eq!(all[1].1, ["Dos"]);
    }

    #[test]
    fn export_keeps_the_second_language() {
        let s = parse_two(
            "{sov}\n[G]Lanterns on the [C]shore\n---\nFaroles en la orilla\n{eov}\n",
            &SecondSplit::Line,
        );
        let mut p = Presentation::new("Lanterns");
        let mut section = crate::Section::new("Verse 1");
        for (paras, second) in s.slides_two(0).remove(0) {
            let mut slide = crate::Slide::new();
            slide.elements.push(crate::Element::text(
                crate::Rect::new(0.0, 0.0, 10.0, 10.0),
                crate::TextBox {
                    paragraphs: paras,
                    ..crate::TextBox::default()
                },
            ));
            slide.translation = Some(crate::SlideTranslation {
                language: "es".into(),
                ..Default::default()
            });
            let _ = second;
            section.slides.push(slide);
        }
        p.sections.push(section);
        let second = vec!["Faroles en la orilla".to_string()];
        let text = export(&p, |sl| {
            let paras = sl
                .elements
                .iter()
                .find_map(|e| e.text_box())
                .map(|t| t.paragraphs.clone())
                .unwrap_or_default();
            (paras, second.clone())
        });
        assert!(
            text.contains("{meta: second_language es}\n{meta: second_language_split comments}\n")
        );
        assert!(text.contains("[G]Lanterns on the [C]shore\n{ci: Faroles en la orilla}\n"));
        let again = parse_two(&text, &SecondSplit::Auto);
        assert_eq!(again.second_language.as_deref(), Some("es"));
        assert_eq!(again.sections[0].stanzas, s.sections[0].stanzas);
        assert_eq!(again.sections[0].seconds, s.sections[0].seconds);
    }

    /// A presentation of one section whose slides have these primary paragraphs and second
    /// lines, with this second-language code.
    fn two_language_presentation(slides: &[(&[&str], &[&str])], lang: &str) -> Presentation {
        let mut p = Presentation::new("Harbour Song");
        let mut section = crate::Section::new("Verse 1");
        for (paras, second) in slides {
            let mut slide = crate::Slide::new();
            let el = crate::Element::text(
                crate::Rect::new(0.0, 0.0, 10.0, 10.0),
                crate::TextBox {
                    paragraphs: paras.iter().map(|t| parse_inline(t)).collect(),
                    ..crate::TextBox::default()
                },
            );
            slide.translation = Some(crate::SlideTranslation {
                language: lang.into(),
                texts: vec![crate::ElementTranslation {
                    element: el.id,
                    paragraphs: second.iter().map(|t| paragraph(t)).collect(),
                }],
                ..Default::default()
            });
            slide.elements.push(el);
            section.slides.push(slide);
        }
        p.sections.push(section);
        p
    }

    /// What the server's export passes: the text box's paragraphs and its second text's lines.
    fn export_two(p: &Presentation) -> String {
        export(p, |sl| {
            let el = &sl.elements[0];
            let paras = el.text_box().unwrap().paragraphs.clone();
            let second = sl
                .translation
                .as_ref()
                .and_then(|t| t.text_of(el.id))
                .map(|t| t.paragraphs.iter().map(Paragraph::plain_text).collect())
                .unwrap_or_default();
            (paras, second)
        })
    }

    /// A slide's empty paragraphs don't cut its `{ci:}` lines off from their lines: a
    /// trailing one is dropped, an inner one splits the slide into stanzas that each keep
    /// their own second lines.
    #[test]
    fn export_keeps_the_second_language_past_empty_paragraphs() {
        let p = two_language_presentation(
            &[(
                &["", "[G]Line one", "Line two", ""],
                &["Línea uno", "Línea dos"],
            )],
            "es",
        );
        let text = export_two(&p);
        assert!(
            text.contains("[G]Line one\nLine two\n{ci: Línea uno}\n{ci: Línea dos}\n"),
            "{text}"
        );
        let again = parse_two(&text, &SecondSplit::Auto);
        assert_eq!(again.second_lines(), 2);
        assert_eq!(again.slides_two(0)[0][0].1, ["Línea uno", "Línea dos"]);

        // A blank line inside the slide, the second text mirroring it.
        let p = two_language_presentation(
            &[(
                &["Line one", "", "", "Line two"],
                &["Línea uno", "", "Línea dos"],
            )],
            "es",
        );
        let again = parse_two(&export_two(&p), &SecondSplit::Auto);
        let slides: Vec<(Vec<String>, Vec<String>)> = again.slides_two(0)[0]
            .iter()
            .map(|(p, b)| (p.iter().map(Paragraph::plain_text).collect(), b.clone()))
            .collect();
        assert_eq!(
            slides,
            [
                (vec!["Line one".to_string()], vec!["Línea uno".to_string()]),
                (vec!["Line two".to_string()], vec!["Línea dos".to_string()]),
            ]
        );

        // Not line by line: all second lines stay with the first piece.
        let p = two_language_presentation(&[(&["Line one", "", "Line two"], &["Líneas"])], "es");
        let again = parse_two(&export_two(&p), &SecondSplit::Auto);
        assert_eq!(
            again.sections[0].seconds,
            [vec!["Líneas".to_string()], vec![]]
        );
    }

    /// The second language's code is written normalized, and a value that is not a code is
    /// left out (the comments are still written).
    #[test]
    fn export_writes_the_second_language_as_a_code() {
        let p = two_language_presentation(&[(&["One"], &["Uno"])], "ES_mx");
        assert!(export_two(&p).contains("{meta: second_language es-MX}\n"));
        let p = two_language_presentation(&[(&["One"], &["Uno"])], "Español");
        let text = export_two(&p);
        assert!(!text.contains("second_language "), "{text}");
        assert!(text.contains("One\n{ci: Uno}\n"), "{text}");
    }

    /// The presentation's own language: `{meta: language …}` read (in its usual case; a value
    /// that is not a code is ignored) and written back, beside the second language.
    #[test]
    fn the_songs_language_is_read_and_written() {
        let s = parse(
            "{title: Harbour Song}\n{meta: language pt_br}\n{meta: second_language en}\n\n[D]Luz do [A]porto\n",
        );
        assert_eq!(s.language.as_deref(), Some("pt-BR"));
        assert_eq!(s.second_language.as_deref(), Some("en"));
        // The second language is a code too: normalized, and a name is ignored.
        assert_eq!(
            parse("{meta: second_language ES_mx}\nLa la\n")
                .second_language
                .as_deref(),
            Some("es-MX")
        );
        assert_eq!(
            parse("{meta: second_language Español}\nLa la\n").second_language,
            None
        );
        assert_eq!(
            parse("{meta: language Portuguese please}\nLa la\n").language,
            None
        );
        assert_eq!(parse("Morning light\n").language, None);

        let mut p = Presentation::new("Harbour Song");
        p.language = s.language.clone();
        let mut section = crate::Section::new("Verse 1");
        let mut slide = crate::Slide::new();
        slide.elements.push(crate::Element::text(
            crate::Rect::new(0.0, 0.0, 10.0, 10.0),
            crate::TextBox {
                paragraphs: s.sections[0].stanzas[0].clone(),
                ..crate::TextBox::default()
            },
        ));
        section.slides.push(slide);
        p.sections.push(section);
        let text = export(&p, |sl| {
            let paras = sl
                .elements
                .iter()
                .find_map(|e| e.text_box())
                .map(|t| t.paragraphs.clone())
                .unwrap_or_default();
            (paras, Vec::new())
        });
        assert!(text.contains("{meta: language pt-BR}\n"), "{text}");
        assert_eq!(parse(&text).language.as_deref(), Some("pt-BR"));
        p.language = None;
        assert!(!export(&p, |_| (Vec::new(), Vec::new())).contains("language"));
    }
}
