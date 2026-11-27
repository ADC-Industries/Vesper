//! Scripture routes that need the show: a passage's slides with a theme, their previews, and
//! the presentation made of them (the read-only routes are [`vesper_scripture::api`]).
//!
//! | Route | |
//! |---|---|
//! | `POST /scripture/slides` | `ScriptureSlideRequest` → `ScriptureSlides` (the split, nothing written) |
//! | `GET /scripture/preview?translation=&reference=&theme=&template=&versesPerSlide=&maxChars=&verseNumbers=&referenceOn=&secondTranslation=&secondLayout=&slide=&w=` | PNG of one generated slide (default width 320; a parallel passage with both translations, as its layout shows them) |
//! | `POST /scripture/presentations` | `ScriptureCreate` → `201 ScriptureCreated`: in the active collection's **Scripture** library (created when missing); `then: playlist` appends it to `playlist`, `then: show` shows its first slide |
//! | `POST /scripture/presentations/{id}/resplit` | `ScriptureResplit` → `ScriptureResplitResult`: the passage split again with its theme as it is now (`409` when the operator edited it, unless `force`) |
//!
//! **Re-split on theme change** (batch 9): a created passage records how it was made
//! (`scripture.generated`, with a digest of its slides). When a theme is edited, a background
//! job ([`after_theme_change`], debounced, one at a time) splits again every passage made with
//! it that the operator hasn't edited; an edited one is left as it is and marked `stale`, so
//! Oriel offers to split it again; so is one someone holds the edit lock on. Slides that still
//! start at the same verse keep their IDs (a live passage keeps its place), and the edit applies
//! only to the revision it was split from. Themes are only read. On a show box the writes go
//! to the site's Codex like every content write ([`crate::codex::write_through`]); the box
//! shows the presentation from its copy. A theme edit a show box forwarded is followed up in
//! the collection that box shows.

use super::*;
use crate::api::Forwarded;
use crate::data::CollectionContent;
use model::{CodexRole, DocumentKind, Library, ScriptureGenerated, Slide, SlideId, ThemeId};
use protocol::{
    CreateLibrary, CreatePresentation, PlaylistEdit, PlaylistOp, PresentationEdit, PresentationOp,
    ReferencePlacement, ScriptureCreate, ScriptureCreated, ScriptureResplit,
    ScriptureResplitResult, ScriptureSlideRequest, ScriptureSlides, ScriptureThen,
};
use vesper_scripture::api::api_error;
use vesper_scripture::slides::{self, SlideOptions};

/// Name of the library generated passages go to.
pub const LIBRARY: &str = "Scripture";

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/scripture/slides", post(slides_route))
        .route("/scripture/preview", get(preview))
        .route("/scripture/presentations", post(create))
        .route("/scripture/presentations/{id}/resplit", post(resplit_route))
}

/// A passage as a presentation (not stored) and its split, on a blocking thread (the split
/// lays the slides' text out with [`crate::Shared::measurer`]).
async fn build(
    app: &AppState,
    req: &ScriptureSlideRequest,
    name: Option<String>,
) -> Result<(Presentation, ScriptureSlides), Error> {
    let (app, req, content) = (app.clone(), req.clone(), content(app));
    tokio::task::spawn_blocking(move || build_now(&app, &content, &req, name))
        .await
        .map_err(|e| err(ErrorCode::Internal, format!("scripture split failed: {e}")))?
}

