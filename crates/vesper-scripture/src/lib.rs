//! Vesper's scripture module (G11): Bible translations stored as files on the server, so the
//! Bible always works offline; reference parsing, passages, word search, and presentations
//! generated from a passage. `docs/SCRIPTURE-PLAN.md` is the plan.
//!
//! - [`format`]: the translation file format (`<ID>.vbible`).
//! - [`books`]: the canonical books and the abbreviations people type.
//! - [`reference`]: references ("Jn 3:16-18", "1 Cor 13", "Psalm 23:1–4, 6") and labels.
//! - [`slides`]: a passage split into slides, and the presentation made of them.
//! - [`api`]: the read-only `/api/v1/scripture/...` routes (generic over the router state).
//!
//! Translations are found in the folders given to [`Scripture::load`]: the bundled folder
//! (read-only, installed with the server) and the data folder's `scripture/` (the church's own
//! files). A translation's verse text is read on first use.

pub mod api;
pub mod books;
pub mod format;
pub mod names;
pub mod reference;
pub mod slides;

use format::{Header, HeaderBook};
use protocol::{
    ScriptureBook, ScriptureOrigin, ScriptureParse, ScripturePassage, ScriptureSearch,
    ScriptureTranslation, ScriptureVerse, VerseRange,
};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Every translation this server has.
#[derive(Default)]
pub struct Scripture {
    translations: Vec<Translation>,
}

/// One translation: its header, and its text once read.
pub struct Translation {
    header: Header,
    origin: ScriptureOrigin,
    path: PathBuf,
    text: OnceLock<Result<Text, String>>,
}

/// A translation's verses, indexed.
struct Text {
    verses: Vec<String>,
    /// Lower-cased copies for search.
    lower: Vec<String>,
    /// Index of each book's first verse of each chapter, `[book][chapter - 1]`.
    starts: Vec<Vec<usize>>,
}

/// A failed lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookupError {
    /// No translation with that ID.
    UnknownTranslation(String),
    /// The reference doesn't parse or isn't in the translation.
    BadReference(String),
    /// The translation's file can't be read.
    Unreadable(String),
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LookupError::UnknownTranslation(id) => write!(f, "no translation {id:?}"),
            LookupError::BadReference(m) | LookupError::Unreadable(m) => f.write_str(m),
        }
    }
}

impl Scripture {
    /// Finds the translations in `dirs` (each `(folder, origin)`): every `*.vbible` file. A
    /// later folder's translation replaces an earlier one with the same ID. Folders that don't
    /// exist are skipped; unreadable files are logged and skipped.
    pub fn load(dirs: &[(PathBuf, ScriptureOrigin)]) -> Scripture {
        let mut out: Vec<Translation> = Vec::new();
        for (dir, origin) in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut paths: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == format::EXTENSION))
                .collect();
            paths.sort();
            for path in paths {
                match Translation::open(&path, *origin) {
                    Ok(t) => {
                        out.retain(|o| !o.header.id.eq_ignore_ascii_case(&t.header.id));
                        out.push(t);
                    }
                    Err(e) => tracing::warn!("scripture: skipping {}: {e}", path.display()),
                }
            }
        }
        if !out.is_empty() {
            tracing::info!(
                "scripture: {}",
                out.iter()
                    .map(|t| t.header.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Scripture { translations: out }
    }

    /// The translations, bundled first, then by ID.
    pub fn translations(&self) -> Vec<ScriptureTranslation> {
        let mut v: Vec<ScriptureTranslation> =
            self.translations.iter().map(Translation::info).collect();
        v.sort_by(|a, b| {
            (a.origin != ScriptureOrigin::Bundled, &a.id)
                .cmp(&(b.origin != ScriptureOrigin::Bundled, &b.id))
        });
        v
    }

    /// A translation by ID (case-insensitive).
    pub fn get(&self, id: &str) -> Result<&Translation, LookupError> {
        self.translations
            .iter()
            .find(|t| t.header.id.eq_ignore_ascii_case(id))
            .ok_or_else(|| LookupError::UnknownTranslation(id.to_string()))
    }
}

/// The header with its books named in its language: a book the file names in English (the
/// canonical name) or not at all gets its language's name ([`names`]) when there is a table
/// for it; names the file gives in its language stay.
fn localize(mut h: Header) -> Header {
    for b in &mut h.books {
        let english = b.name.trim().is_empty()
            || books::by_key(&b.key).is_some_and(|c| c.name == b.name.trim());
        if english && let Some(n) = names::name(&h.language, &b.key) {
            b.name = n.to_string();
        }
    }
    h
}

/// A parsed reference: the book (its index in the translation) and the ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    /// Index of the book in the translation's header.
    pub book: usize,
    /// The ranges.
    pub ranges: Vec<VerseRange>,
}

