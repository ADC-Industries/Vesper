//! Image decoding off the render thread: a small worker pool in front of a decoded-image cache.
//!
//! - **Who decodes:** [`ImageDecoder`] owns a few worker threads (default
//!   [`DEFAULT_DECODE_THREADS`]). The render thread never decodes a file when the renderer runs
//!   with [`RendererOptions::wait_for_images`](crate::RendererOptions::wait_for_images) off: it
//!   asks the decoder, uploads what is ready and leaves out what is not (see
//!   [`Renderer::missing_images`](crate::Renderer::missing_images)). With the option on (the
//!   default, for tools and tests) a missing image is decoded on the calling thread instead.
//! - **Priorities:** [`DecodePriority::Live`] before [`DecodePriority::Prefetch`] before
//!   [`DecodePriority::Thumbnail`]. Asking again with a more urgent priority moves a queued
//!   request up. Worker 0 only takes live and prefetch requests, so a burst of huge thumbnail
//!   decodes never leaves a live background waiting for a free worker.
//! - **Decode size:** a request names the box the image is drawn into (the screen or thumbnail
//!   size, in pixels). The image is decoded and downscaled so that it just *covers* that box
//!   (scale `max(box_w / w, box_h / h)`, never upscaled, at most [`MAX_IMAGE_SIDE`]), so a
//!   3840×2400 photo on a 1920×1080 screen becomes 1920×1200 and the render thread only uploads
//!   that. JPEGs use the decoder's DCT scaling (1/2, 1/4, 1/8), which makes thumbnail decodes
//!   of large photos several times faster.
//! - **Cache:** decoded straight-alpha RGBA8 images keyed by file (path, size, modification
//!   time), source-alpha interpretation and box size; a decode for a larger box (or a full-size
//!   one) also serves smaller boxes. Bounded by bytes (least recently used first). Failures are
//!   cached too, per file version. Live and thumbnail rendering share one decoder.
//! - **Completion:** the callback given to [`ImageDecoder::set_notify`] runs on the worker
//!   after every finished decode, so the owner of the renderer can re-render.

use crate::media::MAX_IMAGE_SIDE;
use image::ImageDecoder as _;
use image::metadata::Orientation;
use model::SourceAlpha;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::SystemTime;

/// Worker threads of a decoder created by the renderer.
pub const DEFAULT_DECODE_THREADS: usize = 3;
/// Default byte budget of the decoded-image cache.
pub const DEFAULT_DECODE_CACHE_BYTES: u64 = 384 << 20;

/// How urgently an image is needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DecodePriority {
    /// A live screen is waiting for it.
    Live = 0,
    /// Probably needed soon (the next slide).
    Prefetch = 1,
    /// A thumbnail is waiting for it.
    Thumbnail = 2,
}

/// A decoded image.
pub struct DecodedImage {
    /// Straight-alpha sRGB pixels (source alpha interpretation applied), possibly downscaled.
    pub pixels: image::RgbaImage,
    /// Width of the file's image, px (before any downscale).
    pub source_width: u32,
    /// Height of the file's image, px.
    pub source_height: u32,
    /// Decoded at full size (it serves every box).
    pub full_size: bool,
}

impl DecodedImage {
    fn bytes(&self) -> u64 {
        self.pixels.as_raw().len() as u64
    }
}

/// The state of a requested image.
#[derive(Clone)]
pub enum ImageStatus {
    /// Decoded.
    Ready(Arc<DecodedImage>),
    /// Queued or decoding.
    Pending,
    /// It cannot be decoded (missing, not an image, corrupt).
    Failed(String),
}

/// Counters of a decoder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodeStats {
    /// Bytes held by the cache.
    pub cached_bytes: u64,
    /// Cache budget, bytes.
    pub budget_bytes: u64,
    /// Cached decodes (failures included).
    pub cached_entries: usize,
    /// Requests waiting for a worker.
    pub queued: usize,
    /// Decodes running now.
    pub running: usize,
    /// Decodes finished so far (failures included).
    pub decoded: u64,
    /// Cache entries evicted so far.
    pub evicted: u64,
}

/// A file version: path, size and modification time.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FileKey {
    path: PathBuf,
    len: u64,
    mtime: Option<SystemTime>,
}

impl FileKey {
    /// Stats the file (`Err` = it does not exist).
    pub fn of(path: &Path) -> Result<FileKey, String> {
        let m = std::fs::metadata(path).map_err(|_| "file not found".to_string())?;
        Ok(FileKey {
            path: path.to_path_buf(),
            len: m.len(),
            mtime: m.modified().ok(),
        })
    }
}