fn build_now(
    app: &AppState,
    content: &CollectionContent,
    req: &ScriptureSlideRequest,
    name: Option<String>,
) -> Result<(Presentation, ScriptureSlides), Error> {
    let t = app.scripture.get(&req.translation).map_err(api_error)?;
    let r = t
        .parse(&req.reference)
        .map_err(|(_, e)| err(ErrorCode::BadRequest, e))?;
    let passage = t.passage_of(&r).map_err(api_error)?;
    if passage.verses.is_empty() {
        return Err(err(
            ErrorCode::BadRequest,
            format!(
                "{} has no text in {}",
                passage.label,
                t.header().abbreviation
            ),
        ));
    }
    let opts = SlideOptions::from(req);
    let second = match req.second_translation.as_deref() {
        Some(id) if id != req.translation => {
            let t2 = app.scripture.get(id).map_err(api_error)?;
            let passage = slides::parallel_passage(t, &r, t2).map_err(api_error)?;
            Some(slides::Parallel {
                translation: t2,
                passage,
            })
        }
        _ => None,
    };
    let name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| match &second {
            Some(s) => format!(
                "{} ({} · {})",
                t.label(&r),
                t.header().abbreviation,
                s.translation.header().abbreviation
            ),
            None => slides::name(t, &r),
        });
    if !crate::content::name_ok(&name) {
        return Err(err(
            ErrorCode::BadRequest,
            format!(
                "names are at most {} characters, without control characters",
                crate::content::MAX_NAME
            ),
        ));
    }
    let theme = match req.theme {
        Some(id) => Some(
            content
                .themes
                .get(&id)
                .ok_or_else(|| err(ErrorCode::NotFound, format!("no theme {}", id.0)))?,
        ),
        None => None,
    };
    let template = match theme {
        Some(th) => Some((
            th,
            slides::pick_template(th, req.template).ok_or_else(|| {
                err(
                    ErrorCode::BadRequest,
                    format!("theme {:?} has no such template slide", th.name),
                )
            })?,
        )),
        None => None,
    };
    // Each candidate slide is built as it will be stored and its text laid out by the
    // renderer's engine at the authored size (see `vesper_scripture::slides`).
    // An explicit `maxChars` still caps the characters on a slide.
    let mut chars = slides::chars_fit(opts.max_chars, opts.verse_numbers);
    let capped = req.max_chars.is_some();
    let mut measurer = lock(app.shared.measurer());
    // A parallel passage is measured as its preferred layout draws it, in the theme's
    // second-language style (`model::language`).
    let style = theme.map(|t| t.second_language.clone()).unwrap_or_default();
    let mut fits = |planned: &slides::PlannedSlide| {
        if capped && !chars(planned) {
            return false;
        }
        let (slide, ids) = slides::slide_with_text(planned, &opts, template);
        let shown = model::language::show_language(&slide, opts.second_layout, &style);
        let added = |id: &model::ElementId| !slide.elements.iter().any(|e| e.id == *id);
        shown
            .as_ref()
            .unwrap_or(&slide)
            .elements
            .iter()
            .filter(|e| ids.contains(&e.id) || added(&e.id))
            .all(|e| e.text_box().is_none_or(|tb| measurer.fits(tb, e.bounds)))
    };
    let planned = slides::plan_parallel(t, t.book(&r), &passage, second.as_ref(), &opts, &mut fits);
    drop(measurer);
    let p = slides::presentation(t, &r, &planned, &opts, template, name.clone());
    Ok((p, slides::preview(name, &planned, &opts)))
}

async fn slides_route(State(app): State<AppState>, body: Bytes) -> ApiResult<ScriptureSlides> {
    let req: ScriptureSlideRequest = parse_body(&body)?;
    ok(build(&app, &req, None).await?.1)
}

/// The preview's query (the slide request's fields, plus `slide` and `w`).
#[derive(serde::Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct PreviewQuery {
    translation: String,
    reference: String,
    theme: Option<String>,
    template: Option<String>,
    verses_per_slide: Option<u32>,
    max_chars: Option<u32>,
    verse_numbers: Option<bool>,
    reference_on: Option<ReferencePlacement>,
    second_translation: Option<String>,
    second_layout: Option<model::LanguageDisplay>,
    slide: Option<usize>,
    w: Option<u32>,
}

