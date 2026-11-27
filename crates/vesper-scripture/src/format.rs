//! The translation file format, `vesper-bible/1` (`<ID>.vbible`).
//!
//! ```text
//! {"format":"vesper-bible/1","id":"KJV",…,"books":[{"key":"GEN","name":"Genesis","chapters":[31,25,…]},…]}
//! In the beginning …            ← Genesis 1:1
//! …                             ← one line per verse, books and chapters in header order
//! ```
//!
//! Line 1 is the JSON [`Header`]; it carries the translation's versification (each chapter's
//! verse count). Then exactly one line per verse in that order, UTF-8, no line breaks inside a
//! verse; an empty line is a verse this translation doesn't have (it is left out of passages).

use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};

/// The format name in [`Header::format`].
pub const FORMAT: &str = "vesper-bible/1";

/// File extension.
pub const EXTENSION: &str = "vbible";

/// Line 1 of a translation file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    /// [`FORMAT`].
    pub format: String,
    /// ID (`KJV`).
    pub id: String,
    /// Abbreviation (`KJV`).
    pub abbreviation: String,
    /// Full name.
    pub name: String,
    /// Language (BCP 47).
    pub language: String,
    /// Licence.
    pub license: String,
    /// The notice generated slides must carry, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<String>,
    /// Where the text came from (URL), for the record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Books in order.
    pub books: Vec<HeaderBook>,
}

/// A book in the header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderBook {
    /// USFM key.
    pub key: String,
    /// Name.
    pub name: String,
    /// Verse count of each chapter.
    pub chapters: Vec<u32>,
}

impl Header {
    /// Number of verse lines that follow the header.
    pub fn verse_count(&self) -> usize {
        self.books
            .iter()
            .flat_map(|b| b.chapters.iter())
            .map(|&n| n as usize)
            .sum()
    }
}

/// Reads only the header (listing translations without loading their text).
pub fn read_header(r: impl BufRead) -> Result<Header, String> {
    let mut line = String::new();
    let mut r = r;
    r.read_line(&mut line).map_err(|e| e.to_string())?;
    parse_header(&line)
}

fn parse_header(line: &str) -> Result<Header, String> {
    let h: Header = serde_json::from_str(line.trim_start_matches('\u{feff}').trim())
        .map_err(|e| format!("bad header: {e}"))?;
    if h.format != FORMAT {
        return Err(format!("format {:?}, expected {FORMAT:?}", h.format));
    }
    if h.id.is_empty() || h.id.contains(['/', '\\']) {
        return Err(format!("bad id {:?}", h.id));
    }
    Ok(h)
}

/// Reads a whole file: the header and every verse line (checked against the versification).
pub fn read(text: &str) -> Result<(Header, Vec<String>), String> {
    let mut lines = text.split('\n');
    let header = parse_header(lines.next().unwrap_or_default())?;
    let want = header.verse_count();
    let mut verses: Vec<String> = lines
        .map(|l| l.trim_end_matches('\r').to_string())
        .collect();
    // A final newline leaves one empty piece.
    if verses.len() == want + 1 && verses.last().is_some_and(|l| l.is_empty()) {
        verses.pop();
    }
    if verses.len() != want {
        return Err(format!(
            "{} verse lines, the header's versification has {want}",
            verses.len()
        ));
    }
    Ok((header, verses))
}

/// Writes a file. `verses` must follow the header's versification.
pub fn write(mut w: impl Write, header: &Header, verses: &[String]) -> std::io::Result<()> {
    if verses.len() != header.verse_count() {
        return Err(std::io::Error::other(format!(
            "{} verses, the versification has {}",
            verses.len(),
            header.verse_count()
        )));
    }
    serde_json::to_writer(&mut w, header)?;
    w.write_all(b"\n")?;
    for v in verses {
        let clean: String = v
            .chars()
            .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
            .collect();
        w.write_all(clean.trim().as_bytes())?;
        w.write_all(b"\n")?;
    }
    Ok(())
}
