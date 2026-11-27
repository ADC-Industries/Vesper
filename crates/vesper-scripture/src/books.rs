//! The canonical books: USFM keys, English names and the abbreviations people type.
//!
//! Book names are matched after [`normalize`]: lower case, no spaces or full stops, a leading
//! "First" / "I" / "1st" turned into `1` (and so on). A name matches a book when it is one of
//! the book's aliases, or a prefix (at least two letters past any number) of the book's full
//! name that no other book shares.

use protocol::Testament;

/// A canonical book.
#[derive(Clone, Copy, Debug)]
pub struct CanonBook {
    /// USFM key (`JHN`).
    pub key: &'static str,
    /// English name ("John").
    pub name: &'static str,
    /// The name of one chapter of it ("Psalm" for "Psalms"), when it differs.
    pub singular: Option<&'static str>,
    /// Old or New Testament.
    pub testament: Testament,
    /// Abbreviations, normalised (no spaces, lower case).
    pub aliases: &'static [&'static str],
}

macro_rules! books {
    ($($key:literal, $name:literal, $t:ident, [$($a:literal),*]);* $(;)?) => {
        &[$(CanonBook { key: $key, name: $name, singular: None, testament: Testament::$t, aliases: &[$($a),*] }),*]
    };
}

/// The 66 books of the Protestant canon in canonical order.
pub static CANON: &[CanonBook] = books![
    "GEN", "Genesis", Old, ["gen", "ge", "gn"];
    "EXO", "Exodus", Old, ["ex", "exo", "exod"];
    "LEV", "Leviticus", Old, ["lev", "le", "lv"];
    "NUM", "Numbers", Old, ["num", "nu", "nm", "nb"];
    "DEU", "Deuteronomy", Old, ["deut", "deu", "dt", "de"];
    "JOS", "Joshua", Old, ["josh", "jos", "jsh"];
    "JDG", "Judges", Old, ["judg", "jdg", "jg", "jdgs"];
    "RUT", "Ruth", Old, ["ru", "rth", "rut"];
    "1SA", "1 Samuel", Old, ["1sam", "1sa", "1sm", "1s"];
    "2SA", "2 Samuel", Old, ["2sam", "2sa", "2sm", "2s"];
    "1KI", "1 Kings", Old, ["1kgs", "1ki", "1kg", "1k"];
    "2KI", "2 Kings", Old, ["2kgs", "2ki", "2kg", "2k"];
    "1CH", "1 Chronicles", Old, ["1chr", "1ch", "1chron"];
    "2CH", "2 Chronicles", Old, ["2chr", "2ch", "2chron"];
    "EZR", "Ezra", Old, ["ezr", "ezra"];
    "NEH", "Nehemiah", Old, ["neh", "ne"];
    "EST", "Esther", Old, ["esth", "est", "es"];
    "JOB", "Job", Old, ["job", "jb"];
    "PSA", "Psalms", Old, ["ps", "psa", "psalm", "pss", "psm", "pslm"];
    "PRO", "Proverbs", Old, ["prov", "pro", "prv", "pr"];
    "ECC", "Ecclesiastes", Old, ["eccl", "ecc", "ec", "eccles", "qoh"];
    "SNG", "Song of Songs", Old, ["song", "sos", "so", "sng", "songofsolomon", "canticles", "cant"];
    "ISA", "Isaiah", Old, ["isa", "is"];
    "JER", "Jeremiah", Old, ["jer", "je", "jr"];
    "LAM", "Lamentations", Old, ["lam", "la"];
    "EZK", "Ezekiel", Old, ["ezek", "eze", "ezk"];
    "DAN", "Daniel", Old, ["dan", "da", "dn"];
    "HOS", "Hosea", Old, ["hos", "ho"];
    "JOL", "Joel", Old, ["joel", "jl", "jol"];
    "AMO", "Amos", Old, ["am", "amo"];
    "OBA", "Obadiah", Old, ["obad", "ob", "oba"];
    "JON", "Jonah", Old, ["jonah", "jnh", "jon"];
    "MIC", "Micah", Old, ["mic", "mc"];
    "NAM", "Nahum", Old, ["nah", "na", "nam"];
    "HAB", "Habakkuk", Old, ["hab", "hb"];
    "ZEP", "Zephaniah", Old, ["zeph", "zep", "zp"];
    "HAG", "Haggai", Old, ["hag", "hg"];
    "ZEC", "Zechariah", Old, ["zech", "zec", "zc"];
    "MAL", "Malachi", Old, ["mal", "ml"];
    "MAT", "Matthew", New, ["matt", "mt", "mat"];
    "MRK", "Mark", New, ["mk", "mrk", "mar", "mr"];
    "LUK", "Luke", New, ["lk", "luk", "lu"];
    "JHN", "John", New, ["jn", "jhn", "joh"];
    "ACT", "Acts", New, ["ac", "act"];
    "ROM", "Romans", New, ["rom", "ro", "rm"];
    "1CO", "1 Corinthians", New, ["1cor", "1co"];
    "2CO", "2 Corinthians", New, ["2cor", "2co"];
    "GAL", "Galatians", New, ["gal", "ga"];
    "EPH", "Ephesians", New, ["eph", "ephes"];
    "PHP", "Philippians", New, ["phil", "php", "pp"];
    "COL", "Colossians", New, ["col", "co"];
    "1TH", "1 Thessalonians", New, ["1thess", "1th", "1thes"];
    "2TH", "2 Thessalonians", New, ["2thess", "2th", "2thes"];
    "1TI", "1 Timothy", New, ["1tim", "1ti", "1tm"];
    "2TI", "2 Timothy", New, ["2tim", "2ti", "2tm"];
    "TIT", "Titus", New, ["tit", "ti"];
    "PHM", "Philemon", New, ["philem", "phm", "phlm"];
    "HEB", "Hebrews", New, ["heb"];
    "JAS", "James", New, ["jas", "jm", "jam"];
    "1PE", "1 Peter", New, ["1pet", "1pe", "1pt", "1p"];
    "2PE", "2 Peter", New, ["2pet", "2pe", "2pt", "2p"];
    "1JN", "1 John", New, ["1jn", "1jhn", "1jo", "1joh"];
    "2JN", "2 John", New, ["2jn", "2jhn", "2jo", "2joh"];
    "3JN", "3 John", New, ["3jn", "3jhn", "3jo", "3joh"];
    "JUD", "Jude", New, ["jud", "jude", "jd"];
    "REV", "Revelation", New, ["rev", "re", "rv", "revelations", "apocalypse"];
];

