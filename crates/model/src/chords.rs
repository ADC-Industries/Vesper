//! Chords: chord marks on lyric lines, song keys, transposition, capo and Nashville numbers.
//!
//! ## Chords in the model
//! A lyric line (a [`Paragraph`](crate::text::Paragraph)) carries `chords`: [`ChordMark`]s, each
//! a chord as written (`"G/B"`, `"Am7"`, `"N.C."`) and the **character it sits over** (`at`, a
//! Unicode-character index into the paragraph's plain text; the text's length = after the last
//! character). Chords are not text: search, the slide grid and the audience screens never see
//! them. A text box draws them above the words only when its [`ChordDisplay`] says so (stage
//! chord charts; an editor's chord view).
//!
//! **Editing the words** keeps the chords where it can ([`remap_chords`]): characters before
//! and after the changed part keep their chords; a chord inside the changed part stays at the
//! same offset into the new words, at most on their last character.
//!
//! ## Keys and transposition
//! A song's written key is `SongInfo.originalKey` ("G", "Bb", "F#m") and `SongInfo.transpose`
//! the semitones the band plays it away from the written chords (the song's key control). A
//! stage layout adds its own [`ChordView`]: an extra transposition (a transposing instrument),
//! a **capo** (a guitarist with capo 2 reads chord shapes two semitones lower), the spelling of
//! accidentals (sharps, flats, or by key) and letter chords or **Nashville numbers**.
//!
//! Chord spelling: a root `A`–`G` with `#` / `b` (also `♯` / `♭`), a suffix kept as written
//! (`m`, `7`, `sus4`, `add9`, `maj7`, `6/9`, `dim`), and an optional bass after the last `/`
//! when it is a note (`G/B`). Anything else (`N.C.`, `(x2)`) is kept unchanged.

use crate::common::{Color, is_default};
use serde::{Deserialize, Serialize};

/// A chord over one character of a lyric line.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ChordMark {
    /// The character the chord sits over: a Unicode-character index into the paragraph's plain
    /// text (its length = after the last character).
    #[serde(default, skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub at: u32,
    /// The chord as written (`"G/B"`, `"Am7"`, `"N.C."`).
    pub chord: String,
}

impl ChordMark {
    /// A chord at character `at`.
    pub fn new(at: u32, chord: impl Into<String>) -> Self {
        ChordMark {
            at,
            chord: chord.into(),
        }
    }
}

/// Whether and how a text box draws its paragraphs' chords above the words.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ChordDisplay {
    /// Draw the chords (each line with chords gets a chord row above it).
    #[serde(skip_serializing_if = "crate::common::is_false")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub show: bool,
    /// Chord colour (`None` = [`ChordDisplay::DEFAULT_COLOR`], a warm yellow).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub color: Option<Color>,
    /// Chord size as a fraction of the line's text size (`None` = 0.7).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub size: Option<f64>,
}

impl ChordDisplay {
    /// The default chord colour.
    pub const DEFAULT_COLOR: Color = Color {
        r: 1.0,
        g: 0.84,
        b: 0.25,
        a: 1.0,
    };
    /// The default chord size (fraction of the text size).
    pub const DEFAULT_SIZE: f64 = 0.7;

    /// Shown chords in the default style.
    pub fn shown() -> Self {
        ChordDisplay {
            show: true,
            ..ChordDisplay::default()
        }
    }
}

/// How accidentals are spelled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Accidentals {
    /// By the key: flats in F, Bb, Eb, Ab, Db (and their relative minors), else sharps; without
    /// a key, as written.
    #[default]
    Auto,
    /// Always sharps (`C#`, `F#`).
    Sharps,
    /// Always flats (`Db`, `Gb`).
    Flats,
    /// Unknown value (auto).
    #[serde(other)]
    Unknown,
}

/// Letter chords or Nashville numbers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum ChordNotation {
    /// `G`, `D/F#`, `Em7`.
    #[default]
    Letters,
    /// Scale degrees of the song's key: `1`, `5/7`, `6m7` (letters when the key is unknown).
    Nashville,
    /// Unknown value (letters).
    #[serde(other)]
    Unknown,
}

