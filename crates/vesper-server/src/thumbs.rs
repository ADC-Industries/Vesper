//! Slide thumbnails for the slide grid.
//!
//! A thumbnail is the slide drawn over its background (the most recent background media
//! action at or before it, like `render-slide --background`), opaque, at the requested width.
//!
//! - **Content hash:** SHA-256 over the slide JSON, canvas size, background file (path, size,
//!   mtime) and display settings, the machine's font substitution table, the width and the
//!   server version. It names the cache entries and is the HTTP `ETag`, so a request whose
//!   `If-None-Match` matches costs no rendering at all.
//! - **Caches:** memory (LRU, [`MEMORY_BUDGET`] bytes) in front of disk
//!   (`<data>/cache/thumbnails/<presentation>/<slide>-<width>-<hash>.jpg`; older files of the
//!   same slide and width are removed when a new one is written).
//! - **Rendering** goes through the render thread's thumbnail queue ([`crate::engine`]), which
//!   never delays a live change by more than one thumbnail. Requests for the same thumbnail
//!   while it renders share one render. JPEG encoding and file I/O run on the blocking pool.
//! - **Prefetch** (`POST /api/v1/presentations/{id}/thumbnails` or the `prefetchThumbnails`
//!   command) queues every slide of a presentation behind interactive requests.

use crate::engine::{Cmd, ThumbJob};
use crate::preview::encode_jpeg;
use crate::show::{background_for, doc_slides};
use crate::{Shared, lock};
use axum::body::Bytes;
use model::{ElementId, MediaDisplay, PresentationId, Size, Slide, SlideId};
use protocol::{ApiError, ErrorCode, ThumbnailPrefetch};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{oneshot, watch};

/// Memory cache budget, bytes.
pub const MEMORY_BUDGET: usize = 64 << 20;
/// JPEG quality of thumbnails (the Codex's stored media thumbnails use it too).
pub use crate::codex::blobs::THUMB_QUALITY as QUALITY;
/// Smallest / largest width.
pub const WIDTH_RANGE: (u32, u32) = (16, protocol::MAX_IMAGE_WIDTH);

/// A finished thumbnail.
#[derive(Clone)]
pub struct Thumbnail {
    /// JPEG bytes.
    pub jpeg: Bytes,
    /// Strong ETag value (with quotes).
    pub etag: String,
}

/// Everything needed to produce one thumbnail.
pub struct Spec {
    key: String,
    /// Strong ETag value (with quotes).
    pub etag: String,
    slide: Slide,
    canvas: Size,
    background: Option<(PathBuf, MediaDisplay)>,
    width: u32,
    height: u32,
    file: PathBuf,
    prefix: String,
}

type InFlight = Option<Result<Bytes, String>>;

/// Renders in progress, by key: one producer per key, and every other request for it waits
/// for that producer's result. Cancellation-safe: a producer whose request is dropped (the
/// browser gave up on an image) takes its entry with it, and the requests waiting on it
/// produce the thumbnail themselves.
#[derive(Default)]
struct InFlightMap(Mutex<HashMap<String, watch::Receiver<InFlight>>>);

/// The producer's claim on a key: removed when dropped, finished or not.
struct Claim<'a> {
    map: &'a InFlightMap,
    key: &'a str,
    tx: watch::Sender<InFlight>,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        let mut m = lock(&self.map.0);
        if m.get(self.key)
            .is_some_and(|rx| rx.same_channel(&self.tx.subscribe()))
        {
            m.remove(self.key);
        }
    }
}

