//! PowerPoint import jobs on the Codex box (`/api/v1/codex/imports/pptx`, see
//! [`super::api`]): the uploaded `.pptx` is converted by [`pptx_import`] on a blocking thread
//! (slide pictures by LibreOffice when the server has it), its media are taken into the
//! collection like uploads (the same checks, deduplication and media library), and the
//! presentation is written into the collection's library through the Codex writer. One
//! import runs at a time; the others wait (`queued`). Jobs live in memory (the last
//! [`KEEP`]); a restart forgets them, and an import cut off by one saved nothing.

use crate::api::AppState;
use crate::auth::Access;
use crate::content::ContentOp;
use axum::body::{Body, Bytes};
use model::{CollectionId, FillKind, Library, LibraryId, MediaAssetId, Presentation};
use pptx_import::render::Renderer;
use protocol::{
    CreateLibrary, CreatePresentation, CreateUpload, ErrorCode, FinishUpload, PptxImportJob,
    PptxImportNote, PptxImportReport, PptxImportState, PptxPictureSource, PptxSlideReport,
};
use sha2::Digest;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Finished jobs kept for their reports.
pub const KEEP: usize = 20;

/// Most unfinished jobs at once (one importing, one waiting): each holds its whole file.
pub const MAX_UNFINISHED: usize = 2;

/// Name of the library imports go to when the request names none.
pub const LIBRARY: &str = "Imported Slides";

/// What to import.
#[derive(Clone, Debug)]
pub struct Request {
    pub collection: CollectionId,
    pub library: Option<LibraryId>,
    pub name: Option<String>,
    pub file_name: String,
}

/// The import jobs.
pub struct PptxJobs {
    jobs: Mutex<VecDeque<Arc<Mutex<PptxImportJob>>>>,
    one: tokio::sync::Semaphore,
    renderer: Renderer,
}

impl PptxJobs {
    /// Jobs whose slide pictures come from `renderer`.
    pub fn new(renderer: Renderer) -> Self {
        PptxJobs {
            jobs: Mutex::new(VecDeque::new()),
            one: tokio::sync::Semaphore::new(1),
            renderer,
        }
    }

    /// Every job kept, newest first.
    pub fn list(&self) -> Vec<PptxImportJob> {
        let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.iter()
            .rev()
            .map(|j| j.lock().unwrap_or_else(|e| e.into_inner()).clone())
            .collect()
    }

    /// One job.
    pub fn get(&self, id: &str) -> Option<PptxImportJob> {
        let jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.iter()
            .map(|j| j.lock().unwrap_or_else(|e| e.into_inner()).clone())
            .find(|j| j.id == id)
    }

    /// Starts importing `bytes` as `access` asked (runs in the background). `None` when
    /// [`MAX_UNFINISHED`] imports are already running or waiting.
    pub fn start(
        self: &Arc<Self>,
        app: &AppState,
        access: Access,
        req: Request,
        bytes: Bytes,
    ) -> Option<PptxImportJob> {
        let job = PptxImportJob {
            id: uuid::Uuid::new_v4().to_string(),
            collection: req.collection,
            file_name: req.file_name.clone(),
            state: PptxImportState::Queued,
            done: 0,
            total: 0,
            error: None,
            report: None,
            created_at: crate::auth::rfc3339(std::time::SystemTime::now(), true),
        };
        let handle = Arc::new(Mutex::new(job.clone()));
        {
            let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
            let unfinished = jobs
                .iter()
                .filter(|j| {
                    !matches!(
                        j.lock().unwrap_or_else(|e| e.into_inner()).state,
                        PptxImportState::Done | PptxImportState::Failed
                    )
                })
                .count();
            if unfinished >= MAX_UNFINISHED {
                return None;
            }
            jobs.push_back(handle.clone());
            // Drop the oldest finished ones beyond KEEP.
            while jobs.len() > KEEP {
                let Some(i) = jobs.iter().position(|j| {
                    matches!(
                        j.lock().unwrap_or_else(|e| e.into_inner()).state,
                        PptxImportState::Done | PptxImportState::Failed
                    )
                }) else {
                    break;
                };
                jobs.remove(i);
            }
        }
        let (me, app) = (self.clone(), app.clone());
        let by = crate::api::change_by(&app.shared);
        tokio::spawn(async move {
            let _one = me.one.acquire().await;
            let r = run(&app, &me.renderer, access, by, &req, bytes, &handle).await;
            let mut j = handle.lock().unwrap_or_else(|e| e.into_inner());
            match r {
                Ok(report) => {
                    tracing::info!(job = %j.id, presentation = %report.presentation.0, slides = report.slides.len(), pictured = report.pictured_slides, "PowerPoint import done");
                    j.state = PptxImportState::Done;
                    j.report = Some(report);
                }
                Err(e) => {
                    tracing::warn!(job = %j.id, "PowerPoint import failed: {e}");
                    j.state = PptxImportState::Failed;
                    j.error = Some(e);
                }
            }
        });
        Some(job)
    }
}

