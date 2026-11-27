//! GPU plumbing: adapter selection, the one shader's pipelines, per-draw uniforms and a tiny
//! pass list ("render graph") that is encoded in order.

use crate::geom::Affine;
use anyhow::{Context, anyhow};
use bytemuck::{Pod, Zeroable};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Intermediate format: linear light, premultiplied alpha, half float.
pub(crate) const LINEAR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Output format: sRGB-encoded premultiplied RGBA8 (encoded by the shader, so not `*Srgb`).
pub(crate) const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Uniform slot stride (≥ every adapter's `min_uniform_buffer_offset_alignment`).
const UNIFORM_STRIDE: u64 = 512;

/// What the renderer runs on.
#[derive(Clone, Debug)]
pub struct AdapterSummary {
    /// Adapter name ("AMD Radeon 680M (RADV REMBRANDT)", "llvmpipe (LLVM 20.1, 256 bits)").
    pub name: String,
    /// Graphics backend (Vulkan, Dx12, Metal, Gl).
    pub backend: String,
    /// Device type (IntegratedGpu, DiscreteGpu, Cpu, ...).
    pub device_type: String,
    /// Driver name and version.
    pub driver: String,
    /// True for software rasterizers (lavapipe / llvmpipe / WARP): fine for CI and tests, too
    /// slow for production playout.
    pub is_software: bool,
}

impl std::fmt::Display for AdapterSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({}, {}{}{})",
            self.name,
            self.backend,
            self.device_type,
            if self.driver.is_empty() { "" } else { ", " },
            self.driver
        )?;
        if self.is_software {
            write!(f, " [software rendering]")?;
        }
        Ok(())
    }
}

/// Adapter preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AdapterPreference {
    /// A hardware GPU (discrete, then integrated) if there is one, else anything (lavapipe).
    #[default]
    HardwareFirst,
    /// Force a software adapter (deterministic CI output).
    Software,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
#[repr(C)]
pub(crate) struct Uniforms {
    pub xf0: [f32; 4],
    pub xf1: [f32; 4],
    pub quad: [f32; 4],
    pub target_info: [f32; 4],
    pub shape: [f32; 4],
    pub shape_p: [f32; 4],
    pub mask_xf: [f32; 4],
    pub paint_xf: [f32; 4],
    pub uv_m: [f32; 4],
    pub uv_crop: [f32; 4],
    pub color: [f32; 4],
    pub modes: [u32; 4],
    pub blur: [f32; 4],
    pub grad: [f32; 4],
    pub grad_geo: [f32; 4],
    pub stop_c: [[f32; 4]; 8],
    pub stop_p: [[f32; 4]; 2],
    pub stop_m: [[f32; 4]; 2],
}

const _: () = assert!(std::mem::size_of::<Uniforms>() as u64 <= UNIFORM_STRIDE);

/// Coverage modes (`modes.x`).
pub(crate) mod cov {
    pub const FULL: u32 = 0;
    pub const MASK: u32 = 1;
    pub const RRECT_FILL: u32 = 2;
    pub const RRECT_STROKE: u32 = 3;
    pub const ELLIPSE_FILL: u32 = 4;
    pub const ELLIPSE_STROKE: u32 = 5;
    pub const BLUR: u32 = 6;
}

/// Paint modes (`modes.y`).
pub(crate) mod paint {
    pub const SOLID: u32 = 0;
    pub const TEXTURE_PREMUL: u32 = 1;
    pub const TEXTURE_STRAIGHT: u32 = 2;
    pub const GRADIENT: u32 = 3;
}

/// Mask channels (`modes.z`).
pub(crate) mod channel {
    pub const R: u32 = 0;
    pub const A: u32 = 3;
}

/// Flag: paint texture outside its unit rect is transparent.
pub(crate) const FLAG_CLIP_PAINT: u32 = 1;