/// The canonical book with this USFM key.
pub fn by_key(key: &str) -> Option<&'static CanonBook> {
    CANON.iter().find(|b| b.key.eq_ignore_ascii_case(key))
}

/// The canonical index of a key (Genesis = 1).
pub fn index_of(key: &str) -> Option<u32> {
    CANON
        .iter()
        .position(|b| b.key.eq_ignore_ascii_case(key))
        .map(|i| i as u32 + 1)
}

/// The testament of a key (`other` for books outside the table).
pub fn testament(key: &str) -> Testament {
    by_key(key).map_or(Testament::Other, |b| b.testament)
}

/// The name of one chapter of a book: "Psalm" for the Psalms, else the book's name.
pub fn chapter_name<'a>(key: &str, name: &'a str) -> &'a str {
    if key.eq_ignore_ascii_case("PSA") && name == "Psalms" {
        return "Psalm";
    }
    // The Psalms named in another language ("Salmos" → "Salmo").
    crate::names::LANGUAGES
        .iter()
        .find_map(|l| crate::names::psalm(l, key, name))
        .unwrap_or(name)
}

/// A letter without its accent, for matching typed names ("é" → "e"); `None` keeps it.
fn fold(c: char) -> Option<&'static str> {
    Some(match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => "a",
        'é' | 'è' | 'ê' | 'ë' => "e",
        'í' | 'ì' | 'î' | 'ï' => "i",
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => "o",
        'ú' | 'ù' | 'û' | 'ü' => "u",
        'ç' => "c",
        'ñ' => "n",
        'ß' => "ss",
        _ => return None,
    })
}

