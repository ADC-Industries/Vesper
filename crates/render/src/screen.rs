//! Per-screen render targets and the GPU → CPU readback ring.
//!
//! Each [`ScreenTarget`] owns a linear composite texture, an RGBA8 output texture and a ring of
//! readback buffers (default 3). [`Renderer::submit`](crate::Renderer::submit) records a frame
//! and its copy into the next free buffer and returns immediately; the buffer is mapped
//! asynchronously and [`Renderer::poll`](crate::Renderer::poll) hands back finished frames
//! in submission order. With 2–3 buffers in flight, the GPU renders frame *n+1* while frame *n*
//! is being copied out, which is what 30–60 fps on several screens needs. Rows are copied with
//! `bytes_per_row` padded to 256 bytes (wgpu's `COPY_BYTES_PER_ROW_ALIGNMENT`) and unpadded
//! into a tightly packed frame.

use crate::adjust::Adjust;
use crate::gpu::{Gpu, OUTPUT_FORMAT, Tex, create_texture};
use model::{AlphaMode, Color};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// A rendered frame: RGBA8, **sRGB-encoded, premultiplied alpha** (premultiplied in the
/// encoded domain), tightly packed rows (`width * 4` bytes), top row first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub data: Vec<u8>,
    /// The screen's alpha intent. `Opaque` frames always have alpha 255.
    pub alpha: AlphaMode,
    /// Submission number on its screen target (frames come back in order).
    pub sequence: u64,
}

impl RenderedFrame {
    /// The pixel at (x, y) as `[r, g, b, a]` (premultiplied).
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ]
    }

    /// Straight-alpha copy (what PNG and most image tools expect).
    pub fn to_straight_rgba(&self) -> image::RgbaImage {
        let mut out = self.data.clone();
        for p in out.as_chunks_mut::<4>().0 {
            let a = p[3] as u32;
            if a > 0 && a < 255 {
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        image::RgbaImage::from_raw(self.width, self.height, out).expect("frame size")
    }

    /// Writes the frame as a PNG (straight alpha; opaque frames are written as RGB).
    pub fn save_png(&self, path: impl AsRef<std::path::Path>) -> anyhow::Result<()> {
        let img = self.to_straight_rgba();
        if self.alpha == AlphaMode::Opaque || self.alpha == AlphaMode::Unknown {
            image::DynamicImage::ImageRgba8(img)
                .to_rgb8()
                .save(path.as_ref())?;
        } else {
            img.save(path.as_ref())?;
        }
        Ok(())
    }
}

/// Output settings of one screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenConfig {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Alpha intent: `Opaque` flattens over `background` (then black); the others keep alpha.
    pub alpha: AlphaMode,
    /// Colour behind all layers (straight sRGB).
    pub background: Color,
}

impl ScreenConfig {
    /// An opaque screen over black.
    pub fn opaque(width: u32, height: u32) -> Self {
        ScreenConfig {
            width,
            height,
            alpha: AlphaMode::Opaque,
            background: Color::BLACK,
        }
    }

    /// A screen with alpha (transparent background, premultiplied delivery).
    pub fn transparent(width: u32, height: u32) -> Self {
        ScreenConfig {
            width,
            height,
            alpha: AlphaMode::Premultiplied,
            background: Color::TRANSPARENT,
        }
    }

    pub(crate) fn is_opaque(&self) -> bool {
        matches!(self.alpha, AlphaMode::Opaque | AlphaMode::Unknown)
    }
}

impl From<model::Size> for ScreenConfig {
    /// An opaque screen of that size over black.
    fn from(s: model::Size) -> Self {
        ScreenConfig::opaque(
            s.width.round().max(1.0) as u32,
            s.height.round().max(1.0) as u32,
        )
    }
}

impl From<(u32, u32)> for ScreenConfig {
    fn from((w, h): (u32, u32)) -> Self {
        ScreenConfig::opaque(w, h)
    }
}

impl From<&model::Screen> for ScreenConfig {
    fn from(s: &model::Screen) -> Self {
        ScreenConfig {
            width: s.size.width.round().max(1.0) as u32,
            height: s.size.height.round().max(1.0) as u32,
            alpha: s.alpha,
            background: s.background,
        }
    }
}