impl Uniforms {
    /// A draw of `quad` (local frame) through `xf`, with full coverage and white paint.
    pub fn new(xf: &Affine, quad: [f32; 4]) -> Self {
        Uniforms {
            xf0: [xf.a, xf.b, xf.c, 0.0],
            xf1: [xf.d, xf.e, xf.f, 0.0],
            quad,
            target_info: [1.0, 1.0, 1.0, 0.0],
            mask_xf: [0.0, 0.0, 1.0, 1.0],
            paint_xf: [0.0, 0.0, 1.0, 1.0],
            uv_m: [1.0, 0.0, 0.0, 1.0],
            uv_crop: [0.0, 0.0, 1.0, 1.0],
            color: [1.0, 1.0, 1.0, 1.0],
            modes: [cov::FULL, paint::SOLID, channel::R, 0],
            ..Uniforms::zeroed()
        }
    }
    pub fn opacity(mut self, o: f32) -> Self {
        self.target_info[2] = o;
        self
    }
    pub fn color(mut self, c: [f32; 4]) -> Self {
        self.color = c;
        self
    }
    pub fn coverage(mut self, mode: u32) -> Self {
        self.modes[0] = mode;
        self
    }
    pub fn paint(mut self, mode: u32) -> Self {
        self.modes[1] = mode;
        self
    }
    /// Coverage from a mask texture placed at `rect` (x, y, w, h) in the local frame.
    pub fn mask(mut self, rect: [f32; 4], ch: u32) -> Self {
        self.modes[0] = cov::MASK;
        self.mask_xf = rect;
        self.modes[2] = ch;
        self
    }
    /// Texture paint mapped onto `rect` (x, y, w, h) of the local frame.
    pub fn paint_rect(mut self, rect: [f32; 4]) -> Self {
        self.paint_xf = rect;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pipe {
    /// Premultiplied "over" into a linear target.
    Over,
    /// Overwrite a linear target.
    Replace,
    /// Erase the destination by the source alpha (cut-out).
    DestOut,
    /// Final conversion into the RGBA8 output.
    Final,
    /// Final conversion with the screen's corner pin and colour correction (`adjust`).
    Adjust,
    /// Additive into a linear target (the incoming side of a dissolve).
    Add,
    /// Video YCbCr planes (mask = Y, paint = CbCr) → linear RGB, overwriting.
    Yuv,
}

pub(crate) struct Draw {
    pub pipe: Pipe,
    pub u: Uniforms,
    pub mask: Option<wgpu::TextureView>,
    pub paint: Option<wgpu::TextureView>,
}

impl Draw {
    pub fn new(pipe: Pipe, u: Uniforms) -> Self {
        Draw {
            pipe,
            u,
            mask: None,
            paint: None,
        }
    }
    pub fn with_mask(mut self, v: &wgpu::TextureView) -> Self {
        self.mask = Some(v.clone());
        self
    }
    pub fn with_paint(mut self, v: &wgpu::TextureView) -> Self {
        self.paint = Some(v.clone());
        self
    }
}

/// One render pass: clears (optionally) and draws in order into one target.
pub(crate) struct Pass {
    pub label: &'static str,
    pub target: wgpu::TextureView,
    pub size: (u32, u32),
    pub clear: Option<wgpu::Color>,
    pub draws: Vec<Draw>,
}

/// A texture plus its default view and size.
#[derive(Clone)]
pub(crate) struct Tex {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl Tex {
    pub fn bytes(&self) -> u64 {
        let bpp = match self.texture.format() {
            wgpu::TextureFormat::R8Unorm => 1,
            wgpu::TextureFormat::Rgba16Float => 8,
            _ => 4,
        };
        let mips = self.texture.mip_level_count() as u64;
        self.width as u64 * self.height as u64 * bpp * if mips > 1 { 4 } else { 3 } / 3
    }
}

pub(crate) struct Gpu {
    /// The process-wide instance and the adapter the device was made from (window surfaces,
    /// see [`crate::present`]).
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub summary: AdapterSummary,
    pub max_texture: u32,
    bgl_uniform: wgpu::BindGroupLayout,
    bgl_tex: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipe_over: wgpu::RenderPipeline,
    pipe_replace: wgpu::RenderPipeline,
    pipe_destout: wgpu::RenderPipeline,
    pipe_final: wgpu::RenderPipeline,
    pipe_adjust: wgpu::RenderPipeline,
    pipe_add: wgpu::RenderPipeline,
    pipe_yuv: wgpu::RenderPipeline,
    /// 1×1 white (linear) texture bound where a draw has no mask / paint.
    white: Tex,
    /// GPU errors (validation, out of memory, device lost) not yet returned to a caller.
    errors: Arc<Mutex<Vec<String>>>,
    /// Set by the device-lost callback: a lost device is not handed to new renderers.
    lost: Arc<AtomicBool>,
}

/// The process-wide wgpu instance and devices (see "One GPU device per process" in the crate
/// docs): every [`Gpu`] is created from one instance, at most once per [`AdapterPreference`],
/// under this lock, and is never destroyed (a lost device is replaced, not dropped).
struct Process {
    instance: Option<wgpu::Instance>,
    gpus: Vec<(AdapterPreference, Arc<Gpu>)>,
}

static PROCESS: Mutex<Process> = Mutex::new(Process {
    instance: None,
    gpus: Vec::new(),
});

fn adapter_rank(t: wgpu::DeviceType) -> u8 {
    match t {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        wgpu::DeviceType::VirtualGpu => 2,
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::Cpu => 4,
    }
}

impl Gpu {
    /// The process's device for `pref`: the existing one, or a new one made from the
    /// process-wide instance. Never creates a second wgpu instance and never destroys a
    /// device, so a renderer's GPU calls cannot race another renderer's instance or device
    /// creation and teardown inside the Vulkan loader (see the crate docs).
    pub fn new(pref: AdapterPreference) -> anyhow::Result<Arc<Gpu>> {
        let mut process = PROCESS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, gpu)) = process
            .gpus
            .iter()
            .find(|(p, g)| *p == pref && !g.lost.load(Ordering::Acquire))
        {
            return Ok(gpu.clone());
        }
        let instance = match &process.instance {
            Some(i) => i.clone(),
            None => {
                let i = pollster::block_on(Self::create_instance());
                process.instance = Some(i.clone());
                i
            }
        };
        let gpu = pollster::block_on(Self::create(&instance, pref))?;
        // A lost device of this preference stays alive (in `gpus`) for the renderers still
        // holding it; only new renderers get the replacement.
        process.gpus.push((pref, gpu.clone()));
        Ok(gpu)
    }

    /// The device the process already renders with (the first one made that is not lost),
    /// else a new hardware-first one: what window surfaces present from.
    pub fn current() -> anyhow::Result<Arc<Gpu>> {
        let found = {
            let process = PROCESS.lock().unwrap_or_else(|e| e.into_inner());
            process
                .gpus
                .iter()
                .find(|(_, g)| !g.lost.load(Ordering::Acquire))
                .map(|(_, g)| g.clone())
        };
        match found {
            Some(g) => Ok(g),
            None => Gpu::new(AdapterPreference::HardwareFirst),
        }
    }

    /// One instance with the primary native backends (WGPU_BACKEND etc. are honoured), or
    /// with every backend (GL) when the primary ones have no adapter at all.
    async fn create_instance() -> wgpu::Instance {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        if std::env::var_os("WGPU_BACKEND").is_none() {
            desc.backends = wgpu::Backends::PRIMARY;
        }
        let instance = wgpu::Instance::new(desc);
        if !instance
            .enumerate_adapters(wgpu::Backends::all())
            .await
            .is_empty()
        {
            return instance;
        }
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        desc.backends = wgpu::Backends::all();
        wgpu::Instance::new(desc)
    }

    async fn create(
        instance: &wgpu::Instance,
        pref: AdapterPreference,
    ) -> anyhow::Result<Arc<Gpu>> {
        let adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        for a in &adapters {
            let i = a.get_info();
            log::debug!(
                "render: found adapter {} ({:?}, {:?})",
                i.name,
                i.backend,
                i.device_type
            );
        }
        let pick = match pref {
            AdapterPreference::HardwareFirst => adapters.into_iter().min_by_key(|a| {
                (
                    adapter_rank(a.get_info().device_type),
                    a.get_info().backend != wgpu::Backend::Vulkan
                        && a.get_info().backend != wgpu::Backend::Dx12
                        && a.get_info().backend != wgpu::Backend::Metal,
                )
            }),
            AdapterPreference::Software => adapters
                .into_iter()
                .find(|a| a.get_info().device_type == wgpu::DeviceType::Cpu),
        };
        let adapter = match pick {
            Some(a) => a,
            None => instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: pref == AdapterPreference::Software,
                    compatible_surface: None,
                    apply_limit_buckets: false,
                })
                .await
                .map_err(|e| anyhow!("no usable GPU adapter (not even a software one): {e}"))?,
        };
        let info = adapter.get_info();
        let summary = AdapterSummary {
            name: info.name.clone(),
            backend: format!("{:?}", info.backend),
            device_type: format!("{:?}", info.device_type),
            driver: [info.driver.as_str(), info.driver_info.as_str()]
                .iter()
                .filter(|s| !s.is_empty())
                .copied()
                .collect::<Vec<_>>()
                .join(" "),
            is_software: info.device_type == wgpu::DeviceType::Cpu,
        };
        log::info!("render: using adapter {summary}");
        if summary.is_software {
            log::warn!(
                "render: no hardware GPU available (is the user in the `render` group?); \
                 using a software rasterizer"
            );
        }

