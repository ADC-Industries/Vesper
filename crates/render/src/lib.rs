//! # `render` — headless GPU slide renderer
//!
//! Turns [`model`] slides and media into frames, offscreen with wgpu (no window, no display
//! server). One [`Renderer`] owns the GPU device, the font system and the caches and is shared
//! by every screen; each screen has its own [`ScreenTarget`] (size, alpha intent, background,
//! readback ring).
//!
//! ```no_run
//! # use render::*;
//! # fn demo(slide: &model::Slide) -> anyhow::Result<()> {
//! let mut r = Renderer::new(RendererOptions::default())?;
//! println!("rendering on {}", r.adapter());
//! let frame = r.render_layers(
//!     ScreenConfig::opaque(1920, 1080),
//!     &[LayerContent::slide(slide, model::Size::HD)],
//! )?;
//! frame.save_png("slide.png")?;
//! # Ok(()) }
//! ```
//!
//! ## Pipeline
//! 1. **Layers** are composited bottom to top in the fixed [`model::Layer::COMPOSITE_ORDER`]
//!    (video input, media, slide, announcements, props, messages, captions), whatever order they are
//!    passed in. Layer content: a still image ([`LayerSource::Media`], scaled by its
//!    [`model::MediaDisplay`]), a video frame ([`LayerSource::Video`]: NV12 / P010 planes
//!    converted YUV → linear RGB on the GPU once per frame and stream, then drawn like an
//!    image; see `video`), a slide ([`LayerSource::Slide`]) or a solid colour.
//!    A **transition** is a [`LayerSource::Mix`]: outgoing and incoming content of one layer
//!    blended `(1 − p)·from + p·to` in linear light, premultiplied, into a screen-sized texture
//!    the screen keeps while frames use it, so a new transition can start from the last blend
//!    ([`MixInput::Snapshot`]) without a jump. Its [`LayerMix::kind`] is the model's
//!    [`model::TransitionKind`] (one type for slides, media, looks, props and messages):
//!    push, slide, wipe and zoom draw each side into its own texture first and move, clip or
//!    scale them (see `transition`); every other kind is that dissolve.
//! 2. **Slides** are scaled uniformly from their owner's canvas to the screen (letterboxed,
//!    centred) and rendered once into a cached screen-sized texture; the cache key is the
//!    slide content, so an unchanged live slide costs one textured quad per frame.
//! 3. **Elements**: shapes (analytic rounded rectangles / ellipses, rasterized paths for the
//!    rest), fills (colour, gradient, image with fit / fill / stretch / none), strokes,
//!    element shadows, feather, opacity, rotation and flips. See `scene`.
//! 4. **Text**: cosmic-text layout ([`model::TextBox`] cascade, alignment, line height,
//!    margins, shrink / scale / grow to fit, ALL CAPS and friends, super/subscript), swash glyph
//!    coverage plus a true vector stroke for the outline, GPU gaussian blur for the text
//!    shadow. Composited shadow → outline → fill.
//!
//! ## Colour pipeline
//! - **Inputs are sRGB-encoded, straight alpha**: model [`model::Color`]s and decoded images.
//!   Model colours are converted on the CPU, images by an `Rgba8UnormSrgb` texture fetch, to
//!   **linear light, premultiplied alpha**.
//! - **All blending happens in linear light, premultiplied**, in `Rgba16Float` intermediates
//!   (element layers, cached slide layers, the screen composite). Images get a mip chain
//!   (built in linear premultiplied space) so downscaled backgrounds do not alias.
//! - **Output** ([`RenderedFrame`]): the composite is put over the screen background (opaque
//!   screens additionally over black, alpha forced to 255), converted to straight colour,
//!   encoded with the sRGB transfer function and then premultiplied *in the encoded domain*:
//!   `rgb8 = srgb(linear_rgb / a) * a`. That is the conventional "premultiplied sRGB RGBA8"
//!   that keyers, network video (IDN/OMT) senders and compositors expect.
//! - **Screen adjustments** ([`ScreenTarget::set_adjustments`]): a screen's colour correction
//!   and corner pin are applied by that same output pass (no extra pass or texture): the
//!   composite is sampled through the corner pin's inverse homography, and the colour
//!   correction works on the straight sRGB-encoded colour before it is premultiplied. Alpha
//!   is kept (outside a corner-pinned quad it is 0). A screen without adjustments uses the
//!   plain output pass, so its frames are unchanged bit for bit.
//! - Gradients interpolate in sRGB-encoded space (as design tools show them), then convert.
//!
//! ## Fonts
//! See [`fonts`]: PostScript name → family + weight + italic → the machine's
//! [`model::FontSubstitutionTable`] → fallback family (default "DejaVu Sans"). Every
//! substitution, fallback and synthetic bold/italic is counted in [`RenderReport::fonts`].
//!
//! ## Images
//! Image files are decoded by a small worker pool ([`ImageDecoder`]), downscaled to just
//! cover the size they are drawn at, and kept in a byte-bounded LRU of decoded images; the
//! rendering thread only uploads them. With [`RendererOptions::wait_for_images`] off, an image
//! that is still decoding is left out of the frame ([`Renderer::missing_images`]) instead of
//! being waited for. [`Renderer::render_layers`] (thumbnails, tools) uses its own slide and
//! texture caches, so it never evicts what screens made with [`Renderer::create_screen`] use.
//!
//! ## Throughput
//! [`Renderer::submit`] records a frame and its readback copy into the next free buffer of the
//! screen's ring and returns; [`Renderer::poll`] returns finished frames in order.
//! [`Renderer::render_layers`] is the blocking convenience (submit + wait).
//!
//! ## One GPU device per process
//! Every [`Renderer`] in a process shares one wgpu instance and one device per
//! [`AdapterPreference`]; they are created on first use and never destroyed (a lost device is
//! replaced: new renderers get the replacement, and existing ones move to it in
//! [`Renderer::recover`]). Each renderer still has its own caches, screens and decoder.
//! Consequences: GPU errors are device-wide, so [`Renderer::submit`] may report an error raised
//! by another renderer in the same process, and waiting for a frame waits for the device's
//! other queued work too.
//!
//! The reason is a use-after-free in the Khronos Vulkan loader (seen with libvulkan 1.4.341 on
//! RADV and lavapipe alike), which crashed `cargo test` with SIGSEGV now and then: creating a
//! Vulkan instance unloads the drivers without devices (`unload_drivers_without_physical_devices`
//! during `vkEnumeratePhysicalDevices`, and failed ICDs in `vkCreateInstance`), freeing that
//! instance's driver records while the instance is already on the loader's global list and
//! without the lock that guards it. Meanwhile any other thread whose call goes through
//! `loader_get_icd_and_device` walks every instance's driver and device lists:
//! `vkSetDebugUtilsObjectNameEXT` (wgpu names every object in debug builds, since
//! `InstanceFlags::DEBUG` is on with `debug_assertions`), `vkGetDeviceProcAddr`,
//! `vkGetDeviceQueue2`. Device teardown (`vkDestroyDevice`) changes the per-driver device lists
//! the same way. So two renderers made or dropped concurrently could crash a third one's
//! rendering. The server has a single renderer for its lifetime and was not exposed, but its
//! tests (many servers per test process) and the render tests were; sharing one instance and
//! never destroying a device removes every such window, whatever creates renderers.

#![warn(missing_docs)]

mod adjust;
mod decode;
pub mod fonts;
mod geom;
mod gpu;
pub mod live;
pub mod mask;
mod media;
pub mod patterns;
pub mod present;
mod raster;
mod report;
mod scene;
mod screen;
mod scroll;
pub mod text;
mod transition;
mod video;