/// A musician's view of a song's chords (a stage layout's chord options).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default, rename_all = "camelCase")]
pub struct ChordView {
    /// Extra semitones on top of the song's own transposition (a transposing instrument).
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub transpose: i32,
    /// Capo fret: the chords are shown as the shapes played with the capo (this many semitones
    /// lower); the key line names both.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub capo: u8,
    /// Spelling of accidentals.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub accidentals: Accidentals,
    /// Letters or Nashville numbers.
    #[serde(skip_serializing_if = "is_default")]
    #[cfg_attr(feature = "ts", ts(as = "Option<_>", optional))]
    pub notation: ChordNotation,
}

// ---- notes and keys -------------------------------------------------------------------------

const SHARP_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
const FLAT_NAMES: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

/// Parses a note name at the start of `s` (`"C#"`, `"Bb"`, `"E♭"`): its pitch class (C = 0),
/// whether it was spelled with a flat, and the bytes it used.
pub fn parse_note(s: &str) -> Option<(u8, bool, usize)> {
    let mut chars = s.char_indices();
    let (_, c) = chars.next()?;
    let base: i32 = match c {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let mut len = c.len_utf8();
    let mut pc = base;
    let mut flat = false;
    if let Some((i, a)) = chars.next() {
        match a {
            '#' | '♯' => {
                pc += 1;
                len = i + a.len_utf8();
            }
            'b' | '♭' => {
                pc -= 1;
                flat = true;
                len = i + a.len_utf8();
            }
            _ => {}
        }
    }
    Some((pc.rem_euclid(12) as u8, flat, len))
}

/// The name of pitch class `pc` (mod 12) with sharps or flats.
pub fn note_name(pc: i32, flats: bool) -> &'static str {
    let i = pc.rem_euclid(12) as usize;
    if flats { FLAT_NAMES[i] } else { SHARP_NAMES[i] }
}

/// A key: tonic and mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// Tonic pitch class (C = 0).
    pub root: u8,
    /// Minor key.
    pub minor: bool,
    /// Written with a flat (`Bb`): kept when the key is shown untransposed.
    pub flat: bool,
}

impl Key {
    /// Parses `"G"`, `"Bb"`, `"F#m"`, `"Ebmin"`, `"C minor"`, `"A major"`.
    pub fn parse(s: &str) -> Option<Key> {
        let s = s.trim();
        let (root, flat, n) = parse_note(s)?;
        let rest = s[n..].trim().to_ascii_lowercase();
        let minor = match rest.as_str() {
            "" | "maj" | "major" | "dur" => false,
            "m" | "min" | "minor" | "-" | "moll" => true,
            _ => return None,
        };
        Some(Key { root, minor, flat })
    }

    /// This key moved by `semitones`.
    pub fn transposed(self, semitones: i32) -> Key {
        Key {
            root: (self.root as i32 + semitones).rem_euclid(12) as u8,
            minor: self.minor,
            flat: self.flat && semitones.rem_euclid(12) == 0,
        }
    }

    /// Whether this key is spelled with flats (F, Bb, Eb, Ab, Db and the relative minors; Gb /
    /// F# as written).
    pub fn uses_flats(self) -> bool {
        let major = if self.minor {
            (self.root + 3) % 12
        } else {
            self.root
        };
        match major {
            5 | 10 | 3 | 8 | 1 => true,
            6 => self.flat,
            _ => false,
        }
    }

    /// The key's name (`"Bb"`, `"F#m"`) with the given spelling.
    pub fn name(self, accidentals: Accidentals) -> String {
        let flats = match accidentals {
            Accidentals::Flats => true,
            Accidentals::Sharps => false,
            _ => self.uses_flats(),
        };
        format!(
            "{}{}",
            note_name(self.root as i32, flats),
            if self.minor { "m" } else { "" }
        )
    }
}

// ---- chords ---------------------------------------------------------------------------------

