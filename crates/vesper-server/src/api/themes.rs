//! Themes: list, read, create, rename, duplicate, delete, edit as a document, used-by, template
//! renders and thumbnails (see [`crate::themes`]).

use super::docs::{one_slide_render, slide_thumbnail};
use super::edit::{client_header, content};
use super::*;
use crate::content::DeleteTheme;
use model::{Theme, ThemeId};
use protocol::{DocumentUse, TemplateSlideSummary, ThemeEntry, ThemeSummary};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/themes", get(themes).post(create))
        .route("/themes/{id}", get(theme).put(update).delete(remove))
        .route("/themes/{id}/duplicate", post(duplicate))
        .route("/themes/{id}/document", get(document))
        .route("/themes/{id}/edits", post(edit))
        .route("/themes/{id}/used-by", get(used_by))
        .route("/themes/{id}/slides/{slide}/render", get(render))
        .route("/themes/{id}/slides/{slide}/thumbnail", get(thumbnail))
}

async fn themes(State(app): State<AppState>) -> ApiResult<Vec<ThemeSummary>> {
    let d = app.shared.data();
    let counts = crate::themes::use_counts(&d.content);
    let mut out: Vec<ThemeSummary> = d
        .content
        .themes
        .values()
        .map(|t| ThemeSummary {
            id: t.id,
            name: t.name.clone(),
            canvas_size: t.canvas_size,
            slides: t
                .slides
                .iter()
                .map(|s| TemplateSlideSummary {
                    id: s.id,
                    name: s.name.clone(),
                    slide: s.slide.id,
                })
                .collect(),
            used_by: counts.get(&t.id).copied().unwrap_or(0),
        })
        .collect();
    out.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.0.cmp(&b.id.0))
    });
    ok(out)
}

fn find(app: &AppState, id: &str) -> Result<Theme, Error> {
    let id: ThemeId = parse_id(id, "theme id")?;
    app.shared
        .data()
        .content
        .themes
        .get(&id)
        .cloned()
        .ok_or_else(|| err(ErrorCode::NotFound, format!("theme {} not found", id.0)))
}

async fn theme(State(app): State<AppState>, Path(id): Path<String>) -> Result<Response, Error> {
    let t = find(&app, &id)?;
    let revision = crate::config::revision(&t);
    Ok(with_etag(
        StatusCode::OK,
        &revision.clone(),
        ThemeEntry { theme: t, revision },
    ))
}