/// What a decode request names: a file version, alpha interpretation and box size.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DecodeKey {
    pub file: FileKey,
    pub alpha: SourceAlpha,
    pub size: (u32, u32),
}

struct Variant {
    size: (u32, u32),
    result: Result<Arc<DecodedImage>, String>,
    tick: u64,
    bytes: u64,
}

impl Variant {
    fn covers(&self, size: (u32, u32)) -> bool {
        match &self.result {
            Err(_) => true,
            Ok(img) => img.full_size || (self.size.0 >= size.0 && self.size.1 >= size.1),
        }
    }
}

type Notify = Arc<dyn Fn() + Send + Sync>;

struct Inner {
    cache: HashMap<(FileKey, SourceAlpha), Vec<Variant>>,
    bytes: u64,
    budget: u64,
    tick: u64,
    queues: [VecDeque<DecodeKey>; 3],
    queued: HashMap<DecodeKey, DecodePriority>,
    running: HashSet<DecodeKey>,
    notify: Option<Notify>,
    stop: bool,
    decoded: u64,
    evicted: u64,
}

/// Size counted for a cached failure.
const FAILURE_BYTES: u64 = 256;

impl Inner {
    fn lookup(&mut self, key: &DecodeKey) -> Option<ImageStatus> {
        self.tick += 1;
        let t = self.tick;
        let v = self
            .cache
            .get_mut(&(key.file.clone(), key.alpha))?
            .iter_mut()
            .filter(|v| v.covers(key.size))
            // the smallest variant that covers the box
            .min_by_key(|v| v.bytes)?;
        v.tick = t;
        Some(match &v.result {
            Ok(img) => ImageStatus::Ready(img.clone()),
            Err(e) => ImageStatus::Failed(e.clone()),
        })
    }

    fn insert(&mut self, key: DecodeKey, result: Result<Arc<DecodedImage>, String>) {
        self.tick += 1;
        let bytes = match &result {
            Ok(i) => i.bytes(),
            Err(_) => FAILURE_BYTES,
        };
        let list = self.cache.entry((key.file, key.alpha)).or_default();
        if let Some(i) = list.iter().position(|v| v.size == key.size) {
            self.bytes -= list.remove(i).bytes;
        }
        list.push(Variant {
            size: key.size,
            result,
            tick: self.tick,
            bytes,
        });
        self.bytes += bytes;
        self.evict();
    }

    fn evict(&mut self) {
        while self.bytes > self.budget {
            let mut oldest: Option<((FileKey, SourceAlpha), usize, u64)> = None;
            let mut count = 0;
            for (k, list) in &self.cache {
                for (i, v) in list.iter().enumerate() {
                    count += 1;
                    if oldest.as_ref().is_none_or(|o| v.tick < o.2) {
                        oldest = Some((k.clone(), i, v.tick));
                    }
                }
            }
            // The newest entry stays even when it alone is over budget.
            let Some((k, i, _)) = oldest.filter(|_| count > 1) else {
                break;
            };
            if let Some(list) = self.cache.get_mut(&k) {
                self.bytes -= list.remove(i).bytes;
                if list.is_empty() {
                    self.cache.remove(&k);
                }
            }
            self.evicted += 1;
        }
    }

    /// The next request for a worker (`live_only`: live and prefetch requests only).
    fn pop(&mut self, live_only: bool) -> Option<DecodeKey> {
        let levels = if live_only { 2 } else { 3 };
        for p in 0..levels {
            while let Some(k) = self.queues[p].pop_front() {
                // Upgraded requests leave a stale copy in the lower queue.
                if self.queued.get(&k).is_some_and(|q| *q as usize == p) {
                    self.queued.remove(&k);
                    self.running.insert(k.clone());
                    return Some(k);
                }
            }
        }
        None
    }
}

struct Pool {
    inner: Mutex<Inner>,
    cv: Condvar,
}