/// A parsed chord symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    /// Root pitch class.
    pub root: u8,
    /// Root written with a flat.
    pub root_flat: bool,
    /// Everything between the root and the bass, as written (`"m7"`, `"sus4"`, `""`).
    pub suffix: String,
    /// Bass note (pitch class, written with a flat).
    pub bass: Option<(u8, bool)>,
}

impl Chord {
    /// Parses a chord symbol (`None` for `N.C.`, `(x2)` and other non-chords).
    pub fn parse(s: &str) -> Option<Chord> {
        let s = s.trim();
        let (root, root_flat, n) = parse_note(s)?;
        let rest = &s[n..];
        let (suffix, bass) = match rest.rfind('/') {
            Some(i) => match parse_note(&rest[i + 1..]) {
                Some((b, bf, len)) if i + 1 + len == rest.len() => (&rest[..i], Some((b, bf))),
                _ => (rest, None),
            },
            None => (rest, None),
        };
        if !valid_suffix(suffix) {
            return None;
        }
        Some(Chord {
            root,
            root_flat,
            suffix: suffix.to_string(),
            bass,
        })
    }

    /// A minor chord (`m`, `m7`, `min`, `-7`; not `maj7`).
    pub fn is_minor(&self) -> bool {
        let s = self.suffix.as_str();
        (s.starts_with('m') && !s.starts_with("maj")) || s.starts_with('-')
    }
}

/// A chord suffix made only of chord words (`m7`, `sus4`, `add9`, `maj7`, `6/9`, `dim`, `+`,
/// `(b9)`): `"ay"` (in "Day") is not one.
fn valid_suffix(mut s: &str) -> bool {
    const TOKENS: &[&str] = &[
        "maj", "min", "dim", "aug", "sus", "add", "alt", "no", "M", "m", "+", "-", "°", "ø", "Δ",
        "#", "b", "♯", "♭", "(", ")", "/", ",",
    ];
    while !s.is_empty() {
        if let Some(t) = TOKENS.iter().find(|t| s.starts_with(**t)) {
            s = &s[t.len()..];
        } else if s.starts_with(|c: char| c.is_ascii_digit()) {
            s = &s[1..];
        } else {
            return false;
        }
    }
    true
}

/// Nashville degree name of an interval above the tonic (`0` → `"1"`, `10` → `"b7"`).
fn degree(interval: i32) -> &'static str {
    [
        "1", "b2", "2", "b3", "3", "4", "#4", "5", "b6", "6", "b7", "7",
    ][interval.rem_euclid(12) as usize]
}

/// What chords are shown as: the shift from the written chords, the key they are in after it,
/// the spelling and the notation. Made by [`ChordView::context`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChordContext {
    /// Semitones from the written chords to the shown ones.
    pub shift: i32,
    /// The written key (`None` = unknown).
    pub written_key: Option<Key>,
    /// Spelling.
    pub accidentals: Accidentals,
    /// Letters or numbers.
    pub notation: ChordNotation,
}

impl ChordView {
    /// The context for a song written in `key` (`SongInfo.originalKey`) and transposed by
    /// `song_transpose` (`SongInfo.transpose`): shift = song transpose + this view's transpose
    /// − capo.
    pub fn context(&self, key: Option<&str>, song_transpose: i32) -> ChordContext {
        ChordContext {
            shift: song_transpose + self.transpose - self.capo as i32,
            written_key: key.and_then(Key::parse),
            accidentals: self.accidentals,
            notation: self.notation,
        }
    }

    /// The key line for a musician: `"Key A"`, `"Key A · Capo 2 (G shapes)"`, `"Capo 2"` (no
    /// key known), `""` (neither).
    pub fn key_label(&self, key: Option<&str>, song_transpose: i32) -> String {
        let concert = key
            .and_then(Key::parse)
            .map(|k| k.transposed(song_transpose + self.transpose));
        let acc = self.accidentals;
        match (concert, self.capo) {
            (Some(k), 0) => format!("Key {}", k.name(acc)),
            (Some(k), c) => format!(
                "Key {} · Capo {c} ({} shapes)",
                k.name(acc),
                k.transposed(-(c as i32)).name(acc)
            ),
            (None, 0) => String::new(),
            (None, c) => format!("Capo {c}"),
        }
    }
}

