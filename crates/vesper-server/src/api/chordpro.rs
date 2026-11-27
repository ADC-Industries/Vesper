//! ChordPro (batch 16, see `model::chordpro` and `crates/protocol/README.md` "Chords"):
//! reading a song into a new presentation (one or two languages), and writing a presentation
//! as ChordPro (its second-language texts too). Putting a
//! ChordPro file's chords on an existing song is the `applyChordPro` edit.

use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/chordpro/parse", post(parse_chordpro))
        .route("/presentations/{id}/chordpro", get(export_chordpro))
}

/// Largest ChordPro text accepted (a long song is a few KB).
const MAX_TEXT: usize = crate::edit::MAX_CHORDPRO_TEXT;

async fn parse_chordpro(
    State(app): State<AppState>,
    body: Bytes,
) -> ApiResult<protocol::ChordProParsed> {
    let b: protocol::ChordProParse = parse_body(&body)?;
    if b.text.len() > MAX_TEXT {
        return Err(err(ErrorCode::BadRequest, "the ChordPro text is too long"));
    }
    let song = model::chordpro::parse_two(&b.text, &b.split.clone().unwrap_or_default());
    if song.sections.is_empty() {
        return Err(err(
            ErrorCode::BadRequest,
            "the ChordPro text has no lyric lines",
        ));
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
    let chord_lines = p
        .slides()
        .flat_map(|s| s.elements.iter())
        .filter_map(|e| e.text_box())
        .flat_map(|t| t.paragraphs.iter())
        .filter(|para| para.has_chords())
        .count() as u32;
    let second_language_slides = p
        .slides()
        .filter(|s| s.translation.as_ref().is_some_and(|t| !t.texts.is_empty()))
        .count() as u32;
    ok(protocol::ChordProParsed {
        sections: p.sections.len() as u32,
        slides: p.slides().count() as u32,
        chord_lines,
        second_language_slides,
        unpaired_stanzas: song.unpaired_stanzas(lines) as u32,
        capo: song.capo,
        presentation: p,
    })
}

async fn export_chordpro(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    let id: PresentationId = parse_id(&id, "presentation id")?;
    let d = app.shared.data();
    let p = d.content.presentation(id).ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("presentation {} not found", id.0),
        )
    })?;
    let text = model::chordpro::export(p, |s| {
        let Some(i) = crate::edit::primary_text(s) else {
            return (Vec::new(), Vec::new());
        };
        let el = &s.elements[i];
        let paras = el
            .text_box()
            .map(|t| t.paragraphs.clone())
            .unwrap_or_default();
        // The primary text's second text, a line per paragraph.
        let second = s
            .translation
            .as_ref()
            .and_then(|t| t.text_of(el.id))
            .map(|t| t.paragraphs.iter().map(|p| p.plain_text()).collect())
            .unwrap_or_default();
        (paras, second)
    });
    let file: String = p
        .name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let file = if file.trim().is_empty() {
        "song".to_string()
    } else {
        file.trim().to_string()
    };
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!("attachment; filename=\"{file}.cho\""))
                    .unwrap_or(HeaderValue::from_static("attachment")),
            ),
        ],
        text,
    )
        .into_response())
}