impl InFlightMap {
    /// `produce`'s result for `key`, or the result of the producer already running for it.
    async fn run<F>(&self, key: &str, produce: impl Fn() -> F) -> Result<Bytes, ApiError>
    where
        F: Future<Output = Result<Bytes, ApiError>>,
    {
        loop {
            let waiting = {
                let mut m = lock(&self.0);
                match m.get(key) {
                    Some(rx) => Err(rx.clone()),
                    None => {
                        let (tx, rx) = watch::channel(None);
                        m.insert(key.to_string(), rx);
                        Ok(tx)
                    }
                }
            };
            match waiting {
                Ok(tx) => {
                    let claim = Claim { map: self, key, tx };
                    let res = produce().await;
                    let _ = claim.tx.send(Some(res.clone().map_err(|e| e.message)));
                    return res;
                }
                Err(mut rx) => match rx.wait_for(Option::is_some).await {
                    Ok(r) => {
                        return match &*r {
                            Some(Ok(j)) => Ok(j.clone()),
                            Some(Err(e)) => Err(ApiError::new(ErrorCode::Internal, e.clone())),
                            None => Err(ApiError::new(ErrorCode::Internal, "no thumbnail")),
                        };
                    }
                    // Its producer was dropped before it finished: produce it here.
                    Err(_) => continue,
                },
            }
        }
    }
}

/// A byte-bounded LRU of encoded images (least recently used first out).
pub struct BytesLru {
    map: HashMap<String, (Bytes, u64)>,
    bytes: usize,
    tick: u64,
    budget: usize,
}

impl BytesLru {
    /// An empty cache holding at most `budget` bytes (and always the newest entry).
    pub fn new(budget: usize) -> Self {
        BytesLru {
            map: HashMap::new(),
            bytes: 0,
            tick: 0,
            budget,
        }
    }

    /// Entries and bytes held.
    pub fn size(&self) -> (u64, u64) {
        (self.map.len() as u64, self.bytes as u64)
    }

    /// The entry, marked as recently used.
    pub fn get(&mut self, k: &str) -> Option<Bytes> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|e| {
            e.1 = t;
            e.0.clone()
        })
    }

    /// Adds an entry, evicting the least recently used beyond the budget.
    pub fn insert(&mut self, k: String, v: Bytes) {
        self.tick += 1;
        self.bytes += v.len();
        if let Some((old, _)) = self.map.insert(k, (v, self.tick)) {
            self.bytes -= old.len();
        }
        while self.bytes > self.budget && self.map.len() > 1 {
            let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, (_, t))| *t)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((v, _)) = self.map.remove(&oldest) {
                self.bytes -= v.len();
            }
        }
    }
}

/// The thumbnail service.
pub struct Thumbnails {
    shared: Arc<Shared>,
    mem: Mutex<BytesLru>,
    inflight: InFlightMap,
}

fn not_found(what: String) -> ApiError {
    ApiError::new(ErrorCode::NotFound, what)
}

impl Thumbnails {
    /// A service rendering through `shared`'s render thread.
    pub fn new(shared: Arc<Shared>) -> Self {
        Thumbnails {
            shared,
            mem: Mutex::new(BytesLru::new(MEMORY_BUDGET)),
            inflight: InFlightMap::default(),
        }
    }

    /// Entries and bytes of the memory cache.
    pub fn mem_size(&self) -> (u64, u64) {
        lock(&self.mem).size()
    }

    /// Clamps a requested width.
    pub fn clamp_width(w: Option<u32>) -> u32 {
        w.unwrap_or(protocol::DEFAULT_THUMBNAIL_WIDTH)
            .clamp(WIDTH_RANGE.0, WIDTH_RANGE.1)
    }

