//! Reference parsing and labels.
//!
//! A reference is a book name ([`crate::books`]) followed by items separated by `,` or `;`:
//!
//! | Item | Means |
//! |---|---|
//! | `3` | chapter 3 (after `C:V,` a bare number is a verse of chapter C) |
//! | `3:16` | one verse (`3.16` too) |
//! | `3:16-18` | verses of one chapter (`-`, `–` or `—`) |
//! | `1:31-2:3` | across chapters |
//! | `1-2` | whole chapters (verses after `C:V,`) |
//!
//! `;` always starts a new chapter context. In a one-chapter book (Jude) bare numbers are
//! verses. A trailing `a` / `b` on a verse is ignored. Everything is checked against the
//! translation's versification.

use crate::format::HeaderBook;
use protocol::VerseRange;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tok {
    Num(u32),
    Colon,
    Dash,
    Comma,
}

fn tokens(s: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '0'..='9' => {
                let mut n = c.to_digit(10).unwrap_or(0);
                while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
                    n = n.saturating_mul(10).saturating_add(d);
                    chars.next();
                }
                // "16a", "16b": part of a verse.
                if matches!(chars.peek(), Some('a' | 'b' | 'c')) {
                    chars.next();
                }
                out.push(Tok::Num(n));
            }
            ':' | '.' => out.push(Tok::Colon),
            '-' | '\u{2013}' | '\u{2014}' | '\u{2010}' | '\u{2011}' => out.push(Tok::Dash),
            ',' => out.push(Tok::Comma),
            c if c.is_whitespace() => {}
            c => return Err(format!("unexpected {c:?} in the reference")),
        }
    }
    Ok(out)
}

/// Splits typed text into the book part and the numbers part ("1 Cor 13:4" → "1 Cor",
/// "13:4"). The book part may start with a number ("1 John").
pub fn split_book(q: &str) -> (&str, &str) {
    let q = q.trim();
    let bytes: Vec<(usize, char)> = q.char_indices().collect();
    // Skip a leading book number ("1", "2", "3") followed by letters.
    let mut i = 0;
    while i < bytes.len() && bytes[i].1.is_ascii_digit() {
        i += 1;
    }
    let lead_num = i > 0 && {
        let mut j = i;
        while j < bytes.len() && (bytes[j].1 == ' ' || bytes[j].1 == '.') {
            j += 1;
        }
        j < bytes.len() && bytes[j].1.is_alphabetic()
    };
    if !lead_num {
        i = 0;
    }
    // The book name runs until the first digit after a letter.
    let mut seen_letter = false;
    while i < bytes.len() {
        let c = bytes[i].1;
        if c.is_alphabetic() {
            seen_letter = true;
        } else if c.is_ascii_digit() && seen_letter {
            break;
        }
        i += 1;
    }
    let at = bytes.get(i).map_or(q.len(), |b| b.0);
    (q[..at].trim(), q[at..].trim())
}

fn chapter_len(book: &HeaderBook, chapter: u32) -> Result<u32, String> {
    match book.chapters.get((chapter as usize).wrapping_sub(1)) {
        Some(&n) if chapter >= 1 => Ok(n),
        _ => Err(if book.chapters.len() == 1 {
            format!("{} has 1 chapter", book.name)
        } else {
            format!("{} has {} chapters", book.name, book.chapters.len())
        }),
    }
}

fn check_verse(book: &HeaderBook, chapter: u32, verse: u32) -> Result<(), String> {
    let n = chapter_len(book, chapter)?;
    if verse == 0 || verse > n {
        let name = crate::books::chapter_name(&book.key, &book.name);
        return Err(format!("{name} {chapter} has {n} verses"));
    }
    Ok(())
}