const PENDING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    sequence: u64,
    busy: bool,
}

/// GPU resources of one screen (see the module docs). Create with
/// [`Renderer::create_screen`](crate::Renderer::create_screen).
pub struct ScreenTarget {
    pub(crate) config: ScreenConfig,
    pub(crate) composite: Tex,
    pub(crate) output: Tex,
    slots: Vec<Slot>,
    in_flight: VecDeque<usize>,
    ready: VecDeque<RenderedFrame>,
    next_sequence: u64,
    padded_bpr: u32,
    recycled: Vec<Vec<u8>>,
    /// Results of layer mixes (transitions), by store key.
    pub(crate) stores: std::collections::HashMap<u64, Tex>,
    /// Colour correction and corner pin as set (see [`ScreenTarget::set_adjustments`]).
    settings: (model::ColorCorrection, model::CornerPin),
    /// What the output pass applies (`None`: the plain output pass).
    pub(crate) adjust: Option<Adjust>,
    /// The device its resources live on ([`Gpu::id`]).
    pub(crate) device: usize,
}

impl ScreenTarget {
    pub(crate) fn new(gpu: &Gpu, config: ScreenConfig, depth: usize) -> Self {
        let w = config.width.clamp(1, gpu.max_texture);
        let h = config.height.clamp(1, gpu.max_texture);
        let config = ScreenConfig {
            width: w,
            height: h,
            ..config
        };
        let composite = gpu.linear_target("screen composite", w, h);
        let output = create_texture(
            &gpu.device,
            "screen output",
            w,
            h,
            OUTPUT_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bpr = (w * 4).div_ceil(align) * align;
        let slots = (0..depth.max(1))
            .map(|_| Slot {
                buffer: gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("readback"),
                    size: padded_bpr as u64 * h as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(PENDING)),
                sequence: 0,
                busy: false,
            })
            .collect();
        ScreenTarget {
            config,
            composite,
            output,
            slots,
            in_flight: VecDeque::new(),
            ready: VecDeque::new(),
            next_sequence: 0,
            padded_bpr,
            recycled: Vec::new(),
            stores: Default::default(),
            settings: (
                model::ColorCorrection::IDENTITY,
                model::CornerPin::default(),
            ),
            adjust: None,
            device: gpu.id(),
        }
    }

    /// Makes the GPU resources again on `gpu` (after the device they lived on was lost),
    /// keeping the settings. Frames in flight on the old device are dropped.
    pub(crate) fn rebuild(&mut self, gpu: &Gpu) {
        let mut fresh = ScreenTarget::new(gpu, self.config, self.slots.len());
        fresh.next_sequence = self.next_sequence;
        fresh.settings = self.settings;
        fresh.adjust = self.adjust;
        *self = fresh;
    }

    /// A frame already read back, else moves the screen to `gpu` (the renderer replaced the
    /// device its frames in flight were submitted on; they are dropped) and `None`.
    pub(crate) fn move_to(&mut self, gpu: &Gpu) -> Option<RenderedFrame> {
        if let Some(f) = self.ready.pop_front() {
            return Some(f);
        }
        if !self.in_flight.is_empty() {
            log::warn!(
                "render: {} frame(s) in flight were lost with the GPU device",
                self.in_flight.len()
            );
        }
        self.rebuild(gpu);
        None
    }

    /// Sets the screen's colour correction and corner pin, applied from the next submitted
    /// frame on by the output pass (no new textures; frames in flight are unaffected).
    /// Settings that change nothing keep the plain output pass: frames bit for bit as
    /// without them. Returns whether anything changed.
    pub fn set_adjustments(
        &mut self,
        color: &model::ColorCorrection,
        pin: &model::CornerPin,
    ) -> bool {
        if self.settings == (*color, *pin) {
            return false;
        }
        self.settings = (*color, *pin);
        self.adjust = Adjust::new(color, pin, self.config.width, self.config.height);
        true
    }

    /// The colour correction and corner pin last set.
    pub fn adjustments(&self) -> (model::ColorCorrection, model::CornerPin) {
        self.settings
    }

    /// The screen's settings (size clamped to the device limit).
    pub fn config(&self) -> ScreenConfig {
        self.config
    }