    /// Specs for slides of a presentation (all of them when `only` is `None`). Cheap: no
    /// rendering, one `stat` per background file.
    pub fn specs(
        &self,
        presentation: PresentationId,
        only: Option<SlideId>,
        width: u32,
    ) -> Result<Vec<Spec>, ApiError> {
        let data = self.shared.data();
        let p = data
            .content
            .presentation(presentation)
            .ok_or_else(|| not_found(format!("presentation {} not found", presentation.0)))?;
        let slides = doc_slides(p);
        let subs = serde_json::to_vec(&data.font_substitutions).unwrap_or_default();
        let canvas = p.canvas_size;
        let height = ((width as f64 * canvas.height / canvas.width.max(1.0)).round() as u32).max(1);
        let mut out = Vec::new();
        for (i, slide) in slides.iter().enumerate() {
            if only.is_some_and(|id| id != slide.id) {
                continue;
            }
            // Image backgrounds only (a video background's thumbnail shows the slide alone).
            let background = background_for(&slides, i)
                .filter(|(m, _)| {
                    data.content.assets.get(&m.asset).map(|a| a.kind)
                        == Some(model::MediaKind::Image)
                })
                .and_then(|(m, _)| {
                    data.content
                        .asset_path(m.asset)
                        .ok()
                        .map(|p| (p, m.display.clone()))
                });
            let mut h = Sha256::new();
            h.update(b"thumbnail v2\0");
            h.update(crate::VERSION.as_bytes());
            h.update(width.to_le_bytes());
            h.update(canvas.width.to_le_bytes());
            h.update(canvas.height.to_le_bytes());
            h.update(serde_json::to_vec(slide).unwrap_or_default());
            h.update(&subs);
            if let Some((path, display)) = &background {
                h.update(path.to_string_lossy().as_bytes());
                if let Ok(m) = std::fs::metadata(path) {
                    h.update(m.len().to_le_bytes());
                    if let Ok(t) = m.modified()
                        && let Ok(d) = t.duration_since(std::time::UNIX_EPOCH)
                    {
                        h.update(d.as_nanos().to_le_bytes());
                    }
                }
                h.update(serde_json::to_vec(display).unwrap_or_default());
            }
            let hash: String = h.finalize()[..8]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let dir = data
                .root
                .join("cache")
                .join("thumbnails")
                .join(presentation.0.to_string());
            let prefix = format!("{}-{width}-", slide.id.0);
            out.push(Spec {
                key: format!("{}/{prefix}{hash}", presentation.0),
                etag: format!("\"{hash}\""),
                slide: (*slide).clone(),
                canvas,
                background,
                width,
                height,
                file: dir.join(format!("{prefix}{hash}.jpg")),
                prefix,
            });
        }
        if out.is_empty()
            && let Some(id) = only
        {
            return Err(not_found(format!("slide {} not in presentation", id.0)));
        }
        Ok(out)
    }

    /// The thumbnail for a spec: memory, then disk, then a render.
    pub async fn get(&self, spec: Spec, interactive: bool) -> Result<Thumbnail, ApiError> {
        if let Some(j) = lock(&self.mem).get(&spec.key) {
            return Ok(Thumbnail {
                jpeg: j,
                etag: spec.etag,
            });
        }
        let jpeg = self
            .inflight
            .run(&spec.key, || async {
                let res = self.produce(&spec, interactive).await;
                if let Ok(j) = &res {
                    lock(&self.mem).insert(spec.key.clone(), j.clone());
                }
                res
            })
            .await?;
        Ok(Thumbnail {
            jpeg,
            etag: spec.etag,
        })
    }

    async fn produce(&self, spec: &Spec, interactive: bool) -> Result<Bytes, ApiError> {
        let internal = |e: String| ApiError::new(ErrorCode::Internal, e);
        let file = spec.file.clone();
        let cached = tokio::task::spawn_blocking(move || std::fs::read(file).ok())
            .await
            .map_err(|e| internal(e.to_string()))?;
        if let Some(b) = cached {
            return Ok(Bytes::from(b));
        }
        let (reply, rx) = oneshot::channel();
        let job = ThumbJob {
            slide: spec.slide.clone(),
            canvas: spec.canvas,
            background: spec.background.clone(),
            width: spec.width,
            height: spec.height,
            interactive,
            transparent: false,
            reply,
        };
        if !self.shared.send_render(Cmd::Thumbnail(Box::new(job))) {
            return Err(ApiError::new(
                ErrorCode::Unavailable,
                "the renderer is not running",
            ));
        }
        let frame = rx
            .await
            .map_err(|_| ApiError::new(ErrorCode::Unavailable, "the renderer stopped"))?
            .map_err(|e| internal(format!("thumbnail render failed: {e:#}")))?;
        let (file, prefix) = (spec.file.clone(), spec.prefix.clone());
        let jpeg = tokio::task::spawn_blocking(move || {
            let jpeg = encode_jpeg(&frame, None, QUALITY)?;
            if let Err(e) = write_cache(&file, &prefix, &jpeg) {
                tracing::warn!("thumbnail cache {}: {e}", file.display());
            }
            anyhow::Ok(jpeg)
        })
        .await
        .map_err(|e| internal(e.to_string()))?
        .map_err(|e| internal(format!("{e:#}")))?;
        Ok(Bytes::from(jpeg))
    }