/// Parses one `;`-separated group of the numbers part against a book's versification.
pub fn parse_ranges(book: &HeaderBook, part: &str) -> Result<Vec<VerseRange>, String> {
    let toks = tokens(part)?;
    if toks.is_empty() {
        return Err(format!("which chapter of {}?", book.name));
    }
    let one_chapter = book.chapters.len() == 1;
    let mut out = Vec::new();
    // The chapter bare numbers refer to after "C:V,".
    let mut verse_ctx: Option<u32> = if one_chapter { Some(1) } else { None };
    for item in toks.split(|t| *t == Tok::Comma) {
        if item.is_empty() {
            continue;
        }
        let (start, end) = match item.iter().position(|t| *t == Tok::Dash) {
            Some(d) => (&item[..d], Some(&item[d + 1..])),
            None => (item, None),
        };
        let point = |t: &[Tok]| -> Result<(Option<u32>, u32), String> {
            match t {
                [Tok::Num(c), Tok::Colon, Tok::Num(v)] => Ok((Some(*c), *v)),
                [Tok::Num(n)] => Ok((None, *n)),
                _ => Err("that isn't a reference like \"John 3:16\"".into()),
            }
        };
        let (sc, sn) = point(start)?;
        let r = match (sc, verse_ctx) {
            // "C:V…"
            (Some(c), _) => {
                check_verse(book, c, sn)?;
                let (ec, ev) = match end {
                    None => (c, sn),
                    Some(e) => match point(e)? {
                        (Some(ec), ev) => (ec, ev),
                        (None, ev) => (c, ev),
                    },
                };
                verse_ctx = Some(ec);
                VerseRange {
                    chapter: c,
                    verse: sn,
                    end_chapter: ec,
                    end_verse: ev,
                }
            }
            // A bare number after "C:V,": a verse of that chapter.
            (None, Some(c)) => {
                let (ec, ev) = match end {
                    None => (c, sn),
                    Some(e) => match point(e)? {
                        (Some(ec), ev) => (ec, ev),
                        (None, ev) => (c, ev),
                    },
                };
                verse_ctx = Some(ec);
                VerseRange {
                    chapter: c,
                    verse: sn,
                    end_chapter: ec,
                    end_verse: ev,
                }
            }
            // Whole chapters ("13", "1-2"), or "1-2:3" (chapter 1 to 2:3).
            (None, None) => {
                chapter_len(book, sn)?;
                match end.map(point).transpose()? {
                    None => VerseRange {
                        chapter: sn,
                        verse: 1,
                        end_chapter: sn,
                        end_verse: chapter_len(book, sn)?,
                    },
                    Some((None, ec)) => VerseRange {
                        chapter: sn,
                        verse: 1,
                        end_chapter: ec,
                        end_verse: chapter_len(book, ec)?,
                    },
                    Some((Some(ec), ev)) => {
                        verse_ctx = Some(ec);
                        VerseRange {
                            chapter: sn,
                            verse: 1,
                            end_chapter: ec,
                            end_verse: ev,
                        }
                    }
                }
            }
        };
        check_verse(book, r.chapter, r.verse)?;
        check_verse(book, r.end_chapter, r.end_verse)?;
        if (r.end_chapter, r.end_verse) < (r.chapter, r.verse) {
            return Err("the range ends before it starts".into());
        }
        out.push(r);
    }
    Ok(out)
}

/// Parses the numbers part, honouring `;` (a new chapter context) and `,` (same context).
pub fn parse(book: &HeaderBook, part: &str) -> Result<Vec<VerseRange>, String> {
    let mut out = Vec::new();
    for group in part.split(';') {
        if group.trim().is_empty() {
            continue;
        }
        out.extend(parse_ranges(book, group)?);
    }
    if out.is_empty() {
        return Err(format!("which chapter of {}?", book.name));
    }
    Ok(out)
}

/// The label of ranges ("Psalm 23:1–4, 6", "John 3:35–36; 4:1–2", "Genesis 1–2").
pub fn label(book: &HeaderBook, ranges: &[VerseRange]) -> String {
    label_with(book, ranges, true)
}

