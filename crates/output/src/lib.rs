//! # `output` — frame sinks
//!
//! Everything that takes a finished, composited frame of a screen and gets it out of the
//! process: an IDN Open Protocol source (compatible with NDI®), a PNG snapshot, an
//! `ffmpeg`/`ffplay` pipe for preview or recording, and an Open Media Transport (OMT) source.
//!
//! ## The frame contract
//! The render engine produces frames as **RGBA8, sRGB-encoded, premultiplied alpha, tightly
//! packed** (`width * 4` bytes per row, no padding), together with the screen's
//! [`model::AlphaMode`]. The server wraps such a buffer in a borrowed [`Frame`] and hands it to
//! every [`FrameSink`] bound to that screen. Only `model` types are shared between the render
//! and output crates; neither depends on the other.
//!
//! Each sink owns its format conversion. [`convert`] holds the shared, allocation-free pixel
//! conversion routines (premultiplied RGBA to straight/premultiplied BGRA, RGB, UYVY and UYVA
//! with BT.709 limited-range coefficients).
//!
//! ## Alpha
//! - [`AlphaMode::Opaque`] (and `Unknown`): the frame is flattened over black. For
//!   premultiplied input that is simply "drop the alpha byte", so opaque paths never divide.
//! - [`AlphaMode::Straight`] / [`AlphaMode::Premultiplied`]: alpha is sent. Each sink sends
//!   the alpha representation its transport defines (the IDN wire format, PNG and ffmpeg's
//!   `rgba` are all straight alpha), so a `Premultiplied` screen is un-premultiplied for them.
//!
//! ## Sinks
//! | Sink | Blocking behaviour |
//! |------|--------------------|
//! | [`PngSink`] | copies the frame and encodes on a worker thread; drops frames if the worker is busy |
//! | [`FfmpegSink`] | copies the frame; a worker thread converts and writes to the child's stdin; drops frames if the pipe is backed up |
//! | [`IdnSink`] | hands the frame to the IDN Open Protocol library's mailbox; it converts and encodes on its own thread |
//! | [`OmtSink`] | converts and codes (VMX) on the caller's thread: the server's per-output worker, which drops frames while busy; nothing is done without a receiver |
//!
//! Live sources for the video-input layer: IDN ([`LiveInput`], [`SourceWatcher`]) and OMT
//! ([`OmtInput`], [`omt::sources`]), both reporting [`LiveEvent`]s.
//!
//! For verification there is also a minimal receiver ([`IdnFinder`], [`IdnReceiver`]): find a
//! source, receive RGBA video frames and audio ([`recv`]). It is not an input layer.
//!
//! `ffmpeg`/`ffplay` are spawned as processes; the IDN Open Protocol library and `idn-omt`
//! (with its VMX codec) are compiled in.

#![warn(missing_docs)]

pub mod convert;
pub mod ffmpeg;
pub mod idn;
pub mod idn_live;
pub mod omt;
pub mod png_sink;
pub mod recv;
mod worker;

use anyhow::{Result, ensure};
pub use model::{AlphaMode, FrameRate};

pub use convert::PixelFormat;
pub use ffmpeg::{FfmpegConfig, FfmpegSink};
pub use idn::{IdnAudioSender, IdnFinder, IdnReceiver, IdnSenderConfig, IdnSink, IdnSourceInfo};
pub use idn_live::{
    AacSound, ListedSource, LiveDecoder, LiveEvent, LiveInput, LiveInputConfig, LiveMatrix,
    LivePicture, LiveSound, SourceChange, SourceWatcher,
};
pub use omt::{OmtAudioSender, OmtInput, OmtSenderConfig, OmtSink};
pub use png_sink::{PngSink, encode_png, write_png};
pub use recv::{Received, ReceivedAudio, ReceivedFrame};

/// A borrowed video frame handed to a [`FrameSink`].
///
/// `pixels` is RGBA8, sRGB-encoded, **premultiplied** alpha, tightly packed:
/// `pixels.len() == width * height * 4`.
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA8 sRGB premultiplied pixels, row-major, `width * 4` bytes per row.
    pub pixels: &'a [u8],
    /// Alpha intent of the screen this frame belongs to.
    pub alpha: AlphaMode,
    /// The screen's frame rate (used by sinks that need a nominal rate, e.g. IDN and ffmpeg).
    pub frame_rate: FrameRate,
    /// Timecode in 100 ns units (the network video convention).
    pub timecode: i64,
}