    /// The editor's picture of a slide: the slide alone (its background colour, no background
    /// media), transparent where it draws nothing, as PNG with straight alpha, `width` px wide.
    /// Elements in `hide` are left out (the editor draws those itself while they are being
    /// edited). Never cached: the editor asks again after every edit. Rendered at interactive
    /// priority, behind any pending live change.
    pub async fn editor_png(
        &self,
        presentation: PresentationId,
        slide: SlideId,
        width: u32,
        hide: &[ElementId],
        chords: bool,
    ) -> Result<Bytes, ApiError> {
        let (mut slide, canvas) = {
            let data = self.shared.data();
            let p = data
                .content
                .presentation(presentation)
                .ok_or_else(|| not_found(format!("presentation {} not found", presentation.0)))?;
            let s = doc_slides(p)
                .into_iter()
                .find(|s| s.id == slide)
                .ok_or_else(|| not_found(format!("slide {} not in presentation", slide.0)))?;
            ((*s).clone(), p.canvas_size)
        };
        if chords {
            // The editor's chord view: every text box draws its lines' chords.
            for e in slide.elements.iter_mut() {
                if let Some(tb) = e.text_box_mut() {
                    tb.chords.show = true;
                }
            }
        }
        self.slide_png(slide, canvas, width, hide).await
    }

    /// A slide alone (no background media), transparent where it draws nothing, as a PNG
    /// `width` wide (editor renders, prop thumbnails). Elements in `hide` are left out.
    pub async fn slide_png(
        &self,
        mut slide: model::Slide,
        canvas: model::Size,
        width: u32,
        hide: &[ElementId],
    ) -> Result<Bytes, ApiError> {
        let internal = |e: String| ApiError::new(ErrorCode::Internal, e);
        slide.elements.retain(|e| !hide.contains(&e.id));
        let height = ((width as f64 * canvas.height / canvas.width.max(1.0)).round() as u32).max(1);
        let (reply, rx) = oneshot::channel();
        let job = ThumbJob {
            slide,
            canvas,
            background: None,
            width,
            height,
            interactive: true,
            transparent: true,
            reply,
        };
        if !self.shared.send_render(Cmd::Thumbnail(Box::new(job))) {
            return Err(ApiError::new(
                ErrorCode::Unavailable,
                "the renderer is not running",
            ));
        }
        let frame = rx
            .await
            .map_err(|_| ApiError::new(ErrorCode::Unavailable, "the renderer stopped"))?
            .map_err(|e| internal(format!("slide render failed: {e:#}")))?;
        let png = tokio::task::spawn_blocking(move || encode_png(&frame))
            .await
            .map_err(|e| internal(e.to_string()))?
            .map_err(|e| internal(format!("{e:#}")))?;
        Ok(Bytes::from(png))
    }

    /// Queues every slide thumbnail of a presentation that is not cached yet.
    pub async fn prefetch(
        self: &Arc<Self>,
        presentation: PresentationId,
        width: u32,
    ) -> Result<ThumbnailPrefetch, ApiError> {
        let specs = self.specs(presentation, None, width)?;
        let slides = specs.len() as u32;
        let (hot, rest): (Vec<Spec>, Vec<Spec>) = {
            let mut mem = lock(&self.mem);
            specs.into_iter().partition(|s| mem.get(&s.key).is_some())
        };
        let (on_disk, todo): (Vec<Spec>, Vec<Spec>) =
            tokio::task::spawn_blocking(move || rest.into_iter().partition(|s| s.file.exists()))
                .await
                .map_err(|e| ApiError::new(ErrorCode::Internal, e.to_string()))?;
        let cached = hot.len() + on_disk.len();
        let queued = todo.len() as u32;
        for spec in todo {
            let me = self.clone();
            tokio::spawn(async move {
                if let Err(e) = me.get(spec, false).await {
                    tracing::debug!("thumbnail prefetch: {e}");
                }
            });
        }
        Ok(ThumbnailPrefetch {
            slides,
            cached: cached as u32,
            queued,
        })
    }
}