async fn preview(
    State(app): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<PreviewQuery>,
) -> Result<Response, Error> {
    let req = ScriptureSlideRequest {
        translation: q.translation,
        reference: q.reference,
        theme: q
            .theme
            .filter(|t| !t.is_empty())
            .map(|t| parse_id(&t, "theme"))
            .transpose()?,
        template: q
            .template
            .filter(|t| !t.is_empty())
            .map(|t| parse_id(&t, "template"))
            .transpose()?,
        verses_per_slide: q.verses_per_slide,
        max_chars: q.max_chars,
        verse_numbers: q.verse_numbers.unwrap_or(false),
        reference_on: q.reference_on.unwrap_or_default(),
        second_translation: q.second_translation.filter(|t| !t.is_empty()),
        second_layout: q.second_layout,
    };
    let (p, _) = build(&app, &req, None).await?;
    let index = q.slide.unwrap_or(0);
    let slide = p
        .slides()
        .nth(index)
        .cloned()
        .ok_or_else(|| err(ErrorCode::NotFound, format!("no slide {index}")))?;
    // A parallel passage as a screen with the default look shows it (both translations).
    let style = req
        .theme
        .and_then(|t| {
            content(&app)
                .themes
                .get(&t)
                .map(|t| t.second_language.clone())
        })
        .unwrap_or_default();
    let slide = model::language::show_language(&slide, model::LanguageDisplay::Auto, &style)
        .unwrap_or(slide);
    let width = q.w.unwrap_or(320).clamp(16, 1920);
    let png = app
        .thumbs
        .slide_png(slide, p.canvas_size, width, &[])
        .await?;
    Ok((
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/png")),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        Body::from(png),
    )
        .into_response())
}

/// A refused request's answer (boxed: a response is large).
type Fail = Box<Response>;

fn fail(r: impl IntoResponse) -> Fail {
    Box::new(r.into_response())
}

/// A content write of this box's collection: through the Codex writer, or on a show box
/// forwarded to the site's Codex as the client that asked (`POST path` with `body`). Returns
/// the answer's JSON.
async fn write(
    app: &AppState,
    access: &Access,
    path: String,
    body: serde_json::Value,
    op: ContentOp,
) -> Result<serde_json::Value, Fail> {
    write_as(app, access, Method::POST, path, body, op).await
}

/// The collection content this request or job works on: a show box's forwarded collection
/// (on the Codex, see [`crate::api::forwarded_collection`]), else this box's active one.
fn content(app: &AppState) -> Arc<CollectionContent> {
    crate::api::forwarded_collection()
        .and_then(|c| app.shared.collection_content(c))
        .unwrap_or_else(|| app.shared.data().content.clone())
}

/// [`write`] with the method the site's route takes.
async fn write_as(
    app: &AppState,
    access: &Access,
    method: Method,
    path: String,
    body: serde_json::Value,
    op: ContentOp,
) -> Result<serde_json::Value, Fail> {
    write_by(app, access, method, path, body, op, None, None).await
}

/// [`write_as`] for client `client` (its locks; echoed in `documentChanged`) and with
/// `if_match` on a forwarded write (a local one carries it in `op`).
#[allow(clippy::too_many_arguments)]
async fn write_by(
    app: &AppState,
    access: &Access,
    method: Method,
    path: String,
    body: serde_json::Value,
    op: ContentOp,
    client: Option<String>,
    if_match: Option<String>,
) -> Result<serde_json::Value, Fail> {
    if app.shared.codex().config.role == CodexRole::Client {
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        let mut req = Request::builder()
            .method(method)
            .uri(format!("{}{path}", protocol::API_PREFIX))
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(c) = &client {
            req = req.header("x-client-id", c.as_str());
        }
        if let Some(m) = &if_match {
            req = req.header(header::IF_MATCH, m.as_str());
        }
        let mut req = req
            .body(body)
            .map_err(|e| fail(err(ErrorCode::Internal, e.to_string())))?;
        req.extensions_mut().insert(access.clone());
        let r = crate::codex::write_through::forward(&app.codex, req).await;
        let status = r.status();
        let bytes = axum::body::to_bytes(r.into_body(), 1 << 20)
            .await
            .map_err(|e| fail(err(ErrorCode::Internal, e.to_string())))?;
        if !status.is_success() {
            return Err(fail((status, bytes)));
        }
        return serde_json::from_slice(&bytes)
            .map_err(|e| fail(err(ErrorCode::Internal, e.to_string())));
    }
    let r =
        crate::codex::server::content(&app.shared, &app.show, op, client, change_by(&app.shared))
            .await
            .map_err(fail)?;
    let v = match r {
        ContentResult::PresentationEdit(r) => serde_json::to_value(r),
        ContentResult::PlaylistEdit(r) => serde_json::to_value(r),
        ContentResult::MediaBinEdit(r) => serde_json::to_value(r),
        ContentResult::Revision(r, _) => serde_json::to_value(r),
        ContentResult::Deleted(d) => serde_json::to_value(d),
    };
    v.map_err(|e| fail(err(ErrorCode::Internal, e.to_string())))
}