impl<'a> Frame<'a> {
    /// Number of bytes a tightly packed RGBA8 frame of this size has.
    pub fn expected_len(width: u32, height: u32) -> usize {
        width as usize * height as usize * 4
    }

    /// Checks that the buffer length matches the dimensions and that the frame is not empty.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.width > 0 && self.height > 0,
            "empty frame ({}x{})",
            self.width,
            self.height
        );
        let expected = Self::expected_len(self.width, self.height);
        ensure!(
            self.pixels.len() == expected,
            "frame buffer is {} bytes but {}x{} RGBA needs {}",
            self.pixels.len(),
            self.width,
            self.height,
            expected
        );
        Ok(())
    }

    /// True when the alpha channel should be transmitted (`Straight` or `Premultiplied`).
    pub fn has_alpha(&self) -> bool {
        matches!(self.alpha, AlphaMode::Straight | AlphaMode::Premultiplied)
    }
}

/// Embedded program audio for a network output, sent from a thread of its own
/// ([`IdnAudioSender`]).
pub trait AudioSink: Send {
    /// Sends `interleaved` samples (`channels` per frame) at `sample_rate`, stamped with
    /// `timecode` (100 ns units, the same scale as the video's).
    fn send(
        &mut self,
        interleaved: &[f32],
        channels: usize,
        sample_rate: u32,
        timecode: i64,
    ) -> Result<()>;
    /// Samples (per channel) accepted so far.
    fn samples_sent(&self) -> u64;
}

/// A destination for rendered frames (a network source, a file, a preview window...).
///
/// `send` must not block for long: sinks that do slow work (encoding, writing to a pipe) copy
/// the frame and do it on their own thread, dropping frames rather than stalling the caller.
pub trait FrameSink: Send {
    /// Human-readable name for logs and the UI ("IDN: Audience", "PNG: /tmp/shot.png").
    fn name(&self) -> &str;
    /// Delivers one frame. An error means the sink is broken (process died, runtime failure);
    /// a frame dropped because the sink is busy is not an error.
    fn send(&mut self, frame: &Frame<'_>) -> Result<()>;
}

impl<S: FrameSink + ?Sized> FrameSink for Box<S> {
    fn name(&self) -> &str {
        (**self).name()
    }
    fn send(&mut self, frame: &Frame<'_>) -> Result<()> {
        (**self).send(frame)
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    /// A deterministic synthetic premultiplied RGBA frame: gradients plus an alpha ramp.
    pub fn pattern(width: u32, height: u32, seed: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity(Frame::expected_len(width, height));
        for y in 0..height {
            for x in 0..width {
                let a = ((x * 255) / width.max(1)) as u8;
                let r = ((x + seed) * 7 % 256) as u8;
                let g = ((y + seed) * 13 % 256) as u8;
                let b = ((x ^ y) % 256) as u8;
                let pm = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
                v.extend_from_slice(&[pm(r), pm(g), pm(b), a]);
            }
        }
        v
    }

    pub fn frame(width: u32, height: u32, pixels: &[u8], alpha: AlphaMode) -> Frame<'_> {
        Frame {
            width,
            height,
            pixels,
            alpha,
            frame_rate: FrameRate::FPS30,
            timecode: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_validation() {
        let px = vec![0u8; 4 * 4 * 2];
        let f = test_util::frame(4, 2, &px, AlphaMode::Opaque);
        f.validate().unwrap();
        assert!(!f.has_alpha());
        let bad = test_util::frame(4, 3, &px, AlphaMode::Straight);
        assert!(bad.validate().is_err());
        assert!(bad.has_alpha());
        let empty = test_util::frame(0, 0, &[], AlphaMode::Opaque);
        assert!(empty.validate().is_err());
    }

    #[test]
    fn sinks_are_object_safe_and_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Box<dyn FrameSink>>();
        assert_send::<PngSink>();
        assert_send::<FfmpegSink>();
        assert_send::<IdnSink>();
        assert_send::<OmtSink>();
    }
}