impl Translation {
    fn open(path: &Path, origin: ScriptureOrigin) -> Result<Translation, String> {
        let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let header = localize(format::read_header(std::io::BufReader::new(f))?);
        Ok(Translation {
            header,
            origin,
            path: path.to_path_buf(),
            text: OnceLock::new(),
        })
    }

    /// A translation from a header and its verses (tests, conversions).
    pub fn from_parts(header: Header, verses: Vec<String>) -> Result<Translation, String> {
        let t = Translation {
            header: localize(header),
            origin: ScriptureOrigin::Local,
            path: PathBuf::new(),
            text: OnceLock::new(),
        };
        let text = Text::index(&t.header, verses)?;
        let _ = t.text.set(Ok(text));
        Ok(t)
    }

    /// The header.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// The wire summary.
    pub fn info(&self) -> ScriptureTranslation {
        let h = &self.header;
        ScriptureTranslation {
            id: h.id.clone(),
            abbreviation: h.abbreviation.clone(),
            name: h.name.clone(),
            language: h.language.clone(),
            license: h.license.clone(),
            attribution: h.attribution.clone().filter(|a| !a.trim().is_empty()),
            origin: self.origin,
            books: h.books.len() as u32,
        }
    }

    /// The books with their versification.
    pub fn books(&self) -> Vec<ScriptureBook> {
        self.header
            .books
            .iter()
            .map(|b| ScriptureBook {
                key: b.key.clone(),
                name: b.name.clone(),
                testament: books::testament(&b.key),
                chapters: b.chapters.clone(),
            })
            .collect()
    }

    fn text(&self) -> Result<&Text, LookupError> {
        self.text
            .get_or_init(|| {
                let s = std::fs::read_to_string(&self.path)
                    .map_err(|e| format!("{}: {e}", self.path.display()))?;
                let (_, verses) = format::read(&s)?;
                Text::index(&self.header, verses)
            })
            .as_ref()
            .map_err(|e| LookupError::Unreadable(e.clone()))
    }

    /// Parses a typed reference.
    pub fn parse(&self, q: &str) -> Result<Reference, (Option<usize>, String)> {
        let (name, numbers) = reference::split_book(q);
        if name.is_empty() {
            return Err((None, "type a book, chapter and verse (John 3:16)".into()));
        }
        // The translation's own names, then its language's table (another spelling than the
        // file's), English last ([`books::find`] knows the English names and abbreviations).
        let mut pairs: Vec<(String, String)> = self
            .header
            .books
            .iter()
            .map(|b| (b.key.clone(), b.name.clone()))
            .collect();
        for b in &self.header.books {
            if let Some(n) = names::name(&self.header.language, &b.key)
                && n != b.name
            {
                pairs.push((b.key.clone(), n.to_string()));
            }
        }
        let found = books::find(name, &pairs).map_err(|c| {
            let msg = match c.as_slice() {
                [] => format!("no book {name:?} in {}", self.header.abbreviation),
                more => format!("{name:?} could be {}", more.join(", ")),
            };
            (None, msg)
        })?;
        let index = self
            .header
            .books
            .iter()
            .position(|b| b.key == found.0)
            .unwrap_or_default();
        let book = &self.header.books[index];
        let ranges = reference::parse(book, numbers).map_err(|e| (Some(index), e))?;
        Ok(Reference {
            book: index,
            ranges,
        })
    }

