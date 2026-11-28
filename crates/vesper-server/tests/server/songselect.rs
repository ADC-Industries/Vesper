//! SongSelect files and pasted SongSelect lyrics into a presentation with its song metadata
//! (`POST /songselect/parse`): a lyrics file in UTF-16 with both footer orders, a SongShow
//! Plus file, pasted text, duplicates by CCLI number, and refused input. Every song here is
//! made up (titles, writers, words, numbers).

use crate::common::*;
use base64::Engine as _;
use model::Presentation;
use serde_json::json;

const LYRICS: &str = "Lanterns Over Kettle Bay\r
\r
Verse 1\r
Lanterns over Kettle Bay\r
Calling every sleeper home\r
\r
Chorus\r
Rise and sing the harbour song\r
Rise and sing it all night long\r
\r
Verse 2\r
Morning comes on silver wings\r
\r
Alex Example | Jamie Sample\r
CCLI Song # 1234567\r
© 2019 Example Songs Publishing | Sample Music Co.\r
For use solely with the SongSelect® Terms of Use. All rights reserved. www.ccli.com\r
CCLI License # 7654321\r
";

const USR: &str = "[File]
Type=SongSelect Import File
Version=3.0
[S A7000001]
Title=Paper Boats
Author=Riley Placeholder | Casey Invented
Copyright=2004 Imaginary Hymns
Keys=D
Fields=Verse 1/tChorus
Words=Paper boats upon the stream/nFloating far away/tSail on, sail on
";

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn songselect_files_and_pasted_lyrics() {
    let fx = fixture("songselect");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let parse =
        |body: serde_json::Value| http(addr, "POST", "/api/v1/songselect/parse", &[], Some(body));

    // A lyrics file in UTF-16 LE with a byte-order mark, CRLF line ends.
    let utf16: Vec<u8> = [0xFF, 0xFE]
        .into_iter()
        .chain(LYRICS.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let r =
        parse(json!({"data": b64(&utf16), "fileName": "lanterns.txt", "like": fx.song.id})).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let parsed = r.json();
    assert_eq!(parsed["format"], "lyrics");
    assert_eq!(parsed["sections"], 3);
    assert_eq!(parsed["slides"], 3);
    assert_eq!(parsed["licenseNumber"], "7654321");
    assert!(
        parsed.get("duplicates").is_none(),
        "nothing in the library has its number yet"
    );
    let p: Presentation = serde_json::from_value(parsed["presentation"].clone()).unwrap();
    assert_eq!(p.name, "Lanterns Over Kettle Bay");
    let names: Vec<&str> = p.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Verse 1", "Chorus", "Verse 2"]);
    let info = p.song_info.clone().unwrap();
    assert_eq!(info.title.as_deref(), Some("Lanterns Over Kettle Bay"));
    assert_eq!(info.ccli_number.as_deref(), Some("1234567"));
    assert_eq!(info.authors.as_deref(), Some("Alex Example, Jamie Sample"));
    assert_eq!(info.copyright_year, Some(2019));
    assert_eq!(
        info.publisher.as_deref(),
        Some("Example Songs Publishing; Sample Music Co.")
    );
    assert!(info.display, "the copyright line shows under the words");
    let tb = p.sections[1].slides[0].elements[0].text_box().unwrap();
    assert_eq!(
        tb.plain_text(),
        "Rise and sing the harbour song\nRise and sing it all night long"
    );
    // One line per slide: 2 + 2 + 1 slides.
    let r = parse(json!({"text": LYRICS, "linesPerSlide": 1})).await;
    assert_eq!(r.json()["slides"], 5);

    // Create it; reading the same song again flags the duplicate by CCLI number.
    let r = http(
        addr,
        "POST",
        "/api/v1/presentations",
        &[],
        Some(json!({"document": p, "library": fx.library.id})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    // Pasted text with the old footer order (authors after the number) is the same song.
    let old = LYRICS.replace(
        "Alex Example | Jamie Sample\r\nCCLI Song # 1234567\r\n",
        "CCLI Song # 1234567\r\nAlex Example | Jamie Sample\r\n",
    );
    let r = parse(json!({"text": old})).await;
    assert_eq!(r.status, 200);
    let parsed = r.json();
    assert_eq!(parsed["duplicates"][0]["id"], json!(p.id));
    assert_eq!(parsed["duplicates"][0]["name"], "Lanterns Over Kettle Bay");
    assert_eq!(
        parsed["presentation"]["songInfo"]["authors"],
        "Alex Example, Jamie Sample"
    );

    // A SongShow Plus file (format by content; its name ends in .bin).
    let r = parse(json!({"data": b64(USR.as_bytes()), "fileName": "7000001.bin"})).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let parsed = r.json();
    assert_eq!(parsed["format"], "usr");
    let p: Presentation = serde_json::from_value(parsed["presentation"].clone()).unwrap();
    assert_eq!(p.name, "Paper Boats");
    let info = p.song_info.unwrap();
    assert_eq!(info.ccli_number.as_deref(), Some("7000001"));
    assert_eq!(info.original_key.as_deref(), Some("D"));
    assert_eq!(
        info.authors.as_deref(),
        Some("Riley Placeholder, Casey Invented")
    );

    // ChordPro from SongSelect: chords and the footer's metadata.
    let cho = "{title: Paper Lanterns}\n{key: A}\n\n{c: Verse 1}\n[A]Paper lanterns [E]high\n\n{c: Alex Example}\n{c: CCLI Song # 1234568}\n";
    let r = parse(json!({"text": cho})).await;
    let parsed = r.json();
    assert_eq!(parsed["format"], "chordPro");
    assert_eq!(parsed["chordLines"], 1);
    assert_eq!(parsed["presentation"]["songInfo"]["ccliNumber"], "1234568");

    // Refused: no lyrics, not base64, too large, nothing given.
    for body in [
        json!({"text": "Just A Title\n\nCCLI Song # 1234567\n"}),
        json!({"data": "not base64!"}),
        json!({"data": b64(&vec![b'a'; model::songselect::MAX_FILE_BYTES + 1])}),
        json!({}),
    ] {
        let r = parse(body).await;
        assert_eq!(r.status, 400, "{}", String::from_utf8_lossy(&r.body));
    }
}