    /// Frames submitted but not yet returned.
    pub fn pending(&self) -> usize {
        self.in_flight.len() + self.ready.len()
    }

    /// Gives a frame buffer back for reuse (avoids an allocation per frame).
    pub fn recycle(&mut self, frame: RenderedFrame) {
        if self.recycled.len() < self.slots.len() + 2 {
            self.recycled.push(frame.data);
        }
    }

    /// A free slot index, waiting for the oldest frame if all are busy.
    pub(crate) fn acquire_slot(&mut self, gpu: &Gpu) -> anyhow::Result<usize> {
        if let Some(i) = self.slots.iter().position(|s| !s.busy) {
            return Ok(i);
        }
        if let Some(f) = self.wait_in_flight(gpu)? {
            self.ready.push_back(f);
        }
        self.slots
            .iter()
            .position(|s| !s.busy)
            .ok_or_else(|| anyhow::anyhow!("no free readback slot"))
    }

    /// Records the copy of the output texture into `slot`.
    pub(crate) fn encode_copy(&self, enc: &mut wgpu::CommandEncoder, slot: usize) {
        let (w, h) = (self.config.width, self.config.height);
        enc.copy_texture_to_buffer(
            self.output.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.slots[slot].buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bpr),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    /// After submission: start mapping `slot` asynchronously.
    pub(crate) fn start_map(&mut self, slot: usize) -> u64 {
        let seq = self.next_sequence;
        self.next_sequence += 1;
        let s = &mut self.slots[slot];
        s.busy = true;
        s.sequence = seq;
        s.state.store(PENDING, Ordering::Release);
        let state = s.state.clone();
        s.buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
        });
        self.in_flight.push_back(slot);
        seq
    }

    /// The oldest finished frame, without blocking.
    pub(crate) fn poll(&mut self, gpu: &Gpu) -> anyhow::Result<Option<RenderedFrame>> {
        if let Some(f) = self.ready.pop_front() {
            return Ok(Some(f));
        }
        let Some(&slot) = self.in_flight.front() else {
            return Ok(None);
        };
        let _ = gpu.device.poll(wgpu::PollType::Poll);
        if self.slots[slot].state.load(Ordering::Acquire) == PENDING {
            return Ok(None);
        }
        self.take(slot).map(Some)
    }

    /// The oldest frame, blocking until the GPU has finished it. `None` when nothing is
    /// pending.
    pub(crate) fn wait_oldest(&mut self, gpu: &Gpu) -> anyhow::Result<Option<RenderedFrame>> {
        if let Some(f) = self.ready.pop_front() {
            return Ok(Some(f));
        }
        self.wait_in_flight(gpu)
    }

    /// Waits for the oldest in-flight frame (ignoring frames already read back).
    fn wait_in_flight(&mut self, gpu: &Gpu) -> anyhow::Result<Option<RenderedFrame>> {
        let Some(&slot) = self.in_flight.front() else {
            return Ok(None);
        };
        while self.slots[slot].state.load(Ordering::Acquire) == PENDING {
            gpu.device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| anyhow::anyhow!("GPU poll failed: {e}"))?;
        }
        self.take(slot).map(Some)
    }

    fn take(&mut self, slot: usize) -> anyhow::Result<RenderedFrame> {
        self.in_flight.pop_front();
        let (w, h) = (self.config.width, self.config.height);
        let s = &mut self.slots[slot];
        s.busy = false;
        if s.state.load(Ordering::Acquire) == FAILED {
            anyhow::bail!("mapping the readback buffer failed");
        }
        let row = (w * 4) as usize;
        let mut data = self.recycled.pop().unwrap_or_default();
        data.clear();
        data.reserve(row * h as usize);
        {
            let view = s
                .buffer
                .get_mapped_range(..)
                .map_err(|e| anyhow::anyhow!("readback map: {e:?}"))?;
            let pbpr = self.padded_bpr as usize;
            for y in 0..h as usize {
                data.extend_from_slice(&view[y * pbpr..y * pbpr + row]);
            }
        }
        s.buffer.unmap();
        Ok(RenderedFrame {
            width: w,
            height: h,
            data,
            alpha: self.config.alpha,
            sequence: s.sequence,
        })
    }
}
