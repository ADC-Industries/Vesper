//! Showing finished frames in a window: a [`WindowPresenter`] owns one window surface made
//! from the process's shared instance and device (see "One GPU device per process" in the
//! crate docs: it never creates another), uploads each CPU frame (premultiplied RGBA8, the
//! same frame the network outputs send) and draws it letterboxed into the window.
//!
//! Pacing: the surface prefers `Mailbox` (presenting never waits; the newest frame is shown at
//! the next vertical blank), else `Fifo` (presenting waits for a free image, at most a frame or
//! two). Either way only the calling thread waits, never the renderer.

use crate::gpu::Gpu;
use anyhow::{Context, anyhow, bail};
use std::sync::Arc;

const SHADER: &str = r#"
@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;

struct V {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> V {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: V;
    o.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    o.uv = uv;
    return o;
}

@fragment
fn fs(v: V) -> @location(0) vec4<f32> {
    // Premultiplied colour over black is the colour itself.
    return vec4<f32>(textureSample(frame, smp, v.uv).rgb, 1.0);
}
"#;

/// What a window surface was set up with (for logs and the output's status).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentInfo {
    /// The adapter presenting ("AMD Radeon …").
    pub adapter: String,
    /// The surface's pixel format.
    pub format: String,
    /// `Mailbox` or `Fifo`.
    pub present_mode: String,
}

struct FrameTexture {
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    size: (u32, u32),
}

/// One window's surface and the pass that draws frames into it.
pub struct WindowPresenter {
    gpu: Arc<Gpu>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    frame_format: wgpu::TextureFormat,
    frame: Option<FrameTexture>,
    info: PresentInfo,
}

impl std::fmt::Debug for WindowPresenter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowPresenter")
            .field("info", &self.info)
            .finish()
    }
}

impl WindowPresenter {
    /// A surface on `window` (`size` = its inner size in pixels) from the shared device.
    /// Fails when the adapter cannot present to it (for example a GL-only fallback adapter,
    /// or an X server without the extensions the driver needs).
    pub fn new<W: Into<wgpu::SurfaceTarget<'static>>>(
        window: W,
        size: (u32, u32),
    ) -> anyhow::Result<WindowPresenter> {
        let gpu = Gpu::current()?;
        let surface = gpu
            .instance
            .create_surface(window)
            .context("cannot create a window surface")?;
        if !gpu.adapter.is_surface_supported(&surface) {
            bail!(
                "the GPU ({}) cannot present to this window",
                gpu.summary.name
            );
        }
        let caps = surface.get_capabilities(&gpu.adapter);
        // The frames are already sRGB-encoded bytes: a plain 8-bit format takes them as they
        // are; an sRGB one gets an sRGB texture, so decoding and encoding cancel out.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| anyhow!("the window surface offers no format"))?;
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let mut config = surface
            .get_default_config(&gpu.adapter, size.0.max(1), size.1.max(1))
            .ok_or_else(|| anyhow!("the window surface cannot be configured"))?;
        config.format = format;
        config.present_mode = present_mode;
        config.desired_maximum_frame_latency = 2;
        if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
        }
        let frame_format = if format.is_srgb() {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };

        let device = &gpu.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        surface.configure(device, &config);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("present shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present frame"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("present"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        if let Some(e) = pollster::block_on(scope.pop()) {
            bail!("cannot set up the window surface: {e}");
        }
        let info = PresentInfo {
            adapter: gpu.summary.name.clone(),
            format: format!("{format:?}"),
            present_mode: format!("{present_mode:?}"),
        };
        Ok(WindowPresenter {
            gpu,
            surface,
            config,
            pipeline,
            layout,
            sampler,
            frame_format,
            frame: None,
            info,
        })
    }

    /// What the surface was set up with.
    pub fn info(&self) -> &PresentInfo {
        &self.info
    }

    /// The GPU device it presents from was lost (driver reset): make a new presenter for the
    /// window, which takes the process's replacement device.
    pub fn is_lost(&self) -> bool {
        self.gpu.is_lost()
    }

    /// Shows one frame (premultiplied RGBA8, `width`×`height`, tightly packed), letterboxed in
    /// a window whose inner size is now `window` pixels. Returns false when nothing was shown
    /// (a minimized or hidden window, or the compositor not ready): that is not an error.
    pub fn present(
        &mut self,
        pixels: &[u8],
        width: u32,
        height: u32,
        window: (u32, u32),
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            width > 0 && height > 0 && pixels.len() as u64 >= width as u64 * height as u64 * 4,
            "bad frame"
        );
        if window.0 == 0 || window.1 == 0 {
            return Ok(false);
        }
        let device = self.gpu.device.clone();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        if (self.config.width, self.config.height) != window {
            self.config.width = window.0;
            self.config.height = window.1;
            self.surface.configure(&device, &self.config);
        }
        if self.frame.as_ref().map(|f| f.size) != Some((width, height)) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("present frame"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.frame_format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("present frame"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.frame = Some(FrameTexture {
                texture,
                bind,
                size: (width, height),
            });
        }
        let Some(frame) = &self.frame else {
            return Ok(false);
        };
        self.gpu.queue.write_texture(
            frame.texture.as_image_copy(),
            &pixels[..width as usize * height as usize * 4],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let mut reconfigure = false;
        let target = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                reconfigure = true;
                t
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                drop(pollster::block_on(scope.pop()));
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&device, &self.config);
                drop(pollster::block_on(scope.pop()));
                return Ok(false);
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                let e = pollster::block_on(scope.pop());
                bail!(
                    "the window surface failed: {}",
                    e.map(|e| e.to_string()).unwrap_or_default()
                );
            }
        };
        let view = target.texture.create_view(&Default::default());
        let (x, y, w, h) = letterbox((width, height), window);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("present"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_viewport(x as f32, y as f32, w as f32, h as f32, 0.0, 1.0);
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &frame.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        self.gpu.queue.submit([enc.finish()]);
        self.gpu.queue.present(target);
        if reconfigure {
            self.surface.configure(&device, &self.config);
        }
        if let Some(e) = pollster::block_on(scope.pop()) {
            bail!("presenting failed: {e}");
        }
        Ok(true)
    }
}

/// Where a frame goes in a window: centred, aspect ratio kept, `(x, y, w, h)`.
fn letterbox((fw, fh): (u32, u32), (dw, dh): (u32, u32)) -> (u32, u32, u32, u32) {
    let s = (dw as f64 / fw as f64).min(dh as f64 / fh as f64);
    let w = ((fw as f64 * s).round() as u32).clamp(1, dw);
    let h = ((fh as f64 * s).round() as u32).clamp(1, dh);
    ((dw - w) / 2, (dh - h) / 2, w, h)
}

#[cfg(test)]
mod tests {
    #[test]
    fn frames_are_letterboxed() {
        assert_eq!(
            super::letterbox((1920, 1080), (1280, 720)),
            (0, 0, 1280, 720)
        );
        assert_eq!(
            super::letterbox((1920, 1080), (1024, 768)),
            (0, 96, 1024, 576)
        );
        assert_eq!(
            super::letterbox((1440, 1080), (1920, 1080)),
            (240, 0, 1440, 1080)
        );
    }
}