impl Pool {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The decode worker pool and decoded-image cache (see the module docs). Cheap to share
/// (`Arc`); dropping the last handle stops the workers after their current decode.
pub struct ImageDecoder {
    pool: Arc<Pool>,
}

impl ImageDecoder {
    /// Starts `threads` workers (at least 1) with a cache of `budget` bytes.
    pub fn new(threads: usize, budget: u64) -> Arc<Self> {
        let pool = Arc::new(Pool {
            inner: Mutex::new(Inner {
                cache: HashMap::new(),
                bytes: 0,
                budget,
                tick: 0,
                queues: Default::default(),
                queued: HashMap::new(),
                running: HashSet::new(),
                notify: None,
                stop: false,
                decoded: 0,
                evicted: 0,
            }),
            cv: Condvar::new(),
        });
        let threads = threads.max(1);
        for i in 0..threads {
            let pool = pool.clone();
            // With several workers, worker 0 is kept for live / prefetch decodes.
            let live_only = threads > 1 && i == 0;
            let spawned = std::thread::Builder::new()
                .name(format!("decode {i}"))
                .spawn(move || worker(&pool, live_only));
            if let Err(e) = spawned {
                log::error!("render: cannot start decode worker: {e}");
            }
        }
        Arc::new(ImageDecoder { pool })
    }

    /// Sets the callback run (on a worker thread) after every finished decode.
    pub fn set_notify(&self, f: impl Fn() + Send + Sync + 'static) {
        self.pool.lock().notify = Some(Arc::new(f));
    }

    /// The image of `path` for a `size` box: cached, or queued with `priority` (an already
    /// queued request moves up when `priority` is more urgent).
    pub fn request(
        &self,
        path: &Path,
        alpha: SourceAlpha,
        size: (u32, u32),
        priority: DecodePriority,
    ) -> ImageStatus {
        let key = match key_for(path, alpha, size) {
            Ok(k) => k,
            Err(e) => return ImageStatus::Failed(e),
        };
        let mut inner = self.pool.lock();
        if let Some(s) = inner.lookup(&key) {
            return s;
        }
        if inner.running.contains(&key) {
            return ImageStatus::Pending;
        }
        match inner.queued.get(&key) {
            Some(q) if *q <= priority => {}
            _ => {
                inner.queued.insert(key.clone(), priority);
                inner.queues[priority as usize].push_back(key);
                self.pool.cv.notify_all();
            }
        }
        ImageStatus::Pending
    }

    /// The image of `path` for a `size` box, decoding it on the calling thread when it is not
    /// cached (for callers that would rather wait than draw without it).
    pub fn decode_now(&self, path: &Path, alpha: SourceAlpha, size: (u32, u32)) -> ImageStatus {
        let key = match key_for(path, alpha, size) {
            Ok(k) => k,
            Err(e) => return ImageStatus::Failed(e),
        };
        {
            let mut inner = self.pool.lock();
            if let Some(s) = inner.lookup(&key) {
                return s;
            }
            // Ours now; a stale queue entry is skipped by the workers.
            inner.queued.remove(&key);
        }
        let result = decode_file(&key.file.path, alpha, size).map(Arc::new);
        let status = match &result {
            Ok(i) => ImageStatus::Ready(i.clone()),
            Err(e) => ImageStatus::Failed(e.clone()),
        };
        let mut inner = self.pool.lock();
        inner.decoded += 1;
        inner.insert(key, result);
        status
    }

    /// The cached image for `path` and `size`, without queueing anything (`None` = not
    /// decoded yet).
    pub fn cached(&self, path: &Path, alpha: SourceAlpha, size: (u32, u32)) -> Option<ImageStatus> {
        let key = key_for(path, alpha, size).ok()?;
        self.pool.lock().lookup(&key)
    }

    /// Counters.
    pub fn stats(&self) -> DecodeStats {
        let inner = self.pool.lock();
        DecodeStats {
            cached_bytes: inner.bytes,
            budget_bytes: inner.budget,
            cached_entries: inner.cache.values().map(Vec::len).sum(),
            queued: inner.queued.len(),
            running: inner.running.len(),
            decoded: inner.decoded,
            evicted: inner.evicted,
        }
    }
}

impl Drop for ImageDecoder {
    fn drop(&mut self) {
        let mut inner = self.pool.lock();
        inner.stop = true;
        inner.notify = None;
        inner.queues.iter_mut().for_each(VecDeque::clear);
        inner.queued.clear();
        self.pool.cv.notify_all();
    }
}

fn worker(pool: &Pool, live_only: bool) {
    loop {
        let key = {
            let mut inner = pool.lock();
            loop {
                if inner.stop {
                    return;
                }
                if let Some(k) = inner.pop(live_only) {
                    break k;
                }
                inner = pool.cv.wait(inner).unwrap_or_else(|e| e.into_inner());
            }
        };
        let t = std::time::Instant::now();
        let result = decode_file(&key.file.path, key.alpha, key.size).map(Arc::new);
        log::debug!(
            "render: decoded {} for {}x{} in {:.0} ms",
            key.file.path.display(),
            key.size.0,
            key.size.1,
            t.elapsed().as_secs_f64() * 1000.0
        );
        let notify = {
            let mut inner = pool.lock();
            inner.running.remove(&key);
            inner.decoded += 1;
            inner.insert(key, result);
            inner.notify.clone()
        };
        if let Some(n) = notify {
            n();
        }
    }
}

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "m4v", "avi", "mkv", "webm", "wmv", "mpg", "mpeg", "m2ts", "mts", "flv", "mxf",
    "ts", "ogv", "3gp",
];
const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "wav", "aac", "m4a", "flac", "ogg", "aif", "aiff", "wma",
];