pub use decode::{
    DEFAULT_DECODE_CACHE_BYTES, DEFAULT_DECODE_THREADS, DecodePriority, DecodeStats, DecodedImage,
    ImageDecoder, ImageStatus,
};
pub use fonts::{FontIssue, FontMatch};
pub use geom::RectF;
pub use gpu::{AdapterPreference, AdapterSummary};
pub use media::{ImageSource, MAX_IMAGE_SIDE, MediaResolver};
pub use report::{MediaIssue, RenderReport};
pub use screen::{RenderedFrame, ScreenConfig, ScreenTarget};
pub use video::{VideoAlpha, VideoFrameRef, VideoMatrix, VideoPlanes};

use geom::Affine;
use gpu::{Draw, Gpu, Pass, Pipe, Tex, Uniforms, paint};
use media::{Fetch, Lookup};
use model::{Color, FillKind, FontSubstitutionTable, Layer, MediaDisplay, Size, Slide};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

/// An installed font family (see [`Renderer::font_families`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFamily {
    /// Family name.
    pub family: String,
    /// Its faces.
    pub faces: Vec<FontFace>,
}

/// One installed face.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFace {
    /// CSS weight.
    pub weight: u16,
    /// Italic or oblique.
    pub italic: bool,
    /// PostScript name.
    pub postscript_name: Option<String>,
}

/// Renderer construction options.
#[derive(Clone, Debug)]
pub struct RendererOptions {
    /// Which GPU adapter to prefer.
    pub adapter: AdapterPreference,
    /// Extra font files / directories (e.g. `<data>/fonts`), loaded after the system fonts.
    pub font_dirs: Vec<PathBuf>,
    /// Load the operating system's fonts (default true; tests may turn it off).
    pub system_fonts: bool,
    /// Family used when a font is neither installed nor substituted.
    pub fallback_family: String,
    /// The machine's font substitution table.
    pub font_substitutions: FontSubstitutionTable,
    /// Readback buffers per screen (2–3 keeps the GPU busy while frames are copied out).
    pub readback_depth: usize,
    /// Rendered slides kept for screens made with [`Renderer::create_screen`] (each is one
    /// screen-sized f16 texture). [`Renderer::render_layers`] (thumbnails, tools) keeps its
    /// own few, so it never evicts these.
    pub slide_cache_entries: usize,
    /// GPU memory budget for uploaded images of screens made with
    /// [`Renderer::create_screen`], bytes. [`Renderer::render_layers`] has its own
    /// [`SCRATCH_MEDIA_BYTES`].
    pub media_cache_bytes: u64,
    /// Image decode worker threads.
    pub decode_threads: usize,
    /// Byte budget of the decoded-image cache (CPU memory).
    pub decode_cache_bytes: u64,
    /// `true` (default): an image that is not decoded yet is decoded on the rendering thread
    /// before the frame is drawn. `false` (servers): it is queued on the decode workers and
    /// the frame is drawn without it; [`Renderer::missing_images`] says how many were left
    /// out and the decoder's notify callback says when to render again.
    pub wait_for_images: bool,
}

/// GPU budget for images uploaded by [`Renderer::render_layers`], bytes.
pub const SCRATCH_MEDIA_BYTES: u64 = 64 << 20;
/// Slides cached by [`Renderer::render_layers`].
const SCRATCH_SLIDE_ENTRIES: usize = 4;

impl Default for RendererOptions {
    fn default() -> Self {
        RendererOptions {
            adapter: AdapterPreference::HardwareFirst,
            font_dirs: Vec::new(),
            system_fonts: true,
            fallback_family: "DejaVu Sans".into(),
            font_substitutions: FontSubstitutionTable::default(),
            readback_depth: 3,
            slide_cache_entries: 8,
            media_cache_bytes: 1 << 30,
            decode_threads: DEFAULT_DECODE_THREADS,
            decode_cache_bytes: DEFAULT_DECODE_CACHE_BYTES,
            wait_for_images: true,
        }
    }
}

/// What a layer shows.
#[derive(Clone)]
pub enum LayerSource<'a> {
    /// A still image drawn over the whole screen per `display` (scale mode, anchor, crop,
    /// rotation, flips, alpha adjust).
    Media {
        /// The image.
        image: ImageSource,
        /// How it is fitted and drawn.
        display: MediaDisplay,
    },
    /// A slide, scaled from its owner's canvas size to the screen.
    Slide {
        /// The slide.
        slide: &'a Slide,
        /// Canvas size of the owning document (e.g. `Presentation::canvas_size`).
        canvas: Size,
        /// Resolves media fills of its elements (none = media fills draw nothing).
        media: Option<&'a dyn MediaResolver>,
        /// Seconds the slide has been live, for its scrolling text (`None`: a still picture
        /// such as a thumbnail, with scrolling text where it is laid out). See `scroll`.
        time: Option<f64>,
        /// The current frames of its video elements (elements whose media fill plays a
        /// video), by element: drawn in the element's box with its fill's display settings.
        /// An element without a frame here draws no fill.
        videos: Vec<(model::ElementId, VideoFrameRef<'a>)>,
    },
    /// A solid colour over the whole screen.
    Color(Color),
    /// Solid rectangles in screen pixels (test patterns' moving parts).
    Rects(Vec<(model::Rect, Color)>),
    /// A decoded video frame drawn over the whole screen per `display` (like a still image).
    Video {
        /// The frame (its planes are uploaded once per stream and frame number).
        frame: VideoFrameRef<'a>,
        /// How it is fitted and drawn.
        display: MediaDisplay,
    },
    /// A transition of one layer: `from` and `to` cross-fade in linear light (see
    /// [`LayerMix`]).
    Mix(Box<LayerMix<'a>>),
    /// Erases what is under it by the image's alpha ("destination-out", over the whole
    /// screen): the first half of a keyhole mask with a see-through colour
    /// ([`mask::keyhole_erase`]).
    Erase {
        /// The picture whose alpha erases.
        image: ImageSource,
    },
}

/// One input of a [`LayerMix`].
#[derive(Clone)]
pub enum MixInput<'a> {
    /// Nothing (a transition from or to a cleared layer).
    Empty,
    /// Live content at an opacity (a video keeps playing during its dissolve).
    Content(LayerSource<'a>, f32),
    /// What an earlier mix of this screen last produced (its `store` key): a transition
    /// started while another one runs begins from the blended picture, with no jump.
    Snapshot(u64),
}

/// A transition between two contents of one layer. A dissolve shows
/// `(1 − progress) · from + progress · to`, premultiplied, in linear light; push, slide, wipe
/// and zoom move, clip or scale the two sides instead (see [`LayerMix::kind`]). The result is
/// rendered into a screen-sized texture kept under `store` (per [`ScreenTarget`]) until a
/// frame no longer uses it, so a later mix can start from it ([`MixInput::Snapshot`]). A cut
/// is not a mix.
#[derive(Clone)]
pub struct LayerMix<'a> {
    /// Outgoing content.
    pub from: MixInput<'a>,
    /// Incoming content.
    pub to: MixInput<'a>,
    /// `0..=1`, already eased (the caller applies [`model::Easing`]).
    pub progress: f32,
    /// Key of the texture the result is kept in (unique per transition on the screen).
    pub store: u64,
    /// The style: `push`, `slide`, `wipe` and `zoom` move; anything else (cut, dissolve,
    /// vendor, …) is drawn as a dissolve.
    pub kind: model::TransitionKind,
}

impl<'a> LayerMix<'a> {
    /// A dissolve from `from` to `to`.
    pub fn dissolve(from: MixInput<'a>, to: MixInput<'a>, progress: f32, store: u64) -> Self {
        LayerMix {
            from,
            to,
            progress,
            store,
            kind: model::TransitionKind::Dissolve,
        }
    }
}

/// One layer of a screen.
#[derive(Clone)]
pub struct LayerContent<'a> {
    /// Compositor layer (decides the stacking order).
    pub layer: Layer,
    /// What it shows.
    pub source: LayerSource<'a>,
    /// Layer opacity `0..=1` (transitions).
    pub opacity: f32,
}