async fn create(State(app): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let body = if body.is_empty() {
        Bytes::from_static(b"{}")
    } else {
        body
    };
    match parse_body::<protocol::CreateTheme>(&body) {
        Ok(b) => {
            let canvas = app.shared.data().settings.default_canvas_size;
            content(
                &app,
                ContentOp::CreateTheme(Box::new(b), canvas),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

async fn update(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = (|| Ok::<_, Error>((parse_id::<ThemeId>(&id, "theme id")?, parse_body(&body)?)))();
    match r {
        Ok((id, b)) => {
            content(
                &app,
                ContentOp::UpdateTheme(id, b, if_match(&headers)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

async fn duplicate(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let body = if body.is_empty() {
        Bytes::from_static(b"{}")
    } else {
        body
    };
    let r = (|| Ok::<_, Error>((parse_id::<ThemeId>(&id, "theme id")?, parse_body(&body)?)))();
    match r {
        Ok((id, b)) => {
            content(
                &app,
                ContentOp::DuplicateTheme(id, b),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

/// `DELETE /themes/{id}?reassign=<theme>` or `?unlink=true`.
async fn remove(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Request,
) -> Response {
    let q = req.uri().query().map(str::to_owned);
    let r = (|| {
        let id = parse_id::<ThemeId>(&id, "theme id")?;
        let reassign = match query_param(q.as_deref(), "reassign").filter(|s| !s.is_empty()) {
            Some(r) => Some(parse_id::<ThemeId>(r, "reassign")?),
            None => None,
        };
        let unlink = matches!(query_param(q.as_deref(), "unlink"), Some("true" | "1"));
        Ok::<_, Error>((id, DeleteTheme { reassign, unlink }))
    })();
    match r {
        Ok((id, opts)) => {
            content(
                &app,
                ContentOp::DeleteTheme(id, opts, if_match(&headers)),
                client_header(&headers),
            )
            .await
        }
        Err(e) => e.into_response(),
    }
}

/// `GET /themes/{id}/document`: the theme as a presentation (the Edit view's document).
async fn document(State(app): State<AppState>, Path(id): Path<String>) -> Result<Response, Error> {
    let t = find(&app, &id)?;
    let rev = crate::config::revision(&t);
    Ok(with_etag(
        StatusCode::OK,
        &rev,
        crate::themes::theme_document(&t),
    ))
}

async fn edit(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = (|| {
        Ok::<_, Error>((
            parse_id::<ThemeId>(&id, "theme id")?,
            parse_body::<protocol::PresentationEdit>(&body)?,
        ))
    })();
    match r {
        Ok((id, edit)) => {
            let client = edit.client_id.clone().or(client_header(&headers));
            let r = content(&app, ContentOp::EditTheme(id, edit), client).await;
            if r.status().is_success() {
                // Passages made with the theme are split again (batch 9).
                super::scripture::after_theme_change(&app, id);
            }
            r
        }
        Err(e) => e.into_response(),
    }
}

/// Presentations that follow the theme, then messages that use its template slides.
/// `?template=<template slide id>`: only those that use that template slide.
async fn used_by(
    State(app): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> ApiResult<Vec<DocumentUse>> {
    let t = find(&app, &id)?;
    let template = match query_param(req.uri().query(), "template").filter(|s| !s.is_empty()) {
        Some(s) => Some(parse_id::<model::TemplateSlideId>(s, "template")?),
        None => None,
    };
    let d = app.shared.data();
    let mut out = crate::themes::users_of(&d.content, t.id, template);
    let mut messages: Vec<DocumentUse> = d
        .content
        .messages
        .iter()
        .filter(|m| {
            m.template
                .as_ref()
                .is_some_and(|r| r.theme.id == t.id && template.is_none_or(|x| r.slide == x))
        })
        .map(|m| DocumentUse {
            kind: model::DocumentKind::Message,
            id: m.id.0.to_string(),
            name: m.name.clone(),
        })
        .collect();
    messages.sort_by_key(|a| a.name.to_lowercase());
    out.extend(messages);
    ok(out)
}

fn template_slide(t: &Theme, slide: &str) -> Result<model::Slide, Error> {
    let sid: SlideId = parse_id(slide, "slide id")?;
    t.slides
        .iter()
        .find(|s| s.slide.id == sid)
        .map(|s| s.slide.clone())
        .ok_or_else(|| err(ErrorCode::NotFound, "slide not in theme"))
}

/// `GET /themes/{id}/slides/{slide}/render?w=960&hide=…`: the Edit view's picture.
async fn render(
    State(app): State<AppState>,
    Path((id, slide)): Path<(String, String)>,
    req: Request,
) -> Result<Response, Error> {
    let t = find(&app, &id)?;
    let s = template_slide(&t, &slide)?;
    one_slide_render(&app, s, t.canvas_size, req.uri().query().map(str::to_owned)).await
}

/// `GET /themes/{id}/slides/{slide}/thumbnail?w=320`: a template slide as a PNG (`ETag`).
async fn thumbnail(
    State(app): State<AppState>,
    Path((id, slide)): Path<(String, String)>,
    headers: HeaderMap,
    req: Request,
) -> Result<Response, Error> {
    let t = find(&app, &id)?;
    let s = template_slide(&t, &slide)?;
    // The picture's shape comes from the theme's canvas, which the slide doesn't hold.
    let rev = crate::config::revision(&(&s, t.canvas_size));
    slide_thumbnail(
        &app,
        s,
        t.canvas_size,
        &rev,
        &headers,
        req.uri().query().map(str::to_owned),
    )
    .await
}
