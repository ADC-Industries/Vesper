//! Media assets, media bins and media thumbnails.

use super::edit::{create_media_bin, delete_media_bin, edit_media_bin, update_media_bin};
use super::*;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/media/{id}", get(media))
        .route("/media/{id}/file", get(media_file))
        .route("/media/{id}/thumbnail", get(media_thumbnail))
        .route("/media-bins", get(media_bins).post(create_media_bin))
        .route(
            "/media-bins/{id}",
            get(media_bin)
                .put(update_media_bin)
                .delete(delete_media_bin),
        )
        .route("/media-bins/{id}/edits", post(edit_media_bin))
}

async fn media(State(app): State<AppState>, Path(id): Path<String>) -> ApiResult<MediaAssetInfo> {
    let id: MediaAssetId = parse_id(&id, "media id")?;
    let d = app.shared.data();
    let asset = d.content.assets.get(&id).ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("media asset {} not found", id.0),
        )
    })?;
    let path = d.content.asset_path(id);
    let mut asset = asset.clone();
    // Durations of video and audio files that were never probed (one probe per file, cached).
    if matches!(asset.kind, MediaKind::Video | MediaKind::Audio)
        && asset.probe.as_ref().and_then(|p| p.duration).is_none()
    {
        let (sh, dd, a) = (app.shared.clone(), d.clone(), asset.clone());
        let dur = tokio::task::spawn_blocking(move || media_duration(&sh, &dd, &a))
            .await
            .ok()
            .flatten();
        if let Some(s) = dur {
            asset.probe.get_or_insert_with(Default::default).duration = Some(s);
        }
    }
    ok(MediaAssetInfo {
        asset,
        online: path.is_ok(),
        problem: path.err(),
    })
}