fn set(job: &Mutex<PptxImportJob>, state: PptxImportState, done: u32, total: u32) {
    let mut j = job.lock().unwrap_or_else(|e| e.into_inner());
    j.state = state;
    j.done = done;
    j.total = total;
}

async fn run(
    app: &AppState,
    renderer: &Renderer,
    access: Access,
    by: model::ChangeBy,
    req: &Request,
    bytes: Bytes,
    job: &Arc<Mutex<PptxImportJob>>,
) -> Result<PptxImportReport, String> {
    set(job, PptxImportState::Reading, 0, 0);
    let opts = pptx_import::Options {
        name: req.name.clone().or_else(|| {
            let stem = req.file_name.rsplit(['/', '\\']).next().unwrap_or_default();
            let stem = stem.rsplit_once('.').map(|(s, _)| s).unwrap_or(stem).trim();
            (!stem.is_empty()).then(|| stem.to_string())
        }),
        renderer: renderer.clone(),
        work_dir: None,
    };
    let j = job.clone();
    let deck = tokio::task::spawn_blocking(move || {
        pptx_import::import(&bytes, &opts, &mut |stage, done, total| {
            let s = match stage {
                pptx_import::Stage::Slides => PptxImportState::Reading,
                pptx_import::Stage::Pictures => PptxImportState::Pictures,
            };
            set(&j, s, done, total);
        })
    })
    .await
    .map_err(|e| format!("the import stopped: {e}"))?
    .map_err(|e| format!("{e:#}"))?;
    let pptx_import::Deck {
        mut presentation,
        media,
        report,
    } = deck;

    // Media, like uploads (one at a time).
    let total = media.len() as u32;
    let mut assets: HashMap<MediaAssetId, MediaAssetId> = HashMap::new();
    for (i, m) in media.into_iter().enumerate() {
        set(job, PptxImportState::Media, i as u32, total);
        let id = upload(app, req.collection, &m.name, m.bytes, by.clone()).await?;
        assets.insert(m.asset, id);
    }
    set(job, PptxImportState::Saving, 0, 0);
    remap(&mut presentation, &assets);
    let now = crate::auth::rfc3339(std::time::SystemTime::now(), true);
    presentation.created_at = Some(now.clone());
    presentation.modified_at = Some(now.clone());
    if let Some(i) = presentation.import.as_mut() {
        i.imported_at = Some(now);
        i.document_path = Some(req.file_name.clone());
    }
    let fwd = crate::api::Forwarded {
        access,
        machine: app.shared.auth.identity().id.to_string(),
        collection: req.collection,
    };
    let library = crate::api::as_forwarded(
        fwd.clone(),
        library_for(app, req.collection, req.library, by.clone()),
    )
    .await?;
    presentation.library = Some(library);
    presentation.sort_key = {
        let content = app
            .shared
            .collection_content(req.collection)
            .ok_or("the collection is no longer in the Codex")?;
        let last = content
            .presentations
            .iter()
            .filter(|p| p.library == Some(library))
            .map(|p| p.sort_key.clone())
            .max();
        model::sort_key_between(last.as_deref().filter(|k| !k.is_empty()), None)
    };
    let pid = presentation.id;
    let name = presentation.name.clone();
    let canvas = presentation.canvas_size;
    let create = CreatePresentation {
        name: None,
        library: None,
        document: Some(presentation),
    };
    crate::api::as_forwarded(
        fwd,
        crate::codex::server::content(
            &app.shared,
            &app.show,
            ContentOp::CreatePresentation(Box::new(create)),
            None,
            by,
        ),
    )
    .await
    .map_err(|e| e.error.message)?;
    Ok(wire_report(report, pid, name, library, canvas, total))
}