impl ChordContext {
    /// No change: the chords as written.
    pub const WRITTEN: ChordContext = ChordContext {
        shift: 0,
        written_key: None,
        accidentals: Accidentals::Auto,
        notation: ChordNotation::Letters,
    };

    /// The key the shown chords are in.
    pub fn shown_key(&self) -> Option<Key> {
        self.written_key.map(|k| k.transposed(self.shift))
    }

    /// A written chord as shown (see the module docs). Non-chords come back unchanged.
    pub fn chord(&self, written: &str) -> String {
        let Some(c) = Chord::parse(written) else {
            return written.to_string();
        };
        if self.notation == ChordNotation::Nashville
            && let Some(k) = self.written_key
        {
            // Numbers are relative to the key: no shift changes them.
            let mut out = degree(c.root as i32 - k.root as i32).to_string();
            out.push_str(&c.suffix);
            if let Some((b, _)) = c.bass {
                out.push('/');
                out.push_str(degree(b as i32 - k.root as i32));
            }
            return out;
        }
        if self.shift.rem_euclid(12) == 0 && self.accidentals == Accidentals::Auto {
            return written.trim().to_string();
        }
        let flats = |was_flat: bool| match self.accidentals {
            Accidentals::Flats => true,
            Accidentals::Sharps => false,
            _ => match self.shown_key() {
                Some(k) => k.uses_flats(),
                None => was_flat,
            },
        };
        let mut out = note_name(c.root as i32 + self.shift, flats(c.root_flat)).to_string();
        out.push_str(&c.suffix);
        if let Some((b, bf)) = c.bass {
            out.push('/');
            out.push_str(note_name(b as i32 + self.shift, flats(bf)));
        }
        out
    }
}

/// Transposes one chord by `semitones` with the given spelling (`Auto` = as written).
pub fn transpose_chord(chord: &str, semitones: i32, accidentals: Accidentals) -> String {
    ChordContext {
        shift: semitones,
        accidentals,
        ..ChordContext::WRITTEN
    }
    .chord(chord)
}

/// Semitones from key `from` to key `to` (`-5..=6`; `None` when either doesn't parse).
pub fn key_distance(from: &str, to: &str) -> Option<i32> {
    let (a, b) = (Key::parse(from)?, Key::parse(to)?);
    let d = (b.root as i32 - a.root as i32).rem_euclid(12);
    Some(if d > 6 { d - 12 } else { d })
}

// ---- keeping chords on edited words ---------------------------------------------------------

