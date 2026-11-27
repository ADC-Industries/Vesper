//! The media worker: probes new media and makes their thumbnails, off the request path, on
//! [`super::CodexOptions::media_workers`] threads at the lowest CPU priority
//! ([`super::disk::lower_thread_priority`]; on Linux FFmpeg's own threads inherit it), so a Codex box
//! that also runs outputs keeps its frame pacing.
//!
//! - **Probe** (after an upload or a relink, [`MediaWorker::probe`]): the asset's file is
//!   probed with the media crate (IDN's `idn_ffmpeg::media` probe) and the facts saved in its
//!   `probe` through the Codex writer (`documentChanged` follows); a file that can't be read is
//!   marked `unsupported`. Its thumbnail is made on the way. The probe is shared with the media
//!   library's own background probe of the same file ([`super::library::ProbeMemo`]): each file
//!   is probed once.
//! - **Thumbnail** ([`MediaWorker::thumbnail`]): a 320 px JPEG of an image or of a video's frame
//!   at 10 % (IDN's `thumbnail`), stored as `codex/thumbs/<sha256>-320.jpg`. Asked for by a
//!   request, it goes to the front of the queue.

use super::blobs;
use crate::Shared;
use crate::Show;
use model::{ChangeBy, CollectionId, FrameRate, MediaAssetId, MediaKind, MediaProbe, MediaStatus};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

enum Job {
    Probe {
        collection: CollectionId,
        asset: MediaAssetId,
    },
    Thumb {
        file: String,
        path: PathBuf,
        reply: tokio::sync::oneshot::Sender<Result<Vec<u8>, String>>,
    },
}

struct Inner {
    queue: Mutex<VecDeque<Job>>,
    wake: Condvar,
    /// Probes queued (each asset once).
    queued: Mutex<HashSet<MediaAssetId>>,
    stop: tokio::sync::watch::Receiver<bool>,
    /// Jobs waiting or running (tests wait for zero).
    busy: Mutex<usize>,
    /// Media preparation, queued after probes.
    prep: super::prep::Prep,
}

/// The media worker's handle (cheap to clone).
#[derive(Clone)]
pub struct MediaWorker {
    inner: Arc<Inner>,
}

impl MediaWorker {
    /// Starts `threads` workers and media preparation (`auto_prep`: new media are prepared
    /// automatically, see [`super::prep`]; `converter`: how its conversions run). They stop
    /// when `stop` turns `true`.
    pub fn start(
        sh: Arc<Shared>,
        show: Arc<dyn Show>,
        threads: usize,
        auto_prep: bool,
        converter: super::prep::child::Converter,
        stop: tokio::sync::watch::Receiver<bool>,
    ) -> MediaWorker {
        let prep =
            super::prep::Prep::start(sh.clone(), show.clone(), auto_prep, converter, stop.clone());
        let inner = Arc::new(Inner {
            prep,
            queue: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            queued: Mutex::new(HashSet::new()),
            stop,
            busy: Mutex::new(0),
        });
        let rt = tokio::runtime::Handle::current();
        for i in 0..threads.max(1) {
            let (inner, sh, show, rt) = (inner.clone(), sh.clone(), show.clone(), rt.clone());
            let spawned = std::thread::Builder::new()
                .name(format!("codex-media-{i}"))
                .spawn(move || run(&inner, &sh, &show, &rt));
            if let Err(e) = spawned {
                tracing::error!("starting a media worker: {e}");
            }
        }
        MediaWorker { inner }
    }

    fn push(&self, job: Job, front: bool) {
        *crate::lock(&self.inner.busy) += 1;
        let mut q = crate::lock(&self.inner.queue);
        if front {
            q.push_front(job);
        } else {
            q.push_back(job);
        }
        self.inner.wake.notify_one();
    }

    /// Queues a probe of a collection's asset (and its thumbnail).
    pub fn probe(&self, collection: CollectionId, asset: MediaAssetId) {
        if crate::lock(&self.inner.queued).insert(asset) {
            self.push(Job::Probe { collection, asset }, false);
        }
    }