        let alimits = adapter.limits();
        let mut limits = if adapter.get_downlevel_capabilities().is_webgpu_compliant() {
            wgpu::Limits::default()
        } else {
            wgpu::Limits::downlevel_defaults()
        };
        limits.max_texture_dimension_2d = alimits.max_texture_dimension_2d.min(16384);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("render"),
                required_features: wgpu::Features::empty(),
                required_limits: limits.clone(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .context("request GPU device")?;
        anyhow::ensure!(
            device.limits().min_uniform_buffer_offset_alignment as u64 <= UNIFORM_STRIDE,
            "uniform offset alignment too large"
        );
        // GPU errors are collected and returned by the next submit instead of aborting the
        // process (a playout server must keep running).
        let errors: Arc<Mutex<Vec<String>>> = Arc::default();
        let lost: Arc<AtomicBool> = Arc::default();
        let errs = errors.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            log::error!("render: GPU error: {e}");
            if let Ok(mut v) = errs.lock() {
                v.push(e.to_string());
            }
        }));
        let errs = errors.clone();
        let is_lost = lost.clone();
        device.set_device_lost_callback(move |reason, msg| {
            is_lost.store(true, Ordering::Release);
            log::error!("render: GPU device lost ({reason:?}): {msg}");
            if let Ok(mut v) = errs.lock() {
                v.push(format!("device lost ({reason:?}): {msg}"));
            }
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("render shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });
        let bgl_uniform = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uniforms"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<Uniforms>() as u64),
                },
                count: None,
            }],
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bgl_tex = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("textures"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("render"),
            bind_group_layouts: &[Some(&bgl_uniform), Some(&bgl_tex)],
            immediate_size: 0,
        });
        let make = |label: &str, fs: &str, format, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let destout = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipe_over = make(
            "over",
            "fs_main",
            LINEAR_FORMAT,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let pipe_replace = make("replace", "fs_main", LINEAR_FORMAT, None);
        let pipe_destout = make(
            "destout",
            "fs_main",
            LINEAR_FORMAT,
            Some(wgpu::BlendState {
                color: destout,
                alpha: destout,
            }),
        );
        let pipe_final = make("final", "fs_final", OUTPUT_FORMAT, None);
        let pipe_adjust = make("adjust", "fs_adjust", OUTPUT_FORMAT, None);
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let pipe_add = make(
            "add",
            "fs_main",
            LINEAR_FORMAT,
            Some(wgpu::BlendState {
                color: add,
                alpha: add,
            }),
        );
        let pipe_yuv = make("yuv", "fs_yuv", LINEAR_FORMAT, None);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let max_texture = device.limits().max_texture_dimension_2d;
        let white = create_texture(
            &device,
            "white",
            1,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        queue.write_texture(
            white.texture.as_image_copy(),
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        Ok(Arc::new(Gpu {
            instance: instance.clone(),
            adapter,
            device,
            queue,
            summary,
            max_texture,
            bgl_uniform,
            bgl_tex,
            sampler,
            pipe_over,
            pipe_replace,
            pipe_destout,
            pipe_final,
            pipe_adjust,
            pipe_add,
            pipe_yuv,
            white,
            errors,
            lost,
        }))
    }

    /// The device was lost (driver reset, GPU hang, `TDR`): nothing drawn on it works any
    /// more, and [`Gpu::new`] hands out a replacement.
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::Acquire)
    }

    /// The identity of this device (stable: devices are never dropped).
    pub(crate) fn id(&self) -> usize {
        self as *const Gpu as usize
    }

    /// Queues `msg` as an uncaptured GPU error (tests of the error paths).
    pub(crate) fn push_error(&self, msg: &str) {
        if let Ok(mut v) = self.errors.lock() {
            v.push(msg.to_string());
        }
    }

    /// Destroys the device as a driver reset would lose it (tests of the recovery).
    pub(crate) fn lose(&self) {
        self.device.destroy();
        // Not every backend calls the lost callback for an explicit destroy.
        self.lost.store(true, Ordering::Release);
    }

    /// GPU errors raised since the last call, as one error.
    pub fn take_errors(&self) -> anyhow::Result<()> {
        let errs = match self.errors.lock() {
            Ok(mut v) => std::mem::take(&mut *v),
            Err(_) => Vec::new(),
        };
        if errs.is_empty() {
            Ok(())
        } else {
            anyhow::bail!("GPU error: {}", errs.join("; "))
        }
    }

    /// A render-target / sampled intermediate in the linear premultiplied format.
    pub fn linear_target(&self, label: &str, w: u32, h: u32) -> Tex {
        create_texture(
            &self.device,
            label,
            w.clamp(1, self.max_texture),
            h.clamp(1, self.max_texture),
            LINEAR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        )
    }

    /// Uploads a CPU image (tightly packed rows) into a new sampled texture.
    pub fn upload(
        &self,
        label: &str,
        w: u32,
        h: u32,
        format: wgpu::TextureFormat,
        data: &[u8],
    ) -> Tex {
        let t = create_texture(
            &self.device,
            label,
            w,
            h,
            format,
            1,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        );
        let bpp = if format == wgpu::TextureFormat::R8Unorm {
            1
        } else {
            4
        };
        self.queue.write_texture(
            t.texture.as_image_copy(),
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * bpp),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        t
    }

    /// Encodes passes in order into `enc`.
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, passes: &[Pass]) {
        for pass in passes {
            let n = pass.draws.len().max(1) as u64;
            let mut bytes = vec![0u8; (n * UNIFORM_STRIDE) as usize];
            for (i, d) in pass.draws.iter().enumerate() {
                let mut u = d.u;
                u.target_info[0] = pass.size.0 as f32;
                u.target_info[1] = pass.size.1 as f32;
                let off = i * UNIFORM_STRIDE as usize;
                bytes[off..off + std::mem::size_of::<Uniforms>()]
                    .copy_from_slice(bytemuck::bytes_of(&u));
            }
            let ubuf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("uniforms"),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&ubuf, 0, &bytes);
            let ubg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("uniforms"),
                layout: &self.bgl_uniform,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &ubuf,
                        offset: 0,
                        size: wgpu::BufferSize::new(std::mem::size_of::<Uniforms>() as u64),
                    }),
                }],
            });
            let tex_groups: Vec<wgpu::BindGroup> = pass
                .draws
                .iter()
                .map(|d| {
                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("textures"),
                        layout: &self.bgl_tex,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(
                                    d.mask.as_ref().unwrap_or(&self.white.view),
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(
                                    d.paint.as_ref().unwrap_or(&self.white.view),
                                ),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&self.sampler),
                            },
                        ],
                    })
                })
                .collect();
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(pass.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &pass.target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: match pass.clear {
                            Some(c) => wgpu::LoadOp::Clear(c),
                            None => wgpu::LoadOp::Load,
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            for (i, d) in pass.draws.iter().enumerate() {
                let pipe = match d.pipe {
                    Pipe::Over => &self.pipe_over,
                    Pipe::Replace => &self.pipe_replace,
                    Pipe::DestOut => &self.pipe_destout,
                    Pipe::Final => &self.pipe_final,
                    Pipe::Adjust => &self.pipe_adjust,
                    Pipe::Add => &self.pipe_add,
                    Pipe::Yuv => &self.pipe_yuv,
                };
                rp.set_pipeline(pipe);
                rp.set_bind_group(0, &ubg, &[(i as u64 * UNIFORM_STRIDE) as u32]);
                rp.set_bind_group(1, &tex_groups[i], &[]);
                rp.draw(0..6, 0..1);
            }
        }
    }
}

pub(crate) fn create_texture(
    device: &wgpu::Device,
    label: &str,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    mips: u32,
    usage: wgpu::TextureUsages,
) -> Tex {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: mips.max(1),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Tex {
        texture,
        view,
        width: w.max(1),
        height: h.max(1),
    }
}