/// Moves chords from `old` text to `new` text (see the module docs): positions before and
/// after the changed part follow their characters; positions inside it keep their offset into
/// the change, at most its last new character. Positions past the end are clamped to the end.
pub fn remap_chords(old: &str, new: &str, chords: &[ChordMark]) -> Vec<ChordMark> {
    if chords.is_empty() || old == new {
        return chords.to_vec();
    }
    let a: Vec<char> = old.chars().collect();
    let b: Vec<char> = new.chars().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let max_suffix = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(max_suffix)
        .take_while(|(x, y)| x == y)
        .count();
    let (a_end, b_end) = (a.len() - suffix, b.len() - suffix);
    chords
        .iter()
        .map(|c| {
            let x = (c.at as usize).min(a.len());
            let at = if x < prefix {
                x
            } else if x >= a_end {
                // In the unchanged tail (or at the very end).
                x - a_end + b_end
            } else {
                // Inside the change: the same offset into it, at most its last character.
                x.min(if b_end > prefix { b_end - 1 } else { prefix })
            };
            ChordMark {
                at: at.min(b.len()) as u32,
                chord: c.chord.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(c: &str, n: i32) -> String {
        transpose_chord(c, n, Accidentals::Auto)
    }

    #[test]
    fn notes_and_keys() {
        assert_eq!(parse_note("C#m"), Some((1, false, 2)));
        assert_eq!(parse_note("Bb"), Some((10, true, 2)));
        assert_eq!(parse_note("E♭"), Some((3, true, 4)));
        assert_eq!(parse_note("Cb"), Some((11, true, 2)));
        assert_eq!(parse_note("B#"), Some((0, false, 2)));
        assert_eq!(parse_note("H"), None);
        assert_eq!(Key::parse("F#m").unwrap().name(Accidentals::Auto), "F#m");
        assert_eq!(Key::parse("Bb").unwrap().name(Accidentals::Auto), "Bb");
        assert_eq!(Key::parse("A#").unwrap().name(Accidentals::Auto), "Bb");
        assert_eq!(Key::parse("C minor").unwrap().name(Accidentals::Auto), "Cm");
        assert_eq!(Key::parse("Gb").unwrap().name(Accidentals::Auto), "Gb");
        assert_eq!(Key::parse("F#").unwrap().name(Accidentals::Auto), "F#");
        assert_eq!(Key::parse("Dm").unwrap().name(Accidentals::Auto), "Dm");
        assert!(Key::parse("Dm").unwrap().uses_flats());
        assert!(Key::parse("Banana").is_none());
        assert_eq!(key_distance("G", "A"), Some(2));
        assert_eq!(key_distance("C", "Bb"), Some(-2));
        assert_eq!(key_distance("C", "F#"), Some(6));
    }

    #[test]
    fn chord_parsing() {
        let c = Chord::parse("G/B").unwrap();
        assert_eq!(
            (c.root, c.suffix.as_str(), c.bass),
            (7, "", Some((11, false)))
        );
        let c = Chord::parse("C6/9").unwrap();
        assert_eq!((c.suffix.as_str(), c.bass), ("6/9", None));
        assert!(Chord::parse("Am7").unwrap().is_minor());
        assert!(!Chord::parse("Cmaj7").unwrap().is_minor());
        assert!(Chord::parse("N.C.").is_none());
        assert!(Chord::parse("(x2)").is_none());
        assert!(Chord::parse("Day").is_none());
        assert!(Chord::parse("Go").is_none());
        assert!(Chord::parse("Bridge").is_none());
        assert!(Chord::parse("C#m7b5").is_some());
        assert!(Chord::parse("Bbmaj9(#11)").is_some());
        assert!(Chord::parse("Fadd2/A").is_some());
    }

    #[test]
    fn transposition() {
        // Up a whole step: slash chords, minors, sus/add/7ths.
        assert_eq!(t("G", 2), "A");
        assert_eq!(t("G/B", 2), "A/C#");
        assert_eq!(t("Em7", 2), "F#m7");
        assert_eq!(t("Csus4", 2), "Dsus4");
        assert_eq!(t("Cadd9", 2), "Dadd9");
        assert_eq!(t("Dmaj7", -2), "Cmaj7");
        assert_eq!(t("C6/9", 1), "C#6/9");
        // Auto keeps the written spelling without a key.
        assert_eq!(t("Bb", 1), "B");
        assert_eq!(t("Bb", 2), "C");
        assert_eq!(t("Eb", 2), "F");
        assert_eq!(t("Ab", 1), "A");
        assert_eq!(t("Db", -1), "C");
        assert_eq!(t("Bb", 3), "Db");
        assert_eq!(t("A#", 3), "C#");
        // Forced spelling (enharmonics).
        assert_eq!(transpose_chord("C", 1, Accidentals::Flats), "Db");
        assert_eq!(transpose_chord("C", 1, Accidentals::Sharps), "C#");
        assert_eq!(transpose_chord("Gb", 0, Accidentals::Sharps), "F#");
        assert_eq!(transpose_chord("F#m7/C#", 0, Accidentals::Flats), "Gbm7/Db");
        // Wrap around the octave; non-chords unchanged.
        assert_eq!(t("B", 1), "C");
        assert_eq!(t("C", -1), "B");
        assert_eq!(t("C", 12), "C");
        assert_eq!(t("N.C.", 3), "N.C.");
    }

    #[test]
    fn keys_choose_the_spelling() {
        // G → F (down a whole step): flats.
        let v = ChordView {
            transpose: -2,
            ..ChordView::default()
        };
        let ctx = v.context(Some("G"), 0);
        assert_eq!(ctx.chord("D/F#"), "C/E");
        assert_eq!(ctx.chord("C"), "Bb");
        assert_eq!(ctx.shown_key().unwrap().name(Accidentals::Auto), "F");
        // C → D: sharps.
        let ctx = ChordView::default().context(Some("C"), 2);
        assert_eq!(ctx.chord("Bb"), "C");
        assert_eq!(ctx.chord("F/A"), "G/B");
        assert_eq!(ctx.chord("Eb"), "F");
        assert_eq!(ctx.chord("A7"), "B7");
        assert_eq!(ctx.chord("E7"), "F#7");
    }

    #[test]
    fn capo_and_key_lines() {
        // Written in G, played in A; a guitarist with capo 2 reads G shapes.
        let guitar = ChordView {
            capo: 2,
            ..ChordView::default()
        };
        assert_eq!(guitar.key_label(Some("G"), 2), "Key A · Capo 2 (G shapes)");
        let ctx = guitar.context(Some("G"), 2);
        assert_eq!(ctx.chord("G"), "G");
        assert_eq!(ctx.chord("D/F#"), "D/F#");
        // Keys in concert pitch see A.
        let keys = ChordView::default();
        assert_eq!(keys.key_label(Some("G"), 2), "Key A");
        assert_eq!(keys.context(Some("G"), 2).chord("D/F#"), "E/G#");
        assert_eq!(keys.key_label(None, 0), "");
        assert_eq!(guitar.key_label(None, 0), "Capo 2");
        let flats = ChordView {
            accidentals: Accidentals::Flats,
            ..ChordView::default()
        };
        assert_eq!(flats.key_label(Some("F#"), 0), "Key Gb");
    }

    #[test]
    fn nashville_numbers() {
        let v = ChordView {
            notation: ChordNotation::Nashville,
            ..ChordView::default()
        };
        let ctx = v.context(Some("G"), 3);
        assert_eq!(ctx.chord("G"), "1");
        assert_eq!(ctx.chord("D/F#"), "5/7");
        assert_eq!(ctx.chord("Em7"), "6m7");
        assert_eq!(ctx.chord("Csus2"), "4sus2");
        assert_eq!(ctx.chord("F"), "b7");
        assert_eq!(ctx.chord("Am/C"), "2m/4");
        // No key: letters.
        assert_eq!(v.context(None, 0).chord("Em"), "Em");
    }

    #[test]
    fn chords_follow_edited_words() {
        let ch = |v: &[(u32, &str)]| -> Vec<ChordMark> {
            v.iter().map(|(a, c)| ChordMark::new(*a, *c)).collect()
        };
        let old = "Sing a song of joy";
        let marks = ch(&[(0, "G"), (7, "C"), (15, "D"), (18, "G")]);
        // A word changed in the middle: the chords after it move with their words.
        let got = remap_chords(old, "Sing a hymn of joy", &marks);
        assert_eq!(got, ch(&[(0, "G"), (7, "C"), (15, "D"), (18, "G")]));
        let got = remap_chords(old, "Sing a happy song of joy", &marks);
        assert_eq!(got, ch(&[(0, "G"), (13, "C"), (21, "D"), (24, "G")]));
        // Text added at the start.
        let got = remap_chords(old, "Oh, sing a song of joy", &marks);
        assert_eq!(got[2], ChordMark::new(19, "D"));
        // The words under a chord replaced: it stays at the start of the change.
        let got = remap_chords("Sing a song", "Sing it", &ch(&[(7, "C"), (5, "G")]));
        assert_eq!(got, ch(&[(6, "C"), (5, "G")]));
        // Everything deleted: clamped.
        let got = remap_chords(old, "", &marks);
        assert!(got.iter().all(|c| c.at == 0));
        // Unicode counts characters.
        let got = remap_chords("Él canta", "Él sí canta", &ch(&[(3, "Am")]));
        assert_eq!(got, ch(&[(6, "Am")]));
    }
}