async fn media_file(
    State(app): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Result<Response, Error> {
    let id: MediaAssetId = parse_id(&id, "media id")?;
    let path = app
        .shared
        .data()
        .content
        .asset_path(id)
        .map_err(|e| err(ErrorCode::NotFound, e))?;
    // ServeFile handles Range / If-Modified-Since and the content type.
    let res = tower_http::services::ServeFile::new(path)
        .oneshot(req)
        .await
        .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
    Ok(res.map(Body::new))
}

fn bin_items(b: &MediaBin) -> &[model::MediaBinItem] {
    match &b.kind {
        MediaBinKind::Manual { items } => items,
        MediaBinKind::Watched { last_scan, .. } => last_scan,
        _ => &[],
    }
}

fn bin_type(b: &MediaBin) -> MediaBinNodeType {
    match b.kind {
        MediaBinKind::Watched { .. } => MediaBinNodeType::Watched,
        _ => MediaBinNodeType::Manual,
    }
}

/// The bin tree: folders from the bins' folder paths (sub-folders first, by name), then the
/// bins of each folder in their stored order.
pub fn bin_tree(bins: &[MediaBin]) -> Vec<MediaBinNode> {
    fn build(bins: &[&MediaBin], path: &[String]) -> Vec<MediaBinNode> {
        let depth = path.len();
        let mut folders: Vec<String> = bins
            .iter()
            .filter(|b| b.folder.len() > depth && b.folder[..depth] == *path)
            .map(|b| b.folder[depth].clone())
            .collect();
        folders.sort_by_key(|f| f.to_lowercase());
        folders.dedup();
        let mut out = Vec::new();
        for f in folders {
            let mut p = path.to_vec();
            p.push(f.clone());
            let children = build(bins, &p);
            out.push(MediaBinNode {
                id: None,
                name: f,
                item_count: children.iter().map(|c| c.item_count).sum(),
                path: p,
                kind: MediaBinNodeType::Folder,
                browser: BinBrowser::Media,
                children,
            });
        }
        for b in bins.iter().filter(|b| b.folder == path) {
            out.push(MediaBinNode {
                id: Some(b.id),
                name: b.name.clone(),
                path: b.folder.clone(),
                kind: bin_type(b),
                browser: b.browser,
                item_count: bin_items(b).len() as u32,
                children: Vec::new(),
            });
        }
        out
    }
    let refs: Vec<&MediaBin> = bins.iter().collect();
    build(&refs, &[])
}

async fn media_bins(State(app): State<AppState>) -> ApiResult<Vec<MediaBinNode>> {
    ok(bin_tree(&app.shared.data().content.media_bins))
}

async fn media_bin(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<MediaBinDetail> {
    let id: MediaBinId = parse_id(&id, "media bin id")?;
    let d = app.shared.data();
    if !d.content.media_bins.iter().any(|b| b.id == id) {
        return Err(err(
            ErrorCode::NotFound,
            format!("media bin {} not found", id.0),
        ));
    }
    // One `stat` per item: off the async threads.
    let sh = app.shared.clone();
    let detail = tokio::task::spawn_blocking(move || {
        let b = d.content.media_bins.iter().find(|b| b.id == id)?;
        let items = bin_items(b)
            .iter()
            .map(|it| {
                let asset = d.content.assets.get(&it.trigger.asset);
                MediaBinItemInfo {
                    id: it.id,
                    name: it
                        .name
                        .clone()
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| d.content.asset_name(it.trigger.asset)),
                    kind: asset.map(|a| a.kind).unwrap_or(MediaKind::Unknown),
                    asset: it.trigger.asset,
                    availability: crate::codex::relink::availability(&d.content, it.trigger.asset)
                        .0,
                    enabled: it.enabled,
                    duration: asset.and_then(|a| media_duration(&sh, &d, a)),
                }
            })
            .collect();
        Some(MediaBinDetail {
            id: b.id,
            name: b.name.clone(),
            path: b.folder.clone(),
            kind: bin_type(b),
            browser: b.browser,
            items,
        })
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))?
    .ok_or_else(|| err(ErrorCode::NotFound, "media bin not found"))?;
    ok(detail)
}

/// Length of a video or audio asset: its import probe, else the file's (probed once, cached).
fn media_duration(shared: &Shared, d: &ShowData, a: &model::MediaAsset) -> Option<f64> {
    if !matches!(a.kind, MediaKind::Video | MediaKind::Audio) {
        return None;
    }
    if let Some(s) = a
        .probe
        .as_ref()
        .and_then(|p| p.duration)
        .filter(|s| *s > 0.0)
    {
        return Some((s * 1000.0).round() / 1000.0);
    }
    let path = d.content.asset_path(a.id).ok()?;
    if let Some(v) = lock(&shared.video_durations).get(&path) {
        return *v;
    }
    let v = if a.kind == MediaKind::Video {
        ::media::probe(&path).ok().and_then(|i| i.duration)
    } else {
        ::media::probe_audio(&path)
            .ok()
            .flatten()
            .and_then(|i| i.duration)
    }
    .map(|s| (s * 1000.0).round() / 1000.0);
    lock(&shared.video_durations).insert(path, v);
    v
}

/// A generic picture for media without a still (video, audio): dark grey 16:9 with a light
/// play triangle (video) or three bars (audio).
pub(crate) fn placeholder_thumbnail(kind: MediaKind, width: u32) -> image::RgbImage {
    let h = (width * 9 / 16).max(1);
    let mut img = image::RgbImage::from_pixel(width, h, image::Rgb([38, 38, 42]));
    let (cx, cy) = (width as f32 / 2.0, h as f32 / 2.0);
    let r = h as f32 * 0.22;
    let fg = image::Rgb([150, 150, 160]);
    for y in 0..h {
        for x in 0..width {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let on = match kind {
                MediaKind::Audio => {
                    let bar = r * 0.35;
                    [-1.5f32, 0.0, 1.5].iter().enumerate().any(|(i, k)| {
                        let height = r * [0.8, 1.3, 0.6][i];
                        (dx - k * bar * 1.4).abs() <= bar / 2.0 && dy.abs() <= height
                    })
                }
                // A triangle pointing right.
                _ => dx >= -r * 0.6 && dx <= r && dy.abs() <= (r - dx) * 0.6,
            };
            if on {
                img.put_pixel(x, y, fg);
            }
        }
    }
    img
}

async fn media_thumbnail(
    State(app): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    req: Request,
) -> Result<Response, Error> {
    let id: MediaAssetId = parse_id(&id, "media id")?;
    let w = query_param(req.uri().query(), "w").and_then(|w| w.parse().ok());
    let width = Thumbnails::clamp_width(w);
    let d = app.shared.data();
    let asset = d.content.assets.get(&id).ok_or_else(|| {
        err(
            ErrorCode::NotFound,
            format!("media asset {} not found", id.0),
        )
    })?;
    let kind = asset.kind;
    let path = if matches!(kind, MediaKind::Image | MediaKind::Video) {
        match d.content.asset_path(id) {
            Ok(p) => Some(p),
            Err(e) if kind == MediaKind::Image => return Err(err(ErrorCode::NotFound, e)),
            // A missing video gets the placeholder.
            Err(_) => None,
        }
    } else {
        None
    };
    // ETag: asset, file version, width, server version.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(id.0, width, crate::VERSION, format!("{kind:?}")), &mut h);
    if let Some(p) = &path
        && let Ok(m) = std::fs::metadata(p)
    {
        std::hash::Hash::hash(&(p, m.len(), m.modified().ok()), &mut h);
    }
    let etag = format!("\"m{:016x}\"", std::hash::Hasher::finish(&h));
    let cache = [
        (
            header::ETAG,
            HeaderValue::from_str(&etag).unwrap_or(HeaderValue::from_static("\"\"")),
        ),
        (header::CACHE_CONTROL, HeaderValue::from_static("no-cache")),
    ];
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag))
    {
        return Ok((StatusCode::NOT_MODIFIED, cache).into_response());
    }
    let decoder = match &path {
        Some(_) if kind == MediaKind::Image => Some(
            app.shared
                .decoder
                .get()
                .cloned()
                .ok_or_else(|| err(ErrorCode::Unavailable, "the renderer is not running"))?,
        ),
        _ => None,
    };
    let _permit = app
        .media_thumbs
        .acquire()
        .await
        .map_err(|_| err(ErrorCode::Unavailable, "shutting down"))?;
    let video_key = path
        .as_ref()
        .filter(|_| kind == MediaKind::Video)
        .map(|p| format!("{etag}|{}", p.display()));
    if let Some(k) = &video_key
        && let Some(jpeg) = lock(&app.shared.video_thumbs).get(k)
    {
        return Ok((
            cache,
            [(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"))],
            Body::from(jpeg),
        )
            .into_response());
    }
    let video_path = path.clone().filter(|_| kind == MediaKind::Video);
    let jpeg = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, Error> {
        if let Some(p) = video_path {
            // The frame at about 10 % in (past black openings), software decode.
            let rgb = match ::media::thumbnail(&p, 0.1, width) {
                Ok(t) => image::RgbImage::from_raw(t.width, t.height, t.rgb)
                    .unwrap_or_else(|| placeholder_thumbnail(kind, width)),
                Err(e) => {
                    tracing::debug!("video thumbnail of {}: {e:#}", p.display());
                    placeholder_thumbnail(kind, width)
                }
            };
            let mut out = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, crate::thumbs::QUALITY)
                .encode_image(&rgb)
                .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
            return Ok(out);
        }
        let rgb = match (path, decoder) {
            (Some(path), Some(decoder)) => {
                let boxh = (width * 9 / 16).max(1);
                let decoded = match decoder.decode_now(&path, Default::default(), (width, boxh)) {
                    render::ImageStatus::Ready(i) => i,
                    render::ImageStatus::Failed(e) => {
                        return Err(err(ErrorCode::NotFound, format!("cannot decode: {e}")));
                    }
                    render::ImageStatus::Pending => {
                        return Err(err(ErrorCode::Unavailable, "still decoding"));
                    }
                };
                let px = &decoded.pixels;
                let h =
                    ((px.height() as u64 * width as u64) / px.width().max(1) as u64).max(1) as u32;
                let img = if px.width() > width {
                    image::imageops::resize(px, width, h, image::imageops::FilterType::Triangle)
                } else {
                    px.clone()
                };
                // Flatten over the dark checker-free panel colour.
                let mut rgb = image::RgbImage::new(img.width(), img.height());
                for (x, y, p) in img.enumerate_pixels() {
                    let a = p.0[3] as u32;
                    let bg = 24u32;
                    let mix = |c: u8| ((c as u32 * a + bg * (255 - a)) / 255) as u8;
                    rgb.put_pixel(x, y, image::Rgb([mix(p.0[0]), mix(p.0[1]), mix(p.0[2])]));
                }
                rgb
            }
            _ => placeholder_thumbnail(kind, width),
        };
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, crate::thumbs::QUALITY)
            .encode_image(&rgb)
            .map_err(|e| err(ErrorCode::Internal, e.to_string()))?;
        Ok(out)
    })
    .await
    .map_err(|e| err(ErrorCode::Internal, e.to_string()))??;
    if let Some(k) = video_key {
        lock(&app.shared.video_thumbs).insert(k, Bytes::from(jpeg.clone()));
    }
    Ok((
        cache,
        [(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"))],
        Body::from(jpeg),
    )
        .into_response())
}