fn write_cache(file: &std::path::Path, prefix: &str, jpeg: &[u8]) -> std::io::Result<()> {
    let dir = file.parent().unwrap_or(file);
    std::fs::create_dir_all(dir)?;
    // Drop stale versions of this slide at this width.
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with(prefix) && name.ends_with(".jpg") && e.path() != file {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let tmp = file.with_extension("jpg.tmp");
    std::fs::write(&tmp, jpeg)?;
    std::fs::rename(&tmp, file)
}

/// PNG of a premultiplied RGBA8 frame, with straight alpha (what browsers expect).
pub fn encode_png(frame: &render::RenderedFrame) -> anyhow::Result<Vec<u8>> {
    use image::ImageEncoder;
    let mut rgba = Vec::with_capacity(frame.data.len());
    for p in frame.data.as_chunks::<4>().0 {
        let a = p[3];
        if a == 0 {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
        } else if a == 255 {
            rgba.extend_from_slice(p);
        } else {
            let un = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
            rgba.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2]), a]);
        }
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        &rgba,
        frame.width,
        frame.height,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::Poll;

    fn ok(b: &'static [u8]) -> impl Fn() -> std::future::Ready<Result<Bytes, ApiError>> {
        move || std::future::ready(Ok(Bytes::from_static(b)))
    }

    /// Polls `f` once.
    async fn poll_once<F: Future>(f: &mut Pin<Box<F>>) -> Poll<F::Output> {
        std::future::poll_fn(|cx| Poll::Ready(f.as_mut().poll(cx))).await
    }

    /// A04-1: a request dropped while it renders (the browser gave up on the image) leaves
    /// nothing behind: requests waiting on it render it themselves, and later ones too.
    #[tokio::test]
    async fn a_dropped_render_does_not_wedge_its_thumbnail() {
        let m = InFlightMap::default();
        let mut owner = Box::pin(m.run("k", std::future::pending));
        let mut waiter = Box::pin(m.run("k", ok(b"jpeg")));
        assert!(poll_once(&mut owner).await.is_pending());
        assert!(
            poll_once(&mut waiter).await.is_pending(),
            "waits for the owner"
        );
        drop(owner);
        assert_eq!(&waiter.await.unwrap()[..], b"jpeg");
        assert!(lock(&m.0).is_empty());

        // Dropped with no one waiting: the next request is the producer.
        let mut owner = Box::pin(m.run("k", std::future::pending));
        assert!(poll_once(&mut owner).await.is_pending());
        drop(owner);
        assert!(lock(&m.0).is_empty());
        assert_eq!(&m.run("k", ok(b"again")).await.unwrap()[..], b"again");

        // A finished producer shares its result with the requests waiting on it.
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let rx = std::sync::Mutex::new(Some(rx));
        let mut owner = Box::pin(m.run("k", || {
            let rx = rx.lock().unwrap().take();
            async move {
                let _ = rx.unwrap().await;
                Ok(Bytes::from_static(b"shared"))
            }
        }));
        let mut waiter = Box::pin(m.run("k", || async {
            panic!("the waiter must not render");
        }));
        assert!(poll_once(&mut owner).await.is_pending());
        assert!(poll_once(&mut waiter).await.is_pending());
        tx.send(()).unwrap();
        assert_eq!(&owner.await.unwrap()[..], b"shared");
        assert_eq!(&waiter.await.unwrap()[..], b"shared");
        assert!(lock(&m.0).is_empty());
    }
}
