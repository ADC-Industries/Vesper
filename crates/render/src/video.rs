//! Video frames on the GPU: plane upload and YUV → linear RGB conversion.
//!
//! A [`VideoFrameRef`] borrows the two planes of a decoded frame (NV12, or P010 for deeper
//! sources) from whatever decoded it (the `media` crate in the server). Per video **stream**
//! the renderer keeps a Y texture, a CbCr texture and a converted linear `Rgba16Float` texture
//! at the picture's native size, turned upright by the frame's rotation (a phone's portrait
//! video). A new frame (`seq`) is uploaded once and converted by one full-frame draw
//! (`fs_yuv`), however many screens show it; the converted texture is then drawn like an
//! image (scaled to the layer on the GPU by the media display settings).
//!
//! ## Alpha
//! A frame with an alpha plane (a keyed network source) comes as [`VideoPlanes::Nv12Alpha`]:
//! Y and A packed as byte pairs (one `Rg8` texture), CbCr as for NV12. The conversion then
//! writes premultiplied linear RGB: straight alpha is applied after linearising; premultiplied
//! (in R'G'B') is divided out first, linearised and applied again; [`VideoAlpha::Opaque`]
//! ignores the plane.
//!
//! ## Conversion
//! - Planes are uploaded as plain bytes (`R8` / `Rg8` for NV12, `Rg8` / `Rgba8` holding 16-bit
//!   little-endian words for P010) and read with `textureLoad`, so no 16-bit-normalized texture
//!   feature is needed.
//! - Chroma is upsampled bilinearly with MPEG-2 / H.264 default siting (co-sited with the left
//!   luma sample horizontally, centred vertically).
//! - The YCbCr → R'G'B' matrix is built on the CPU from the stream's matrix (BT.601 / 709 /
//!   2020), range (limited / full) and bit depth, applied to raw code values, clamped, then
//!   decoded with the sRGB transfer function into linear light. The output encodes with the
//!   inverse function, so R'G'B' code values pass through unchanged (what every video player
//!   shows on an sRGB display). There is no tone mapping of HDR (PQ / HLG) and no gamut
//!   conversion of BT.2020 primaries yet.

use crate::geom::Affine;
use crate::gpu::{Draw, Gpu, Pass, Pipe, Tex, Uniforms, create_texture};
use std::collections::HashMap;

/// Plane layout of a [`VideoFrameRef`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoPlanes {
    /// 8-bit Y plane plus interleaved CbCr at half resolution.
    Nv12,
    /// 16-bit little-endian words, MSB-aligned (read at 10-bit scale; a 12-bit source's
    /// extra bits are kept as fractions); same plane layout.
    P010,
    /// 8-bit Y and alpha packed as pairs (`y`: two bytes a pixel, Y first) plus interleaved
    /// CbCr at half resolution, as NV12; how the alpha is read.
    Nv12Alpha(VideoAlpha),
}

/// How the alpha plane of a [`VideoPlanes::Nv12Alpha`] frame is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoAlpha {
    /// Colour independent of alpha (NDI's convention).
    Straight,
    /// Colour (R'G'B') already multiplied by alpha.
    Premultiplied,
    /// Ignored: drawn opaque.
    Opaque,
}

/// YCbCr → RGB matrix of a video.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VideoMatrix {
    /// BT.601 (SD).
    Bt601,
    /// BT.709 (HD).
    Bt709,
    /// BT.2020 non-constant luminance.
    Bt2020,
}

impl VideoMatrix {
    fn kr_kb(self) -> (f64, f64) {
        match self {
            VideoMatrix::Bt601 => (0.299, 0.114),
            VideoMatrix::Bt709 => (0.2126, 0.0722),
            VideoMatrix::Bt2020 => (0.2627, 0.0593),
        }
    }
}

/// A decoded video frame to draw, borrowed from its decoder.
#[derive(Clone, Copy, Debug)]
pub struct VideoFrameRef<'a> {
    /// The video stream (player) it belongs to: uploads are cached per stream.
    pub stream: u64,
    /// Frame number within the stream (a new number = new pixels to upload).
    pub seq: u64,
    /// Picture width, px.
    pub width: u32,
    /// Picture height, px.
    pub height: u32,
    /// Plane layout.
    pub planes: VideoPlanes,
    /// Y plane.
    pub y: &'a [u8],
    /// Bytes per Y row.
    pub y_stride: usize,
    /// Interleaved CbCr plane (`ceil(width / 2)` × `ceil(height / 2)` pairs).
    pub uv: &'a [u8],
    /// Bytes per CbCr row.
    pub uv_stride: usize,
    /// Colour matrix.
    pub matrix: VideoMatrix,
    /// Full range (otherwise limited / "TV" range).
    pub full_range: bool,
    /// Counter-clockwise turn (0, 90, 180, 270) that shows the picture upright; the converted
    /// texture is turned (`height` × `width` for 90 and 270).
    pub rotation: u16,
}