/// `Err` for files that are not still images (video, audio).
pub(crate) fn check_extension(path: &Path) -> Result<(), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if VIDEO_EXTENSIONS.contains(&ext.as_str()) {
        return Err("video playback is not supported yet (M1 renders still images only)".into());
    }
    if AUDIO_EXTENSIONS.contains(&ext.as_str()) {
        return Err("audio file has no picture".into());
    }
    Ok(())
}

/// The request key for a file (`Err` = it can never be decoded: video, audio, missing).
fn key_for(path: &Path, alpha: SourceAlpha, size: (u32, u32)) -> Result<DecodeKey, String> {
    check_extension(path)?;
    Ok(DecodeKey {
        file: FileKey::of(path)?,
        alpha,
        size: (size.0.max(1), size.1.max(1)),
    })
}

/// The size an image of `w × h` is decoded at for a `size` box: just covering the box, never
/// upscaled, at most [`MAX_IMAGE_SIDE`].
pub(crate) fn decode_size(w: u32, h: u32, size: (u32, u32)) -> (u32, u32) {
    let (w, h) = (w.max(1) as f64, h.max(1) as f64);
    let s = (size.0.max(1) as f64 / w)
        .max(size.1.max(1) as f64 / h)
        .min(MAX_IMAGE_SIDE as f64 / w.max(h))
        .min(1.0);
    (
        ((w * s).ceil() as u32).max(1),
        ((h * s).ceil() as u32).max(1),
    )
}

/// Whether an EXIF orientation turns the picture by a quarter (width and height swap).
fn swaps_axes(o: Orientation) -> bool {
    matches!(
        o,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

/// The decode size ([`decode_size`]) of a picture stored `sw × sh` and shown turned by `o`,
/// in stored (unturned) pixels.
fn stored_decode_size(sw: u32, sh: u32, o: Orientation, size: (u32, u32)) -> (u32, u32) {
    if swaps_axes(o) {
        let (h, w) = decode_size(sh, sw, size);
        (w, h)
    } else {
        decode_size(sw, sh, size)
    }
}

/// Decodes a file for a `size` box to straight-alpha RGBA8, applying the source alpha
/// interpretation and the EXIF orientation (a phone's portrait photo is stored sideways
/// with "turn 90°" in its EXIF data): the picture and its source size are as shown.
pub(crate) fn decode_file(
    path: &Path,
    alpha: SourceAlpha,
    size: (u32, u32),
) -> Result<DecodedImage, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let (img, sw, sh, orientation) = match decode_jpeg_scaled(&bytes, size) {
        Some(r) => r,
        None => {
            let mut decoder = image::ImageReader::new(std::io::Cursor::new(&bytes))
                .with_guessed_format()
                .map_err(|e| e.to_string())?
                .into_decoder()
                .map_err(|e| e.to_string())?;
            let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
            let img = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
            let (w, h) = (img.width(), img.height());
            (img, w, h, orientation)
        }
    };
    let (tw, th) = stored_decode_size(sw, sh, orientation, size);
    let mut img = if img.width() > tw || img.height() > th {
        // Area-averaging downscale: fast and alias-free for large ratios.
        img.thumbnail_exact(tw, th)
    } else {
        img
    };
    // Turned after the downscale: fewer pixels to move.
    img.apply_orientation(orientation);
    let (sw, sh, tw, th) = if swaps_axes(orientation) {
        (sh, sw, th, tw)
    } else {
        (sw, sh, tw, th)
    };
    let mut rgba = img.into_rgba8();
    apply_alpha(&mut rgba, alpha);
    Ok(DecodedImage {
        pixels: rgba,
        source_width: sw,
        source_height: sh,
        full_size: (tw, th) == (sw, sh),
    })
}

/// Baseline / progressive JPEG with DCT scaling to the smallest of 1/1, 1/2, 1/4, 1/8 that
/// still covers the decode size. `None` = not a JPEG, or one this path does not handle (CMYK,
/// 16-bit, corrupt): the caller falls back to the general decoder.
fn decode_jpeg_scaled(
    bytes: &[u8],
    size: (u32, u32),
) -> Option<(image::DynamicImage, u32, u32, Orientation)> {
    if !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return None;
    }
    let mut d = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    d.read_info().ok()?;
    let info = d.info()?;
    let (sw, sh) = (info.width as u32, info.height as u32);
    if !matches!(
        info.pixel_format,
        jpeg_decoder::PixelFormat::RGB24 | jpeg_decoder::PixelFormat::L8
    ) {
        return None;
    }
    // EXIF (APP1) comes before the frame header, so `read_info` has seen it.
    let orientation = d
        .exif_data()
        .and_then(Orientation::from_exif_chunk)
        .unwrap_or(Orientation::NoTransforms);
    let (tw, th) = stored_decode_size(sw, sh, orientation, size);
    let (w, h) = d
        .scale(
            tw.min(u16::MAX as u32) as u16,
            th.min(u16::MAX as u32) as u16,
        )
        .ok()?;
    let px = d.decode().ok()?;
    let (w, h) = (w as u32, h as u32);
    let img = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, px)?)
        }
        _ => image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(w, h, px)?),
    };
    Some((img, sw, sh, orientation))
}