/// Normalises a typed book name: lower case, without spaces, full stops, apostrophes and
/// accents, a leading ordinal ("First", "I", "1st", "II") as a digit.
pub fn normalize(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    let mut words: Vec<&str> = lower
        .split(|c: char| c.is_whitespace() || c == '.')
        .filter(|w| !w.is_empty())
        .collect();
    let mut prefix = "";
    if let Some(first) = words.first() {
        let n = match *first {
            "1" | "i" | "1st" | "first" => "1",
            "2" | "ii" | "2nd" | "second" => "2",
            "3" | "iii" | "3rd" | "third" => "3",
            _ => "",
        };
        // "I" alone (a Roman numeral) only when a book name follows it.
        if !n.is_empty() && words.len() > 1 {
            prefix = n;
            words.remove(0);
        }
    }
    let mut out = prefix.to_string();
    for w in words {
        for c in w.chars().filter(|c| *c != '\'' && *c != '\u{2019}') {
            match fold(c) {
                Some(f) => out.push_str(f),
                None => out.push(c),
            }
        }
    }
    // "1john" written with the digit attached: already fine.
    out
}

/// Finds a book among `names` (key, name pairs of the translation, canonical names used for
/// keys the table knows). `Err` lists the candidates when the name is ambiguous.
pub fn find<'a>(
    typed: &str,
    books: &'a [(String, String)],
) -> Result<&'a (String, String), Vec<String>> {
    let n = normalize(typed);
    if n.is_empty() {
        return Err(Vec::new());
    }
    // Exact: the translation's own name, the canonical name, an alias or the key.
    for b in books {
        let canon = by_key(&b.0);
        if normalize(&b.1) == n
            || b.0.to_lowercase() == n
            || canon.is_some_and(|c| normalize(c.name) == n || c.aliases.contains(&n.as_str()))
        {
            return Ok(b);
        }
    }
    // A unique prefix of a name (at least two letters past the number).
    let letters = n.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if letters < 2 {
        return Err(Vec::new());
    }
    let mut hits: Vec<&(String, String)> = books
        .iter()
        .filter(|b| {
            normalize(&b.1).starts_with(&n)
                || by_key(&b.0).is_some_and(|c| normalize(c.name).starts_with(&n))
        })
        .collect();
    // One book named twice (the translation's name and another language's) is one hit.
    let mut keys: Vec<&str> = Vec::new();
    hits.retain(|b| {
        let new = !keys.contains(&b.0.as_str());
        keys.push(&b.0);
        new
    });
    match hits.as_slice() {
        [one] => Ok(one),
        _ => Err(hits.iter().map(|b| b.1.clone()).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<(String, String)> {
        CANON
            .iter()
            .map(|b| (b.key.to_string(), b.name.to_string()))
            .collect()
    }

    #[test]
    fn the_table_is_whole_and_aliases_are_unique() {
        assert_eq!(CANON.len(), 66);
        let mut seen = std::collections::HashSet::new();
        for b in CANON {
            for a in b.aliases {
                assert!(seen.insert(*a), "alias {a} used twice");
                assert_eq!(normalize(a), *a, "alias {a} isn't normalised");
            }
        }
    }

    #[test]
    fn typed_names_find_their_book() {
        let books = all();
        for (typed, key) in [
            ("Jn", "JHN"),
            ("john", "JHN"),
            ("1 Cor", "1CO"),
            ("1Cor.", "1CO"),
            ("First Corinthians", "1CO"),
            ("I Corinthians", "1CO"),
            ("II Kings", "2KI"),
            ("Psalm", "PSA"),
            ("Psalms", "PSA"),
            ("Ps", "PSA"),
            ("song of solomon", "SNG"),
            ("Song of Songs", "SNG"),
            ("Gen", "GEN"),
            ("Genes", "GEN"),
            ("Rev", "REV"),
            ("Philem", "PHM"),
            ("Phil", "PHP"),
            ("3 jn", "3JN"),
            ("Obad", "OBA"),
        ] {
            assert_eq!(
                find(typed, &books).map(|b| b.0.as_str()),
                Ok(key),
                "{typed}"
            );
        }
    }

    #[test]
    fn ambiguous_names_list_the_candidates() {
        let books = all();
        let e = find("Jo", &books).unwrap_err();
        assert!(e.contains(&"Joel".to_string()) && e.contains(&"Jonah".to_string()));
        assert!(find("x", &books).is_err());
    }
}