/// The active collection's Scripture library, created when missing.
async fn scripture_library(app: &AppState, access: &Access) -> Result<LibraryId, Fail> {
    let find = || {
        let d = app.shared.data();
        let mut libs: Vec<&Library> = d
            .content
            .libraries
            .values()
            .filter(|l| l.name.trim().eq_ignore_ascii_case(LIBRARY))
            .collect();
        libs.sort_by_key(|l| l.id.0);
        libs.first().map(|l| l.id)
    };
    if let Some(id) = find() {
        return Ok(id);
    }
    // One ID per collection: two creates at the same time (a double click, two operators, two
    // show boxes) make one library, the later one is refused as a conflict and uses it.
    let id = LibraryId::from(uuid::Uuid::new_v5(
        &app.shared.codex().active.0,
        b"vesper:scripture-library",
    ));
    let sort_key = {
        let d = app.shared.data();
        let last = d
            .content
            .libraries
            .values()
            .map(|l| l.sort_key.clone())
            .max();
        model::sort_key_between(last.as_deref().filter(|k| !k.is_empty()), None)
    };
    let body = CreateLibrary {
        name: None,
        document: Some(Library {
            id,
            name: LIBRARY.into(),
            sort_key,
            ..Library::default()
        }),
    };
    match write(
        app,
        access,
        "/libraries".into(),
        serde_json::to_value(&body).unwrap_or_default(),
        ContentOp::CreateLibrary(Box::new(body)),
    )
    .await
    {
        Ok(_) => Ok(id),
        Err(r) if r.status() == StatusCode::CONFLICT => Ok(id),
        Err(r) => Err(r),
    }
}

/// Appends `item` to playlist `pl`. The end is read just before the edit (the passage took a
/// while to build), and read again when another edit got there first.
async fn append_to_playlist(
    app: &AppState,
    access: &Access,
    pl: PlaylistId,
    item: model::PlaylistItem,
) -> Result<(), Fail> {
    let mut attempt = 0;
    loop {
        let after = {
            let d = app.shared.data();
            match d.content.playlists.get(&pl).map(|n| &n.kind) {
                Some(model::PlaylistNodeKind::Playlist { items, .. }) => items.last().map(|i| i.id),
                _ => {
                    return Err(fail(err(
                        ErrorCode::NotFound,
                        format!("no playlist {}", pl.0),
                    )));
                }
            }
        };
        let edit = PlaylistEdit {
            ops: vec![PlaylistOp::AddItem {
                after,
                item: item.clone(),
            }],
            client_id: None,
            base_revision: None,
        };
        match write(
            app,
            access,
            format!("/playlists/{}/edits", pl.0),
            serde_json::to_value(&edit).unwrap_or_default(),
            ContentOp::EditPlaylist(pl, edit),
        )
        .await
        {
            Ok(_) => return Ok(()),
            Err(r) if r.status() == StatusCode::CONFLICT && attempt < 3 => {
                attempt += 1;
                // On a show box the copy catches up with the site's change meanwhile.
                tokio::time::sleep(std::time::Duration::from_millis(100 * attempt)).await;
            }
            Err(r) => return Err(r),
        }
    }
}

async fn create(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    body: Bytes,
) -> Response {
    match create_inner(&app, &access, &body).await {
        Ok(c) => (StatusCode::CREATED, axum::Json(c)).into_response(),
        Err(r) => *r,
    }
}

