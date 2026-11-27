//! File conversion for media preparation, through IDN's `media::convert` (Beta14): a file made
//! into an MP4 every player takes, H.264 (or HEVC) 8-bit 4:2:0 at a constant frame rate with
//! AAC 48 kHz stereo, in this process on the machine's FFmpeg libraries (no `ffmpeg` tool).
//! IDN decodes in software, deinterlaces what is interlaced, turns the picture upright, puts it
//! on a constant rate and encodes on the GPU's encoder where there is one for the codec, else on
//! the CPU, at a low priority; it writes `OUTPUT.part` and renames it once complete.
//!
//! Blocking for the whole conversion (minutes): call it on a thread of its own.

use idn_ffmpeg::media as idn;
use idn_open_protocol::codec::Codec;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// What a conversion makes.
#[derive(Clone, Debug, PartialEq)]
pub struct ConvertOptions {
    /// HEVC instead of H.264.
    pub hevc: bool,
    /// The constant frame rate written (numerator, denominator); `None`: the source's average
    /// as the nearest common rate (IDN's choice).
    pub frame_rate: Option<(u32, u32)>,
    /// Fitted inside this size, its shape kept (never made larger).
    pub max_size: Option<(u32, u32)>,
    /// Deinterlace every picture (a file interlaced without saying so); else those the file or
    /// the picture says are interlaced.
    pub deinterlace_all: bool,
    /// The video's bits a second, kbit/s; `None`: IDN's choice by its size and rate (1080p30
    /// H.264 about 7.5 Mbit/s).
    pub video_kbps: Option<u32>,
    /// The GPU's encoder where this machine has one for the codec; else (or `false`) the CPU's.
    pub gpu: bool,
    /// IDN lowers the converting thread's priority while it lasts (default on).
    pub low_priority: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        ConvertOptions {
            hevc: false,
            frame_rate: None,
            max_size: None,
            deinterlace_all: false,
            video_kbps: None,
            gpu: true,
            low_priority: true,
        }
    }
}

/// How far a conversion is (about four times a second).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConvertProgress {
    /// Converted so far, seconds of the output.
    pub done: f64,
    /// `done` over the whole, 0 to 1, where the whole is known.
    pub fraction: Option<f64>,
    /// How many times faster than real time so far.
    pub speed: f64,
}

/// What a finished conversion made.
#[derive(Clone, Debug, PartialEq)]
pub struct Converted {
    /// The encoder used (`VA-API`, `NVENC`, `OpenH264`, …); empty without video.
    pub encoder: String,
    /// The picture's size and rate (0 × 0 without video).
    pub width: u32,
    /// Height, px.
    pub height: u32,
    /// Frames a second (numerator, denominator).
    pub frame_rate: (u32, u32),
    /// The output's length, seconds.
    pub duration: f64,
    /// Pictures written, repeated (gaps in a variable rate), left out, deinterlaced.
    pub frames: u64,
    /// Pictures repeated.
    pub repeated: u64,
    /// Source pictures left out.
    pub dropped: u64,
    /// Pictures deinterlaced.
    pub deinterlaced: u64,
    /// Has sound.
    pub audio: bool,
    /// What it couldn't keep (an HDR picture made 8-bit, an alpha channel left out).
    pub notes: Vec<String>,
    /// How long it took.
    pub took: Duration,
}

/// Why a conversion didn't finish.
#[derive(Debug)]
pub enum ConvertError {
    /// `cancel` was set (nothing is left behind).
    Cancelled,
    /// It failed (nothing is left behind).
    Failed(String),
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConvertError::Cancelled => f.write_str("the conversion was stopped"),
            ConvertError::Failed(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for ConvertError {}

fn codec(hevc: bool) -> Codec {
    if hevc { Codec::Hevc } else { Codec::H264 }
}

/// The encoder a conversion to H.264 (`hevc`: HEVC) uses on this machine, for people (`VA-API`,
/// `OpenH264 (CPU)`); `None`: none here encodes it (`gpu: false`: the CPU's only).
pub fn encoder(hevc: bool, gpu: bool) -> Option<String> {
    let ff = crate::ffmpeg();
    let c = codec(hevc);
    if gpu && let Some((_, b)) = ff.encoders().iter().find(|e| e.0 == c) {
        return Some(b.label().to_string());
    }
    ff.cpu_encoders()
        .iter()
        .find(|e| e.0 == c)
        .map(|(_, b)| format!("{} (CPU)", b.label()))
}

/// This machine's encoders for H.264 (`hevc`: HEVC), for people: the GPU's (`VA-API`) and the
/// CPU's (`OpenH264 (CPU)`), each `None` where there is none.
pub fn encoders(hevc: bool) -> (Option<String>, Option<String>) {
    let ff = crate::ffmpeg();
    let c = codec(hevc);
    let gpu = ff
        .encoders()
        .iter()
        .find(|e| e.0 == c)
        .map(|(_, b)| b.label().to_string());
    let cpu = ff
        .cpu_encoders()
        .iter()
        .find(|e| e.0 == c)
        .map(|(_, b)| format!("{} (CPU)", b.label()));
    (gpu, cpu)
}

/// Converts `input` into the MP4 `output` (replaced once the new one is complete). `progress`
/// is called about four times a second; setting `cancel` stops it. The converting thread runs
/// at a low priority while it lasts.
pub fn convert(
    input: &Path,
    output: &Path,
    opts: &ConvertOptions,
    mut progress: impl FnMut(ConvertProgress),
    cancel: &AtomicBool,
) -> Result<Converted, ConvertError> {
    let cfg = idn::ConvertConfig {
        codec: codec(opts.hevc),
        frame_rate: opts.frame_rate,
        deinterlace: if opts.deinterlace_all {
            idn::Deinterlace::On
        } else {
            idn::Deinterlace::Auto
        },
        max_size: opts.max_size.map(|(w, h)| (w as usize, h as usize)),
        video_kbps: opts.video_kbps,
        gpu: opts.gpu,
        low_priority: opts.low_priority,
        ..idn::ConvertConfig::default()
    };
    let r = idn::convert(
        input,
        output,
        &cfg,
        crate::ffmpeg(),
        |p| {
            progress(ConvertProgress {
                done: p.done.as_secs_f64(),
                fraction: p.fraction,
                speed: p.speed,
            })
        },
        cancel,
    );
    match r {
        Ok(r) => Ok(Converted {
            encoder: r.encoder,
            width: r.width as u32,
            height: r.height as u32,
            frame_rate: r.frame_rate,
            duration: r.duration.as_secs_f64(),
            frames: r.frames,
            repeated: r.repeated,
            dropped: r.dropped,
            deinterlaced: r.deinterlaced,
            audio: r.audio,
            notes: r.notes,
            took: r.took,
        }),
        Err(_) if cancel.load(std::sync::atomic::Ordering::Relaxed) => Err(ConvertError::Cancelled),
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => Err(ConvertError::Cancelled),
        Err(e) => Err(ConvertError::Failed(format!(
            "converting {}: {e}",
            input
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ))),
    }
}
