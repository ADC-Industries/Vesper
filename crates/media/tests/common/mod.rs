//! Synthetic test videos, generated at test time with the system `ffmpeg` CLI.
#![allow(dead_code)]

use media::VideoFrame;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// A temporary directory removed on drop (only directories this helper created).
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "media-test-{}-{}-{}",
            tag,
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).expect("temp dir");
        TempDir(p)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let tmp = std::env::temp_dir();
        if self.0.starts_with(&tmp)
            && self
                .0
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("media-test-"))
        {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

/// Bits of the frame counter drawn as blocks across the top quarter of the picture.
pub const BITS: u32 = 12;

fn run_ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("running the system ffmpeg");
    assert!(
        out.status.success(),
        "ffmpeg {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Codec settings for [`counter_video`].
#[derive(Clone, Copy, Debug)]
pub enum Codec {
    H264,
    Hevc,
    /// HEVC Main 10 (P010 through VA-API).
    Hevc10,
    /// H.264 4:2:2 (no VA-API profile: software).
    H264_422,
}

#[allow(clippy::too_many_arguments)]
/// A video whose frame `n` shows `n` in binary (12 blocks across the top quarter, white = 1)
/// over mid grey, `w`×`h` at `fps` (`num/den`) for `frames` frames, keyframe every `gop`.
pub fn counter_video(
    dir: &Path,
    name: &str,
    w: u32,
    h: u32,
    fps: (u32, u32),
    frames: u32,
    codec: Codec,
    gop: u32,
) -> PathBuf {
    let path = dir.join(name);
    // Drawn small (fast `geq`), scaled up with nearest neighbour: hard block edges.
    let src = format!(
        "color=c=gray:s=192x108:r={}/{},format=yuv420p,\
         geq=lum='if(lt(Y,27),if(bitand(N,pow(2,{bits}-1-floor(X/16))),235,16),126)':cb=128:cr=128,\
         scale={w}:{h}:flags=neighbor",
        fps.0,
        fps.1,
        bits = BITS
    );
    let frames_s = frames.to_string();
    let gop_s = gop.to_string();
    let mut args: Vec<&str> = vec!["-f", "lavfi", "-i", &src, "-frames:v", &frames_s];
    match codec {
        Codec::H264 => args.extend([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-bf",
            "2",
        ]),
        Codec::Hevc => args.extend([
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
            "-x265-params",
            "log-level=error",
            "-tag:v",
            "hvc1",
        ]),
        Codec::Hevc10 => args.extend([
            "-c:v",
            "libx265",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p10le",
            "-x265-params",
            "log-level=error",
            "-tag:v",
            "hvc1",
        ]),
        Codec::H264_422 => args.extend([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv422p",
        ]),
    }
    args.extend(["-g", &gop_s, "-colorspace", "bt709", "-color_range", "tv"]);
    let p = path.to_string_lossy().into_owned();
    args.push(&p);
    run_ffmpeg(&args);
    path
}

/// Reads the frame counter drawn by [`counter_video`].
pub fn read_counter(f: &VideoFrame) -> u32 {
    let (w, h) = (f.width, f.height);
    let threshold = match f.format {
        media::PlaneFormat::Nv12 => 128,
        media::PlaneFormat::P010 => 512,
    };
    let mut n = 0;
    for bit in 0..BITS {
        let x = (bit * 16 + 8) * w / 192;
        let y = h * 12 / 108;
        n = (n << 1) | (f.luma(x, y) > threshold) as u32;
    }
    n
}

/// Luma in the grey area under the counter (126 when the picture is there; 16 = black).
pub fn grey_level(f: &VideoFrame) -> u32 {
    let v = f.luma(f.width / 2, f.height * 3 / 4);
    match f.format {
        media::PlaneFormat::Nv12 => v,
        media::PlaneFormat::P010 => v >> 2,
    }
}

/// Whether this process can use VA-API (render node accessible).
pub fn render_node_accessible() -> bool {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/dri/renderD128")
        .is_ok()
}

/// Encodes SMPTE colour bars losslessly with x264 as limited-range YCbCr with `matrix`
/// (`bt709` or `smpte170m`) and tags matrix and range. The lavfi `smptebars` source draws BT.601
/// YCbCr; for BT.709 it is converted to RGB with the BT.601 matrix and back with BT.709
/// (both at full precision, so the 75 % bars stay at R'G'B' 191 / 0).
pub fn bars_video(dir: &Path, name: &str, matrix: &str, size: &str) -> PathBuf {
    let path = dir.join(name);
    let src = format!("smptebars=size={size}:rate=25:duration=0.2");
    let vf = if matrix == "bt709" {
        "scale=in_color_matrix=bt601:in_range=tv:flags=accurate_rnd+full_chroma_int,format=rgb24,\
         scale=out_color_matrix=bt709:out_range=tv:flags=accurate_rnd+full_chroma_int,format=yuv420p"
            .to_string()
    } else {
        "format=yuv420p".to_string()
    };
    let p = path.to_string_lossy().into_owned();
    run_ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &src,
        "-vf",
        &vf,
        "-c:v",
        "libx264",
        "-qp",
        "0",
        "-colorspace",
        matrix,
        "-color_primaries",
        matrix,
        "-color_trc",
        matrix,
        "-color_range",
        "tv",
        &p,
    ]);
    path
}
