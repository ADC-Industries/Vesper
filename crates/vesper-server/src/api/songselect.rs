//! SongSelect files and pasted SongSelect lyrics (see `model::songselect` and
//! `crates/protocol/README.md` "SongSelect"): read into a new presentation with its song
//! metadata, through the ChordPro import's builder (same section naming, global groups and
//! slide splitting). Vesper never contacts CCLI: the user brings the files.

use super::*;
use base64::Engine as _;
use model::songselect::{self, Format};
use protocol::{SongSelectDuplicate, SongSelectFormat, SongSelectParse, SongSelectParsed};

pub(super) fn routes() -> Router<AppState> {
    Router::new().route(
        "/songselect/parse",
        // A 2 MiB file as base64, and the JSON around it.
        post(parse_songselect).layer(axum::extract::DefaultBodyLimit::max(
            songselect::MAX_FILE_BYTES * 4 / 3 + 64 * 1024,
        )),
    )
}

/// A CCLI song number as digits only (`"1234567"`, `" 1234567 "` → the same song).
fn song_number(n: &str) -> String {
    n.chars().filter(char::is_ascii_digit).collect()
}

async fn parse_songselect(State(app): State<AppState>, body: Bytes) -> ApiResult<SongSelectParsed> {
    let b: SongSelectParse = parse_body(&body)?;
    let text = match (&b.data, &b.text) {
        (Some(data), _) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data.trim())
                .map_err(|_| err(ErrorCode::BadRequest, "data is not base64"))?;
            if bytes.len() > songselect::MAX_FILE_BYTES {
                return Err(err(ErrorCode::BadRequest, "the file is too large"));
            }
            songselect::decode(&bytes)
        }
        (None, Some(text)) => text.clone(),
        (None, None) => return Err(err(ErrorCode::BadRequest, "give data or text")),
    };
    if text.len() > crate::edit::MAX_CHORDPRO_TEXT {
        return Err(err(ErrorCode::BadRequest, "the text is too long"));
    }
    let format = songselect::sniff(&text, b.file_name.as_deref());
    let song = songselect::parse(&text, format);
    if song.sections.is_empty() {
        return Err(err(ErrorCode::BadRequest, "no lyrics found in the song"));
    }
    let d = app.shared.data();
    let like = match b.like {
        Some(id) => Some(d.content.presentation(id).ok_or_else(|| {
            err(
                ErrorCode::NotFound,
                format!("presentation {} not found", id.0),
            )
        })?),
        None => None,
    };
    let lines = b.lines_per_slide.unwrap_or(0).min(64) as usize;
    let p = crate::edit::presentation_from_chordpro(
        &song,
        b.name.as_deref(),
        lines,
        like,
        &d.content.global_groups,
    );
    if let Some(license) = song.license.clone() {
        // Offered in Settings ▸ CCLI while the church's number isn't set.
        let sh = app.shared.clone();
        tokio::task::spawn_blocking(move || sh.ccli.note_imported_license(&license));
    }
    let number = song.ccli.as_deref().map(song_number).unwrap_or_default();
    let duplicates = if number.is_empty() {
        Vec::new()
    } else {
        d.content
            .presentations
            .iter()
            .filter(|x| {
                x.song_info
                    .as_ref()
                    .and_then(|i| i.ccli_number.as_deref())
                    .is_some_and(|n| song_number(n) == number)
            })
            .map(|x| SongSelectDuplicate {
                id: x.id,
                name: x.name.clone(),
            })
            .collect()
    };
    let chord_lines = p
        .slides()
        .flat_map(|s| s.elements.iter())
        .filter_map(|e| e.text_box())
        .flat_map(|t| t.paragraphs.iter())
        .filter(|para| para.has_chords())
        .count() as u32;
    ok(SongSelectParsed {
        format: match format {
            Format::Lyrics => SongSelectFormat::Lyrics,
            Format::Usr => SongSelectFormat::Usr,
            Format::ChordPro => SongSelectFormat::ChordPro,
        },
        sections: p.sections.len() as u32,
        slides: p.slides().count() as u32,
        chord_lines,
        license_number: song.license.clone(),
        duplicates,
        presentation: p,
    })
}
