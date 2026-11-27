//! Stage layouts and their templates.

use super::docs::{
    create_doc, delete_doc, one_slide_render, run_docs, slide_thumbnail, update_doc,
};
use super::edit::client_header;
use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/stage-layouts",
            get(stage_layouts_list).post(
                |State(app): State<AppState>, headers: HeaderMap, body: Bytes| async move {
                    create_doc(app, headers, body, DocKind::StageLayout).await
                },
            ),
        )
        .route(
            "/stage-layouts/{id}",
            get(stage_layout_get)
                .put(
                    |State(app): State<AppState>,
                     Path(id): Path<String>,
                     headers: HeaderMap,
                     body: Bytes| async move {
                        update_doc(app, id, headers, body, DocKind::StageLayout).await
                    },
                )
                .delete(
                    |State(app): State<AppState>,
                     Path(id): Path<String>,
                     headers: HeaderMap| async move {
                        delete_doc(app, id, headers, DocKind::StageLayout).await
                    },
                ),
        )
        .route("/stage-layouts/{id}/document", get(stage_layout_document))
        .route("/stage-layouts/{id}/edits", post(edit_stage_layout))
        .route("/stage-layouts/{id}/thumbnail", get(stage_layout_thumbnail))
        .route(
            "/stage-layouts/{id}/slides/{slide}/render",
            get(stage_layout_render),
        )
        .route("/stage-layout-templates", get(stage_layout_templates))
        .route(
            "/stage-layout-templates/{id}/thumbnail",
            get(stage_template_thumbnail),
        )
}

async fn stage_layouts_list(
    State(app): State<AppState>,
) -> ApiResult<Vec<protocol::StageLayoutEntry>> {
    ok(app
        .shared
        .data()
        .content
        .stage_layouts
        .iter()
        .map(crate::showdocs::stage_layout_entry)
        .collect())
}

fn find_stage_layout(app: &AppState, id: &str) -> Result<model::StageLayout, Error> {
    let id: uuid::Uuid = parse_id(id, "stage layout id")?;
    crate::showdocs::find(&app.shared.data().content.stage_layouts, id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("stage layout {id} not found")))
}

async fn stage_layout_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    let e = crate::showdocs::stage_layout_entry(&find_stage_layout(&app, &id)?);
    Ok(with_etag(StatusCode::OK, &e.revision.clone(), e))
}

/// `GET /stage-layouts/{id}/document`: the layout as a one-slide presentation (Edit view).
async fn stage_layout_document(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, Error> {
    let l = find_stage_layout(&app, &id)?;
    let rev = crate::config::revision(&l);
    Ok(with_etag(
        StatusCode::OK,
        &rev,
        crate::showdocs::stage_layout_document(&l),
    ))
}

async fn edit_stage_layout(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = (|| {
        Ok::<_, Error>((
            parse_id::<model::StageLayoutId>(&id, "stage layout id")?,
            parse_body::<protocol::PresentationEdit>(&body)?,
        ))
    })();
    match r {
        Ok((id, edit)) => {
            let client = edit.client_id.clone().or(client_header(&headers));
            run_docs(&app, DocOp::EditStageLayout(id, edit), false, client).await
        }
        Err(e) => e.into_response(),
    }
}

/// `GET /stage-layouts/{id}/thumbnail?w=240`: the layout as designed (PNG with alpha).
async fn stage_layout_thumbnail(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Request,
) -> Result<Response, Error> {
    let l = find_stage_layout(&app, &id)?;
    let rev = crate::config::revision(&l);
    slide_thumbnail(
        &app,
        l.slide,
        l.size,
        &rev,
        &headers,
        req.uri().query().map(str::to_owned),
    )
    .await
}

async fn stage_layout_render(
    State(app): State<AppState>,
    Path((id, slide)): Path<(String, String)>,
    req: Request,
) -> Result<Response, Error> {
    let l = find_stage_layout(&app, &id)?;
    let sid: SlideId = parse_id(&slide, "slide id")?;
    if l.slide.id != sid {
        return Err(err(ErrorCode::NotFound, "slide not in stage layout"));
    }
    one_slide_render(&app, l.slide, l.size, req.uri().query().map(str::to_owned)).await
}

/// `GET /stage-layout-templates`: the built-in templates new layouts can start from.
async fn stage_layout_templates() -> ApiResult<Vec<protocol::StageLayoutTemplate>> {
    ok(crate::stage::Template::ALL
        .iter()
        .map(|t| protocol::StageLayoutTemplate {
            id: t.id().into(),
            name: t.name().into(),
        })
        .collect())
}

/// `GET /stage-layout-templates/{id}/thumbnail?w=240`: a built-in template's picture (the
/// built-in default "Current / Next" layout is `currentNext`).
async fn stage_template_thumbnail(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Request,
) -> Result<Response, Error> {
    let t = crate::stage::Template::from_id(&id)
        .ok_or_else(|| err(ErrorCode::NotFound, format!("template {id:?} not found")))?;
    let slide = t.slide();
    let rev = format!("tpl-{id}-{}", crate::VERSION);
    slide_thumbnail(
        &app,
        slide,
        crate::stage::TEMPLATE_CANVAS,
        &rev,
        &headers,
        req.uri().query().map(str::to_owned),
    )
    .await
}