async fn create_inner(
    app: &AppState,
    access: &Access,
    body: &[u8],
) -> Result<ScriptureCreated, Fail> {
    let b: ScriptureCreate = parse_body(body).map_err(fail)?;
    let playlist = match (b.then, b.playlist) {
        (ScriptureThen::Playlist, None) => {
            return Err(fail(err(
                ErrorCode::BadRequest,
                "then: playlist needs a playlist",
            )));
        }
        (ScriptureThen::Playlist, Some(id)) => {
            let d = app.shared.data();
            match d.content.playlists.get(&id).map(|n| &n.kind) {
                // Where it goes is read when it is added (see `append_to_playlist`).
                Some(model::PlaylistNodeKind::Playlist { .. }) => Some(id),
                _ => {
                    return Err(fail(err(
                        ErrorCode::NotFound,
                        format!("no playlist {}", id.0),
                    )));
                }
            }
        }
        _ => None,
    };
    let (mut p, split) = build(app, &b.slides, b.name.clone()).await.map_err(fail)?;
    let digest = crate::config::revision(&p.sections);
    if let Some(s) = &mut p.scripture {
        s.generated = Some(generated(&b.slides, digest));
    }
    p.library = Some(scripture_library(app, access).await?);
    let id = p.id;
    let name = p.name.clone();
    let create = CreatePresentation {
        name: None,
        library: None,
        document: Some(p),
    };
    write(
        app,
        access,
        "/presentations".into(),
        serde_json::to_value(&create).unwrap_or_default(),
        ContentOp::CreatePresentation(Box::new(create)),
    )
    .await?;
    let mut playlist_item = None;
    if let Some(pl) = playlist {
        let item = model::PlaylistItem::new(PlaylistItemKind::Presentation {
            presentation: model::Link::named(id, name.clone()),
            arrangement: None,
            unresolved: None,
        });
        playlist_item = Some(item.id);
        if let Err(r) = append_to_playlist(app, access, pl, item).await {
            // Not half done: the passage goes again (to the trash), so a retry doesn't leave
            // a second copy behind.
            let _ = write_as(
                app,
                access,
                Method::DELETE,
                format!("/presentations/{}", id.0),
                serde_json::Value::Null,
                ContentOp::DeletePresentation(id, None),
            )
            .await;
            return Err(r);
        }
    }
    if b.then == ScriptureThen::Show {
        app.show
            .apply(protocol::Command::TriggerSlide {
                presentation: id,
                slide: None,
                index: Some(0),
                playlist_item: None,
                step: None,
                transition: None,
            })
            .await
            .map_err(|e| fail(Error(e)))?;
    }
    Ok(ScriptureCreated {
        presentation: id,
        name,
        slide_count: split.slides.len() as u32,
        playlist_item,
    })
}

/// What a generated passage records of the request it was made from.
fn generated(req: &ScriptureSlideRequest, slides: String) -> ScriptureGenerated {
    ScriptureGenerated {
        translation: req.translation.clone(),
        reference: req.reference.clone(),
        theme: req.theme,
        template: req.template,
        verses_per_slide: req.verses_per_slide,
        max_chars: req.max_chars,
        verse_numbers: req.verse_numbers,
        reference_on: req.reference_on,
        second_translation: req.second_translation.clone(),
        second_layout: req.second_layout,
        slides,
        stale: false,
    }
}

/// The request a generated passage was made from.
fn request_of(g: &ScriptureGenerated) -> ScriptureSlideRequest {
    ScriptureSlideRequest {
        translation: g.translation.clone(),
        reference: g.reference.clone(),
        theme: g.theme,
        template: g.template,
        verses_per_slide: g.verses_per_slide,
        max_chars: g.max_chars,
        verse_numbers: g.verse_numbers,
        reference_on: g.reference_on,
        second_translation: g.second_translation.clone(),
        second_layout: g.second_layout,
    }
}

/// `v` without its `id` fields (two splits compared by what they show).
fn without_ids(mut v: serde_json::Value) -> serde_json::Value {
    fn strip(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(m) => {
                m.remove("id");
                m.values_mut().for_each(strip);
            }
            serde_json::Value::Array(a) => a.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut v);
    v
}

/// What [`resplit`] did.
#[derive(Debug, PartialEq)]
enum Resplit {
    /// Split again (slide count), or the same split (nothing written).
    Done(ScriptureResplitResult),
    /// The operator edited it, and `force` wasn't given: nothing written.
    Edited,
}