/// Takes one media file into the collection, as an upload; the asset it became.
async fn upload(
    app: &AppState,
    collection: CollectionId,
    name: &str,
    bytes: Vec<u8>,
    by: model::ChangeBy,
) -> Result<MediaAssetId, String> {
    let sha: String = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let size = bytes.len() as u64;
    let info = app
        .shared
        .uploads
        .create(
            &app.codex,
            CreateUpload {
                collection,
                name: name.to_string(),
                size,
                sha256: Some(sha),
                bin: None,
                relative_path: None,
                replaces: None,
            },
            by.clone(),
        )
        .await
        .map_err(|e| format!("{name}: {}", e.message))?;
    let id = uuid::Uuid::parse_str(&info.id).map_err(|e| e.to_string())?;
    let range = format!("bytes 0-{}/{size}", size.saturating_sub(1));
    let body = axum::body::Bytes::from(bytes);
    let mut tries = 0;
    loop {
        match app
            .shared
            .uploads
            .chunk(&app.codex, id, Some(&range), Body::from(body.clone()))
            .await
        {
            Ok(_) => break,
            Err(crate::codex::uploads::UploadError::Api(e))
                if e.error.code == ErrorCode::RateLimited && tries < 50 =>
            {
                tries += 1;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(e) => {
                let _ = app.shared.uploads.cancel(&app.codex, id).await;
                return Err(format!("{name}: {}", upload_message(e)));
            }
        }
    }
    let f = app
        .shared
        .uploads
        .finish(
            &app.codex,
            &app.show,
            &app.media_jobs,
            id,
            FinishUpload::default(),
            None,
            by,
        )
        .await
        .map_err(|e| format!("{name}: {}", upload_message(e)))?;
    Ok(f.asset.id)
}

fn upload_message(e: crate::codex::uploads::UploadError) -> String {
    match e {
        crate::codex::uploads::UploadError::Api(e) => e.error.message,
        crate::codex::uploads::UploadError::Offset(e, _) => e.message,
    }
}

/// The library to import into: `asked` (it must be in the collection), else the
/// collection's [`LIBRARY`], made when missing.
async fn library_for(
    app: &AppState,
    collection: CollectionId,
    asked: Option<LibraryId>,
    by: model::ChangeBy,
) -> Result<LibraryId, String> {
    let content = app
        .shared
        .collection_content(collection)
        .ok_or("the collection is no longer in the Codex")?;
    if let Some(l) = asked {
        return if content.libraries.contains_key(&l) {
            Ok(l)
        } else {
            Err(format!("library {} is no longer in the collection", l.0))
        };
    }
    let mut named: Vec<&Library> = content
        .libraries
        .values()
        .filter(|l| l.name.trim().eq_ignore_ascii_case(LIBRARY))
        .collect();
    named.sort_by_key(|l| l.id.0);
    if let Some(l) = named.first() {
        return Ok(l.id);
    }
    let id = LibraryId::from(uuid::Uuid::new_v5(
        &collection.0,
        b"vesper:imported-slides-library",
    ));
    let last = content.libraries.values().map(|l| l.sort_key.clone()).max();
    let body = CreateLibrary {
        name: None,
        document: Some(Library {
            id,
            name: LIBRARY.into(),
            sort_key: model::sort_key_between(last.as_deref().filter(|k| !k.is_empty()), None),
            ..Library::default()
        }),
    };
    match crate::codex::server::content(
        &app.shared,
        &app.show,
        ContentOp::CreateLibrary(Box::new(body)),
        None,
        by,
    )
    .await
    {
        Ok(_) => Ok(id),
        // Made meanwhile (two imports at once can't happen, but an operator can).
        Err(e) if e.error.code == ErrorCode::Conflict => Ok(id),
        Err(e) => Err(e.error.message),
    }
}

/// Points the presentation's media fills at the Codex's assets.
fn remap(p: &mut Presentation, map: &HashMap<MediaAssetId, MediaAssetId>) {
    for s in p.sections.iter_mut().flat_map(|s| s.slides.iter_mut()) {
        for e in &mut s.elements {
            if let FillKind::Media { asset, .. } = &mut e.fill.kind
                && let Some(n) = map.get(asset)
            {
                *asset = *n;
            }
        }
    }
}

fn note(u: pptx_import::Unmapped) -> PptxImportNote {
    PptxImportNote {
        kind: u.kind,
        name: u.name,
        detail: u.detail,
    }
}

fn wire_report(
    r: pptx_import::Report,
    presentation: model::PresentationId,
    name: String,
    library: LibraryId,
    canvas: model::Size,
    media: u32,
) -> PptxImportReport {
    let slides: Vec<PptxSlideReport> = r
        .slides
        .into_iter()
        .map(|s| PptxSlideReport {
            number: s.number,
            title: s.title,
            editable: s.editable,
            pictured: s.pictured.into_iter().map(note).collect(),
            approximated: s.approximated.into_iter().map(note).collect(),
            notes: s.notes.into_iter().map(note).collect(),
            picture: s.picture.map(|p| match p {
                pptx_import::PictureSource::Renderer => PptxPictureSource::LibreOffice,
                pptx_import::PictureSource::Thumbnail => PptxPictureSource::Thumbnail,
            }),
            hidden: s.hidden,
        })
        .collect();
    PptxImportReport {
        presentation,
        name,
        library,
        width: canvas.width,
        height: canvas.height,
        editable_slides: slides
            .iter()
            .filter(|s| s.pictured.is_empty() && s.approximated.is_empty())
            .count() as u32,
        pictured_slides: slides.iter().filter(|s| s.picture.is_some()).count() as u32,
        slides,
        media,
        renderer: r.renderer,
        renderer_problem: r.renderer_problem,
    }
}
