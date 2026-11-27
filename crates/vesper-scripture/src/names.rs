//! Book names in other languages than English, for parsing references typed in the
//! translation's language and for showing its books in that language when its file names them
//! in English (or not at all). One table per language (BCP 47 primary subtag), the 66 books in
//! canonical order ([`crate::books::CANON`]).

/// Languages with a table.
pub const LANGUAGES: &[&str] = &["es", "pt", "fr", "de"];

const ES: [&str; 66] = [
    "Génesis",
    "Éxodo",
    "Levítico",
    "Números",
    "Deuteronomio",
    "Josué",
    "Jueces",
    "Rut",
    "1 Samuel",
    "2 Samuel",
    "1 Reyes",
    "2 Reyes",
    "1 Crónicas",
    "2 Crónicas",
    "Esdras",
    "Nehemías",
    "Ester",
    "Job",
    "Salmos",
    "Proverbios",
    "Eclesiastés",
    "Cantares",
    "Isaías",
    "Jeremías",
    "Lamentaciones",
    "Ezequiel",
    "Daniel",
    "Oseas",
    "Joel",
    "Amós",
    "Abdías",
    "Jonás",
    "Miqueas",
    "Nahúm",
    "Habacuc",
    "Sofonías",
    "Hageo",
    "Zacarías",
    "Malaquías",
    "Mateo",
    "Marcos",
    "Lucas",
    "Juan",
    "Hechos",
    "Romanos",
    "1 Corintios",
    "2 Corintios",
    "Gálatas",
    "Efesios",
    "Filipenses",
    "Colosenses",
    "1 Tesalonicenses",
    "2 Tesalonicenses",
    "1 Timoteo",
    "2 Timoteo",
    "Tito",
    "Filemón",
    "Hebreos",
    "Santiago",
    "1 Pedro",
    "2 Pedro",
    "1 Juan",
    "2 Juan",
    "3 Juan",
    "Judas",
    "Apocalipsis",
];

const PT: [&str; 66] = [
    "Gênesis",
    "Êxodo",
    "Levítico",
    "Números",
    "Deuteronômio",
    "Josué",
    "Juízes",
    "Rute",
    "1 Samuel",
    "2 Samuel",
    "1 Reis",
    "2 Reis",
    "1 Crônicas",
    "2 Crônicas",
    "Esdras",
    "Neemias",
    "Ester",
    "Jó",
    "Salmos",
    "Provérbios",
    "Eclesiastes",
    "Cânticos",
    "Isaías",
    "Jeremias",
    "Lamentações",
    "Ezequiel",
    "Daniel",
    "Oseias",
    "Joel",
    "Amós",
    "Obadias",
    "Jonas",
    "Miqueias",
    "Naum",
    "Habacuque",
    "Sofonias",
    "Ageu",
    "Zacarias",
    "Malaquias",
    "Mateus",
    "Marcos",
    "Lucas",
    "João",
    "Atos",
    "Romanos",
    "1 Coríntios",
    "2 Coríntios",
    "Gálatas",
    "Efésios",
    "Filipenses",
    "Colossenses",
    "1 Tessalonicenses",
    "2 Tessalonicenses",
    "1 Timóteo",
    "2 Timóteo",
    "Tito",
    "Filemom",
    "Hebreus",
    "Tiago",
    "1 Pedro",
    "2 Pedro",
    "1 João",
    "2 João",
    "3 João",
    "Judas",
    "Apocalipse",
];

const FR: [&str; 66] = [
    "Genèse",
    "Exode",
    "Lévitique",
    "Nombres",
    "Deutéronome",
    "Josué",
    "Juges",
    "Ruth",
    "1 Samuel",
    "2 Samuel",
    "1 Rois",
    "2 Rois",
    "1 Chroniques",
    "2 Chroniques",
    "Esdras",
    "Néhémie",
    "Esther",
    "Job",
    "Psaumes",
    "Proverbes",
    "Ecclésiaste",
    "Cantique des Cantiques",
    "Ésaïe",
    "Jérémie",
    "Lamentations",
    "Ézéchiel",
    "Daniel",
    "Osée",
    "Joël",
    "Amos",
    "Abdias",
    "Jonas",
    "Michée",
    "Nahum",
    "Habacuc",
    "Sophonie",
    "Aggée",
    "Zacharie",
    "Malachie",
    "Matthieu",
    "Marc",
    "Luc",
    "Jean",
    "Actes",
    "Romains",
    "1 Corinthiens",
    "2 Corinthiens",
    "Galates",
    "Éphésiens",
    "Philippiens",
    "Colossiens",
    "1 Thessaloniciens",
    "2 Thessaloniciens",
    "1 Timothée",
    "2 Timothée",
    "Tite",
    "Philémon",
    "Hébreux",
    "Jacques",
    "1 Pierre",
    "2 Pierre",
    "1 Jean",
    "2 Jean",
    "3 Jean",
    "Jude",
    "Apocalypse",
];

