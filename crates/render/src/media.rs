//! Still-image media: upload decoded images, convert them to linear premultiplied with
//! mipmaps, cache the textures. Decoding lives in [`crate::decode`].

use crate::decode::{DecodePriority, FileKey, ImageDecoder, ImageStatus, apply_alpha};
use crate::geom::Affine;
use crate::gpu::{Draw, Gpu, Pass, Pipe, Tex, Uniforms, create_texture, paint};
use model::{MediaAssetId, SourceAlpha};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Largest side an image is decoded / uploaded at (bigger images are downscaled on decode).
pub const MAX_IMAGE_SIDE: u32 = 4096;

/// Where an image comes from.
#[derive(Clone, Debug)]
pub enum ImageSource {
    /// A file on disk (decoded by the renderer's decode workers, cached by path, size and
    /// modification time, at the size it is drawn).
    Path(PathBuf),
    /// An already decoded straight-alpha sRGB image. `key` identifies it in the cache: use a
    /// new key whenever the pixels change.
    Rgba {
        /// Cache key.
        key: u64,
        /// The pixels.
        image: Arc<image::RgbaImage>,
    },
}

impl ImageSource {
    /// A file path.
    pub fn path(p: impl Into<PathBuf>) -> Self {
        ImageSource::Path(p.into())
    }

    /// Short description for reports.
    pub fn describe(&self) -> String {
        match self {
            ImageSource::Path(p) => p.display().to_string(),
            ImageSource::Rgba { key, image } => {
                format!(
                    "decoded image #{key} ({}x{})",
                    image.width(),
                    image.height()
                )
            }
        }
    }
}

/// Resolves the media assets referenced by slide elements (media fills) to images.
pub trait MediaResolver {
    /// The image for an asset, or `None` when it is missing / not an image.
    fn resolve(&self, asset: MediaAssetId) -> Option<ImageSource>;
}

