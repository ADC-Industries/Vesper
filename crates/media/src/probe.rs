//! Stream facts and thumbnails (IDN's `probe` and `thumbnail`).

use crate::frame::Matrix;
use anyhow::{Context as _, anyhow};
use idn_ffmpeg::media as idn;
use std::path::Path;
use std::time::Duration;

/// Facts about a video file's (first) video stream.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoInfo {
    /// Codec name (`h264`, `hevc`, `prores`).
    pub codec: String,
    /// Picture width, px (as stored: before `rotation`).
    pub width: u32,
    /// Picture height, px (as stored).
    pub height: u32,
    /// Frame rate the file states, frames per second (30 when it doesn't): IDN's average
    /// over the stream.
    pub fps: f64,
    /// The average frame rate (frames over the stream's length; 0: not known).
    pub avg_fps: f64,
    /// The lowest rate all its timestamps fit (FFmpeg's `r_frame_rate`; 0: not known).
    pub base_fps: f64,
    /// The average and base rates differ by more than 1 % (IDN's guess from the header and
    /// the first frames): most likely a variable frame rate.
    pub variable_rate: bool,
    /// How its fields are stored, as the file states (`progressive`, `tt`, `bb`, `tb`,
    /// `bt`); `None`: it doesn't say.
    pub field_order: Option<String>,
    /// Interlaced, as the file states.
    pub interlaced: bool,
    /// The video stream's bits a second, as the file states.
    pub bit_rate: Option<u64>,
    /// The whole file's bits a second (the header's, else its size over its duration).
    pub file_bit_rate: Option<u64>,
    /// Duration of the file, seconds.
    pub duration: Option<f64>,
    /// GPU decoding expected: decoding is not forced to software and this machine's GPU
    /// decoder takes the codec (it may still refuse a profile, and then the stream decodes
    /// in software without this saying so).
    pub hw: bool,
    /// The stream's pixel format (`yuv420p`, `yuv422p10le`).
    pub pixel_format: String,
    /// Significant bits per sample (8, 10, 12).
    pub bit_depth: u8,
    /// Has an alpha channel (ProRes 4444, VP9 with alpha).
    pub alpha: bool,
    /// Counter-clockwise turn that shows it upright (0, 90, 180, 270).
    pub rotation: u16,
    /// Colour matrix: a player's is what its pictures declare (once the first is decoded);
    /// [`probe`] gives the default for the size ([`Matrix::default_for`]).
    pub matrix: Matrix,
    /// Full-range video (a player's: from its pictures; [`probe`]: `yuvj*` formats).
    pub full_range: bool,
}

impl VideoInfo {
    pub(crate) fn of(m: &idn::MediaInfo, v: &idn::VideoInfo) -> VideoInfo {
        let (w, h) = (v.width as u32, v.height as u32);
        let fps = if (1.0..=480.0).contains(&v.frame_rate) {
            v.frame_rate
        } else {
            30.0
        };
        VideoInfo {
            codec: v.codec.clone(),
            width: w,
            height: h,
            fps,
            avg_fps: v.avg_frame_rate.max(0.0),
            base_fps: v.base_frame_rate.max(0.0),
            variable_rate: v.variable_frame_rate,
            field_order: v.field_order.clone(),
            interlaced: v.interlaced,
            bit_rate: v.bit_rate,
            file_bit_rate: m.bit_rate,
            duration: m.duration.map(|d| d.as_secs_f64()),
            hw: crate::decode_mode() == crate::DecodeMode::Auto && v.hardware_decodable,
            pixel_format: v.pixel_format.clone(),
            bit_depth: v.bit_depth,
            alpha: v.alpha,
            rotation: v.rotation,
            matrix: Matrix::default_for(w, h),
            full_range: v.pixel_format.starts_with("yuvj"),
        }
    }
}

/// Everything about a file, as IDN's probe reads it.
pub(crate) fn probe_file(path: &Path) -> anyhow::Result<idn::MediaInfo> {
    idn::probe(path, crate::ffmpeg()).with_context(|| format!("cannot open {}", path.display()))
}

/// Stream facts of a video file (no frames decoded).
pub fn probe(path: &Path) -> anyhow::Result<VideoInfo> {
    let m = probe_file(path)?;
    let v = m
        .video
        .as_ref()
        .ok_or_else(|| anyhow!("{} has no video stream", path.display()))?;
    Ok(VideoInfo::of(&m, v))
}

/// A still picture: RGB8, tightly packed rows, turned upright.
#[derive(Clone, Debug)]
pub struct Thumbnail {
    /// Width, px.
    pub width: u32,
    /// Height, px.
    pub height: u32,
    /// `width * height * 3` bytes.
    pub rgb: Vec<u8>,
    /// File position of the frame, seconds.
    pub at: f64,
    /// The file's duration, seconds.
    pub duration: Option<f64>,
}