impl<'a> LayerContent<'a> {
    /// A slide on the slide layer.
    pub fn slide(slide: &'a Slide, canvas: Size) -> Self {
        LayerContent {
            layer: Layer::Slide,
            source: LayerSource::Slide {
                slide,
                canvas,
                media: None,
                time: None,
                videos: Vec::new(),
            },
            opacity: 1.0,
        }
    }

    /// A slide whose media fills are resolved by `media`.
    pub fn slide_with_media(slide: &'a Slide, canvas: Size, media: &'a dyn MediaResolver) -> Self {
        LayerContent {
            layer: Layer::Slide,
            source: LayerSource::Slide {
                slide,
                canvas,
                media: Some(media),
                time: None,
                videos: Vec::new(),
            },
            opacity: 1.0,
        }
    }

    /// A still image on the (background) media layer.
    pub fn media(image: ImageSource, display: MediaDisplay) -> Self {
        LayerContent {
            layer: Layer::Media,
            source: LayerSource::Media { image, display },
            opacity: 1.0,
        }
    }

    /// A video frame on the (background) media layer.
    pub fn video(frame: VideoFrameRef<'a>, display: MediaDisplay) -> Self {
        LayerContent {
            layer: Layer::Media,
            source: LayerSource::Video { frame, display },
            opacity: 1.0,
        }
    }

    /// A transition on `layer`.
    pub fn mix(layer: Layer, mix: LayerMix<'a>) -> Self {
        LayerContent {
            layer,
            source: LayerSource::Mix(Box::new(mix)),
            opacity: 1.0,
        }
    }

    /// A solid colour on `layer`.
    pub fn color(layer: Layer, color: Color) -> Self {
        LayerContent {
            layer,
            source: LayerSource::Color(color),
            opacity: 1.0,
        }
    }

    /// Moves the content to another layer.
    pub fn on_layer(mut self, layer: Layer) -> Self {
        self.layer = layer;
        self
    }

    /// Sets the layer opacity.
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }
}

struct CachedSlide {
    tex: Tex,
    tick: u64,
}

/// Rendered slide layers and uploaded images of one class of targets.
struct Caches {
    slides: HashMap<u64, CachedSlide>,
    slide_entries: usize,
    media: media::MediaCache,
}

impl Caches {
    fn new(slide_entries: usize, media_bytes: u64) -> Self {
        Caches {
            slides: HashMap::new(),
            slide_entries: slide_entries.max(1),
            media: media::MediaCache::new(media_bytes),
        }
    }
}

/// One cache's size ([`Renderer::cache_sizes`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheSize {
    /// What it caches.
    pub name: &'static str,
    /// Entries.
    pub entries: u64,
    /// Bytes held (0: not counted).
    pub bytes: u64,
}

/// The headless renderer (see the crate docs).
pub struct Renderer {
    gpu: Arc<Gpu>,
    text: text::TextEngine,
    glyphs: raster::GlyphRasterizer,
    decoder: Arc<ImageDecoder>,
    /// Caches of screens made with `create_screen` (live output).
    live: Caches,
    /// Caches of `render_layers` (thumbnails, tools): separate, so they never evict live work.
    scratch: Caches,
    wait_for_images: bool,
    missing: usize,
    report: report::ReportState,
    readback_depth: usize,
    screens: HashMap<(u32, u32, u8, [u32; 4]), ScreenTarget>,
    tick: u64,
    videos: video::VideoCache,
    /// Geometry of scrolling text strips (their textures live in the slide caches).
    strips: HashMap<u64, scroll::Strip>,
    /// What a lost device is replaced with, and the cache sizes to start again with.
    adapter_preference: AdapterPreference,
    cache_sizes: (usize, u64),
    /// When the last replacement of a lost device was tried (it is retried every
    /// [`RECOVERY_RETRY`] while the driver is still resetting).
    last_recovery: Option<std::time::Instant>,
    /// How many times the device was replaced ([`Renderer::device_generation`]).
    generation: u64,
}

/// How often [`Renderer::recover`] tries again to replace a lost device.
const RECOVERY_RETRY: std::time::Duration = std::time::Duration::from_secs(2);

impl Renderer {
    /// Picks an adapter (hardware GPU first, then any, including lavapipe), logs it, creates
    /// the device and loads fonts. The device is the process's shared one when another
    /// renderer already made it (see "One GPU device per process" in the crate docs).
    pub fn new(options: RendererOptions) -> anyhow::Result<Self> {
        let gpu = Gpu::new(options.adapter)?;
        let text = text_engine(&options);
        log::info!("render: {} font faces available", text.fs.db().len());
        Ok(Renderer {
            gpu,
            text,
            glyphs: raster::GlyphRasterizer::new(),
            decoder: ImageDecoder::new(options.decode_threads, options.decode_cache_bytes),
            live: Caches::new(options.slide_cache_entries, options.media_cache_bytes),
            scratch: Caches::new(SCRATCH_SLIDE_ENTRIES, SCRATCH_MEDIA_BYTES),
            wait_for_images: options.wait_for_images,
            missing: 0,
            report: report::ReportState::default(),
            readback_depth: options.readback_depth.max(1),
            screens: HashMap::new(),
            tick: 0,
            videos: video::VideoCache::default(),
            strips: HashMap::new(),
            adapter_preference: options.adapter,
            cache_sizes: (options.slide_cache_entries, options.media_cache_bytes),
            last_recovery: None,
            generation: 0,
        })
    }

    /// The GPU device was lost (driver reset, GPU hang, `TDR` on Windows): nothing renders
    /// until [`Renderer::recover`] replaces it.
    pub fn is_lost(&self) -> bool {
        self.gpu.is_lost()
    }

    /// Replaces a lost GPU device with a new one (the process's replacement, see "One GPU
    /// device per process"): drops every cache and internal target made on the old device.
    /// [`ScreenTarget`]s made on the old device are made again by their next
    /// [`Renderer::submit`]. `Ok(true)` when the device was replaced, `Ok(false)` when it was
    /// not lost (or a failed attempt was made less than two seconds ago); `Err` when no new
    /// device could be made (the driver may still be resetting: call again later).
    /// [`Renderer::submit`] calls this first, so a renderer heals on its next frame; callers
    /// that only re-render on change call it on a timer to redraw everything at once.
    pub fn recover(&mut self) -> anyhow::Result<bool> {
        if !self.gpu.is_lost() {
            return Ok(false);
        }
        let now = std::time::Instant::now();
        if self
            .last_recovery
            .is_some_and(|t| now.duration_since(t) < RECOVERY_RETRY)
        {
            return Ok(false);
        }
        self.last_recovery = Some(now);
        let gpu = anyhow::Context::context(
            Gpu::new(self.adapter_preference),
            "replacing the lost GPU device",
        )?;
        log::warn!(
            "render: the GPU device was lost; rendering again on {}",
            gpu.summary
        );
        self.gpu = gpu;
        self.last_recovery = None;
        self.generation += 1;
        let (slides, media) = self.cache_sizes;
        self.live = Caches::new(slides, media);
        self.scratch = Caches::new(SCRATCH_SLIDE_ENTRIES, SCRATCH_MEDIA_BYTES);
        self.screens.clear();
        self.videos = video::VideoCache::default();
        Ok(true)
    }

    /// Counts the replacements of a lost device ([`Renderer::recover`], also when a
    /// [`Renderer::submit`] made it). A caller that renders only on change compares it before
    /// and after a pass: when it moved, frames of other screens still in flight were dropped
    /// by [`Renderer::wait`] / [`Renderer::poll`] and those screens must be drawn again.
    pub fn device_generation(&self) -> u64 {
        self.generation
    }

    /// Loses the GPU device as a driver reset would (tests of [`Renderer::recover`]). Every
    /// renderer of the process on this device is affected.
    #[doc(hidden)]
    pub fn lose_device_for_tests(&self) {
        self.gpu.lose();
    }