const DE: [&str; 66] = [
    "1. Mose",
    "2. Mose",
    "3. Mose",
    "4. Mose",
    "5. Mose",
    "Josua",
    "Richter",
    "Rut",
    "1. Samuel",
    "2. Samuel",
    "1. Könige",
    "2. Könige",
    "1. Chronik",
    "2. Chronik",
    "Esra",
    "Nehemia",
    "Ester",
    "Hiob",
    "Psalmen",
    "Sprüche",
    "Prediger",
    "Hoheslied",
    "Jesaja",
    "Jeremia",
    "Klagelieder",
    "Hesekiel",
    "Daniel",
    "Hosea",
    "Joel",
    "Amos",
    "Obadja",
    "Jona",
    "Micha",
    "Nahum",
    "Habakuk",
    "Zefanja",
    "Haggai",
    "Sacharja",
    "Maleachi",
    "Matthäus",
    "Markus",
    "Lukas",
    "Johannes",
    "Apostelgeschichte",
    "Römer",
    "1. Korinther",
    "2. Korinther",
    "Galater",
    "Epheser",
    "Philipper",
    "Kolosser",
    "1. Thessalonicher",
    "2. Thessalonicher",
    "1. Timotheus",
    "2. Timotheus",
    "Titus",
    "Philemon",
    "Hebräer",
    "Jakobus",
    "1. Petrus",
    "2. Petrus",
    "1. Johannes",
    "2. Johannes",
    "3. Johannes",
    "Judas",
    "Offenbarung",
];

/// The name of one psalm in each language ("Salmo 23").
const PSALM: &[(&str, &str)] = &[
    ("es", "Salmo"),
    ("pt", "Salmo"),
    ("fr", "Psaume"),
    ("de", "Psalm"),
];

/// The primary subtag of a BCP 47 language tag, lower case (`es-419` → `es`).
pub fn primary(language: &str) -> String {
    language
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn table(language: &str) -> Option<&'static [&'static str; 66]> {
    match primary(language).as_str() {
        "es" => Some(&ES),
        "pt" => Some(&PT),
        "fr" => Some(&FR),
        "de" => Some(&DE),
        _ => None,
    }
}

/// The book with USFM key `key` in `language`, when the language has a table.
pub fn name(language: &str, key: &str) -> Option<&'static str> {
    let i = crate::books::index_of(key)? as usize - 1;
    table(language).map(|t| t[i])
}

/// The name of one chapter of the Psalms in `language` when `name` is that language's name
/// of the book.
pub fn psalm(language: &str, key: &str, name: &str) -> Option<&'static str> {
    let lang = primary(language);
    (key.eq_ignore_ascii_case("PSA") && self::name(&lang, key) == Some(name))
        .then(|| PSALM.iter().find(|(l, _)| *l == lang).map(|(_, p)| *p))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::books::{CANON, normalize};

    #[test]
    fn every_table_names_every_book_once() {
        for lang in LANGUAGES {
            let t = table(lang).unwrap();
            let mut seen = std::collections::HashSet::new();
            for n in t {
                assert!(seen.insert(normalize(n)), "{lang}: {n} twice");
            }
        }
        assert_eq!(name("es-419", "JHN"), Some("Juan"));
        assert_eq!(name("de", "GEN"), Some("1. Mose"));
        assert_eq!(name("en", "GEN"), None);
        assert_eq!(name("fr", "XYZ"), None);
        assert_eq!(CANON.len(), ES.len());
        assert_eq!(psalm("pt-BR", "PSA", "Salmos"), Some("Salmo"));
        assert_eq!(psalm("pt", "PSA", "Psalms"), None);
    }
}