    /// Parsing feedback for the reference box.
    pub fn parse_feedback(&self, q: &str) -> ScriptureParse {
        match self.parse(q) {
            Ok(r) => {
                let b = &self.header.books[r.book];
                ScriptureParse {
                    ok: true,
                    book: Some(b.key.clone()),
                    book_name: Some(b.name.clone()),
                    label: Some(reference::label(b, &r.ranges)),
                    ranges: r.ranges,
                    error: None,
                }
            }
            Err((book, e)) => {
                let b = book.map(|i| &self.header.books[i]);
                ScriptureParse {
                    ok: false,
                    book: b.map(|b| b.key.clone()),
                    book_name: b.map(|b| b.name.clone()),
                    ranges: Vec::new(),
                    label: None,
                    error: Some(e),
                }
            }
        }
    }

    /// The label of a parsed reference ("John 3:16–18").
    pub fn label(&self, r: &Reference) -> String {
        reference::label(&self.header.books[r.book], &r.ranges)
    }

    /// The book of a parsed reference.
    pub fn book(&self, r: &Reference) -> &HeaderBook {
        &self.header.books[r.book]
    }

    /// The verses of a typed reference.
    pub fn passage(&self, q: &str) -> Result<ScripturePassage, LookupError> {
        let r = self
            .parse(q)
            .map_err(|(_, e)| LookupError::BadReference(e))?;
        self.passage_of(&r)
    }

    /// The verses of a parsed reference (verses this translation lacks are left out).
    pub fn passage_of(&self, r: &Reference) -> Result<ScripturePassage, LookupError> {
        let text = self.text()?;
        let book = &self.header.books[r.book];
        let mut verses = Vec::new();
        for range in &r.ranges {
            let mut c = range.chapter;
            let mut v = range.verse;
            while (c, v) <= (range.end_chapter, range.end_verse) {
                let n = book.chapters.get(c as usize - 1).copied().unwrap_or(0);
                if v > n {
                    c += 1;
                    v = 1;
                    if c as usize > book.chapters.len() {
                        break;
                    }
                    continue;
                }
                let i = text.starts[r.book][c as usize - 1] + v as usize - 1;
                let t = &text.verses[i];
                if !t.is_empty() {
                    verses.push(ScriptureVerse {
                        book: book.key.clone(),
                        chapter: c,
                        verse: v,
                        text: t.clone(),
                    });
                }
                v += 1;
            }
        }
        Ok(ScripturePassage {
            translation: self.header.id.clone(),
            book: book.key.clone(),
            label: reference::label(book, &r.ranges),
            ranges: r.ranges.clone(),
            attribution: self.info().attribution,
            verses,
        })
    }