fn passage(app: &AppState, id: PresentationId) -> Result<(Presentation, ScriptureGenerated), Fail> {
    let d = content(app);
    let p = d
        .presentations
        .iter()
        .find(|p| p.id == id)
        .map(|p| (**p).clone())
        .ok_or_else(|| {
            fail(err(
                ErrorCode::NotFound,
                format!("presentation {} not found", id.0),
            ))
        })?;
    let g = p
        .scripture
        .as_ref()
        .and_then(|s| s.generated.clone())
        .ok_or_else(|| {
            fail(err(
                ErrorCode::BadRequest,
                "not a passage made on the Bible page (it can't be split again)",
            ))
        })?;
    Ok((p, g))
}

/// Where a generated slide starts: its label ("John 3:16–17", "Psalm 2:4, 6") up to the end
/// of the first verse. Two splits of a passage name the same verse the same way.
fn starts_at(s: &Slide) -> Option<&str> {
    let l = s.label.as_ref()?.text.as_str();
    let end = l.find(['–', ',']).unwrap_or(l.len());
    Some(l[..end].trim()).filter(|k| !k.is_empty())
}

/// `n` (a fresh split) with the IDs of `old`'s slides that start at the same verse (the k-th
/// slide starting at a verse takes the k-th old one's) and, when both have one section, its
/// section's ID: a slide on screen keeps its place across a re-split.
fn keep_ids(old: &Presentation, mut n: Presentation) -> Presentation {
    if let ([o], [s]) = (old.sections.as_slice(), n.sections.as_mut_slice()) {
        s.id = o.id;
    }
    let mut free: HashMap<&str, std::collections::VecDeque<SlideId>> = HashMap::new();
    for s in old.slides() {
        if let Some(k) = starts_at(s) {
            free.entry(k).or_default().push_back(s.id);
        }
    }
    for sec in &mut n.sections {
        for s in &mut sec.slides {
            let id = starts_at(s)
                .and_then(|k| free.get_mut(k))
                .and_then(|q| q.pop_front());
            if let Some(id) = id {
                s.id = id;
            }
        }
    }
    n
}

/// The ops turning `old`'s sections into `n`'s (made by [`keep_ids`]): with one section on
/// both sides, slides are replaced in place, added and deleted by ID, and kept slides the
/// operator moved go back into the split's order (E06-2); otherwise every old section is
/// deleted first, then the new ones added (their slides may carry kept IDs, E06-1).
fn resplit_ops(old: &Presentation, n: &Presentation) -> Vec<PresentationOp> {
    let mut ops = Vec::new();
    if let ([o], [s]) = (old.sections.as_slice(), n.sections.as_slice())
        && o.id == s.id
    {
        let kept: HashSet<SlideId> = s.slides.iter().map(|x| x.id).collect();
        for x in o.slides.iter().filter(|x| !kept.contains(&x.id)) {
            ops.push(PresentationOp::DeleteSlide { slide: x.id });
        }
        let had: HashSet<SlideId> = o.slides.iter().map(|x| x.id).collect();
        // The section's order as the ops leave it so far.
        let mut cur: Vec<SlideId> = o
            .slides
            .iter()
            .map(|x| x.id)
            .filter(|id| kept.contains(id))
            .collect();
        let mut after = None;
        for (i, x) in s.slides.iter().enumerate() {
            if had.contains(&x.id) {
                ops.push(PresentationOp::ReplaceSlide { slide: x.clone() });
                // Slides 0..i are in place, so this one belongs at i.
                if cur.get(i) != Some(&x.id) {
                    ops.push(PresentationOp::MoveSlide {
                        slide: x.id,
                        to_section: s.id,
                        after,
                    });
                    cur.retain(|id| *id != x.id);
                    cur.insert(i, x.id);
                }
            } else {
                ops.push(PresentationOp::AddSlide {
                    section: s.id,
                    after,
                    slide: Some(x.clone()),
                });
                cur.insert(i, x.id);
            }
            after = Some(x.id);
        }
        if o.name != s.name {
            ops.push(PresentationOp::SetSection {
                section: s.id,
                name: Some(s.name.clone()),
                color: None,
                hotkey: None,
                global_group: None,
            });
        }
        return ops;
    }
    // Deleted first: a new slide that kept an old slide's ID can't be added while the old one
    // is still there.
    for sec in &old.sections {
        ops.push(PresentationOp::DeleteSection { section: sec.id });
    }
    let mut after = None;
    for sec in &n.sections {
        ops.push(PresentationOp::AddSection {
            after,
            section: sec.clone(),
        });
        after = Some(sec.id);
    }
    ops
}