    /// Queues an uncaptured GPU error, as the driver would raise one (tests).
    #[doc(hidden)]
    pub fn raise_gpu_error_for_tests(&self, msg: &str) {
        self.gpu.push_error(msg);
    }

    /// The adapter in use.
    pub fn adapter(&self) -> &AdapterSummary {
        &self.gpu.summary
    }

    /// Installed font families (system fonts plus the extra font directories), sorted by name
    /// (case-insensitive), each with its faces sorted by weight, upright before italic.
    pub fn font_families(&self) -> Vec<FontFamily> {
        let mut map: std::collections::BTreeMap<String, FontFamily> = Default::default();
        for face in self.text.fs.db().faces() {
            let Some((name, _)) = face.families.first() else {
                continue;
            };
            let fam = map
                .entry(name.to_lowercase())
                .or_insert_with(|| FontFamily {
                    family: name.clone(),
                    faces: Vec::new(),
                });
            let f = FontFace {
                weight: face.weight.0,
                italic: face.style != fontdb::Style::Normal,
                postscript_name: Some(face.post_script_name.clone()).filter(|n| !n.is_empty()),
            };
            if !fam
                .faces
                .iter()
                .any(|x| x.weight == f.weight && x.italic == f.italic)
            {
                fam.faces.push(f);
            }
        }
        let mut out: Vec<FontFamily> = map.into_values().collect();
        for f in &mut out {
            f.faces.sort_by_key(|x| (x.weight, x.italic));
        }
        out
    }

    /// The family used when a font cannot be resolved.
    pub fn fallback_family(&self) -> &str {
        &self.text.fonts.fallback_family
    }

    /// Replaces the font substitution table (re-renders cached slides).
    pub fn set_font_substitutions(&mut self, table: FontSubstitutionTable) {
        self.text.fonts.set_substitutions(table);
        self.live.slides.clear();
        self.scratch.slides.clear();
    }

    /// The image decoder (shared by live and [`Renderer::render_layers`] rendering): set its
    /// notify callback, prefetch, read its counters.
    pub fn decoder(&self) -> &Arc<ImageDecoder> {
        &self.decoder
    }

    /// What the renderer's caches hold now (diagnostics: `GET /server/stats`).
    pub fn cache_sizes(&self) -> Vec<CacheSize> {
        let slides = |c: &Caches| {
            let bytes = c.slides.values().map(|s| s.tex.bytes()).sum();
            (c.slides.len() as u64, bytes)
        };
        let d = self.decoder.stats();
        let targets: u64 = self.screens.len() as u64;
        [
            (
                "decoded images (CPU)",
                (d.cached_entries as u64, d.cached_bytes),
            ),
            ("glyphs (CPU)", self.glyphs.size()),
            ("live slide layers (GPU)", slides(&self.live)),
            ("live images (GPU)", self.live.media.size()),
            ("scratch slide layers (GPU)", slides(&self.scratch)),
            ("scratch images (GPU)", self.scratch.media.size()),
            ("video frames (GPU)", self.videos.size()),
            ("scratch targets (GPU)", (targets, 0)),
            ("scrolling strips", (self.strips.len() as u64, 0)),
        ]
        .into_iter()
        .map(|(name, (entries, bytes))| CacheSize {
            name,
            entries,
            bytes,
        })
        .collect()
    }

    /// Images left out of the last [`Renderer::submit`] / [`Renderer::render_layers`] because
    /// they were still decoding (always 0 with
    /// [`RendererOptions::wait_for_images`]). Render again once the decoder reports them done.
    pub fn missing_images(&self) -> usize {
        self.missing
    }