impl<F> MediaResolver for F
where
    F: Fn(MediaAssetId) -> Option<ImageSource>,
{
    fn resolve(&self, asset: MediaAssetId) -> Option<ImageSource> {
        self(asset)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CacheKey {
    /// A file version decoded for a box size.
    File(FileKey, SourceAlpha, (u32, u32)),
    /// A caller-decoded image.
    Key(u64, SourceAlpha),
}

/// A GPU image: linear premultiplied Rgba16Float with a full mip chain.
pub(crate) struct GpuImage {
    pub tex: Tex,
    /// Source pixel size (before any downscale on decode): crops and native-size placement
    /// are in these units.
    pub width: u32,
    pub height: u32,
}

/// Result of a [`MediaCache::get`].
pub(crate) enum Lookup {
    /// On the GPU.
    Ready(Arc<GpuImage>),
    /// Still decoding (not drawn this time).
    Pending,
    /// Cannot be drawn.
    Failed(String),
}

/// How [`MediaCache::get`] treats an image that is not decoded yet.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Fetch {
    /// Queue the decode with this priority and report it as pending.
    Queue(DecodePriority),
    /// Decode it on the calling thread.
    Wait,
}

struct Entry {
    image: Arc<GpuImage>,
    tick: u64,
    bytes: u64,
}

/// Uploaded images (GPU textures), LRU within a byte budget. Decoding is the
/// [`ImageDecoder`]'s job; this cache only uploads what is decoded.
pub(crate) struct MediaCache {
    entries: HashMap<CacheKey, Entry>,
    budget: u64,
    total: u64,
    tick: u64,
}

impl MediaCache {
    pub fn new(budget: u64) -> Self {
        MediaCache {
            entries: HashMap::new(),
            budget,
            total: 0,
            tick: 0,
        }
    }

    fn key(src: &ImageSource, alpha: SourceAlpha, size: (u32, u32)) -> Result<CacheKey, String> {
        Ok(match src {
            ImageSource::Path(p) => CacheKey::File(FileKey::of(p)?, alpha, size),
            ImageSource::Rgba { key, .. } => CacheKey::Key(*key, alpha),
        })
    }

    /// Uploaded images and their GPU bytes.
    pub fn size(&self) -> (u64, u64) {
        (self.entries.len() as u64, self.total)
    }

    /// Whether the image is uploaded already (no decode is requested).
    pub fn has(&self, src: &ImageSource, alpha: SourceAlpha, size: (u32, u32)) -> bool {
        let size = (size.0.max(1), size.1.max(1));
        Self::key(src, alpha, size).is_ok_and(|k| self.entries.contains_key(&k))
    }

    /// The GPU image of a source drawn into a `size` box (screen px), uploading it (and
    /// queueing conversion passes into `pre`) once it is decoded.
    #[allow(clippy::too_many_arguments)]
    pub fn get(
        &mut self,
        gpu: &Gpu,
        decoder: &ImageDecoder,
        src: &ImageSource,
        alpha: SourceAlpha,
        size: (u32, u32),
        fetch: Fetch,
        pre: &mut Vec<Pass>,
    ) -> Lookup {
        self.tick += 1;
        let size = (size.0.max(1), size.1.max(1));
        if let ImageSource::Path(p) = src
            && let Err(e) = crate::decode::check_extension(p)
        {
            return Lookup::Failed(e);
        }
        let key = match Self::key(src, alpha, size) {
            Ok(k) => k,
            Err(e) => return Lookup::Failed(e),
        };
        if let Some(e) = self.entries.get_mut(&key) {
            e.tick = self.tick;
            return Lookup::Ready(e.image.clone());
        }
        let image = match src {
            ImageSource::Path(p) => {
                let status = match fetch {
                    Fetch::Queue(prio) => decoder.request(p, alpha, size, prio),
                    Fetch::Wait => decoder.decode_now(p, alpha, size),
                };
                match status {
                    ImageStatus::Ready(d) => {
                        let mut g = upload_image(gpu, &d.pixels, pre);
                        g.width = d.source_width;
                        g.height = d.source_height;
                        g
                    }
                    ImageStatus::Pending => return Lookup::Pending,
                    ImageStatus::Failed(e) => return Lookup::Failed(e),
                }
            }
            ImageSource::Rgba { image, .. } => {
                let mut img = (**image).clone();
                apply_alpha(&mut img, alpha);
                upload_image(gpu, &img, pre)
            }
        };
        let image = Arc::new(image);
        let bytes = image.tex.bytes();
        self.total += bytes;
        if let Some(old) = self.entries.insert(
            key,
            Entry {
                image: image.clone(),
                tick: self.tick,
                bytes,
            },
        ) {
            self.total -= old.bytes;
        }
        self.evict();
        Lookup::Ready(image)
    }

    fn evict(&mut self) {
        while self.total > self.budget && self.entries.len() > 1 {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.tick)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(e) = self.entries.remove(&oldest) {
                self.total -= e.bytes;
            }
        }
    }
}

/// Uploads straight sRGB RGBA8 and converts it on the GPU to linear premultiplied f16 with
/// mipmaps (passes appended to `pre`).
pub(crate) fn upload_image(gpu: &Gpu, img: &image::RgbaImage, pre: &mut Vec<Pass>) -> GpuImage {
    let (w, h) = (img.width().max(1), img.height().max(1));
    let src = gpu.upload(
        "image src",
        w,
        h,
        wgpu::TextureFormat::Rgba8UnormSrgb,
        img.as_raw(),
    );
    let mips = 32 - w.max(h).leading_zeros();
    let tex = create_texture(
        &gpu.device,
        "image",
        w,
        h,
        crate::gpu::LINEAR_FORMAT,
        mips,
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let level_view = |i: u32| {
        tex.texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: i,
            mip_level_count: Some(1),
            ..Default::default()
        })
    };
    let mut prev = src.view.clone();
    let mut straight = true;
    for i in 0..mips {
        let (lw, lh) = ((w >> i).max(1), (h >> i).max(1));
        let u = Uniforms::new(&Affine::IDENTITY, [0.0, 0.0, lw as f32, lh as f32])
            .paint(if straight {
                paint::TEXTURE_STRAIGHT
            } else {
                paint::TEXTURE_PREMUL
            })
            .paint_rect([0.0, 0.0, lw as f32, lh as f32]);
        let view = level_view(i);
        pre.push(Pass {
            label: "image mip",
            target: view.clone(),
            size: (lw, lh),
            clear: None,
            draws: vec![Draw::new(Pipe::Replace, u).with_paint(&prev)],
        });
        prev = view;
        straight = false;
    }
    GpuImage {
        tex,
        width: w,
        height: h,
    }
}
