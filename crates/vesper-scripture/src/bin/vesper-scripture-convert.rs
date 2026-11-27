//! `vesper-scripture-convert`: turns a downloaded public-domain Bible text into Vesper's
//! translation format (`<ID>.vbible`, see `vesper_scripture::format`).
//! `scripts/fetch-scripture.sh` runs it; nothing here downloads.
//!
//! ```text
//! vesper-scripture-convert --kind vpl|tsv --input <file> --out <dir> --id KJV --name "…"
//!        [--abbreviation KJV] [--language en] --license "…" [--attribution "…"] [--source <url>]
//! ```
//!
//! - `vpl`: one verse per line, `GEN 1:1 text` (ebible.org's VPL files; their book codes are
//!   mapped to USFM).
//! - `tsv`: `Book C:V<TAB>text` lines (bereanbible.com's `bsb.txt`); other lines are skipped.
//!
//! Only the 66 books of the Protestant canon are kept, in canonical order. Square brackets
//! (supplied words) and paragraph marks are removed, whitespace is collapsed. The
//! versification is the one the text has: a chapter's verse count is its highest verse
//! number, and a verse the text lacks is an empty line.

use std::collections::BTreeMap;
use std::path::PathBuf;
use vesper_scripture::books::{self, CANON};
use vesper_scripture::format::{self, Header, HeaderBook};

/// ebible.org VPL book codes that differ from USFM.
const VPL_CODES: &[(&str, &str)] = &[
    ("SOL", "SNG"),
    ("EZE", "EZK"),
    ("JOE", "JOL"),
    ("NAH", "NAM"),
    ("MAR", "MRK"),
    ("JOH", "JHN"),
    ("PHI", "PHP"),
    ("JAM", "JAS"),
    ("1JO", "1JN"),
    ("2JO", "2JN"),
    ("3JO", "3JN"),
];

fn clean(text: &str) -> String {
    let t: String = text
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | '\u{b6}' | '\u{feff}'))
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parses "C:V" into numbers.
fn chapter_verse(s: &str) -> Option<(u32, u32)> {
    let (c, v) = s.split_once(':')?;
    Some((c.trim().parse().ok()?, v.trim().parse().ok()?))
}

type Verses = BTreeMap<String, BTreeMap<u32, BTreeMap<u32, String>>>;

fn read_vpl(text: &str, out: &mut Verses) {
    for line in text.lines() {
        let mut parts = line.splitn(3, ' ');
        let (Some(code), Some(cv)) = (parts.next(), parts.next()) else {
            continue;
        };
        let code = code.trim_start_matches('\u{feff}');
        let key = VPL_CODES
            .iter()
            .find(|(v, _)| *v == code)
            .map_or(code, |(_, u)| *u);
        if books::by_key(key).is_none() {
            continue;
        }
        let Some((c, v)) = chapter_verse(cv) else {
            continue;
        };
        let t = clean(parts.next().unwrap_or_default());
        out.entry(key.to_string())
            .or_default()
            .entry(c)
            .or_default()
            .insert(v, t);
    }
}

fn read_tsv(text: &str, out: &mut Verses) -> Result<(), String> {
    let names: Vec<(String, String)> = CANON
        .iter()
        .map(|b| (b.key.to_string(), b.name.to_string()))
        .collect();
    for line in text.lines() {
        let Some((reference, t)) = line.split_once('\t') else {
            continue;
        };
        let Some((name, cv)) = reference.trim().rsplit_once(' ') else {
            continue;
        };
        let Some((c, v)) = chapter_verse(cv) else {
            continue;
        };
        let key = books::find(name, &names)
            .map_err(|_| format!("unknown book {name:?}"))?
            .0
            .clone();
        out.entry(key)
            .or_default()
            .entry(c)
            .or_default()
            .insert(v, clean(t));
    }
    Ok(())
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("vesper-scripture-convert: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let need = |n: &str| arg(&args, n).ok_or_else(|| format!("{n} is required"));
    let kind = need("--kind")?;
    let input = PathBuf::from(need("--input")?);
    let out_dir = PathBuf::from(need("--out")?);
    let id = need("--id")?;
    let text = std::fs::read_to_string(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    let mut verses = Verses::new();
    match kind.as_str() {
        "vpl" => read_vpl(&text, &mut verses),
        "tsv" => read_tsv(&text, &mut verses)?,
        k => return Err(format!("unknown --kind {k:?} (vpl or tsv)")),
    }
    let mut header = Header {
        format: format::FORMAT.into(),
        id: id.clone(),
        abbreviation: arg(&args, "--abbreviation").unwrap_or_else(|| id.clone()),
        name: need("--name")?,
        language: arg(&args, "--language").unwrap_or_else(|| "en".into()),
        license: need("--license")?,
        attribution: arg(&args, "--attribution").filter(|a| !a.is_empty()),
        source: arg(&args, "--source"),
        books: Vec::new(),
    };
    let mut lines = Vec::new();
    for b in CANON {
        let Some(chapters) = verses.get(b.key) else {
            continue;
        };
        let last_chapter = chapters.keys().max().copied().unwrap_or(0);
        let mut counts = Vec::new();
        for c in 1..=last_chapter {
            let ch = chapters.get(&c);
            let n = ch.and_then(|v| v.keys().max().copied()).unwrap_or(0);
            for v in 1..=n {
                lines.push(ch.and_then(|m| m.get(&v)).cloned().unwrap_or_default());
            }
            counts.push(n);
        }
        header.books.push(HeaderBook {
            key: b.key.into(),
            name: b.name.into(),
            chapters: counts,
        });
    }
    if header.books.is_empty() {
        return Err(format!("no verses found in {}", input.display()));
    }
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let path = out_dir.join(format!("{id}.{}", format::EXTENSION));
    let mut buf = Vec::new();
    format::write(&mut buf, &header, &lines).map_err(|e| e.to_string())?;
    std::fs::write(&path, buf).map_err(|e| format!("{}: {e}", path.display()))?;
    let empty = lines.iter().filter(|l| l.is_empty()).count();
    println!(
        "{}: {} books, {} verses ({empty} empty)",
        path.display(),
        header.books.len(),
        lines.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Made-up text in the sources' layouts.
    #[test]
    fn reads_vpl_and_tsv() {
        let mut v = Verses::new();
        read_vpl(
            "GEN 1:1 \u{b6} The [quiet] lamp  was lit.\nJOH 1:2 A second line.\nTOB 1:1 Skipped.\n",
            &mut v,
        );
        assert_eq!(v["GEN"][&1][&1], "The quiet lamp was lit.");
        assert_eq!(v["JHN"][&1][&2], "A second line.");
        assert!(!v.contains_key("TOB"));
        let mut v = Verses::new();
        read_tsv(
            "Some notice\t\r\nVerse\tTitle\r\nPsalm 3:4\tA made-up verse.\r\nSong of Solomon 1:1\tAnother.\r\n",
            &mut v,
        )
        .unwrap();
        assert_eq!(v["PSA"][&3][&4], "A made-up verse.");
        assert_eq!(v["SNG"][&1][&1], "Another.");
    }
}