    /// Verses holding every word of `q` (case-insensitive), in canonical order: at most
    /// `limit`, with the total count.
    pub fn search(&self, q: &str, limit: usize) -> Result<ScriptureSearch, LookupError> {
        let text = self.text()?;
        let words: Vec<String> = fold(q)
            .split(|c: char| !c.is_alphanumeric() && c != '\'')
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect();
        let mut hits = Vec::new();
        let mut total = 0u32;
        if !words.is_empty() {
            for (bi, b) in self.header.books.iter().enumerate() {
                for (ci, &n) in b.chapters.iter().enumerate() {
                    let start = text.starts[bi][ci];
                    for v in 0..n as usize {
                        let l = &text.lower[start + v];
                        if words.iter().all(|w| l.contains(w.as_str())) {
                            total += 1;
                            if hits.len() < limit {
                                hits.push(ScriptureVerse {
                                    book: b.key.clone(),
                                    chapter: ci as u32 + 1,
                                    verse: v as u32 + 1,
                                    text: text.verses[start + v].clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(ScriptureSearch {
            translation: self.header.id.clone(),
            hits,
            total,
        })
    }
}

/// Lower case with typographic apostrophes as plain ones (search).
fn fold(s: &str) -> String {
    s.to_lowercase().replace(['\u{2019}', '\u{2018}'], "'")
}

impl Text {
    fn index(header: &Header, verses: Vec<String>) -> Result<Text, String> {
        if verses.len() != header.verse_count() {
            return Err(format!(
                "{} verses, the versification has {}",
                verses.len(),
                header.verse_count()
            ));
        }
        let mut starts = Vec::with_capacity(header.books.len());
        let mut at = 0usize;
        for b in &header.books {
            let mut s = Vec::with_capacity(b.chapters.len());
            for &n in &b.chapters {
                s.push(at);
                at += n as usize;
            }
            starts.push(s);
        }
        let lower = verses.iter().map(|v| fold(v)).collect();
        Ok(Text {
            verses,
            lower,
            starts,
        })
    }
}

#[cfg(test)]
pub(crate) mod testdata {
    //! A made-up mini translation: invented verse text, never real scripture.

    use super::*;

    pub fn header(id: &str) -> Header {
        Header {
            format: format::FORMAT.into(),
            id: id.into(),
            abbreviation: id.into(),
            name: "Test Translation (synthetic)".into(),
            language: "en".into(),
            license: "Synthetic test data".into(),
            attribution: None,
            source: None,
            books: vec![
                HeaderBook {
                    key: "GEN".into(),
                    name: "Genesis".into(),
                    chapters: vec![3, 2],
                },
                HeaderBook {
                    key: "PSA".into(),
                    name: "Psalms".into(),
                    chapters: vec![2, 6],
                },
                HeaderBook {
                    key: "JHN".into(),
                    name: "John".into(),
                    chapters: vec![2, 4],
                },
                HeaderBook {
                    key: "JUD".into(),
                    name: "Jude".into(),
                    chapters: vec![3],
                },
            ],
        }
    }

    pub fn verses(h: &Header) -> Vec<String> {
        let mut out = Vec::new();
        for b in &h.books {
            for (c, &n) in b.chapters.iter().enumerate() {
                for v in 1..=n {
                    out.push(format!(
                        "The lantern keeper of {} walked road {} at hour {v}.",
                        b.key,
                        c + 1
                    ));
                }
            }
        }
        // A long verse and a missing one.
        out[0] = "Morning came over the hills of clay. The river woke slowly; \
                  the birds sang their seven songs, and every stone was warm. \
                  Nobody counted the hours, for the bells were silent that day."
            .into();
        out[1] = String::new();
        out
    }

    pub fn translation() -> Translation {
        let h = header("TST");
        let v = verses(&h);
        Translation::from_parts(h, v).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::testdata::*;
    use super::*;

    #[test]
    fn files_round_trip_and_load_from_folders() {
        let dir = std::env::temp_dir().join(format!("vesper-scripture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bundled = dir.join("bundled");
        let local = dir.join("local");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::create_dir_all(&local).unwrap();
        for (folder, id) in [(&bundled, "TST"), (&bundled, "SYN"), (&local, "tst")] {
            let mut h = header(id);
            h.name = format!("{id} in {}", folder.display());
            let mut f = Vec::new();
            format::write(&mut f, &h, &verses(&h)).unwrap();
            std::fs::write(folder.join(format!("{id}.vbible")), f).unwrap();
        }
        std::fs::write(bundled.join("broken.vbible"), "{}\n").unwrap();
        let s = Scripture::load(&[
            (bundled.clone(), ScriptureOrigin::Bundled),
            (local.clone(), ScriptureOrigin::Local),
            (dir.join("missing"), ScriptureOrigin::Local),
        ]);
        let list = s.translations();
        assert_eq!(
            list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["SYN", "tst"]
        );
        // The local file replaced the bundled one with the same ID.
        assert_eq!(list[1].origin, ScriptureOrigin::Local);
        let t = s.get("TST").unwrap();
        let p = t.passage("John 2:3").unwrap();
        assert_eq!(
            p.verses[0].text,
            "The lantern keeper of JHN walked road 2 at hour 3."
        );
        assert!(matches!(
            s.get("XYZ"),
            Err(LookupError::UnknownTranslation(_))
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn passages_follow_the_reference() {
        let t = translation();
        let p = t.passage("Ps 2:1-3, 6").unwrap();
        assert_eq!(p.label, "Psalm 2:1–3, 6");
        assert_eq!(
            p.verses.iter().map(|v| v.verse).collect::<Vec<_>>(),
            [1, 2, 3, 6]
        );
        // Across chapters; the missing verse (Genesis 1:2) is left out.
        let p = t.passage("Gen 1:1-2:1").unwrap();
        assert_eq!(
            p.verses
                .iter()
                .map(|v| (v.chapter, v.verse))
                .collect::<Vec<_>>(),
            [(1, 1), (1, 3), (2, 1)]
        );
        assert_eq!(
            t.passage("John 9"),
            Err(LookupError::BadReference("John has 2 chapters".into()))
        );
    }

    #[test]
    fn feedback_names_the_book_even_when_the_numbers_are_wrong() {
        let t = translation();
        let f = t.parse_feedback("jn 2:9");
        assert!(!f.ok);
        assert_eq!(f.book_name.as_deref(), Some("John"));
        assert_eq!(f.error.as_deref(), Some("John 2 has 4 verses"));
        let f = t.parse_feedback("Psalms 2");
        assert!(f.ok);
        assert_eq!(f.label.as_deref(), Some("Psalm 2"));
        let f = t.parse_feedback("Habakkuk 1");
        assert_eq!(f.error.as_deref(), Some("no book \"Habakkuk\" in TST"));
    }

    /// The made-up translation in another language: its file names the books in English
    /// (as converted files often do), except John, which it names itself.
    fn in_language(lang: &str, john: Option<&str>) -> Translation {
        let mut h = header("TSX");
        h.language = lang.into();
        if let Some(j) = john {
            h.books[2].name = j.into();
        }
        let v = verses(&h);
        Translation::from_parts(h, v).unwrap()
    }

    #[test]
    fn books_are_named_and_parsed_in_the_translations_language() {
        let t = in_language("es-419", None);
        let names: Vec<String> = t.books().into_iter().map(|b| b.name).collect();
        assert_eq!(names, ["Génesis", "Salmos", "Juan", "Judas"]);
        for (typed, label) in [
            ("Juan 1:2", "Juan 1:2"),
            ("Jn 1:2", "Juan 1:2"),
            ("John 1:2", "Juan 1:2"),
            ("genesis 2:1", "Génesis 2:1"),
            ("Gén 2:1", "Génesis 2:1"),
            ("Salmos 2:1-3", "Salmo 2:1\u{2013}3"),
            ("Sal 2", "Salmo 2"),
        ] {
            let f = t.parse_feedback(typed);
            assert!(f.ok, "{typed}: {:?}", f.error);
            assert_eq!(f.label.as_deref(), Some(label), "{typed}");
        }
        assert_eq!(
            t.parse_feedback("Jud 3").book_name.as_deref(),
            Some("Judas")
        );
        // German: the books' own numbering ("1. Mose").
        let t = in_language("de", None);
        assert_eq!(
            t.parse_feedback("1. Mose 1:2").label.as_deref(),
            Some("1. Mose 1:2")
        );
        assert_eq!(
            t.parse_feedback("Johannes 2:1").label.as_deref(),
            Some("Johannes 2:1")
        );
        assert_eq!(
            t.parse_feedback("Psalm 2").label.as_deref(),
            Some("Psalm 2")
        );
        // A name the file gives in its language stays; the table's spelling still parses.
        let t = in_language("fr", Some("Évangile selon Jean"));
        assert_eq!(t.books()[2].name, "Évangile selon Jean");
        assert_eq!(t.books()[0].name, "Genèse");
        let f = t.parse_feedback("Jean 2:1");
        assert_eq!(f.label.as_deref(), Some("Évangile selon Jean 2:1"));
        assert!(t.parse_feedback("evangile 2:1").ok);
        // English translations are untouched.
        assert_eq!(translation().books()[2].name, "John");
        // A passage's label and book are in the language too.
        let pt = in_language("pt", None);
        let p = pt.passage("Joao 2:1").unwrap();
        assert!(p.label.starts_with("João 2:1"), "{}", p.label);
        assert_eq!(
            pt.parse_feedback("Jo 2:1").label.as_deref(),
            Some("João 2:1")
        );
    }

    #[test]
    fn search_finds_every_word() {
        let t = translation();
        let r = t.search("LANTERN hour 3", 2).unwrap();
        // Genesis 1:3, Psalm 2:3, John 2:3, Jude 3.
        assert_eq!(r.total, 4);
        assert_eq!(r.hits.len(), 2);
        assert_eq!((r.hits[1].book.as_str(), r.hits[1].chapter), ("PSA", 2));
        assert_eq!(t.search("  ", 5).unwrap().total, 0);
        assert_eq!(t.search("seven songs", 5).unwrap().total, 1);
    }
}