pub(crate) fn apply_alpha(rgba: &mut image::RgbaImage, alpha: SourceAlpha) {
    match alpha {
        SourceAlpha::Opaque => rgba.pixels_mut().for_each(|p| p.0[3] = 255),
        SourceAlpha::Premultiplied => rgba.pixels_mut().for_each(|p| {
            let a = p.0[3] as u32;
            if a > 0 && a < 255 {
                for k in 0..3 {
                    p.0[k] = ((p.0[k] as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "render-decode-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn wait_ready(d: &ImageDecoder, p: &Path, size: (u32, u32)) -> Arc<DecodedImage> {
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            match d.request(p, SourceAlpha::Auto, size, DecodePriority::Live) {
                ImageStatus::Ready(i) => return i,
                ImageStatus::Failed(e) => panic!("{e}"),
                ImageStatus::Pending => {
                    assert!(Instant::now() < until, "decode timed out");
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    /// A minimal big-endian EXIF (TIFF) block with only an Orientation tag.
    fn exif_orientation(o: u16) -> Vec<u8> {
        let mut e = b"MM\0\x2a\0\0\0\x08".to_vec();
        e.extend_from_slice(&1u16.to_be_bytes()); // one entry
        e.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
        e.extend_from_slice(&3u16.to_be_bytes()); // SHORT
        e.extend_from_slice(&1u32.to_be_bytes()); // one value
        e.extend_from_slice(&o.to_be_bytes());
        e.extend_from_slice(&[0, 0]);
        e.extend_from_slice(&0u32.to_be_bytes()); // no next IFD
        e
    }

    /// A02-4: a picture stored 40 × 20 (red left half, blue right half) with EXIF
    /// "turn 90° clockwise" decodes as shown: 20 × 40, red on top.
    #[test]
    fn exif_orientation_is_applied() {
        use image::ImageEncoder;
        let dir = tmp("exif");
        let stored = image::RgbImage::from_fn(40, 20, |x, _| {
            if x < 20 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        });
        let jpg = dir.join("portrait.jpg");
        let mut buf = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 95);
        enc.set_exif_metadata(exif_orientation(6)).unwrap();
        enc.write_image(&stored, 40, 20, image::ExtendedColorType::Rgb8)
            .unwrap();
        std::fs::write(&jpg, &buf).unwrap();
        let png = dir.join("portrait.png");
        let mut buf = Vec::new();
        let mut enc = image::codecs::png::PngEncoder::new(&mut buf);
        enc.set_exif_metadata(exif_orientation(6)).unwrap();
        enc.write_image(&stored, 40, 20, image::ExtendedColorType::Rgb8)
            .unwrap();
        std::fs::write(&png, &buf).unwrap();
        for file in [&jpg, &png] {
            for (bw, bh) in [(20, 40), (10, 20)] {
                let d = decode_file(file, SourceAlpha::Opaque, (bw, bh)).unwrap();
                let what = format!("{} for {bw}x{bh}", file.display());
                assert_eq!((d.source_width, d.source_height), (20, 40), "{what}");
                assert_eq!(d.pixels.dimensions(), (bw, bh), "{what}");
                let top = d.pixels.get_pixel(bw / 2, bh / 8).0;
                let bottom = d.pixels.get_pixel(bw / 2, bh * 7 / 8).0;
                assert!(top[0] > 200 && top[2] < 60, "{what}: top {top:?}");
                assert!(
                    bottom[2] > 200 && bottom[0] < 60,
                    "{what}: bottom {bottom:?}"
                );
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn decode_size_covers_the_box() {
        assert_eq!(decode_size(3840, 2400, (1920, 1080)), (1920, 1200));
        assert_eq!(decode_size(3840, 2400, (480, 270)), (480, 300));
        assert_eq!(decode_size(800, 600, (1920, 1080)), (800, 600));
        assert_eq!(decode_size(10000, 100, (100, 100)), (4096, 41));
    }

    #[test]
    fn downscales_on_decode_and_reuses_larger_decodes() {
        let dir = tmp("scale");
        let png = dir.join("big.png");
        image::RgbImage::from_pixel(1600, 1000, image::Rgb([10, 200, 30]))
            .save(&png)
            .unwrap();
        let jpg = dir.join("big.jpg");
        image::RgbImage::from_fn(1600, 1000, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 7])
        })
        .save(&jpg)
        .unwrap();
        let d = ImageDecoder::new(2, 64 << 20);
        let a = wait_ready(&d, &png, (800, 450));
        assert_eq!((a.pixels.width(), a.pixels.height()), (800, 500));
        assert_eq!((a.source_width, a.source_height), (1600, 1000));
        // a smaller box is served by the 800 px decode
        let decoded = d.stats().decoded;
        let b = wait_ready(&d, &png, (160, 90));
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(d.stats().decoded, decoded);
        // JPEG: DCT scaling to 1/8 (200 × 125) covers 160 × 90 → 160 × 100
        let j = wait_ready(&d, &jpg, (160, 90));
        assert_eq!((j.pixels.width(), j.pixels.height()), (160, 100));
        assert_eq!((j.source_width, j.source_height), (1600, 1000));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cache_stays_within_its_byte_budget() {
        let dir = tmp("budget");
        // each decode is 100 × 100 × 4 = 40 000 bytes; the budget holds two
        let d = ImageDecoder::new(2, 90_000);
        for i in 0..6 {
            let p = dir.join(format!("{i}.png"));
            image::RgbImage::from_pixel(100, 100, image::Rgb([i * 40, 0, 0]))
                .save(&p)
                .unwrap();
            wait_ready(&d, &p, (100, 100));
            let s = d.stats();
            assert!(s.cached_bytes <= s.budget_bytes, "{s:?}");
        }
        let s = d.stats();
        assert_eq!(s.cached_entries, 2, "{s:?}");
        assert_eq!(s.evicted, 4, "{s:?}");
        // the most recent one is still cached, the first one is not
        assert!(
            d.cached(&dir.join("5.png"), SourceAlpha::Auto, (100, 100))
                .is_some()
        );
        assert!(
            d.cached(&dir.join("0.png"), SourceAlpha::Auto, (100, 100))
                .is_none()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failures_are_reported_and_a_changed_file_is_decoded_again() {
        let dir = tmp("fail");
        let p = dir.join("x.png");
        std::fs::write(&p, b"not a png").unwrap();
        let d = ImageDecoder::new(1, 1 << 20);
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            match d.request(&p, SourceAlpha::Auto, (10, 10), DecodePriority::Thumbnail) {
                ImageStatus::Failed(_) => break,
                ImageStatus::Ready(_) => panic!("decoded garbage"),
                ImageStatus::Pending => {
                    assert!(Instant::now() < until);
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        assert!(matches!(
            d.request(
                &dir.join("nope.png"),
                SourceAlpha::Auto,
                (1, 1),
                DecodePriority::Live
            ),
            ImageStatus::Failed(_)
        ));
        assert!(matches!(
            d.request(
                &dir.join("clip.mp4"),
                SourceAlpha::Auto,
                (1, 1),
                DecodePriority::Live
            ),
            ImageStatus::Failed(_)
        ));
        image::RgbImage::from_pixel(20, 10, image::Rgb([1, 2, 3]))
            .save_with_format(&p, image::ImageFormat::Png)
            .unwrap();
        let i = wait_ready(&d, &p, (10, 10));
        assert_eq!(i.source_width, 20);
        let _ = std::fs::remove_dir_all(dir);
    }
}