/// Splits generated passage `id` again with its theme as it is now (see the module docs): its
/// slides are replaced in one edit (undo brings the old split back; slides that still start at
/// the same verse keep their IDs) and its digest updated. The edit applies only to the
/// revision the split was made from (`409` when the passage changed meanwhile). `client` is
/// the client asking (its lock).
async fn resplit(
    app: &AppState,
    access: &Access,
    id: PresentationId,
    force: bool,
    client: Option<String>,
) -> Result<Resplit, Fail> {
    let (p, g) = passage(app, id)?;
    if !force && crate::config::revision(&p.sections) != g.slides {
        return Ok(Resplit::Edited);
    }
    let base = crate::config::revision(&p);
    let (n, split) = build(app, &request_of(&g), Some(p.name.clone()))
        .await
        .map_err(fail)?;
    let n = keep_ids(&p, n);
    let count = split.slides.len() as u32;
    let same = without_ids(serde_json::to_value(&n.sections).unwrap_or_default())
        == without_ids(serde_json::to_value(&p.sections).unwrap_or_default())
        && n.canvas_size == p.canvas_size;
    if same && !g.stale {
        return Ok(Resplit::Done(ScriptureResplitResult {
            presentation: id,
            slide_count: count,
            changed: false,
        }));
    }
    let mut scripture = n.scripture.clone().or(p.scripture.clone());
    if let Some(s) = &mut scripture {
        s.generated = Some(ScriptureGenerated {
            slides: crate::config::revision(&n.sections),
            stale: false,
            ..g
        });
    }
    let mut ops = resplit_ops(&p, &n);
    ops.push(PresentationOp::SetPresentation {
        name: None,
        song_info: None,
        scripture: Some(scripture),
        transition: None,
        canvas_size: Some(n.canvas_size),
        notes: None,
        category: None,
        language: None,
    });
    let edit = PresentationEdit {
        ops,
        client_id: None,
        base_revision: Some(base.clone()),
    };
    write_by(
        app,
        access,
        Method::POST,
        format!("/presentations/{}/edits", id.0),
        serde_json::to_value(&edit).unwrap_or_default(),
        ContentOp::EditPresentation(id, edit, Some(base.clone())),
        client,
        Some(base),
    )
    .await?;
    Ok(Resplit::Done(ScriptureResplitResult {
        presentation: id,
        slide_count: count,
        changed: true,
    }))
}

async fn resplit_route(
    State(app): State<AppState>,
    axum::Extension(access): axum::Extension<Access>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let r = async {
        let id: PresentationId = parse_id(&id, "presentation id").map_err(fail)?;
        let b: ScriptureResplit = if body.is_empty() {
            ScriptureResplit::default()
        } else {
            parse_body(&body).map_err(fail)?
        };
        // The passage's edit lock (on a show box the site's Codex checks the forwarded edit,
        // which names the client).
        let client = super::edit::client_header(&headers);
        app.shared
            .presence
            .check_write(
                crate::presence::request_collection(&app.shared),
                DocumentKind::Presentation,
                &id.0.to_string(),
                client.as_deref(),
            )
            .map_err(|e| Box::new(Error::from(e).into_response()))?;
        match resplit(&app, &access, id, b.force, client).await? {
            Resplit::Done(r) => Ok(r),
            Resplit::Edited => Err(fail(err(
                ErrorCode::Conflict,
                "the passage was edited: split it again with force (the edits are replaced)",
            ))),
        }
    }
    .await;
    match r {
        Ok(r) => (StatusCode::OK, axum::Json(r)).into_response(),
        Err(r) => *r,
    }
}

