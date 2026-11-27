//! Codex search: an in-memory index per collection (`docs/CODEX-PLAN.md`: "search uses an
//! in-memory index"; there is no database).
//!
//! **What is searched** ([`KINDS`]): presentations (name, details, group names, slide labels,
//! slide text, slide and presentation notes), libraries (shelves), playlists and playlist
//! folders, media assets (name, original file reference), themes (name, template slide
//! names), props (name, slide text), messages (name, text), timers, macros and stage layouts
//! (names). Text is folded for matching: lower case, accents of Latin letters removed
//! (`é` → `e`, `ß` → `ss`, Vietnamese too), apostrophes inside a word dropped (`don't` →
//! `dont`); words are runs of letters and digits.
//!
//! **Queries**: every word must match the start of a word of the document (`gra` finds
//! "Grace"), anywhere in it; `"…"` is a phrase: its words follow each other in one field and
//! match whole words, but the last one of a phrase whose quote isn't closed yet (typing) is a
//! prefix. Ranking: each query word counts by the best field it matches in (name ≫ details,
//! groups, labels, template names > origins > text > notes; a whole word above a prefix), a
//! name that is the query or starts with it comes first; then by name.
//!
//! **The index** follows the documents themselves, not the files: it is built over
//! [`CollectionContent`] (a [`Source`] per collection), and [`SearchIndex::sync`] brings it up
//! to date by comparing each collection's document fingerprints ([`CollectionContent::hashes`])
//! with the ones it indexed, re-indexing only what was added, changed or removed (a collection
//! whose content is the same `Arc` as last time costs nothing). Collections no longer among
//! the sources (moved to the collection trash, deleted for good) are dropped; a restored one is
//! indexed again. On the Codex box the sources are its collections ([`sources`]); a show box
//! can pass its mirrored collection the same way.
//!
//! **When**: a low-priority thread ([`start`]) builds the index at start and syncs again after
//! every change of the documents ([`SearchIndex::changed`], called by the server's
//! setters, which every write of the Codex writer, a reload and a switch go through). A search
//! syncs first too, so it always sees the documents as they are (its own edits included).

use crate::data::{CollectionContent, DocHashes};
use model::{
    CollectionId, DocumentKind, LibraryId, MediaAssetId, PlaylistId, PresentationId, SectionId,
    SlideId, ThemeId,
};
use protocol::{ApiError, ErrorCode, SearchField, SearchHit, SearchResults, SnippetPart};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};

/// The document kinds the index covers.
pub const KINDS: &[DocumentKind] = &[
    DocumentKind::Presentation,
    DocumentKind::Library,
    DocumentKind::PlaylistNode,
    DocumentKind::MediaAsset,
    DocumentKind::Theme,
    DocumentKind::Prop,
    DocumentKind::Message,
    DocumentKind::Timer,
    DocumentKind::Macro,
    DocumentKind::StageLayout,
];

/// Hits answered by default.
pub const DEFAULT_LIMIT: usize = 50;
/// Most hits answered at once.
pub const MAX_LIMIT: usize = 500;
/// Longest query, characters.
pub const MAX_QUERY: usize = 500;
/// Query words and phrases used, at most (the rest are ignored).
const MAX_CLAUSES: usize = 16;
/// Longest indexed word, characters (longer ones are cut: still found by their start).
const MAX_TERM: usize = 48;

/// One collection to search: its ID, its name and its documents now.
#[derive(Clone)]
pub struct Source {
    /// The collection.
    pub id: CollectionId,
    /// Its name (in the hits).
    pub name: String,
    /// Its documents.
    pub content: Arc<CollectionContent>,
}