/// Quarter turns (0–3) of a rotation in degrees (counter-clockwise; others round to the
/// nearest quarter).
fn quarter_turns(rotation: u16) -> u32 {
    (((rotation as u32 % 360) + 45) / 90) % 4
}

/// The 3×4 matrix (rows R, G, B; columns Y, Cb, Cr, 1) from raw code values (at `bits` bits)
/// to R'G'B' in 0..1.
pub(crate) fn yuv_matrix(matrix: VideoMatrix, full_range: bool, bits: u32) -> [[f32; 4]; 3] {
    let (kr, kb) = matrix.kr_kb();
    let kg = 1.0 - kr - kb;
    let s = (1u64 << (bits - 8)) as f64;
    let max = ((1u64 << bits) - 1) as f64;
    // Y' = ys * Y + yo ; C' = cs * C + co
    let (ys, yo, cs, co) = if full_range {
        (
            1.0 / max,
            0.0,
            1.0 / max,
            -((1u64 << (bits - 1)) as f64) / max,
        )
    } else {
        (
            1.0 / (219.0 * s),
            -16.0 * s / (219.0 * s),
            1.0 / (224.0 * s),
            -128.0 * s / (224.0 * s),
        )
    };
    let r_cr = 2.0 * (1.0 - kr);
    let g_cb = -2.0 * kb * (1.0 - kb) / kg;
    let g_cr = -2.0 * kr * (1.0 - kr) / kg;
    let b_cb = 2.0 * (1.0 - kb);
    let row = |a_cb: f64, a_cr: f64| -> [f32; 4] {
        [
            ys as f32,
            (a_cb * cs) as f32,
            (a_cr * cs) as f32,
            (yo + (a_cb + a_cr) * co) as f32,
        ]
    };
    [row(0.0, r_cr), row(g_cb, g_cr), row(b_cb, 0.0)]
}

pub(crate) struct VideoTextures {
    y: Tex,
    uv: Tex,
    pub rgb: Tex,
    planes: VideoPlanes,
    seq: Option<u64>,
    last_tick: u64,
}

/// Uploaded and converted frames, per stream.
#[derive(Default)]
pub(crate) struct VideoCache {
    streams: HashMap<u64, VideoTextures>,
}

/// Streams not drawn for this many renderer ticks give their textures back.
const IDLE_TICKS: u64 = 240;