/// Themes waiting for the re-split job, by theme and the collection it was edited in (with
/// the show box's request that edited it, when one was forwarded here: the job works on its
/// collection).
type ResplitQueue =
    std::collections::BTreeMap<(ThemeId, Option<model::CollectionId>), Option<Forwarded>>;

/// The themes waiting for the re-split job, and whether the job runs.
#[derive(Default)]
pub(crate) struct Resplits(std::sync::Mutex<(ResplitQueue, bool)>);

/// A theme was edited: the passages made with it are split again in the background (see the
/// module docs), in the collection the edit changed. On a show box the site's Codex does it
/// (the edit went there).
pub(super) fn after_theme_change(app: &AppState, theme: ThemeId) {
    if app.shared.codex().config.role == CodexRole::Client {
        return;
    }
    let fwd = crate::api::forwarded();
    {
        let mut q = lock(&app.resplits.0);
        q.0.insert((theme, fwd.as_ref().map(|f| f.collection)), fwd);
        if q.1 {
            return;
        }
        q.1 = true;
    }
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            // Edits come in bursts (a drag): wait for the theme to settle.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let batch = {
                let mut q = lock(&app.resplits.0);
                if q.0.is_empty() {
                    q.1 = false;
                    break;
                }
                std::mem::take(&mut q.0)
            };
            for ((theme, _), fwd) in batch {
                match fwd {
                    Some(f) => crate::api::as_forwarded(f, resplit_theme(&app, theme)).await,
                    None => resplit_theme(&app, theme).await,
                }
            }
        }
    });
}

/// Splits again every generated passage made with `theme` that wasn't edited; marks the
/// edited ones `stale`, and so the ones someone is editing (holds the lock on) or that changed
/// while they were being split.
async fn resplit_theme(app: &AppState, theme: ThemeId) {
    let ids: Vec<PresentationId> = content(app)
        .presentations
        .iter()
        .filter(|p| {
            p.scripture
                .as_ref()
                .and_then(|s| s.generated.as_ref())
                .is_some_and(|g| g.theme == Some(theme))
        })
        .map(|p| p.id)
        .collect();
    let access = Access::Local;
    for id in ids {
        let locked = app
            .shared
            .presence
            .check_write(
                crate::presence::request_collection(&app.shared),
                DocumentKind::Presentation,
                &id.0.to_string(),
                None,
            )
            .is_err();
        let r = if locked {
            Ok(Resplit::Edited)
        } else {
            match resplit(app, &access, id, false, None).await {
                // Changed meanwhile: look again (an operator's edit makes it `Edited`).
                Err(r) if r.status() == StatusCode::CONFLICT => {
                    resplit(app, &access, id, false, None).await
                }
                r => r,
            }
        };
        match r {
            Ok(Resplit::Done(_)) => {}
            Ok(Resplit::Edited) => {
                if let Err(r) = mark_stale(app, &access, id).await {
                    tracing::warn!("scripture: marking {} stale failed: {}", id.0, r.status());
                }
            }
            Err(r) => tracing::warn!("scripture: re-splitting {} failed: {}", id.0, r.status()),
        }
    }
}

/// Marks an edited passage whose theme changed (Oriel offers to split it again).
async fn mark_stale(app: &AppState, access: &Access, id: PresentationId) -> Result<(), Fail> {
    let (p, g) = passage(app, id)?;
    if g.stale {
        return Ok(());
    }
    let mut scripture = p.scripture.clone();
    if let Some(s) = &mut scripture {
        s.generated = Some(ScriptureGenerated { stale: true, ..g });
    }
    let edit = PresentationEdit {
        ops: vec![PresentationOp::SetPresentation {
            name: None,
            song_info: None,
            scripture: Some(scripture),
            transition: None,
            canvas_size: None,
            notes: None,
            category: None,
            language: None,
        }],
        client_id: None,
        base_revision: None,
    };
    write(
        app,
        access,
        format!("/presentations/{}/edits", id.0),
        serde_json::to_value(&edit).unwrap_or_default(),
        ContentOp::EditPresentation(id, edit, None),
    )
    .await
    .map(|_| ())
}
