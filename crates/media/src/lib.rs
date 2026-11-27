//! # `media` — video decoding and playback timing
//!
//! Decodes video and sound files through IDN's FFmpeg adapter (`idn_ffmpeg::media`, on the
//! **system FFmpeg**, linked dynamically; never bundled, never built here — see
//! `ARCHITECTURE.md`, *Media dependency policy*) and hands out timestamped frames in system
//! memory for the renderer to upload. Images keep using the renderer's own decoder.
//!
//! ## Who does what
//! IDN decodes: its `VideoReader` decodes pictures ahead on a thread of its own (the GPU where
//! it decodes the codec, software otherwise), on a timeline that starts at the in point and
//! runs on across the plays; its `AudioReader` gives a file's sound as 48 kHz float on the
//! same timeline rules; `probe`, `thumbnail` and `capabilities` give stream facts, stills and
//! what the machine decodes. Vesper keeps its own API on top: which picture a screen shows
//! (the nearest one), the frame hand-off to the renderer (`seq`, matrix, range, rotation), the
//! lock-free sound ring the audio engine reads, media clocks and A/V sync.
//!
//! ## Pieces
//! - [`Player`]: one per active video, over an IDN `VideoReader` that decodes **ahead**
//!   (default [`DEFAULT_QUEUE`] frames). [`Player::frame_for`] returns the frame whose
//!   timestamp is **nearest** to a media time, so 25 fps media on a 30 fps screen repeats one
//!   frame in six and 60 fps media on a 30 fps screen shows every other frame. A decoder that
//!   falls behind never stalls the caller: the last frame is shown again and the tick is
//!   counted as late ([`PlayerStats::late`]). Looping happens in the reader (in point → out
//!   point, again), decoded ahead across the loop point: no gap, no black frame. Seeking is
//!   exact (the reader decodes forward from the keyframe before).
//! - **Hardware decode first:** the GPU decoder IDN finds (VA-API on Linux), software where
//!   the GPU refuses the file. [`set_decode_mode`] (or `VESPER_VIDEO_DECODE=software`) forces
//!   software decode; otherwise IDN's own choice applies (`IDN_DECODER`). Hardware frames are
//!   copied to system memory. TODO(zero-copy): IDN's `gpu` feature keeps decoded pictures on
//!   the application's wgpu device; not used yet.
//! - [`VideoFrame`]: two planes, **NV12** (8 bit) or **P010** (deeper sources, MSB-aligned in
//!   16 bit), with the stream's colour matrix (BT.601 / 709 / 2020; unspecified = 709 for HD,
//!   601 for SD), range (limited / full) and rotation. Other pixel formats are converted to one
//!   of the two by IDN. The renderer converts YUV → RGB (and turns the picture) on the GPU.
//! - [`audio`]: soundtracks and audio files, read ahead through IDN's `AudioReader` at the bus
//!   format (48 kHz float) into a lock-free ring the audio engine reads ([`AudioTap`]); a
//!   player decodes its video's sound when asked ([`PlayerOptions::audio`]).
//! - [`MediaClock`]: start time, rate, pause offset → media time at any instant.
//! - [`probe`] and [`thumbnail`]: stream facts and a still for media bins; [`poster_frame`]
//!   picks a still that isn't black.
//! - [`convert`]: a file made into the house MP4 (IDN's `media::convert`: H.264 or HEVC 8-bit
//!   4:2:0, a constant frame rate, AAC 48 kHz), for media preparation.
//! - [`loudness`]: a file's EBU R128 integrated loudness and peak (media preparation's gain).
//! - **One FFmpeg probe per process, before rendering.** [`warm_up`] probes IDN's FFmpeg once
//!   (everything here reuses that `Ffmpeg`; its clones share its devices). On Linux the probe
//!   makes a wgpu Vulkan instance and an FFmpeg Vulkan device of its own, so it must not run
//!   while the renderer creates or uses its device (the Vulkan loader race in the `render`
//!   crate docs): the server probes before its renderer exists and runs with
//!   `IDN_VULKAN_ENCODE=off` (`vesper_server::vulkan_encode_off`), which leaves Vulkan out of
//!   the probe and keeps Vulkan Video encoders from opening a device later. Never call
//!   `Ffmpeg::probe` anywhere else, tests included.
//!
//! ## Timeline
//! A player's frames carry a **timeline time** `t` (seconds of media time from the start of
//! playback, continuous across loops: the second pass starts at the length of the first) as
//! well as their position in the file (`pts`). Callers keep a [`MediaClock`] in timeline time.

#![warn(missing_docs)]

pub mod audio;
mod clock;
pub mod convert;
mod frame;
pub mod loudness;
mod player;
mod probe;

pub use audio::{
    AUDIO_RATE, AudioDecoder, AudioInfo, AudioOptions, AudioTap, open_audio, probe_audio,
};
pub use clock::MediaClock;
pub use frame::{Matrix, PlaneFormat, VideoFrame};
pub use loudness::{Loudness, gain_to_target, measure_loudness};
pub use player::{DEFAULT_QUEUE, Loops, Player, PlayerOptions, PlayerStats};
pub use probe::{Thumbnail, VideoInfo, is_blank, poster_frame, probe, thumbnail};

use idn_ffmpeg::{DecoderChoice, Ffmpeg};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// Hardware or software decoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecodeMode {
    /// The GPU when it decodes the stream, else software (default; IDN's choice, which
    /// `IDN_DECODER` can change).
    #[default]
    Auto,
    /// Software decode only.
    Software,
}