    /// The stored thumbnail of a blob (`file`: `<sha>.<ext>` at `path`), made now (first in the
    /// queue) when there is none.
    pub async fn thumbnail(&self, file: &str, path: PathBuf) -> Result<Vec<u8>, String> {
        let (reply, rx) = tokio::sync::oneshot::channel();
        self.push(
            Job::Thumb {
                file: file.to_string(),
                path,
                reply,
            },
            true,
        );
        rx.await
            .map_err(|_| "the media worker stopped".to_string())?
    }

    /// Media preparation.
    pub fn prep(&self) -> &super::prep::Prep {
        &self.inner.prep
    }

    /// Jobs queued or running.
    pub fn busy(&self) -> usize {
        *crate::lock(&self.inner.busy)
    }

    /// Waits until no job is queued or running (tests), at most `timeout`.
    pub async fn idle(&self, timeout: Duration) -> bool {
        let end = std::time::Instant::now() + timeout;
        while self.busy() > 0 {
            if std::time::Instant::now() > end {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        true
    }
}

fn run(inner: &Inner, sh: &Arc<Shared>, show: &dyn Show, rt: &tokio::runtime::Handle) {
    super::disk::lower_thread_priority();
    loop {
        let job = {
            let mut q = crate::lock(&inner.queue);
            loop {
                if *inner.stop.borrow() {
                    return;
                }
                if let Some(j) = q.pop_front() {
                    break j;
                }
                q = inner
                    .wake
                    .wait_timeout(q, Duration::from_millis(500))
                    .map(|(g, _)| g)
                    .unwrap_or_else(|e| e.into_inner().0);
            }
        };
        match job {
            Job::Probe { collection, asset } => {
                crate::lock(&inner.queued).remove(&asset);
                if probe_asset(sh, show, rt, collection, asset, &inner.stop)
                    && let Some(a) = sh
                        .collection_content(collection)
                        .and_then(|c| c.assets.get(&asset).cloned())
                {
                    inner.prep.after_probe(sh, collection, &a);
                }
            }
            Job::Thumb { file, path, reply } => {
                let codex_dir = sh.codex().layout.codex();
                let _ = reply.send(thumbnail(&codex_dir, &file, &path));
            }
        }
        let mut b = crate::lock(&inner.busy);
        *b = b.saturating_sub(1);
    }
}

/// A blob's thumbnail: the stored one, else made and stored.
pub fn stored_thumbnail(codex_dir: &Path, file: &str, path: &Path) -> Result<Vec<u8>, String> {
    thumbnail(codex_dir, file, path)
}

fn thumbnail(codex_dir: &Path, file: &str, path: &Path) -> Result<Vec<u8>, String> {
    let (sha, ext) = blobs::parse_blob_name(file).ok_or("not a blob name")?;
    let stored = blobs::thumb_path(codex_dir, &sha, blobs::THUMB_WIDTH);
    if let Ok(b) = std::fs::read(&stored) {
        return Ok(b);
    }
    let kind = blobs::kind_for(&ext).unwrap_or(MediaKind::Unknown);
    let jpeg = make_thumbnail(path, kind)?;
    if let Some(d) = stored.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Err(e) = crate::store::write_atomic(&stored, &jpeg) {
        tracing::warn!(file = %stored.display(), "storing a thumbnail: {e:#}");
    }
    Ok(jpeg)
}

/// A 320 px JPEG of an image, or of a video's poster frame (the first of a few frames from
/// 10 % on that isn't black: `media::poster_frame`, over IDN's thumbnail).
fn make_thumbnail(path: &Path, kind: MediaKind) -> Result<Vec<u8>, String> {
    let t = match kind {
        MediaKind::Image => media::thumbnail(path, 0.0, blobs::THUMB_WIDTH),
        MediaKind::Video => media::poster_frame(path, blobs::THUMB_WIDTH),
        _ => return Err("no picture in this file".into()),
    }
    .map_err(|e| format!("{e:#}"))?;
    let rgb = image::RgbImage::from_raw(t.width, t.height, t.rgb).ok_or("bad picture")?;
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, super::blobs::THUMB_QUALITY)
        .encode_image(&rgb)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// A frame rate from frames per second (NTSC rates as n/1001).
fn frame_rate(fps: f64) -> FrameRate {
    if (fps - fps.round()).abs() < 0.005 {
        FrameRate {
            num: fps.round() as u32,
            den: 1,
        }
    } else {
        FrameRate {
            num: (fps * 1001.0).round() as u32,
            den: 1001,
        }
    }
}

/// The media facts of a file (IDN's probe).
pub fn probe_file(path: &Path, kind: MediaKind) -> Result<MediaProbe, String> {
    if kind == MediaKind::Audio {
        let a = media::probe_audio(path)
            .map_err(|e| format!("{e:#}"))?
            .ok_or("no audio stream")?;
        return Ok(MediaProbe {
            duration: a.duration,
            codec: Some(a.codec),
            has_audio: Some(true),
            ..MediaProbe::default()
        });
    }
    let v = media::probe(path).map_err(|e| format!("{e:#}"))?;
    let (w, h) = if v.rotation % 180 == 90 {
        (v.height, v.width)
    } else {
        (v.width, v.height)
    };
    let video = kind == MediaKind::Video;
    Ok(MediaProbe {
        width: Some(w),
        height: Some(h),
        duration: if video { v.duration } else { None },
        frame_rate: video.then(|| frame_rate(v.fps)),
        has_audio: if video {
            Some(matches!(media::probe_audio(path), Ok(Some(_))))
        } else {
            None
        },
        has_alpha: Some(v.alpha),
        codec: Some(v.codec),
        from_import_hints: false,
    })
}

fn probe_asset(
    sh: &Arc<Shared>,
    show: &dyn Show,
    rt: &tokio::runtime::Handle,
    collection: CollectionId,
    asset: MediaAssetId,
    stop: &tokio::sync::watch::Receiver<bool>,
) -> bool {
    let Some(content) = sh.collection_content(collection) else {
        return false;
    };
    let Some(a) = content.assets.get(&asset).cloned() else {
        return false;
    };
    let Ok(path) = content.asset_path(asset) else {
        return false;
    };
    let probed = sh.library().probes.probe(&path, a.kind);
    if let Err(e) = &probed {
        tracing::info!(asset = %asset.0, "Codex: media can't be probed: {e}");
    }
    if let Some(file) = blobs::stored_name(&a.location, &path)
        && matches!(a.kind, MediaKind::Image | MediaKind::Video)
        && probed.is_ok()
        && let Err(e) = thumbnail(&sh.codex().layout.codex(), &file, &path)
    {
        tracing::info!(asset = %asset.0, "Codex: no thumbnail: {e}");
    }
    if *stop.borrow() {
        return false;
    }
    let probed_ok = probed.is_ok();
    let by = ChangeBy {
        client: "media probe".into(),
        client_id: None,
        machine: sh.auth().identity().id.to_string(),
    };
    let saved = rt.block_on(sh.codex_writer().media(
        sh,
        show,
        collection,
        |content| {
            let mut plan = super::writer::MediaPlan::default();
            // Unless it moved or went meanwhile.
            if let Some(cur) = content.assets.get(&asset)
                && cur.location == a.location
            {
                let mut cur = cur.clone();
                match probed {
                    Ok(p) => {
                        cur.probe = Some(p);
                        if cur.status != MediaStatus::Online {
                            cur.status = MediaStatus::Online;
                        }
                    }
                    Err(_) => cur.status = MediaStatus::Unsupported,
                }
                plan.assets.push(cur);
            }
            Ok(plan)
        },
        None,
        by,
    ));
    if let Err(e) = saved {
        tracing::warn!(asset = %asset.0, "Codex: saving a probe: {}", e.error.message);
        return false;
    }
    probed_ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rates() {
        assert_eq!(frame_rate(25.0), FrameRate { num: 25, den: 1 });
        assert_eq!(
            frame_rate(29.97),
            FrameRate {
                num: 30000,
                den: 1001
            }
        );
    }
}