    /// Requests decodes (with `priority`) of every image `layers` draw on a `size` target that
    /// is not uploaded yet, without rendering. Returns how many are still decoding (0 = a
    /// render now draws them all; failures count as done).
    pub fn request_images(
        &self,
        layers: &[LayerContent<'_>],
        size: (u32, u32),
        priority: DecodePriority,
    ) -> usize {
        let mut pending = 0;
        let mut check = |src: &ImageSource, alpha: model::SourceAlpha| {
            if self.live.media.has(src, alpha, size) || self.scratch.media.has(src, alpha, size) {
                return;
            }
            if let ImageSource::Path(p) = src
                && matches!(
                    self.decoder.request(p, alpha, size, priority),
                    ImageStatus::Pending
                )
            {
                pending += 1;
            }
        };
        fn sources<'s, 'a>(src: &'s LayerSource<'a>, out: &mut Vec<&'s LayerSource<'a>>) {
            match src {
                LayerSource::Mix(m) => {
                    for side in [&m.from, &m.to] {
                        if let MixInput::Content(s, _) = side {
                            sources(s, out);
                        }
                    }
                }
                other => out.push(other),
            }
        }
        let mut all = Vec::new();
        for l in layers
            .iter()
            .filter(|l| l.layer.is_visual() && l.opacity > 0.0)
        {
            sources(&l.source, &mut all);
        }
        for src in all {
            match src {
                LayerSource::Media { image, display } => check(image, display.alpha),
                LayerSource::Slide {
                    slide,
                    media: Some(res),
                    ..
                } => {
                    for e in &slide.elements {
                        if e.hidden || !e.fill.enabled {
                            continue;
                        }
                        if let FillKind::Media { asset, display, .. } = &e.fill.kind
                            && let Some(src) = res.resolve(*asset)
                        {
                            check(&src, display.alpha);
                        }
                    }
                }
                _ => {}
            }
        }
        pending
    }

    /// Whether `image` is uploaded for drawing on a `size` screen made with
    /// [`Renderer::create_screen`], so a [`Renderer::submit`] draws it without decoding.
    pub fn has_image(
        &self,
        image: &ImageSource,
        alpha: model::SourceAlpha,
        size: (u32, u32),
    ) -> bool {
        self.live.media.has(image, alpha, size)
    }

    /// Diagnostics since the last call (font fallbacks, missing media, unsupported features);
    /// counters restart.
    pub fn take_report(&mut self) -> RenderReport {
        let (media, unsupported) = self.report.take();
        RenderReport {
            adapter: self.gpu.summary.clone(),
            fonts: self.text.fonts.take_issues(),
            media,
            unsupported,
        }
    }

    /// Diagnostics so far, without resetting them.
    pub fn report(&self) -> RenderReport {
        let (media, unsupported) = self.report.snapshot();
        RenderReport {
            adapter: self.gpu.summary.clone(),
            fonts: self.text.fonts.issues(),
            media,
            unsupported,
        }
    }

    /// Creates the GPU resources of one screen.
    pub fn create_screen(&self, config: ScreenConfig) -> ScreenTarget {
        ScreenTarget::new(&self.gpu, config, self.readback_depth)
    }

    /// Renders `layers` and blocks until the frame is read back. Uses an internal target per
    /// distinct screen configuration.
    pub fn render_layers(
        &mut self,
        screen: impl Into<ScreenConfig>,
        layers: &[LayerContent<'_>],
    ) -> anyhow::Result<RenderedFrame> {
        let cfg: ScreenConfig = screen.into();
        let key = (
            cfg.width,
            cfg.height,
            cfg.alpha as u8,
            [
                cfg.background.r.to_bits(),
                cfg.background.g.to_bits(),
                cfg.background.b.to_bits(),
                cfg.background.a.to_bits(),
            ],
        );
        let mut target = match self.screens.remove(&key) {
            Some(t) => t,
            None => ScreenTarget::new(&self.gpu, cfg, 1),
        };
        let result = (|| {
            while target.pending() > 0 {
                target.wait_oldest(&self.gpu)?;
            }
            self.submit_with(&mut target, layers, true)?;
            target
                .wait_oldest(&self.gpu)?
                .ok_or_else(|| anyhow::anyhow!("frame lost"))
        })();
        if self.screens.len() > 8 {
            self.screens.clear();
        }
        self.screens.insert(key, target);
        result
    }

    /// Records and submits one frame for `screen`; returns its sequence number without waiting
    /// (unless every readback buffer is still in flight, then the oldest is waited for and
    /// queued). Collect frames with [`Renderer::poll`] / [`Renderer::wait`].
    pub fn submit(
        &mut self,
        screen: &mut ScreenTarget,
        layers: &[LayerContent<'_>],
    ) -> anyhow::Result<u64> {
        self.submit_with(screen, layers, false)
    }

    /// [`Renderer::submit`] with the live caches (`scratch == false`, live decode priority)
    /// or the scratch caches (thumbnail decode priority).
    fn submit_with(
        &mut self,
        screen: &mut ScreenTarget,
        layers: &[LayerContent<'_>],
        scratch: bool,
    ) -> anyhow::Result<u64> {
        self.tick += 1;
        if let Err(e) = self.recover() {
            anyhow::bail!("the GPU device was lost: {e:#}");
        }
        anyhow::ensure!(!self.gpu.is_lost(), "the GPU device was lost");
        if screen.device != self.gpu.id() {
            screen.rebuild(&self.gpu);
        }
        let slot = screen.acquire_slot(&self.gpu)?;
        let cfg = screen.config;
        let (w, h) = (cfg.width, cfg.height);
        let full = RectF::new(0.0, 0.0, w as f32, h as f32);

        let mut sorted: Vec<&LayerContent<'_>> = layers
            .iter()
            .filter(|l| l.layer.is_visual() && l.opacity > 0.0)
            .collect();
        sorted.sort_by_key(|l| {
            Layer::COMPOSITE_ORDER
                .iter()
                .position(|x| *x == l.layer)
                .unwrap_or(usize::MAX)
        });

        let gpu = self.gpu.clone();
        let caches = if scratch {
            &mut self.scratch
        } else {
            &mut self.live
        };
        let fetch = if self.wait_for_images {
            Fetch::Wait
        } else if scratch {
            Fetch::Queue(DecodePriority::Thumbnail)
        } else {
            Fetch::Queue(DecodePriority::Live)
        };
        let mut scene = scene::Scene {
            gpu: &gpu,
            text: &mut self.text,
            glyphs: &mut self.glyphs,
            media: &mut caches.media,
            decoder: &self.decoder,
            fetch,
            image_box: (w, h),
            pending: 0,
            report: &mut self.report,
            pre: Vec::new(),
            keep: Vec::new(),
        };
        let mut used_stores: Vec<u64> = Vec::new();
        let mut cx = DrawCtx {
            scene: &mut scene,
            slides: &mut caches.slides,
            slide_entries: caches.slide_entries,
            videos: &mut self.videos,
            tick: self.tick,
            size: (w, h),
            stores: &mut screen.stores,
            used_stores: &mut used_stores,
            strips: &mut self.strips,
            flats: 0,
        };
        let mut draws = Vec::new();
        for l in sorted {
            let opacity = l.opacity.clamp(0.0, 1.0);
            draws.extend(source_draws(&mut cx, &l.source, opacity, Pipe::Over));
        }
        screen.stores.retain(|k, _| used_stores.contains(k));
        self.videos.evict(self.tick);
        self.missing = scene.pending;
        let mut passes = std::mem::take(&mut scene.pre);
        let keep = std::mem::take(&mut scene.keep);
        passes.push(Pass {
            label: "screen composite",
            target: screen.composite.view.clone(),
            size: (w, h),
            clear: Some(wgpu::Color::TRANSPARENT),
            draws,
        });
        let mut fu = Uniforms::new(&Affine::IDENTITY, full.corners())
            .color(geom::premul_linear(cfg.background, 1.0));
        fu.modes[0] = cfg.is_opaque() as u32;
        // Colour correction and corner pin ride on the output pass (see `adjust`).
        let pipe = match &screen.adjust {
            Some(a) => {
                fu.modes[1] = a.flags;
                fu.stop_c[..3].copy_from_slice(&a.inverse);
                fu.stop_c[3..5].copy_from_slice(&a.grade);
                Pipe::Adjust
            }
            None => Pipe::Final,
        };
        passes.push(Pass {
            label: "screen output",
            target: screen.output.view.clone(),
            size: (w, h),
            clear: None,
            draws: vec![Draw::new(pipe, fu).with_paint(&screen.composite.view)],
        });
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        gpu.encode(&mut enc, &passes);
        screen.encode_copy(&mut enc, slot);
        gpu.queue.submit([enc.finish()]);
        drop(passes);
        drop(keep);
        // All or nothing: a frame whose submission raised an error is not queued for
        // readback (it would be handed out by the next `wait`, one change late).
        gpu.take_errors()?;
        Ok(screen.start_map(slot))
    }

    /// The oldest finished frame of `screen`, without blocking.
    pub fn poll(&self, screen: &mut ScreenTarget) -> anyhow::Result<Option<RenderedFrame>> {
        if screen.device != self.gpu.id() {
            return Ok(screen.move_to(&self.gpu));
        }
        screen.poll(&self.gpu)
    }

    /// The oldest pending frame of `screen`, blocking until it is ready (`None` when nothing
    /// is pending). Frames still in flight on a device the renderer has replaced
    /// ([`Renderer::recover`]) are dropped, not waited for: only that device could finish them.
    pub fn wait(&self, screen: &mut ScreenTarget) -> anyhow::Result<Option<RenderedFrame>> {
        if screen.device != self.gpu.id() {
            return Ok(screen.move_to(&self.gpu));
        }
        screen.wait_oldest(&self.gpu)
    }

    /// Lays out a text box exactly as the renderer would (canvas px in, canvas px out); for
    /// editors and tests.
    pub fn measure_text(&mut self, text: &model::TextBox, bounds: model::Rect) -> TextMeasure {
        measure(&mut self.text, text, bounds)
    }
}

/// What [`source_draws`] needs while a frame is built.
struct DrawCtx<'s, 'a> {
    scene: &'s mut scene::Scene<'a>,
    slides: &'s mut HashMap<u64, CachedSlide>,
    slide_entries: usize,
    videos: &'s mut video::VideoCache,
    tick: u64,
    size: (u32, u32),
    stores: &'s mut HashMap<u64, Tex>,
    used_stores: &'s mut Vec<u64>,
    /// Scrolling text strips' geometry, by strip key (see `scroll_strip`).
    strips: &'s mut HashMap<u64, scroll::Strip>,
    /// Slides flattened so far this frame (their store keys are `FLAT | n`).
    flats: u64,
}

/// The draws of one layer source covering the whole target (passes it depends on go to
/// `scene.pre`), at `opacity`, with `pipe`. Every source makes at most one draw except
/// `Rects` (which don't overlap) and a slide with moving parts drawn over (with `Pipe::Over`
/// at full opacity; otherwise it is flattened into one draw), so drawing a source at an
/// opacity is exactly a layer opacity.
fn source_draws(
    cx: &mut DrawCtx<'_, '_>,
    source: &LayerSource<'_>,
    opacity: f32,
    pipe: Pipe,
) -> Vec<Draw> {
    let (w, h) = cx.size;
    let full = RectF::new(0.0, 0.0, w as f32, h as f32);
    let gpu = cx.scene.gpu;
    let mut draws = Vec::new();
    let mut with = |d: Draw| {
        let mut d = d;
        d.pipe = pipe;
        draws.push(d);
    };
    match source {
        LayerSource::Color(c) => {
            with(scene::solid_rect(
                &Affine::IDENTITY,
                full,
                geom::premul_linear(*c, opacity),
            ));
        }
        LayerSource::Rects(rects) => {
            for (r, c) in rects {
                let rf = RectF::new(r.x as f32, r.y as f32, r.width as f32, r.height as f32);
                with(scene::solid_rect(
                    &Affine::IDENTITY,
                    rf,
                    geom::premul_linear(*c, opacity),
                ));
            }
        }
        LayerSource::Media { image, display } => {
            let fetch = cx.scene.fetch;
            match cx.scene.media.get(
                gpu,
                cx.scene.decoder,
                image,
                display.alpha,
                (w, h),
                fetch,
                &mut cx.scene.pre,
            ) {
                Lookup::Ready(img) => {
                    let u = scene::media_uniforms(
                        Uniforms::new(&Affine::IDENTITY, full.corners()),
                        display,
                        full,
                        img.width,
                        img.height,
                        1.0,
                        opacity,
                        cx.scene.report,
                    );
                    with(Draw::new(Pipe::Over, u).with_paint(&img.tex.view));
                }
                Lookup::Pending => cx.scene.pending += 1,
                Lookup::Failed(e) => cx.scene.report.media_issue(&image.describe(), &e),
            }
        }
        LayerSource::Video { frame, display } => {
            let tex = cx.videos.frame(gpu, frame, cx.tick, &mut cx.scene.pre);
            let u = scene::media_uniforms(
                Uniforms::new(&Affine::IDENTITY, full.corners()),
                display,
                full,
                tex.width,
                tex.height,
                1.0,
                opacity,
                cx.scene.report,
            );
            with(Draw::new(Pipe::Over, u).with_paint(&tex.view));
            cx.scene.keep.push(tex);
        }
        LayerSource::Slide {
            slide,
            canvas,
            media,
            time,
            videos,
        } => {
            let u = Uniforms::new(&Affine::IDENTITY, full.corners())
                .paint(paint::TEXTURE_PREMUL)
                .paint_rect(full.xywh())
                .opacity(opacity);
            if !scroll::has_scrolling(slide) && !has_video_elements(slide, videos) {
                let tex = slide_texture(cx, slide, *canvas, *media);
                with(Draw::new(Pipe::Over, u).with_paint(&tex.view));
            } else {
                // The still part is cached; what moves is drawn over it every frame. These
                // draws overlap, so at a layer opacity or with another pipe (a dissolve adds
                // its incoming side) they are composited into a texture first and that is
                // drawn as the source's one draw.
                let flatten = pipe != Pipe::Over || opacity < 1.0;
                let o = if flatten { 1.0 } else { opacity };
                let mut base = scroll::base_slide(slide);
                for e in base.elements.iter_mut() {
                    if is_video_element(e, videos) {
                        e.fill.enabled = false;
                    }
                }
                let tex = slide_texture(cx, &base, *canvas, *media);
                let mut parts = vec![Draw::new(Pipe::Over, u.opacity(o)).with_paint(&tex.view)];
                parts.extend(video_element_draws(cx, slide, *canvas, videos, o));
                parts.extend(scroll_draws(cx, slide, *canvas, *media, *time, o));
                if flatten {
                    let key = FLAT | cx.flats;
                    cx.flats += 1;
                    let tex = cx
                        .stores
                        .entry(key)
                        .or_insert_with(|| gpu.linear_target("slide flattened", w, h))
                        .clone();
                    cx.used_stores.push(key);
                    cx.scene.pre.push(Pass {
                        label: "slide flattened",
                        target: tex.view.clone(),
                        size: (w, h),
                        clear: Some(wgpu::Color::TRANSPARENT),
                        draws: parts,
                    });
                    with(Draw::new(Pipe::Over, u).with_paint(&tex.view));
                } else {
                    parts.into_iter().for_each(&mut with);
                }
            }
        }
        LayerSource::Erase { image } => {
            let src = LayerSource::Media {
                image: image.clone(),
                display: MediaDisplay::default(),
            };
            draws.extend(source_draws(cx, &src, opacity, Pipe::DestOut));
        }
        LayerSource::Mix(m) => {
            let p = m.progress.clamp(0.0, 1.0);
            let mut md = Vec::new();
            if let Some(motion) = transition::Motion::of(&m.kind) {
                let from = mix_side(cx, &m.from, m.store, m.store ^ SIDE_FROM);
                let to = mix_side(cx, &m.to, m.store, m.store ^ SIDE_TO);
                md = motion.draws(
                    full,
                    p,
                    from.as_ref().map(|t| &t.view),
                    to.as_ref().map(|t| &t.view),
                );
            } else {
                for (side, weight, side_pipe) in
                    [(&m.from, 1.0 - p, Pipe::Over), (&m.to, p, Pipe::Add)]
                {
                    match side {
                        MixInput::Empty => {}
                        MixInput::Content(LayerSource::Mix(_), _) => {
                            log::warn!("render: nested layer mix ignored");
                        }
                        MixInput::Content(src, o) => {
                            let o = (o.clamp(0.0, 1.0)) * weight;
                            if o > 0.0 {
                                md.extend(source_draws(cx, src, o, side_pipe));
                            }
                        }
                        MixInput::Snapshot(k) => {
                            if *k == m.store {
                                log::warn!("render: a mix cannot start from its own result");
                                continue;
                            }
                            match cx.stores.get(k) {
                                Some(t) => {
                                    cx.used_stores.push(*k);
                                    let u = Uniforms::new(&Affine::IDENTITY, full.corners())
                                        .paint(paint::TEXTURE_PREMUL)
                                        .paint_rect(full.xywh())
                                        .opacity(weight);
                                    md.push(Draw::new(side_pipe, u).with_paint(&t.view));
                                }
                                None => log::debug!("render: mix snapshot {k} is gone"),
                            }
                        }
                    }
                }
            }
            let store = cx
                .stores
                .entry(m.store)
                .or_insert_with(|| gpu.linear_target("layer mix", w, h))
                .clone();
            cx.used_stores.push(m.store);
            cx.scene.pre.push(Pass {
                label: "layer mix",
                target: store.view.clone(),
                size: (w, h),
                clear: Some(wgpu::Color::TRANSPARENT),
                draws: md,
            });
            let u = Uniforms::new(&Affine::IDENTITY, full.corners())
                .paint(paint::TEXTURE_PREMUL)
                .paint_rect(full.xywh())
                .opacity(opacity);
            with(Draw::new(Pipe::Over, u).with_paint(&store.view));
        }
    }
    draws
}

/// The slide layer texture of `slide` at the screen's size: cached by content, rendered into
/// a `pre` pass when it isn't.
fn slide_texture(
    cx: &mut DrawCtx<'_, '_>,
    slide: &Slide,
    canvas: Size,
    media: Option<&dyn MediaResolver>,
) -> Tex {
    let gpu = cx.scene.gpu;
    let (w, h) = cx.size;
    let key = slide_key(slide, canvas, (w, h), media);
    match cx.slides.get_mut(&key) {
        Some(c) => {
            c.tick = cx.tick;
            c.tex.clone()
        }
        None => {
            let tex = gpu.linear_target("slide layer", w, h);
            let pending_before = cx.scene.pending;
            let mut sd = Vec::new();
            cx.scene.slide(
                slide,
                (canvas.width as f32, canvas.height as f32),
                (w, h),
                media,
                &mut sd,
            );
            cx.scene.pre.push(Pass {
                label: "slide layer",
                target: tex.view.clone(),
                size: (w, h),
                clear: Some(wgpu::Color::TRANSPARENT),
                draws: sd,
            });
            // A slide drawn without a still-decoding image is not cached: the next render
            // draws it again. One drawn without an image that could not be read is: its key
            // names each media file's version (see `slide_key`), so the file arriving or
            // changing draws it again.
            if cx.scene.pending == pending_before {
                if cx.slides.len() >= cx.slide_entries
                    && let Some(old) = cx
                        .slides
                        .iter()
                        .min_by_key(|(_, c)| c.tick)
                        .map(|(k, _)| *k)
                {
                    cx.slides.remove(&old);
                }
                cx.slides.insert(
                    key,
                    CachedSlide {
                        tex: tex.clone(),
                        tick: cx.tick,
                    },
                );
            }
            tex
        }
    }
}

/// `e` plays a video: a visible media or live-source fill with a frame in `videos`.
fn is_video_element(e: &model::Element, videos: &[(model::ElementId, VideoFrameRef<'_>)]) -> bool {
    !e.hidden
        && e.fill.enabled
        && matches!(
            e.fill.kind,
            FillKind::Media { .. } | FillKind::LiveSource { .. }
        )
        && videos.iter().any(|(id, _)| *id == e.id)
}

fn has_video_elements(slide: &Slide, videos: &[(model::ElementId, VideoFrameRef<'_>)]) -> bool {
    !videos.is_empty() && slide.elements.iter().any(|e| is_video_element(e, videos))
}

/// Video elements' current frames in their boxes (per the fill's display settings, clipped
/// to the box), drawn over the cached slide. Rotation and flips are not applied, nor the
/// element's stroke order (the frame is drawn above the slide).
fn video_element_draws(
    cx: &mut DrawCtx<'_, '_>,
    slide: &Slide,
    canvas: Size,
    videos: &[(model::ElementId, VideoFrameRef<'_>)],
    opacity: f32,
) -> Vec<Draw> {
    let (w, h) = cx.size;
    let (tw, th) = (w as f32, h as f32);
    let (cw, ch) = (canvas.width.max(1.0) as f32, canvas.height.max(1.0) as f32);
    let s = (tw / cw).min(th / ch);
    let (ox, oy) = ((tw - cw * s) * 0.5, (th - ch * s) * 0.5);
    let mut out = Vec::new();
    for e in slide
        .elements
        .iter()
        .filter(|e| is_video_element(e, videos))
    {
        if !scene::visible(slide, e) {
            continue;
        }
        let (
            FillKind::Media { display, .. } | FillKind::LiveSource { display, .. },
            Some((_, frame)),
        ) = (&e.fill.kind, videos.iter().find(|(id, _)| *id == e.id))
        else {
            continue;
        };
        let b = e.bounds;
        let bx = RectF::new(
            ox + b.x as f32 * s,
            oy + b.y as f32 * s,
            b.width as f32 * s,
            b.height as f32 * s,
        );
        if bx.is_empty() {
            continue;
        }
        let tex = cx
            .videos
            .frame(cx.scene.gpu, frame, cx.tick, &mut cx.scene.pre);
        let u = scene::media_uniforms(
            Uniforms::new(&Affine::IDENTITY, bx.corners()),
            display,
            bx,
            tex.width,
            tex.height,
            s,
            opacity * e.opacity.clamp(0.0, 1.0) as f32,
            cx.scene.report,
        );
        out.push(Draw::new(Pipe::Over, u).with_paint(&tex.view));
        cx.scene.keep.push(tex);
    }
    out
}

/// The draws of a slide's scrolling text (see `scroll`): each scrolling element's strip,
/// rendered once into a cached texture, drawn at its copies' positions clipped to its box.
fn scroll_draws(
    cx: &mut DrawCtx<'_, '_>,
    slide: &Slide,
    canvas: Size,
    media: Option<&dyn MediaResolver>,
    time: Option<f64>,
    opacity: f32,
) -> Vec<Draw> {
    let (w, h) = cx.size;
    let (tw, th) = (w as f32, h as f32);
    let (cw, ch) = (canvas.width.max(1.0) as f32, canvas.height.max(1.0) as f32);
    let s = (tw / cw).min(th / ch);
    let (ox, oy) = ((tw - cw * s) * 0.5, (th - ch * s) * 0.5);
    let mut out = Vec::new();
    for el in slide.elements.iter().filter(|e| scroll::scrolls(e)) {
        if !scene::visible(slide, el) {
            continue;
        }
        let Some((tex, strip)) = scroll_strip(cx, el, s, media) else {
            continue;
        };
        let b = el.bounds;
        let bx = RectF::new(
            ox + b.x as f32 * s,
            oy + b.y as f32 * s,
            b.width as f32 * s,
            b.height as f32 * s,
        );
        let len = if strip.horizontal { b.width } else { b.height } as f32;
        // The texture holds the strip 1:1 up to the largest texture; past that it is cut off.
        let drawn = if strip.horizontal {
            tex.width
        } else {
            tex.height
        } as f32;
        for p in scroll::copies(&el.scroll, &strip, len, time) {
            let r = if strip.horizontal {
                RectF::new(bx.x + p * s, bx.y, drawn, bx.h)
            } else {
                RectF::new(bx.x, bx.y + p * s, bx.w, drawn)
            };
            let clip = scroll::intersect(r, bx);
            if clip.is_empty() {
                continue;
            }
            let u = Uniforms::new(&Affine::IDENTITY, clip.corners())
                .paint(paint::TEXTURE_PREMUL)
                .paint_rect(r.xywh())
                .opacity(opacity);
            out.push(Draw::new(Pipe::Over, u).with_paint(&tex.view));
        }
    }
    out
}

/// The strip texture of scrolling element `el` at scale `s` (screen px per canvas px) and
/// where it sits; both cached by the element's content and the scale.
fn scroll_strip(
    cx: &mut DrawCtx<'_, '_>,
    el: &model::Element,
    s: f32,
    media: Option<&dyn MediaResolver>,
) -> Option<(Tex, scroll::Strip)> {
    let tb = el.text_box()?;
    let key = {
        let mut hh = std::collections::hash_map::DefaultHasher::new();
        "scroll strip".hash(&mut hh);
        serde_json::to_vec(&el.kind)
            .unwrap_or_default()
            .hash(&mut hh);
        serde_json::to_vec(&el.bounds)
            .unwrap_or_default()
            .hash(&mut hh);
        el.opacity.to_bits().hash(&mut hh);
        el.scroll.direction.hash(&mut hh);
        s.to_bits().hash(&mut hh);
        hh.finish()
    };
    let horizontal = matches!(
        el.scroll.direction,
        model::ScrollDirection::Left
            | model::ScrollDirection::Right
            | model::ScrollDirection::Unknown
    );
    let b = el.bounds;
    let strip = match cx.strips.get(&key) {
        Some(st) => *st,
        None => {
            let mut t = tb.clone();
            t.fit = model::FitMode::None;
            let long = scroll::LAYOUT_LENGTH as f64;
            let (layout, in_box) = if horizontal {
                (
                    model::Rect::new(0.0, 0.0, long, b.height),
                    model::Rect::new(0.0, 0.0, b.width, b.height),
                )
            } else {
                (
                    model::Rect::new(0.0, 0.0, b.width, long),
                    model::Rect::new(0.0, 0.0, b.width, b.height),
                )
            };
            let m = measure(cx.scene.text, &t, layout).line_boxes;
            let (start, len) = if horizontal { (m.x, m.w) } else { (m.y, m.h) };
            let length = len + 2.0 * scroll::PAD;
            let fits = len <= if horizontal { b.width } else { b.height } as f32;
            let laid_out = if fits {
                let mb = measure(cx.scene.text, &t, in_box).line_boxes;
                (if horizontal { mb.x } else { mb.y }) - scroll::PAD
            } else {
                -scroll::PAD
            };
            let st = scroll::Strip {
                horizontal,
                length,
                layout_start: start - scroll::PAD,
                laid_out,
            };
            if cx.strips.len() > 256 {
                cx.strips.clear();
            }
            cx.strips.insert(key, st);
            st
        }
    };
    if strip.length <= 2.0 * scroll::PAD {
        return None;
    }
    if let Some(c) = cx.slides.get_mut(&key) {
        c.tick = cx.tick;
        return Some((c.tex.clone(), strip));
    }
    // Render the strip: the text alone, laid out as measured, its start at the strip's start.
    let max = cx.scene.gpu.max_texture as f32;
    let (sw, sh) = if horizontal {
        ((strip.length * s).min(max), b.height as f32 * s)
    } else {
        (b.width as f32 * s, (strip.length * s).min(max))
    };
    let (pw, ph) = ((sw.ceil() as u32).max(1), (sh.ceil() as u32).max(1));
    let long = scroll::LAYOUT_LENGTH as f64;
    let bounds = if horizontal {
        model::Rect::new(-(strip.layout_start as f64), 0.0, long, b.height)
    } else {
        model::Rect::new(0.0, -(strip.layout_start as f64), b.width, long)
    };
    let one = Slide {
        elements: vec![scroll::strip_element(el, bounds)],
        ..Slide::default()
    };
    let tex = cx.scene.gpu.linear_target("scroll strip", pw, ph);
    let mut sd = Vec::new();
    cx.scene.slide(
        &one,
        (pw as f32 / s, ph as f32 / s),
        (pw, ph),
        media,
        &mut sd,
    );
    cx.scene.pre.push(Pass {
        label: "scroll strip",
        target: tex.view.clone(),
        size: (pw, ph),
        clear: Some(wgpu::Color::TRANSPARENT),
        draws: sd,
    });
    if cx.slides.len() >= cx.slide_entries
        && let Some(old) = cx
            .slides
            .iter()
            .min_by_key(|(_, c)| c.tick)
            .map(|(k, _)| *k)
    {
        cx.slides.remove(&old);
    }
    cx.slides.insert(
        key,
        CachedSlide {
            tex: tex.clone(),
            tick: cx.tick,
        },
    );
    Some((tex, strip))
}

/// Store keys of a moving transition's side textures (its `store` key with a high bit set;
/// transition keys count up from 1).
const SIDE_FROM: u64 = 1 << 63;
const SIDE_TO: u64 = 1 << 62;
/// Store keys of slides flattened into one draw (counted per frame).
const FLAT: u64 = 1 << 61;

/// One side of a moving transition as a screen-sized texture (`None`: empty). Live content is
/// drawn into a texture kept under `key`; a snapshot is the earlier mix's own texture.
fn mix_side(cx: &mut DrawCtx<'_, '_>, side: &MixInput<'_>, store: u64, key: u64) -> Option<Tex> {
    let (w, h) = cx.size;
    match side {
        MixInput::Empty => None,
        MixInput::Content(LayerSource::Mix(_), _) => {
            log::warn!("render: nested layer mix ignored");
            None
        }
        MixInput::Content(src, o) => {
            let draws = source_draws(cx, src, o.clamp(0.0, 1.0), Pipe::Over);
            let tex = cx
                .stores
                .entry(key)
                .or_insert_with(|| cx.scene.gpu.linear_target("mix side", w, h))
                .clone();
            cx.used_stores.push(key);
            cx.scene.pre.push(Pass {
                label: "mix side",
                target: tex.view.clone(),
                size: (w, h),
                clear: Some(wgpu::Color::TRANSPARENT),
                draws,
            });
            Some(tex)
        }
        MixInput::Snapshot(k) if *k == store => {
            log::warn!("render: a mix cannot start from its own result");
            None
        }
        MixInput::Snapshot(k) => match cx.stores.get(k) {
            Some(t) => {
                cx.used_stores.push(*k);
                Some(t.clone())
            }
            None => {
                log::debug!("render: mix snapshot {k} is gone");
                None
            }
        },
    }
}

/// Whether `slide` has scrolling text (a screen showing it changes every frame: render it on
/// every tick, with [`LayerSource::Slide`]'s `time`).
pub fn has_scrolling_text(slide: &Slide) -> bool {
    scroll::has_scrolling(slide)
}

/// The text engine of a renderer made with `options`: its font database (system fonts, then
/// `font_dirs`), fallback family and substitution table.
fn text_engine(options: &RendererOptions) -> text::TextEngine {
    let mut db = fonts::load_database(&options.font_dirs, options.system_fonts);
    let locale = std::env::var("LANG")
        .ok()
        .and_then(|l| l.split('.').next().map(|s| s.replace('_', "-")))
        .filter(|l| !l.is_empty() && l != "C" && l != "POSIX")
        .unwrap_or_else(|| "en-US".into());
    db.set_sans_serif_family(options.fallback_family.clone());
    db.set_serif_family("DejaVu Serif");
    db.set_monospace_family("DejaVu Sans Mono");
    let fs = cosmic_text::FontSystem::new_with_locale_and_db(locale, db);
    let resolver = fonts::FontResolver::new(
        options.font_substitutions.clone(),
        options.fallback_family.clone(),
    );
    text::TextEngine::new(fs, resolver)
}

/// Text layout without a GPU: the same fonts, fallback and substitutions as a [`Renderer`]
/// made with the same options, so a box measured here wraps exactly as it renders (for
/// splitting text into slides off the render thread). Building one loads the font database
/// (keep it).
pub struct TextMeasurer {
    text: text::TextEngine,
}

impl TextMeasurer {
    /// Loads the fonts of `options` (only the font fields are used).
    pub fn new(options: &RendererOptions) -> Self {
        TextMeasurer {
            text: text_engine(options),
        }
    }

    /// Replaces the font substitution table.
    pub fn set_font_substitutions(&mut self, table: FontSubstitutionTable) {
        self.text.fonts.set_substitutions(table);
    }

    /// Lays out a text box as [`Renderer::measure_text`] does.
    pub fn measure(&mut self, text: &model::TextBox, bounds: model::Rect) -> TextMeasure {
        measure(&mut self.text, text, bounds)
    }

    /// Whether the text fits its box at its authored size (whatever its fit mode says): the
    /// block fits the height inside the margins and no line is wider than the box.
    pub fn fits(&mut self, text: &model::TextBox, bounds: model::Rect) -> bool {
        let mut tb = text.clone();
        tb.fit = model::FitMode::None;
        let r = RectF::new(0.0, 0.0, bounds.width as f32, bounds.height as f32);
        let l = self.text.layout(&tb, r, 1.0);
        let c = l.content;
        let wide = l
            .lines
            .iter()
            .any(|b| b.x < c.x - 0.5 || b.x + b.w > c.x + c.w + 0.5);
        l.block_height <= c.h + 0.5 && !wide
    }
}

fn measure(t: &mut text::TextEngine, text: &model::TextBox, bounds: model::Rect) -> TextMeasure {
    let r = RectF::new(0.0, 0.0, bounds.width as f32, bounds.height as f32);
    let l = t.layout(text, r, 1.0);
    let ink = l
        .lines
        .iter()
        .fold(RectF::default(), |a, b| a.union(b))
        .translate(bounds.x as f32, bounds.y as f32);
    TextMeasure {
        fit_scale: l.fit_scale,
        block_height: l.block_height,
        lines: l.lines.len(),
        line_boxes: ink,
    }
}

/// Result of [`Renderer::measure_text`].
#[derive(Clone, Debug)]
pub struct TextMeasure {
    /// Fit scale applied (1 = authored size; < 1 = shrunk).
    pub fit_scale: f32,
    /// Height of the text block, canvas px.
    pub block_height: f32,
    /// Number of laid-out lines.
    pub lines: usize,
    /// Union of the line boxes, canvas px.
    pub line_boxes: RectF,
}

fn slide_key(
    slide: &Slide,
    canvas: Size,
    target: (u32, u32),
    media: Option<&dyn MediaResolver>,
) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_vec(slide).unwrap_or_default().hash(&mut h);
    canvas.width.to_bits().hash(&mut h);
    canvas.height.to_bits().hash(&mut h);
    target.hash(&mut h);
    if let Some(m) = media {
        for e in &slide.elements {
            if let FillKind::Media { asset, .. } = &e.fill.kind {
                let src = m.resolve(*asset);
                src.as_ref().map(|s| s.describe()).hash(&mut h);
                if let Some(ImageSource::Path(p)) = &src {
                    decode::FileKey::of(p).ok().hash(&mut h);
                }
            }
        }
    }
    h.finish()
}