/// Decodes the frame at `fraction` (0–1) of the file's duration (software decode), scales it
/// to at most `max_width` px wide (before turning it upright) and turns it by the file's
/// rotation.
pub fn thumbnail(path: &Path, fraction: f64, max_width: u32) -> anyhow::Result<Thumbnail> {
    let m = probe_file(path)?;
    let v = m
        .video
        .as_ref()
        .ok_or_else(|| anyhow!("{} has no video stream", path.display()))?;
    let duration = m.duration.map(|d| d.as_secs_f64());
    let at = duration.unwrap_or(0.0) * fraction.clamp(0.0, 1.0);
    let f = idn::thumbnail(
        path,
        Duration::from_secs_f64(at),
        max_width.clamp(16, 4096) as usize,
    )
    .with_context(|| format!("no frame decoded from {}", path.display()))?;
    if f.format != idn::PixelFormat::Rgba || f.planes.is_empty() {
        return Err(anyhow!("thumbnail of {}: not RGBA", path.display()));
    }
    let (w, h) = (f.width, f.height);
    let p = &f.planes[0];
    if w == 0 || h == 0 || p.stride < w * 4 || p.data.len() < p.stride * (h - 1) + w * 4 {
        return Err(anyhow!("thumbnail of {}: bad picture", path.display()));
    }
    let px = |x: usize, y: usize| {
        let i = y * p.stride + x * 4;
        [p.data[i], p.data[i + 1], p.data[i + 2]]
    };
    // Counter-clockwise turn: output (ox, oy) of the turned picture ← source (x, y).
    let rot = v.rotation % 360;
    let (tw, th) = if rot == 90 || rot == 270 {
        (h, w)
    } else {
        (w, h)
    };
    let mut rgb = Vec::with_capacity(tw * th * 3);
    for oy in 0..th {
        for ox in 0..tw {
            let (x, y) = match rot {
                90 => (w - 1 - oy, ox),
                180 => (w - 1 - ox, h - 1 - oy),
                270 => (oy, h - 1 - ox),
                _ => (ox, oy),
            };
            rgb.extend_from_slice(&px(x, y));
        }
    }
    Ok(Thumbnail {
        width: tw as u32,
        height: th as u32,
        rgb,
        at: f.file_time.as_secs_f64(),
        duration,
    })
}

/// Whether a picture (RGB8) is mostly black or one flat colour: dark on average, or with
/// almost no detail (a fade, a slate of one colour).
pub fn is_blank(rgb: &[u8]) -> bool {
    let px = rgb.len() / 3;
    if px == 0 {
        return true;
    }
    let lumas = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64);
    let (mut sum, mut sq) = (0.0, 0.0);
    for l in lumas {
        sum += l;
        sq += l * l;
    }
    let mean = sum / px as f64;
    let spread = (sq / px as f64 - mean * mean).max(0.0).sqrt();
    mean < 20.0 || (mean < 48.0 && spread < 6.0)
}

/// The poster frame of a video: the first of the frames at 10 %, 25 %, 50 %, 75 % and 3 % of
/// its length that isn't blank ([`is_blank`]), else the brightest of them; `max_width` as for
/// [`thumbnail`].
pub fn poster_frame(path: &Path, max_width: u32) -> anyhow::Result<Thumbnail> {
    let mut best: Option<(f64, Thumbnail)> = None;
    let mut last_err = None;
    for at in [0.1, 0.25, 0.5, 0.75, 0.03] {
        match thumbnail(path, at, max_width) {
            Ok(t) => {
                if !is_blank(&t.rgb) {
                    return Ok(t);
                }
                let mean = t.rgb.iter().map(|&b| b as f64).sum::<f64>() / t.rgb.len().max(1) as f64;
                if best.as_ref().is_none_or(|(m, _)| mean > *m) {
                    best = Some((mean, t));
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    match (best, last_err) {
        (Some((_, t)), _) => Ok(t),
        (None, Some(e)) => Err(e),
        (None, None) => Err(anyhow!("no frame decoded from {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn blank_pictures() {
        assert!(super::is_blank(&[0u8; 300]));
        assert!(super::is_blank(&[30u8; 300]));
        assert!(!super::is_blank(&[128u8; 300]));
        // Dark but detailed (a night scene with lights) is a picture.
        let mut v = vec![0u8; 300];
        for (i, b) in v.iter_mut().enumerate() {
            if i % 6 < 3 {
                *b = 120;
            }
        }
        assert!(!super::is_blank(&v));
    }
}