/// [`label`], with whole chapters written as verses ("Psalm 23:1–6") unless `chapters`.
pub fn label_with(book: &HeaderBook, ranges: &[VerseRange], chapters: bool) -> String {
    let whole = |r: &VerseRange| {
        chapters
            && r.verse == 1
            && book.chapters.get(r.end_chapter as usize - 1) == Some(&r.end_verse)
    };
    let single_chapter = ranges
        .iter()
        .all(|r| r.chapter == ranges[0].chapter && r.end_chapter == ranges[0].chapter);
    let name = if single_chapter {
        crate::books::chapter_name(&book.key, &book.name)
    } else {
        &book.name
    };
    let one_chapter_book = book.chapters.len() == 1;
    let mut s = String::new();
    let mut last_chapter: Option<u32> = None;
    for r in ranges {
        let part = if whole(r) && !one_chapter_book {
            last_chapter = None;
            if r.chapter == r.end_chapter {
                format!("{}", r.chapter)
            } else {
                format!("{}\u{2013}{}", r.chapter, r.end_chapter)
            }
        } else {
            let same = last_chapter == Some(r.chapter);
            let head = if same || one_chapter_book {
                format!("{}", r.verse)
            } else {
                format!("{}:{}", r.chapter, r.verse)
            };
            let tail = if r.end_chapter != r.chapter {
                format!("\u{2013}{}:{}", r.end_chapter, r.end_verse)
            } else if r.end_verse != r.verse {
                format!("\u{2013}{}", r.end_verse)
            } else {
                String::new()
            };
            let sep = if s.is_empty() {
                ""
            } else if same || one_chapter_book {
                ", "
            } else {
                "; "
            };
            last_chapter = Some(r.end_chapter);
            s.push_str(sep);
            s.push_str(&head);
            s.push_str(&tail);
            continue;
        };
        if !s.is_empty() {
            s.push_str("; ");
        }
        s.push_str(&part);
    }
    format!("{name} {s}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(key: &str, name: &str, chapters: &[u32]) -> HeaderBook {
        HeaderBook {
            key: key.into(),
            name: name.into(),
            chapters: chapters.to_vec(),
        }
    }

    fn r(c: u32, v: u32, ec: u32, ev: u32) -> VerseRange {
        VerseRange {
            chapter: c,
            verse: v,
            end_chapter: ec,
            end_verse: ev,
        }
    }

    #[test]
    fn splits_the_book_from_the_numbers() {
        assert_eq!(split_book("Jn 3:16-18"), ("Jn", "3:16-18"));
        assert_eq!(split_book("1 Cor 13"), ("1 Cor", "13"));
        assert_eq!(split_book("1Cor13"), ("1Cor", "13"));
        assert_eq!(split_book("Song of Songs 2:1"), ("Song of Songs", "2:1"));
        assert_eq!(split_book("Psalms"), ("Psalms", ""));
        assert_eq!(split_book("  psalm 23:1–4, 6 "), ("psalm", "23:1–4, 6"));
    }

    #[test]
    fn parses_the_usual_forms() {
        let jn = book("JHN", "John", &[51, 25, 36, 54]);
        assert_eq!(parse(&jn, "3:16").unwrap(), vec![r(3, 16, 3, 16)]);
        assert_eq!(parse(&jn, "3:16-18").unwrap(), vec![r(3, 16, 3, 18)]);
        assert_eq!(parse(&jn, "3.16–18").unwrap(), vec![r(3, 16, 3, 18)]);
        assert_eq!(parse(&jn, "3").unwrap(), vec![r(3, 1, 3, 36)]);
        assert_eq!(parse(&jn, "1-2").unwrap(), vec![r(1, 1, 2, 25)]);
        assert_eq!(parse(&jn, "1:51-2:3").unwrap(), vec![r(1, 51, 2, 3)]);
        assert_eq!(
            parse(&jn, "3:1-4, 6").unwrap(),
            vec![r(3, 1, 3, 4), r(3, 6, 3, 6)]
        );
        assert_eq!(
            parse(&jn, "3:35–36, 4:1–2").unwrap(),
            vec![r(3, 35, 3, 36), r(4, 1, 4, 2)]
        );
        assert_eq!(
            parse(&jn, "3:35-36; 4").unwrap(),
            vec![r(3, 35, 3, 36), r(4, 1, 4, 54)]
        );
        assert_eq!(parse(&jn, "3:16a").unwrap(), vec![r(3, 16, 3, 16)]);
    }

    #[test]
    fn one_chapter_books_take_verses() {
        let jude = book("JUD", "Jude", &[25]);
        assert_eq!(parse(&jude, "3").unwrap(), vec![r(1, 3, 1, 3)]);
        assert_eq!(parse(&jude, "3-5").unwrap(), vec![r(1, 3, 1, 5)]);
        assert_eq!(parse(&jude, "1:3").unwrap(), vec![r(1, 3, 1, 3)]);
        assert_eq!(label(&jude, &[r(1, 3, 1, 5)]), "Jude 3\u{2013}5");
    }

    #[test]
    fn says_what_is_wrong() {
        let jn = book("JHN", "John", &[51, 25, 36]);
        assert_eq!(parse(&jn, "4").unwrap_err(), "John has 3 chapters");
        assert_eq!(parse(&jn, "3:40").unwrap_err(), "John 3 has 36 verses");
        assert_eq!(parse(&jn, "").unwrap_err(), "which chapter of John?");
        assert!(parse(&jn, "3:18-16").is_err());
        assert!(parse(&jn, "3:x").is_err());
        let ps = book("PSA", "Psalms", &[6, 12, 8]);
        assert_eq!(parse(&ps, "2:13").unwrap_err(), "Psalm 2 has 12 verses");
    }

    #[test]
    fn labels_read_naturally() {
        let ps = book("PSA", "Psalms", &[6, 12, 8]);
        assert_eq!(
            label(&ps, &[r(2, 1, 2, 4), r(2, 6, 2, 6)]),
            "Psalm 2:1–4, 6"
        );
        assert_eq!(label(&ps, &[r(2, 1, 2, 12)]), "Psalm 2");
        assert_eq!(label(&ps, &[r(1, 1, 2, 12)]), "Psalms 1–2");
        let jn = book("JHN", "John", &[51, 25, 36, 54]);
        assert_eq!(label(&jn, &[r(3, 16, 3, 18)]), "John 3:16–18");
        assert_eq!(label(&jn, &[r(1, 51, 2, 3)]), "John 1:51–2:3");
        assert_eq!(
            label(&jn, &[r(3, 35, 3, 36), r(4, 1, 4, 2)]),
            "John 3:35–36; 4:1–2"
        );
        assert_eq!(label(&jn, &[r(3, 16, 3, 16)]), "John 3:16");
    }
}