impl VideoCache {
    /// The converted texture of `f`, uploading and converting it first (passes go to `pre`)
    /// unless this frame is already there.
    pub fn frame(
        &mut self,
        gpu: &Gpu,
        f: &VideoFrameRef<'_>,
        tick: u64,
        pre: &mut Vec<Pass>,
    ) -> Tex {
        let (w, h) = (
            f.width.clamp(1, gpu.max_texture),
            f.height.clamp(1, gpu.max_texture),
        );
        let turns = quarter_turns(f.rotation);
        // The converted picture, upright.
        let (rw, rh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
        let fresh = match self.streams.get(&f.stream) {
            Some(v) => {
                v.y.width != w
                    || v.y.height != h
                    || v.rgb.width != rw
                    || v.rgb.height != rh
                    || v.planes != f.planes
            }
            None => true,
        };
        if fresh {
            let (yf, uvf) = match f.planes {
                VideoPlanes::Nv12 => (wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm),
                VideoPlanes::P010 => (
                    wgpu::TextureFormat::Rg8Unorm,
                    wgpu::TextureFormat::Rgba8Unorm,
                ),
                VideoPlanes::Nv12Alpha(_) => {
                    (wgpu::TextureFormat::Rg8Unorm, wgpu::TextureFormat::Rg8Unorm)
                }
            };
            let usage = wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST;
            let v = VideoTextures {
                y: create_texture(&gpu.device, "video y", w, h, yf, 1, usage),
                uv: create_texture(
                    &gpu.device,
                    "video cbcr",
                    w.div_ceil(2),
                    h.div_ceil(2),
                    uvf,
                    1,
                    usage,
                ),
                rgb: gpu.linear_target("video rgb", rw, rh),
                planes: f.planes,
                seq: None,
                last_tick: tick,
            };
            self.streams.insert(f.stream, v);
        }
        let v = self.streams.get_mut(&f.stream).expect("video textures");
        v.last_tick = tick;
        if v.seq != Some(f.seq) {
            v.seq = Some(f.seq);
            // Bytes per Y (and A) sample group, per CbCr sample.
            let (ybpp, cbpp): (usize, usize) = match f.planes {
                VideoPlanes::Nv12 => (1, 1),
                VideoPlanes::P010 => (2, 2),
                VideoPlanes::Nv12Alpha(_) => (2, 1),
            };
            let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
            write_plane(gpu, &v.y, f.y, f.y_stride, w as usize * ybpp, h);
            write_plane(gpu, &v.uv, f.uv, f.uv_stride, cw as usize * 2 * cbpp, ch);
            let (bits, mode) = match f.planes {
                VideoPlanes::Nv12 => (8, 0),
                VideoPlanes::P010 => (10, 1),
                VideoPlanes::Nv12Alpha(VideoAlpha::Straight) => (8, 2),
                VideoPlanes::Nv12Alpha(VideoAlpha::Premultiplied) => (8, 3),
                VideoPlanes::Nv12Alpha(VideoAlpha::Opaque) => (8, 4),
            };
            let m = yuv_matrix(f.matrix, f.full_range, bits);
            let full = [0.0, 0.0, rw as f32, rh as f32];
            let mut u = Uniforms::new(&Affine::IDENTITY, full);
            u.modes = [mode, turns, 0, 0];
            u.stop_c[0] = m[0];
            u.stop_c[1] = m[1];
            u.stop_c[2] = m[2];
            pre.push(Pass {
                label: "video yuv",
                target: v.rgb.view.clone(),
                size: (rw, rh),
                clear: None,
                draws: vec![
                    Draw::new(Pipe::Yuv, u)
                        .with_mask(&v.y.view)
                        .with_paint(&v.uv.view),
                ],
            });
        }
        v.rgb.clone()
    }

    /// Streams with textures and their GPU bytes.
    pub fn size(&self) -> (u64, u64) {
        let bytes = self
            .streams
            .values()
            .map(|v| v.y.bytes() + v.uv.bytes() + v.rgb.bytes())
            .sum();
        (self.streams.len() as u64, bytes)
    }

    /// Drops the textures of streams not drawn for a while.
    pub fn evict(&mut self, tick: u64) {
        self.streams
            .retain(|_, v| tick.saturating_sub(v.last_tick) <= IDLE_TICKS);
    }
}

fn write_plane(gpu: &Gpu, tex: &Tex, data: &[u8], stride: usize, row_bytes: usize, rows: u32) {
    let need = stride * (rows as usize).saturating_sub(1) + row_bytes;
    if data.len() < need || stride < row_bytes {
        log::warn!(
            "render: video plane too small ({} bytes, need {need}); frame not uploaded",
            data.len()
        );
        return;
    }
    gpu.queue.write_texture(
        tex.texture.as_image_copy(),
        &data[..need],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride as u32),
            rows_per_image: Some(rows),
        },
        wgpu::Extent3d {
            width: tex.width,
            height: rows,
            depth_or_array_layers: 1,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &[[f32; 4]; 3], y: f32, cb: f32, cr: f32) -> [f32; 3] {
        let v = [y, cb, cr, 1.0];
        m.map(|r| r.iter().zip(v).map(|(a, b)| a * b).sum())
    }

    #[test]
    fn limited_range_matrices() {
        for mat in [VideoMatrix::Bt601, VideoMatrix::Bt709, VideoMatrix::Bt2020] {
            let m = yuv_matrix(mat, false, 8);
            let black = apply(&m, 16.0, 128.0, 128.0);
            let white = apply(&m, 235.0, 128.0, 128.0);
            for c in 0..3 {
                assert!(
                    black[c].abs() < 1e-5 && (white[c] - 1.0).abs() < 1e-5,
                    "{mat:?}"
                );
            }
            let m10 = yuv_matrix(mat, false, 10);
            let w10 = apply(&m10, 940.0, 512.0, 512.0);
            assert!((w10[0] - 1.0).abs() < 1e-5);
        }
        // BT.709 75% yellow: Y 168, Cb 44, Cr 136 (8-bit limited) → (0.75, 0.75, 0).
        let m = yuv_matrix(VideoMatrix::Bt709, false, 8);
        let yel = apply(&m, 168.0, 44.0, 136.0);
        assert!(
            (yel[0] - 0.75).abs() < 0.01 && (yel[1] - 0.75).abs() < 0.01 && yel[2].abs() < 0.01
        );
        let f = yuv_matrix(VideoMatrix::Bt601, true, 8);
        let w = apply(&f, 255.0, 128.0, 128.0);
        assert!((w[1] - 1.0).abs() < 0.005);
    }
}