impl DecodeMode {
    /// IDN's decoder choice for this mode (`None`: IDN's default, the GPU where it decodes
    /// the codec).
    pub(crate) fn choice(self) -> Option<DecoderChoice> {
        match self {
            DecodeMode::Auto => None,
            DecodeMode::Software => Some(DecoderChoice::Software),
        }
    }
}

static MODE: AtomicU8 = AtomicU8::new(0);

/// Sets the process-wide decode mode (for new players). The environment variable
/// `VESPER_VIDEO_DECODE=software` forces software decode regardless.
pub fn set_decode_mode(mode: DecodeMode) {
    MODE.store(mode as u8, Ordering::Relaxed);
}

/// The effective process-wide decode mode.
pub fn decode_mode() -> DecodeMode {
    if env_var("VESPER_VIDEO_DECODE", "STAGE_VIDEO_DECODE")
        .map(|v| v.eq_ignore_ascii_case("software") || v.eq_ignore_ascii_case("sw"))
        .unwrap_or(false)
    {
        return DecodeMode::Software;
    }
    match MODE.load(Ordering::Relaxed) {
        1 => DecodeMode::Software,
        _ => DecodeMode::Auto,
    }
}

/// Reads the environment variable `name`, or, when it is unset, its deprecated pre-rename
/// spelling `legacy` (logging a deprecation warning once per variable).
/// TODO: drop the legacy `STAGE_*` names one release after the rename to Vesper.
pub(crate) fn env_var(name: &'static str, legacy: &'static str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        return Some(v);
    }
    let v = std::env::var(legacy).ok()?;
    static WARNED: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut warned = WARNED.lock().unwrap_or_else(|e| e.into_inner());
    if !warned.contains(&legacy) {
        warned.push(legacy);
        log::warn!("{legacy} is deprecated and will be removed in the next release; use {name}");
    }
    Some(v)
}

/// The machine's FFmpeg as IDN found it, probed once per process (a fraction of a second).
/// `VESPER_VAAPI_DEVICE` names the VA-API render node (else IDN's default or
/// `IDN_VAAPI_DEVICE`).
pub(crate) fn ffmpeg() -> &'static Ffmpeg {
    static FF: OnceLock<Ffmpeg> = OnceLock::new();
    FF.get_or_init(
        || match env_var("VESPER_VAAPI_DEVICE", "STAGE_VAAPI_DEVICE") {
            Some(dev) => Ffmpeg::probe_device(Some(&dev)),
            None => Ffmpeg::probe(),
        },
    )
}

/// The machine's FFmpeg as probed once per process (its encoders: [`Ffmpeg::encoders`],
/// [`Ffmpeg::cpu_encoders`]), for the recording and streaming gateways.
pub fn machine_ffmpeg() -> &'static Ffmpeg {
    ffmpeg()
}

/// The machine's FFmpeg as the decoder of compressed network video (H.264 / HEVC / AV1 from
/// NDI|HX cameras or IDN sources: an IDN input's `cfg.codec`), following the process-wide
/// [`decode_mode`].
pub fn network_decoder() -> std::sync::Arc<Ffmpeg> {
    let ff = ffmpeg();
    std::sync::Arc::new(ff.with_decoder(decode_mode().choice().unwrap_or(ff.decoding())))
}

/// Probes FFmpeg and what this machine decodes now (once per process; later calls return at
/// once). Call it before any GPU renderer exists: the probe stands up FFmpeg's own Vulkan
/// instance and device, and creating an instance while another thread renders through the
/// Vulkan loader can crash that thread (see the `render` crate docs, "One GPU device per
/// process").
pub fn warm_up() {
    capabilities();
}

/// What this machine decodes, probed once.
fn capabilities() -> &'static idn_ffmpeg::media::Capabilities {
    static CAPS: OnceLock<idn_ffmpeg::media::Capabilities> = OnceLock::new();
    CAPS.get_or_init(|| {
        let c = idn_ffmpeg::media::capabilities(ffmpeg());
        match &c.hardware {
            Some(hw) => log::info!(
                "media: GPU decoding ({hw}) of {}",
                c.hardware_codecs.join(", ")
            ),
            None => log::warn!("media: no GPU decoder; videos decode in software"),
        }
        c
    })
}

/// FFmpeg's version and its libraries' (`FFmpeg 8.0.1 (libavcodec 62.11.100, …)`), for logs
/// and diagnostics.
pub fn ffmpeg_version() -> String {
    let c = capabilities();
    let libs: Vec<String> = c
        .libraries
        .iter()
        .map(|(n, v)| format!("lib{n} {v}"))
        .collect();
    format!("FFmpeg {} ({})", c.ffmpeg, libs.join(", "))
}

/// Whether a GPU decoder is available (VA-API on Linux).
pub fn hardware_available() -> bool {
    capabilities().hardware.is_some()
}

/// The GPU decoder in use (`VA-API`), if any.
pub fn hardware_decoder() -> Option<String> {
    capabilities().hardware.clone()
}

#[cfg(test)]
mod tests {
    #[test]
    fn env_var_prefers_new_name_and_falls_back_to_legacy() {
        const NEW: &str = "VESPER_TEST_ENV_FALLBACK";
        const OLD: &str = "STAGE_TEST_ENV_FALLBACK";
        // SAFETY: these variables are unique to this test; nothing else reads or writes them.
        unsafe {
            std::env::remove_var(NEW);
            std::env::remove_var(OLD);
        }
        assert_eq!(super::env_var(NEW, OLD), None);
        unsafe { std::env::set_var(OLD, "old") };
        assert_eq!(super::env_var(NEW, OLD).as_deref(), Some("old"));
        unsafe { std::env::set_var(NEW, "new") };
        assert_eq!(super::env_var(NEW, OLD).as_deref(), Some("new"));
    }
}