/// The Codex box's sources: every collection of the Codex (not the collection trash), with the
/// active one's documents as the show has them.
pub fn sources(sh: &crate::Shared) -> Vec<Source> {
    let codex = sh.codex();
    codex
        .collections
        .iter()
        .filter_map(|c| {
            Some(Source {
                id: c.collection.id,
                name: c.collection.name.clone(),
                content: sh.collection_content(c.collection.id)?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Folding and words

/// Latin-1 letters U+00C0–U+00FF without their accents ("" for × and ÷).
const LATIN1: [&str; 64] = [
    "a", "a", "a", "a", "a", "a", "ae", "c", "e", "e", "e", "e", "i", "i", "i", "i", //
    "d", "n", "o", "o", "o", "o", "o", "", "o", "u", "u", "u", "u", "y", "th", "ss", //
    "a", "a", "a", "a", "a", "a", "ae", "c", "e", "e", "e", "e", "i", "i", "i", "i", //
    "d", "n", "o", "o", "o", "o", "o", "", "o", "u", "u", "u", "u", "y", "th", "y",
];

/// Latin Extended-A U+0100–U+017F, one base letter each (`#`: "ij", `%`: "oe").
const LATIN_EXT_A: &str = concat!(
    "aaaaaa",
    "cccccccc",
    "dddd",
    "eeeeeeeeee",
    "gggggggg",
    "hhhh",
    "iiiiiiiiii",
    "##",
    "jj",
    "kkk",
    "llllllllll",
    "nnnnnnnnn",
    "oooooo",
    "%%",
    "rrrrrr",
    "ssssssss",
    "tttttt",
    "uuuuuuuuuuuu",
    "ww",
    "yyy",
    "zzzzzz",
    "s",
);

/// Latin Extended Additional U+1EA0–U+1EF9 (Vietnamese), one base letter each.
const VIETNAMESE: &str = concat!(
    "aaaaaaaaaaaaaaaaaaaaaaaa",
    "eeeeeeeeeeeeeeee",
    "iiii",
    "oooooooooooooooooooooooo",
    "uuuuuuuuuuuuuu",
    "yyyyyyyy",
);

/// A letter without its accent, when it is a Latin letter with one.
fn fold_latin(c: char) -> Option<&'static str> {
    let n = c as u32;
    let one = |table: &'static str, i: u32| -> &'static str {
        let i = i as usize;
        match &table[i..i + 1] {
            "#" => "ij",
            "%" => "oe",
            s => s,
        }
    };
    Some(match n {
        0xC0..=0xFF => LATIN1[(n - 0xC0) as usize],
        0x100..=0x17F => one(LATIN_EXT_A, n - 0x100),
        0x1EA0..=0x1EF9 => one(VIETNAMESE, n - 0x1EA0),
        0x192 => "f",
        0x1A0 | 0x1A1 => "o",
        0x1AF | 0x1B0 => "u",
        0x1CD | 0x1CE => "a",
        0x1CF | 0x1D0 => "i",
        0x1D1 | 0x1D2 => "o",
        0x1D3..=0x1DC => "u",
        0x218 | 0x219 => "s",
        0x21A | 0x21B => "t",
        _ => return None,
    })
}

/// A combining mark (decomposed accents): dropped.
fn is_mark(c: char) -> bool {
    matches!(c as u32, 0x300..=0x36F | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x20D0..=0x20FF | 0xFE20..=0xFE2F)
}

/// An apostrophe: inside a word it is dropped (`don't` → `dont`).
fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{02BC}')
}

/// Appends `c` folded (lower case, no accent).
fn fold_into(c: char, out: &mut String) {
    if c.is_ascii() {
        out.push(c.to_ascii_lowercase());
    } else if let Some(s) = fold_latin(c) {
        out.push_str(s);
    } else {
        out.extend(c.to_lowercase());
    }
}

/// The words of `text`, folded, with their byte range in `text`, in order.
fn words(text: &str, mut f: impl FnMut(&str, usize, usize)) {
    let mut word = String::new();
    let mut chars = 0;
    let (mut start, mut end) = (0, 0);
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c.is_alphanumeric() {
            if word.is_empty() {
                start = i;
                chars = 0;
            }
            if chars < MAX_TERM {
                fold_into(c, &mut word);
                chars += 1;
            }
            end = i + c.len_utf8();
        } else if !word.is_empty()
            && (is_mark(c)
                || (is_apostrophe(c) && it.peek().is_some_and(|(_, n)| n.is_alphanumeric())))
        {
            end = i + c.len_utf8();
        } else if !word.is_empty() {
            f(&word, start, end);
            word.clear();
        }
    }
    if !word.is_empty() {
        f(&word, start, end);
    }
}

/// The folded words of `text`.
fn folded(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    words(text, |w, _, _| out.push(w.to_string()));
    out
}

// ---------------------------------------------------------------------------------------------
// Documents as the index sees them

/// Where in a document a field is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct At {
    slide_index: Option<u32>,
    slide: Option<SlideId>,
    section: Option<SectionId>,
}

/// A document's searchable text: its name first, then its other fields.
struct RawDoc {
    title: String,
    fields: Vec<(SearchField, At, String)>,
}

impl RawDoc {
    fn named(name: &str) -> RawDoc {
        RawDoc {
            title: name.to_string(),
            fields: vec![(SearchField::Name, At::default(), name.to_string())],
        }
    }

    fn with(mut self, field: SearchField, at: At, text: String) -> RawDoc {
        if !text.trim().is_empty() {
            self.fields.push((field, at, text));
        }
        self
    }
}

fn weight(f: SearchField) -> u32 {
    match f {
        SearchField::Name => 100,
        SearchField::Details | SearchField::Group | SearchField::Label | SearchField::Template => {
            25
        }
        SearchField::Origin => 15,
        SearchField::Text => 10,
        SearchField::Notes => 5,
    }
}

fn presentation_doc(p: &model::Presentation) -> RawDoc {
    let mut details: Vec<String> = Vec::new();
    if let Some(s) = &p.song_info {
        for v in [
            &s.title,
            &s.artist,
            &s.authors,
            &s.publisher,
            &s.ccli_number,
        ]
        .into_iter()
        .flatten()
        {
            details.push(v.clone());
        }
    }
    if let Some(c) = &p.category {
        details.push(c.clone());
    }
    if let Some(s) = &p.scripture {
        details.push(s.label.clone().unwrap_or_else(|| s.book.name.clone()));
    }
    details.retain(|d| !d.trim().is_empty());
    let mut doc = RawDoc::named(&p.name)
        .with(SearchField::Details, At::default(), details.join(" · "))
        .with(SearchField::Notes, At::default(), p.notes.plain_text());
    let mut index = 0u32;
    for sec in &p.sections {
        let first = At {
            slide_index: (!sec.slides.is_empty()).then_some(index),
            slide: None,
            section: Some(sec.id),
        };
        doc = doc.with(SearchField::Group, first, sec.name.clone());
        for s in &sec.slides {
            let at = At {
                slide_index: Some(index),
                slide: Some(s.id),
                section: Some(sec.id),
            };
            if let Some(l) = &s.label {
                doc = doc.with(SearchField::Label, at, l.text.clone());
            }
            doc = doc.with(SearchField::Text, at, s.plain_text()).with(
                SearchField::Notes,
                at,
                s.notes.plain_text(),
            );
            index += 1;
        }
    }
    doc
}

fn asset_doc(a: &model::MediaAsset) -> RawDoc {
    let origin = a
        .origin
        .as_ref()
        .and_then(|o| o.absolute_hint.clone().or_else(|| o.relative_path.clone()))
        .unwrap_or_default();
    RawDoc::named(&a.name).with(SearchField::Origin, At::default(), origin)
}

fn theme_doc(t: &model::Theme) -> RawDoc {
    let mut doc = RawDoc::named(&t.name);
    for (i, s) in t.slides.iter().enumerate() {
        let at = At {
            slide_index: Some(i as u32),
            ..At::default()
        };
        doc = doc.with(SearchField::Template, at, s.name.clone());
    }
    doc
}

/// A document of the collection as the index sees it (`None`: not there, or not a kind the
/// index covers).
fn raw_doc(c: &CollectionContent, kind: DocumentKind, id: uuid::Uuid) -> Option<RawDoc> {
    fn find<T>(list: &[T], pred: impl Fn(&T) -> bool) -> Option<&T> {
        list.iter().find(|x| pred(x))
    }
    Some(match kind {
        DocumentKind::Presentation => presentation_doc(c.presentation(PresentationId(id))?),
        DocumentKind::Library => RawDoc::named(&c.libraries.get(&LibraryId(id))?.name),
        DocumentKind::PlaylistNode => RawDoc::named(&c.playlists.get(&PlaylistId(id))?.name),
        DocumentKind::MediaAsset => asset_doc(c.assets.get(&MediaAssetId(id))?),
        DocumentKind::Theme => theme_doc(c.themes.get(&ThemeId(id))?),
        DocumentKind::Prop => {
            let p = find(&c.props, |p| p.id.0 == id)?;
            RawDoc::named(&p.name).with(SearchField::Text, At::default(), p.slide.plain_text())
        }
        DocumentKind::Message => {
            let m = find(&c.messages, |m| m.id.0 == id)?;
            RawDoc::named(&m.name).with(SearchField::Text, At::default(), m.text.clone())
        }
        DocumentKind::Timer => RawDoc::named(&find(&c.timers, |t| t.id.0 == id)?.name),
        DocumentKind::Macro => RawDoc::named(&find(&c.macros, |m| m.id.0 == id)?.name),
        DocumentKind::StageLayout => RawDoc::named(&find(&c.stage_layouts, |l| l.id.0 == id)?.name),
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// The index

/// A field of an indexed document: its words as term IDs, in order.
struct Field {
    field: SearchField,
    at: At,
    text: String,
    terms: Vec<u32>,
}

/// An indexed document. Its name is always field 0.
struct Doc {
    kind: DocumentKind,
    id: uuid::Uuid,
    title: String,
    /// The name folded (order of equal scores).
    sort: String,
    fields: Vec<Field>,
}

/// A word in a field of a document.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Posting {
    doc: u32,
    field: u32,
}

/// One collection's index.
#[derive(Default)]
struct CollectionIndex {
    name: String,
    /// The content it was synced with (only compared: a `Weak` keeps no documents alive).
    source: Weak<CollectionContent>,
    /// The fingerprints of the indexed documents (of [`KINDS`]) at that sync.
    hashes: DocHashes,
    /// Every word seen, folded → term ID (ordered: prefixes are ranges).
    dict: BTreeMap<Box<str>, u32>,
    /// Documents by slot (`None`: free).
    docs: Vec<Option<Doc>>,
    free: Vec<u32>,
    slots: HashMap<(DocumentKind, uuid::Uuid), u32>,
    /// Term ID → the fields holding it (once per field).
    postings: HashMap<u32, Vec<Posting>>,
}

impl CollectionIndex {
    fn term(&mut self, word: &str) -> u32 {
        if let Some(&t) = self.dict.get(word) {
            return t;
        }
        let t = self.dict.len() as u32;
        self.dict.insert(word.into(), t);
        t
    }

    fn add(&mut self, kind: DocumentKind, id: uuid::Uuid, raw: RawDoc) {
        let slot = match self.free.pop() {
            Some(s) => s,
            None => {
                self.docs.push(None);
                (self.docs.len() - 1) as u32
            }
        };
        let mut fields = Vec::with_capacity(raw.fields.len());
        for (i, (field, at, text)) in raw.fields.into_iter().enumerate() {
            let mut terms = Vec::new();
            words(&text, |w, _, _| terms.push(self.term(w)));
            let mut distinct = terms.clone();
            distinct.sort_unstable();
            distinct.dedup();
            for t in distinct {
                self.postings.entry(t).or_default().push(Posting {
                    doc: slot,
                    field: i as u32,
                });
            }
            fields.push(Field {
                field,
                at,
                text,
                terms,
            });
        }
        let mut sort = String::new();
        for c in raw.title.chars() {
            fold_into(c, &mut sort);
        }
        self.docs[slot as usize] = Some(Doc {
            kind,
            id,
            title: raw.title,
            sort,
            fields,
        });
        self.slots.insert((kind, id), slot);
    }

    fn remove(&mut self, kind: DocumentKind, id: uuid::Uuid) {
        let Some(slot) = self.slots.remove(&(kind, id)) else {
            return;
        };
        let Some(doc) = self.docs[slot as usize].take() else {
            return;
        };
        let mut seen = HashSet::new();
        for t in doc.fields.iter().flat_map(|f| f.terms.iter().copied()) {
            if !seen.insert(t) {
                continue;
            }
            if let Some(list) = self.postings.get_mut(&t) {
                list.retain(|p| p.doc != slot);
                if list.is_empty() {
                    self.postings.remove(&t);
                }
            }
        }
        self.free.push(slot);
    }

    /// Brings the index up to `content` (see the module docs).
    fn sync(&mut self, content: &Arc<CollectionContent>) -> usize {
        if std::ptr::eq(self.source.as_ptr(), Arc::as_ptr(content)) {
            return 0;
        }
        let hashes: DocHashes = content
            .hashes
            .iter()
            .filter(|(k, _)| KINDS.contains(&k.0))
            .map(|(k, v)| (*k, *v))
            .collect();
        let gone: Vec<_> = self
            .hashes
            .keys()
            .filter(|k| !hashes.contains_key(k))
            .copied()
            .collect();
        let mut touched = gone.len();
        for (kind, id) in gone {
            self.remove(kind, id);
        }
        for (key, h) in &hashes {
            if self.hashes.get(key) == Some(h) {
                continue;
            }
            touched += 1;
            self.remove(key.0, key.1);
            if let Some(raw) = raw_doc(content, key.0, key.1) {
                self.add(key.0, key.1, raw);
            }
        }
        self.hashes = hashes;
        self.source = Arc::downgrade(content);
        touched
    }
}

/// The search index of every collection a box searches (one per server, in
/// [`crate::Server::search`]).
#[derive(Default)]
pub struct SearchIndex {
    collections: Mutex<HashMap<CollectionId, CollectionIndex>>,
    /// Wakes the indexing thread ([`start`]).
    wake: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

/// What a [`SearchIndex::sync`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncStats {
    /// Collections indexed or brought up to date.
    pub collections: usize,
    /// Documents indexed, re-indexed or dropped.
    pub documents: usize,
}

impl SearchIndex {
    /// An empty index.
    pub fn new() -> SearchIndex {
        SearchIndex::default()
    }

    /// The documents may have changed: wakes the indexing thread (when it runs).
    pub fn changed(&self) {
        if let Some(tx) = crate::lock(&self.wake).as_ref() {
            let _ = tx.send(());
        }
    }

    /// Brings the index up to the collections `sources` (dropping every other one).
    pub fn sync(&self, sources: &[Source]) -> SyncStats {
        let mut all = crate::lock(&self.collections);
        all.retain(|id, _| sources.iter().any(|s| s.id == *id));
        let mut stats = SyncStats::default();
        for s in sources {
            let c = all.entry(s.id).or_default();
            if c.name != s.name {
                c.name = s.name.clone();
            }
            let n = c.sync(&s.content);
            if n > 0 {
                stats.collections += 1;
                stats.documents += n;
            }
        }
        stats
    }

    /// Documents of the collections `scope` (as last synced) matching `query`, best first: at
    /// most `limit`, of the `kinds` given (`None`: every kind).
    pub fn search(
        &self,
        query: &str,
        scope: &[CollectionId],
        kinds: Option<&[DocumentKind]>,
        limit: usize,
    ) -> SearchResults {
        let clauses = parse(query);
        let mut out = SearchResults {
            hits: Vec::new(),
            total: 0,
        };
        if clauses.is_empty() {
            return out;
        }
        let all = crate::lock(&self.collections);
        // (score, collection, slot)
        let mut found: Vec<(u32, usize, u32)> = Vec::new();
        let indexes: Vec<(&CollectionId, &CollectionIndex)> = scope
            .iter()
            .filter_map(|id| all.get_key_value(id))
            .collect();
        let mut matched: Vec<Vec<Matcher>> = Vec::with_capacity(indexes.len());
        for (ci, (_, idx)) in indexes.iter().enumerate() {
            let m: Vec<Matcher> = clauses.iter().map(|c| Matcher::new(c, idx)).collect();
            if m.iter().any(Matcher::is_empty) {
                matched.push(m);
                continue;
            }
            for (slot, score) in score_docs(idx, &m, kinds) {
                found.push((score, ci, slot));
            }
            matched.push(m);
        }
        let doc = |ci: usize, slot: u32| -> &Doc {
            indexes[ci].1.docs[slot as usize]
                .as_ref()
                .expect("scored documents exist")
        };
        found.sort_by(|a, b| {
            let (da, db) = (doc(a.1, a.2), doc(b.1, b.2));
            b.0.cmp(&a.0)
                .then_with(|| da.sort.cmp(&db.sort))
                .then_with(|| da.id.cmp(&db.id))
        });
        out.total = found.len() as u32;
        found.truncate(limit);
        for (_, ci, slot) in found {
            let (id, idx) = indexes[ci];
            let d = doc(ci, slot);
            let m = &matched[ci];
            let f = best_field(d, m);
            let field = &d.fields[f];
            out.hits.push(SearchHit {
                kind: d.kind,
                id: d.id.to_string(),
                collection: *id,
                collection_name: idx.name.clone(),
                title: d.title.clone(),
                field: field.field,
                snippet: snippet(&field.text, &marks(&field.terms, m)),
                slide_index: field.at.slide_index,
                slide: field.at.slide,
                section: field.at.section,
            });
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------
// Queries

/// A query word, or a phrase.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Clause {
    words: Vec<String>,
    /// The last word matches as a prefix (every word outside quotes; the last word of a
    /// phrase whose quote isn't closed).
    prefix: bool,
}

/// The clauses of a query (see the module docs).
fn parse(q: &str) -> Vec<Clause> {
    let mut out: Vec<Clause> = Vec::new();
    let parts: Vec<&str> = q.split('"').collect();
    let unclosed = parts.len().is_multiple_of(2);
    for (i, part) in parts.iter().enumerate() {
        let w = folded(part);
        if i % 2 == 0 {
            for w in w {
                out.push(Clause {
                    words: vec![w],
                    prefix: true,
                });
            }
        } else if !w.is_empty() {
            out.push(Clause {
                words: w,
                prefix: unclosed && i == parts.len() - 1,
            });
        }
    }
    let mut seen = Vec::new();
    out.retain(|c| {
        let new = !seen.contains(c);
        if new {
            seen.push(c.clone());
        }
        new
    });
    out.truncate(MAX_CLAUSES);
    out
}

/// A clause against one collection's dictionary: the term IDs each of its words matches.
struct Matcher {
    /// Per word: the matching term IDs.
    sets: Vec<HashSet<u32>>,
    /// Per word: the term that is the word itself (a whole-word match).
    exact: Vec<Option<u32>>,
}

impl Matcher {
    fn new(c: &Clause, idx: &CollectionIndex) -> Matcher {
        let last = c.words.len() - 1;
        let mut sets = Vec::with_capacity(c.words.len());
        let mut exact = Vec::with_capacity(c.words.len());
        for (i, w) in c.words.iter().enumerate() {
            let e = idx.dict.get(w.as_str()).copied();
            exact.push(e);
            if c.prefix && i == last {
                sets.push(
                    idx.dict
                        .range::<str, _>((
                            std::ops::Bound::Included(w.as_str()),
                            std::ops::Bound::Unbounded,
                        ))
                        .take_while(|(k, _)| k.starts_with(w.as_str()))
                        .map(|(_, t)| *t)
                        .collect(),
                );
            } else {
                sets.push(e.into_iter().collect());
            }
        }
        Matcher { sets, exact }
    }

    fn is_empty(&self) -> bool {
        self.sets.iter().any(HashSet::is_empty)
    }

    fn phrase(&self) -> bool {
        self.sets.len() > 1
    }

    /// Word `j` of the clause matches the term `t`.
    fn has(&self, j: usize, t: u32) -> bool {
        let s = &self.sets[j];
        if s.len() == 1 {
            self.exact[j] == Some(t) || s.contains(&t)
        } else {
            s.contains(&t)
        }
    }

    /// Where the clause matches in a field's terms: the start of each match.
    fn starts<'a>(&'a self, terms: &'a [u32]) -> impl Iterator<Item = usize> + 'a {
        let n = self.sets.len();
        (0..terms.len().saturating_sub(n - 1))
            .filter(move |&i| (0..n).all(|j| self.has(j, terms[i + j])))
    }
}

/// The documents of `idx` every matcher matches, with their scores.
fn score_docs(
    idx: &CollectionIndex,
    m: &[Matcher],
    kinds: Option<&[DocumentKind]>,
) -> Vec<(u32, u32)> {
    let n = idx.docs.len();
    let kind_ok = |slot: u32| -> bool {
        match (&idx.docs[slot as usize], kinds) {
            (None, _) => false,
            (Some(_), None) => true,
            (Some(d), Some(k)) => k.contains(&d.kind),
        }
    };
    let mut total = vec![0u32; n];
    let mut hit = vec![0u32; n];
    let mut best = vec![0u32; n];
    for (ci, c) in m.iter().enumerate() {
        best.iter_mut().for_each(|b| *b = 0);
        if c.phrase() {
            let first: Vec<u32> = c.sets[0].iter().copied().collect();
            for t in first {
                for p in idx.postings.get(&t).into_iter().flatten() {
                    let Some(d) = &idx.docs[p.doc as usize] else {
                        continue;
                    };
                    let f = &d.fields[p.field as usize];
                    let w = weight(f.field) * 5;
                    if w > best[p.doc as usize] && c.starts(&f.terms).next().is_some() {
                        best[p.doc as usize] = w;
                    }
                }
            }
        } else {
            for &t in &c.sets[0] {
                let mult = if c.exact[0] == Some(t) { 5 } else { 4 };
                for p in idx.postings.get(&t).into_iter().flatten() {
                    let Some(d) = &idx.docs[p.doc as usize] else {
                        continue;
                    };
                    let w = weight(d.fields[p.field as usize].field) * mult;
                    let b = &mut best[p.doc as usize];
                    if w > *b {
                        *b = w;
                    }
                }
            }
        }
        for slot in 0..n {
            if best[slot] > 0 && hit[slot] == ci as u32 {
                hit[slot] += 1;
                total[slot] += best[slot];
            }
        }
    }
    let all = m.len() as u32;
    let mut out = Vec::new();
    for slot in 0..n {
        if hit[slot] != all || !kind_ok(slot as u32) {
            continue;
        }
        let d = idx.docs[slot].as_ref().expect("checked");
        out.push((
            slot as u32,
            total[slot] + title_bonus(&d.fields[0].terms, m),
        ));
    }
    out
}

/// A name that is the query (its words, whole, in order, nothing else) or starts with it comes
/// first.
fn title_bonus(title: &[u32], m: &[Matcher]) -> u32 {
    let words: Vec<(&Matcher, usize)> = m
        .iter()
        .flat_map(|c| (0..c.sets.len()).map(move |j| (c, j)))
        .collect();
    if title.len() < words.len() || !words.iter().zip(title).all(|((c, j), t)| c.has(*j, *t)) {
        return 0;
    }
    let whole = title.len() == words.len()
        && words
            .iter()
            .zip(title)
            .all(|((c, j), t)| c.exact[*j] == Some(*t));
    if whole { 2000 } else { 1000 }
}

/// The field of a matching document its snippet comes from: the one matching the most clauses,
/// then the heaviest, then the first.
fn best_field(d: &Doc, m: &[Matcher]) -> usize {
    let mut best = (0usize, 0u32, 0usize);
    for (i, f) in d.fields.iter().enumerate() {
        let n = m
            .iter()
            .filter(|c| c.starts(&f.terms).next().is_some())
            .count();
        let w = weight(f.field);
        if n > best.0 || (n == best.0 && n > 0 && w > best.1) {
            best = (n, w, i);
        }
    }
    best.2
}

/// Which words of a field the matchers match.
fn marks(terms: &[u32], m: &[Matcher]) -> Vec<bool> {
    let mut out = vec![false; terms.len()];
    for c in m {
        for s in c.starts(terms) {
            for o in out.iter_mut().skip(s).take(c.sets.len()) {
                *o = true;
            }
        }
    }
    out
}

/// Characters of context before the first match.
const BEFORE: usize = 40;
/// Characters a snippet runs to, about.
const SNIPPET: usize = 140;

/// Text for a snippet: a run of white space with a line break in it as ` / `, any other as one
/// space; none at the snippet's `start` or `end`.
fn clean(s: &str, start: bool, end: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let (mut space, mut line) = (false, false);
    let gap = |out: &mut String, line: bool| out.push_str(if line { " / " } else { " " });
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
            line |= matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
            continue;
        }
        if space && !(start && out.is_empty()) {
            gap(&mut out, line);
        }
        (space, line) = (false, false);
        out.push(c);
    }
    if space && !end && !(start && out.is_empty()) {
        gap(&mut out, line);
    }
    out
}

/// A snippet of `text` around its first marked word (`marks`: per word, see [`words`]).
fn snippet(text: &str, marks: &[bool]) -> Vec<SnippetPart> {
    let mut spans = Vec::with_capacity(marks.len());
    words(text, |_, s, e| spans.push((s, e)));
    let n = spans.len().min(marks.len());
    let first = (0..n).find(|&i| marks[i]).unwrap_or(0);
    // Start: a word at most BEFORE bytes before the first match (or the text's start).
    let mut k = first;
    while k > 0 && n > 0 && spans[first].0 - spans[k - 1].0 <= BEFORE {
        k -= 1;
    }
    let from = if k == 0 || n == 0 { 0 } else { spans[k].0 };
    // End: the last word ending within SNIPPET bytes of the start (at least the first match).
    let mut last = first;
    while last + 1 < n && spans[last + 1].1 - from <= SNIPPET {
        last += 1;
    }
    let to = if n == 0 || last + 1 >= n {
        text.len()
    } else {
        spans[last].1
    };
    let mut parts: Vec<SnippetPart> = Vec::new();
    let mut push = |s: &str, mark: bool| {
        if s.is_empty() {
            return;
        }
        match parts.last_mut() {
            Some(p) if p.mark == mark => p.text.push_str(s),
            _ => parts.push(SnippetPart {
                text: s.to_string(),
                mark,
            }),
        }
    };
    let mut at = from;
    for i in k..n {
        let (s, e) = spans[i];
        if s >= to {
            break;
        }
        if marks[i] {
            push(&text[at..s], false);
            push(&text[s..e], true);
            at = e;
        }
    }
    push(&text[at..to], false);
    let count = parts.len();
    for (i, p) in parts.iter_mut().enumerate() {
        if !p.mark {
            p.text = clean(&p.text, i == 0, i + 1 == count);
        }
    }
    parts.retain(|p| !p.text.is_empty());
    if from > 0 {
        match parts.first_mut() {
            Some(p) if !p.mark => p.text.insert(0, '…'),
            _ => parts.insert(
                0,
                SnippetPart {
                    text: "…".into(),
                    mark: false,
                },
            ),
        }
    }
    if to < text.len() && !text[to..].trim().is_empty() {
        match parts.last_mut() {
            Some(p) if !p.mark => p.text.push('…'),
            _ => parts.push(SnippetPart {
                text: "…".into(),
                mark: false,
            }),
        }
    }
    parts
}

// ---------------------------------------------------------------------------------------------
// The thread and the route

/// Starts the indexing thread: it builds the index at start and syncs it after every change
/// ([`SearchIndex::changed`]), at the lowest CPU priority. It ends with the server's
/// state.
pub fn start(sh: &Arc<crate::Shared>) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    *crate::lock(&sh.search().wake) = Some(tx);
    let weak = Arc::downgrade(sh);
    let spawned = std::thread::Builder::new()
        .name("codex-search".into())
        .spawn(move || {
            super::disk::lower_thread_priority();
            let mut first = true;
            loop {
                {
                    let Some(sh) = weak.upgrade() else { return };
                    let t = std::time::Instant::now();
                    let stats = sh.search().sync(&sources(&sh));
                    if first {
                        tracing::info!(
                            collections = stats.collections,
                            documents = stats.documents,
                            ms = t.elapsed().as_millis() as u64,
                            "Codex: search index built"
                        );
                        first = false;
                    } else if stats.documents > 0 {
                        tracing::debug!(
                            documents = stats.documents,
                            ms = t.elapsed().as_millis() as u64,
                            "Codex: search index updated"
                        );
                    }
                }
                if rx.recv().is_err() {
                    return;
                }
                while rx.try_recv().is_ok() {}
            }
        });
    if let Err(e) = spawned {
        tracing::error!("starting the search index thread: {e}");
    }
}

/// The query of `GET /codex/search`.
#[derive(Clone, Debug, Default)]
pub struct Params {
    /// `q`: the query (required).
    pub q: Option<String>,
    /// `collection`: a collection ID, `all`, or absent (this box's active collection).
    pub collection: Option<String>,
    /// `kinds`: document kinds, comma-separated (absent: every kind).
    pub kinds: Option<String>,
    /// `limit`: hits at most (default [`DEFAULT_LIMIT`], at most [`MAX_LIMIT`]).
    pub limit: Option<String>,
}

/// `GET /codex/search`: syncs the index and searches. Blocking (run it off the async threads).
pub fn run(sh: &crate::Shared, p: &Params) -> Result<SearchResults, ApiError> {
    let q =
        p.q.as_deref()
            .ok_or_else(|| ApiError::invalid("q", "a query is needed (q=…)"))?;
    if q.chars().count() > MAX_QUERY {
        return Err(ApiError::invalid(
            "q",
            format!("the query is longer than {MAX_QUERY} characters"),
        ));
    }
    let limit = match p.limit.as_deref() {
        None => DEFAULT_LIMIT,
        Some(v) => match v.trim().parse::<usize>() {
            Ok(n) if (1..=MAX_LIMIT).contains(&n) => n,
            _ => {
                return Err(ApiError::invalid(
                    "limit",
                    format!("limit must be a whole number from 1 to {MAX_LIMIT}, not {v:?}"),
                ));
            }
        },
    };
    let kinds = match p.kinds.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        None => None,
        Some(list) => {
            let mut out = Vec::new();
            for k in list.split(',').map(str::trim).filter(|k| !k.is_empty()) {
                let kind: Option<DocumentKind> =
                    serde_json::from_value(serde_json::Value::String(k.to_string())).ok();
                match kind {
                    Some(kind) if KINDS.contains(&kind) => out.push(kind),
                    _ => {
                        return Err(ApiError::invalid(
                            "kinds",
                            format!(
                                "{k:?} is not a kind search covers ({})",
                                KINDS
                                    .iter()
                                    .map(|k| crate::data::kind_dir(*k))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        ));
                    }
                }
            }
            Some(out)
        }
    };
    let codex = sh.codex();
    let scope: Vec<CollectionId> = match p.collection.as_deref().map(str::trim) {
        None | Some("") => vec![codex.active],
        Some("all") => codex.collections.iter().map(|c| c.collection.id).collect(),
        Some(s) => {
            let id: CollectionId = s.parse().map_err(|_| {
                ApiError::invalid(
                    "collection",
                    format!("collection must be a collection ID or \"all\", not {s:?}"),
                )
            })?;
            if codex.collection(id).is_none() {
                return Err(ApiError::new(
                    ErrorCode::NotFound,
                    format!("collection {id} is not in the Codex"),
                ));
            }
            vec![id]
        }
    };
    sh.search().sync(&sources(sh));
    Ok(sh.search().search(q, &scope, kinds.as_deref(), limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{ContentChange, doc_hash};
    use model::{
        Element, Library, MediaAsset, MediaOrigin, Message, Paragraph, PlaylistNode, Presentation,
        Prop, Rect, RichText, Section, Slide, SlideLabel, TemplateSlide, TextBox, Theme, Timer,
    };

    fn hit_text(h: &SearchHit) -> String {
        h.snippet
            .iter()
            .map(|p| {
                if p.mark {
                    format!("[{}]", p.text)
                } else {
                    p.text.clone()
                }
            })
            .collect()
    }

    fn slide(text: &str) -> Slide {
        Slide {
            elements: vec![Element::text(
                Rect::new(0.0, 0.0, 100.0, 100.0),
                TextBox {
                    paragraphs: Paragraph::from_lines(text),
                    ..TextBox::default()
                },
            )],
            ..Slide::new()
        }
    }

    fn song(name: &str, groups: &[(&str, &[&str])]) -> Presentation {
        let mut p = Presentation::new(name);
        for (g, slides) in groups {
            let mut s = Section::new(*g);
            s.slides = slides.iter().map(|t| slide(t)).collect();
            p.sections.push(s);
        }
        p
    }

    fn empty() -> CollectionContent {
        CollectionContent::load(
            &std::env::temp_dir().join(format!("search-none-{}", uuid::Uuid::new_v4())),
            std::env::temp_dir(),
            crate::library::MediaLibrary::detached(),
            Arc::default(),
        )
    }

    fn with_presentations(c: &CollectionContent, list: Vec<Presentation>) -> CollectionContent {
        c.with_change(&ContentChange {
            presentations: list
                .into_iter()
                .map(|p| (p.id, Some(Arc::new(p))))
                .collect(),
            ..ContentChange::default()
        })
    }

    fn source(id: CollectionId, name: &str, c: CollectionContent) -> Source {
        Source {
            id,
            name: name.into(),
            content: Arc::new(c),
        }
    }

    #[test]
    fn folding_drops_case_accents_and_apostrophes() {
        assert_eq!(
            folded("Ça VA, Señor? Ünïcode straße ŒUVRE Đường Ḁ"),
            [
                "ca", "va", "senor", "unicode", "strasse", "oeuvre", "duong", "ḁ"
            ]
        );
        // Decomposed accents, apostrophes inside words (straight and typographic), hyphens.
        assert_eq!(
            folded("Cafe\u{301} don't won’t 'quoted' well-known"),
            ["cafe", "dont", "wont", "quoted", "well", "known"]
        );
        assert_eq!(folded("Ứng Việt ơn"), ["ung", "viet", "on"]);
        assert_eq!(folded("  ...  "), Vec::<String>::new());
        // The tables line up with their blocks.
        assert_eq!(fold_latin('ı'), Some("i"));
        assert_eq!(fold_latin('Ĳ'), Some("ij"));
        assert_eq!(fold_latin('ł'), Some("l"));
        assert_eq!(fold_latin('ŉ'), Some("n"));
        assert_eq!(fold_latin('ž'), Some("z"));
        assert_eq!(fold_latin('ſ'), Some("s"));
        assert_eq!(fold_latin('ỹ'), Some("y"));
        assert_eq!(fold_latin('ự'), Some("u"));
        assert_eq!(fold_latin('ộ'), Some("o"));
        assert_eq!(fold_latin('ị'), Some("i"));
        assert_eq!(fold_latin('ễ'), Some("e"));
        assert_eq!(LATIN_EXT_A.len(), 0x80);
        assert_eq!(VIETNAMESE.len(), 0x1EF9 - 0x1EA0 + 1);
    }

    #[test]
    fn queries_parse_into_words_and_phrases() {
        let p = parse(r#"Morning "bright light" star "open fiel"#);
        let w: Vec<(Vec<&str>, bool)> = p
            .iter()
            .map(|c| (c.words.iter().map(String::as_str).collect(), c.prefix))
            .collect();
        assert_eq!(
            w,
            [
                (vec!["morning"], true),
                (vec!["bright", "light"], false),
                (vec!["star"], true),
                (vec!["open", "fiel"], true),
            ]
        );
        assert!(parse("  ¿?  ").is_empty());
        assert_eq!(parse("sun Sun SÚN").len(), 1);
    }

    #[test]
    fn titles_rank_before_text_and_snippets_mark_the_match() {
        let a = song(
            "River Song",
            &[
                ("Verse 1", &["Down by the river\nwe sing"]),
                ("Chorus", &["Morning light"]),
            ],
        );
        let b = song("Morning Light", &[("Verse", &["Lanterns glow"])]);
        let c = song(
            "Harvest",
            &[(
                "Verse",
                &["Fields of gold", "The morning star\nshines on the hills"],
            )],
        );
        let id = CollectionId::new();
        let idx = SearchIndex::new();
        let st = idx.sync(&[source(
            id,
            "Main",
            with_presentations(&empty(), vec![a.clone(), b.clone(), c.clone()]),
        )]);
        assert_eq!(
            st,
            SyncStats {
                collections: 1,
                documents: 3
            }
        );

        let r = idx.search("morn", &[id], None, 10);
        assert_eq!(r.total, 3);
        let titles: Vec<&str> = r.hits.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(titles[0], "Morning Light", "{titles:?}");
        assert_eq!(r.hits[0].field, SearchField::Name);
        assert_eq!(hit_text(&r.hits[0]), "[Morning] Light");
        assert_eq!(r.hits[0].slide_index, None);
        // Body hits: both slide text, by name.
        assert_eq!(&titles[1..], ["Harvest", "River Song"]);
        let h = &r.hits[1];
        assert_eq!(h.field, SearchField::Text);
        assert_eq!(h.slide_index, Some(1));
        assert_eq!(h.slide, Some(c.sections[0].slides[1].id));
        assert_eq!(h.section, Some(c.sections[0].id));
        assert_eq!(hit_text(h), "The [morning] star / shines on the hills");
        assert_eq!(r.hits[2].slide_index, Some(1));

        // Every word must match; the snippet's field matches the most of them.
        let r = idx.search("river sing", &[id], None, 10);
        assert_eq!(r.total, 1);
        assert_eq!(hit_text(&r.hits[0]), "Down by the [river] / we [sing]");
        // Group names, with the group's first slide.
        let r = idx.search("chorus", &[id], None, 10);
        assert_eq!(r.hits.len(), 1);
        assert_eq!(r.hits[0].field, SearchField::Group);
        assert_eq!(r.hits[0].slide_index, Some(1));
        assert_eq!(r.hits[0].slide, None);
        // Phrases: words in order, whole words unless the quote is still open.
        assert_eq!(idx.search("\"morning star\"", &[id], None, 10).total, 1);
        assert_eq!(idx.search("\"star morning\"", &[id], None, 10).total, 0);
        assert_eq!(idx.search("\"morning sta\"", &[id], None, 10).total, 0);
        assert_eq!(idx.search("\"morning sta", &[id], None, 10).total, 1);
        // Accents and case.
        assert_eq!(idx.search("MÓRNÏNG", &[id], None, 10).total, 3);
        // Nothing: an unknown word, no words.
        assert_eq!(idx.search("morning zebra", &[id], None, 10).total, 0);
        assert_eq!(idx.search("!!", &[id], None, 10).total, 0);
        // The limit cuts the list, not the total.
        let r = idx.search("morning", &[id], None, 1);
        assert_eq!((r.hits.len(), r.total), (1, 3));
    }

    #[test]
    fn a_name_that_is_the_query_comes_first() {
        let list = vec![
            song("Lighthouse Keepers", &[]),
            song("Old Lighthouse", &[]),
            song("Lighthouse", &[]),
            song("Lights", &[("Verse", &["lighthouse on the shore"])]),
            song("Lighthouses Far", &[]),
        ];
        let id = CollectionId::new();
        let idx = SearchIndex::new();
        idx.sync(&[source(id, "Main", with_presentations(&empty(), list))]);
        let r = idx.search("lighthouse", &[id], None, 10);
        let titles: Vec<&str> = r.hits.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Lighthouse",
                "Lighthouse Keepers",
                "Lighthouses Far",
                "Old Lighthouse",
                "Lights"
            ]
        );
    }

    #[test]
    fn the_index_follows_changes_and_collections() {
        let (main, youth) = (CollectionId::new(), CollectionId::new());
        let a = song("Evening Hymn", &[("Verse", &["Quiet waters"])]);
        let base = with_presentations(&empty(), vec![a.clone()]);
        let idx = SearchIndex::new();
        let c1 = Arc::new(base);
        let src = |c: &Arc<CollectionContent>| Source {
            id: main,
            name: "Main".into(),
            content: c.clone(),
        };
        idx.sync(&[src(&c1)]);
        // The same content again: nothing to do.
        assert_eq!(idx.sync(&[src(&c1)]), SyncStats::default());
        assert_eq!(idx.search("quiet", &[main], None, 10).total, 1);

        // Changed: the old text is gone, the new one found.
        let mut a2 = a.clone();
        a2.sections[0].slides[0] = slide("Still waters");
        let c2 = Arc::new(with_presentations(&c1, vec![a2]));
        assert_eq!(idx.sync(&[src(&c2)]).documents, 1);
        assert_eq!(idx.search("quiet", &[main], None, 10).total, 0);
        assert_eq!(idx.search("still", &[main], None, 10).total, 1);

        // Other kinds come and go with their own lists.
        let t = Timer {
            name: "Countdown to service".into(),
            ..Timer::default()
        };
        let m = Message {
            name: "Nursery".into(),
            text: "Parent of child {number}, please come".into(),
            ..Message::default()
        };
        let c3 = Arc::new(c2.with_show_docs(Some(vec![t.clone()]), None, Some(vec![m]), None));
        idx.sync(&[src(&c3)]);
        let r = idx.search("parent", &[main], None, 10);
        assert_eq!(r.hits[0].kind, DocumentKind::Message);
        assert_eq!(r.hits[0].field, SearchField::Text);
        assert_eq!(
            idx.search("countdown", &[main], None, 10).hits[0].kind,
            DocumentKind::Timer
        );
        // Kinds filter.
        assert_eq!(
            idx.search(
                "countdown",
                &[main],
                Some(&[DocumentKind::Presentation]),
                10
            )
            .total,
            0
        );

        // Removed.
        let c4 = Arc::new(c3.with_change(&ContentChange {
            presentations: vec![(a.id, None)],
            ..ContentChange::default()
        }));
        idx.sync(&[src(&c4)]);
        assert_eq!(idx.search("still", &[main], None, 10).total, 0);

        // A second collection; its name in the hits; dropped when it isn't a source any more.
        let y = song("Still Pond Sketch", &[]);
        let ys = source(youth, "Youth", with_presentations(&empty(), vec![y]));
        idx.sync(&[src(&c4), ys.clone()]);
        let r = idx.search("still", &[main, youth], None, 10);
        assert_eq!(r.total, 1);
        assert_eq!(r.hits[0].collection, youth);
        assert_eq!(r.hits[0].collection_name, "Youth");
        assert_eq!(idx.search("still", &[main], None, 10).total, 0);
        idx.sync(&[src(&c4)]);
        assert_eq!(idx.search("still", &[main, youth], None, 10).total, 0);
        // Back: indexed again.
        idx.sync(&[src(&c4), ys]);
        assert_eq!(idx.search("still", &[youth], None, 10).total, 1);
    }

    #[test]
    fn every_kind_has_its_fields() {
        let mut c = empty();
        let lib = Library {
            name: "Hymnal".into(),
            ..Library::default()
        };
        let pl = PlaylistNode {
            name: "Christmas Eve".into(),
            ..PlaylistNode::default()
        };
        let asset = MediaAsset {
            name: "Sunrise loop".into(),
            origin: Some(MediaOrigin {
                absolute_hint: Some("X:\\Media\\Backgrounds\\Golden Hour.mov".into()),
                ..MediaOrigin::default()
            }),
            ..MediaAsset::default()
        };
        let theme = Theme {
            name: "Lower Thirds".into(),
            slides: vec![
                TemplateSlide {
                    name: "Lyrics".into(),
                    ..TemplateSlide::default()
                },
                TemplateSlide {
                    name: "Scripture".into(),
                    ..TemplateSlide::default()
                },
            ],
            ..Theme::default()
        };
        let prop = Prop {
            name: "Welcome bug".into(),
            slide: slide("Welcome home"),
            ..Prop::default()
        };
        let mut p = song("Talk", &[("Intro", &["Opening words"])]);
        p.notes = RichText::plain("Bring the lantern");
        p.sections[0].slides[0].label = Some(SlideLabel {
            text: "Kickoff".into(),
            ..SlideLabel::default()
        });
        p.sections[0].slides[0].notes = RichText::plain("Pause for applause");
        p.song_info = Some(model::SongInfo {
            artist: Some("The Example Choir".into()),
            ccli_number: Some("1234567".into()),
            ..model::SongInfo::default()
        });
        c = with_presentations(&c, vec![p]);
        c = c.with_change(&ContentChange {
            libraries: vec![(lib.id, Some(lib))],
            playlists: vec![(pl.id, Some(pl))],
            assets: vec![(asset.id, Some(asset))],
            ..ContentChange::default()
        });
        c = c.with_show_docs(None, Some(vec![prop]), None, None);
        c.hashes
            .insert((DocumentKind::Theme, theme.id.0), doc_hash(&theme));
        c.themes = Arc::new([(theme.id, theme)].into_iter().collect());
        let id = CollectionId::new();
        let idx = SearchIndex::new();
        idx.sync(&[source(id, "Main", c)]);
        let one = |q: &str| -> SearchHit {
            let r = idx.search(q, &[id], None, 10);
            assert_eq!(r.total, 1, "{q}: {:?}", r.hits);
            r.hits.into_iter().next().unwrap()
        };
        assert_eq!(one("hymnal").kind, DocumentKind::Library);
        assert_eq!(one("christmas").kind, DocumentKind::PlaylistNode);
        let h = one("golden");
        assert_eq!(
            (h.kind, h.field),
            (DocumentKind::MediaAsset, SearchField::Origin)
        );
        assert_eq!(hit_text(&h), "X:\\Media\\Backgrounds\\[Golden] Hour.mov");
        let h = one("scripture");
        assert_eq!(
            (h.kind, h.field, h.slide_index),
            (DocumentKind::Theme, SearchField::Template, Some(1))
        );
        let h = one("home");
        assert_eq!((h.kind, h.field), (DocumentKind::Prop, SearchField::Text));
        assert_eq!(one("lantern").field, SearchField::Notes);
        let h = one("applause");
        assert_eq!((h.field, h.slide_index), (SearchField::Notes, Some(0)));
        assert_eq!(one("kickoff").field, SearchField::Label);
        assert_eq!(one("1234567").field, SearchField::Details);
        assert_eq!(hit_text(&one("choir")), "The Example [Choir] · 1234567");
    }

    #[test]
    fn long_text_is_cut_around_the_match() {
        let text = "one two three four five six seven eight nine ten eleven twelve \
                    thirteen fourteen fifteen sixteen seventeen eighteen nineteen twenty \
                    twentyone twentytwo twentythree twentyfour twentyfive twentysix \
                    twentyseven twentyeight twentynine thirty thirtyone thirtytwo";
        let terms = folded(text);
        let marks: Vec<bool> = terms.iter().map(|t| t == "fifteen").collect();
        let s = snippet(text, &marks);
        let joined: String = s.iter().map(|p| p.text.as_str()).collect();
        assert!(joined.starts_with('…') && joined.ends_with('…'), "{joined}");
        assert!(s.iter().any(|p| p.mark && p.text == "fifteen"));
        assert!(joined.chars().count() < SNIPPET + 10, "{joined}");
        // Short text: whole, no ellipsis.
        let s = snippet("Amen", &[true]);
        assert_eq!(
            s,
            [SnippetPart {
                text: "Amen".into(),
                mark: true
            }]
        );
    }

    /// Synthetic songs: made-up words, `n` presentations of 12 slides of 4 lines each.
    fn corpus(n: usize) -> Vec<Presentation> {
        const SYL: [&str; 24] = [
            "ka", "lo", "mi", "ra", "te", "so", "vu", "ne", "pa", "di", "ro", "ze", "fa", "gu",
            "hi", "jo", "be", "ty", "wo", "xe", "qu", "ly", "ce", "sa",
        ];
        let mut seed = 0x2545_f491_u64;
        let mut rnd = move |m: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % m as u64) as usize
        };
        let word = |rnd: &mut dyn FnMut(usize) -> usize| {
            let k = 1 + rnd(3);
            (0..k).map(|_| SYL[rnd(SYL.len())]).collect::<String>()
        };
        (0..n)
            .map(|i| {
                let name = format!("{} {} {i}", word(&mut rnd), word(&mut rnd));
                let mut p = Presentation::new(name);
                for g in 0..4 {
                    let mut s = Section::new(["Verse", "Chorus", "Bridge", "Tag"][g]);
                    for _ in 0..3 {
                        let lines: Vec<String> = (0..4)
                            .map(|_| (0..6).map(|_| word(&mut rnd)).collect::<Vec<_>>().join(" "))
                            .collect();
                        s.slides.push(slide(&lines.join("\n")));
                    }
                    p.sections.push(s);
                }
                p
            })
            .collect()
    }

    /// A few thousand presentations answer well under 100 ms (debug build included); the
    /// numbers are printed (`--nocapture`).
    #[test]
    fn a_few_thousand_presentations_answer_fast() {
        let n = 3000;
        let content = with_presentations(&empty(), corpus(n));
        let id = CollectionId::new();
        let idx = SearchIndex::new();
        let t = std::time::Instant::now();
        idx.sync(&[source(id, "Main", content)]);
        let build = t.elapsed();
        let words: usize = {
            let all = crate::lock(&idx.collections);
            let c = &all[&id];
            c.docs
                .iter()
                .flatten()
                .flat_map(|d| &d.fields)
                .map(|f| f.terms.len())
                .sum()
        };
        eprintln!(
            "search index: {n} presentations, {words} words, built in {} ms",
            build.as_millis()
        );
        for q in [
            "ka", "lo mi", "kalo", "\"ra te", "zezeze", "verse", "qu ly ce", "nothing",
        ] {
            let t = std::time::Instant::now();
            let r = idx.search(q, &[id], None, DEFAULT_LIMIT);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            eprintln!("  {q:>10}: {:>5} matches, {ms:.1} ms", r.total);
            assert!(ms < 100.0, "{q}: {ms} ms");
        }
    }
}
