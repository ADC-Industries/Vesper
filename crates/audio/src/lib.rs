//! # `audio` — the program audio bus
//!
//! Everything audible on the server goes through one bus at [`SAMPLE_RATE`] (48 kHz, 32-bit
//! float, [`DEFAULT_CHANNELS`] channels by default):
//!
//! ```text
//!  sources (video soundtracks, the audio layer, test tone)  — each a [`Source`]
//!     │  per-source gain (dB, smoothed), fades (equal power), channel routing, mute
//!     ▼
//!  groups (layers: audio / media / slide)  gain + mute
//!     ▼
//!  master gain + mute → soft clip → peak meters
//!     ├──► taps (program audio for network outputs, with timecodes; bounded queues, never wait)
//!     └──► channel map → sound card (idn-audio) or the null output
//! ```
//!
//! - [`dsp`]: dB ↔ gain, fade curves, routing, soft clip — pure functions.
//! - [`mixer::Mixer`]: the bus above as a plain struct (no threads), so it is testable
//!   sample by sample.
//! - [`engine::AudioEngine`]: **one thread drives the output** and owns the mixer. It mixes
//!   one period (10 ms) at a time, hands it to the output and sleeps until the next period is
//!   due (the machine's clock paces it). On a sound card the period is pushed into an
//!   `idn-audio` lane, whose drift correction plays it at a steady latency on the card's own
//!   clock; the **null output** just keeps time, so the audio clock keeps running on a box
//!   without speakers (video sync and embedded network audio still work).
//!   Sources are fed by their decoder threads through lock-free rings; the engine thread never
//!   takes a lock another thread holds for long (it only `try_lock`s), so a busy control path
//!   cannot cause an underrun. Underruns (xruns) are counted.
//! - **Audio clock.** For every period the engine knows when its first sample will be *heard*:
//!   `now + latency`, on a sound card the lane's `latency()` (what it holds plus the device's
//!   output delay, from the device's callback timestamps), smoothed over the callbacks; the
//!   null output's latency is its pacing lead. Sources receive that instant ([`BlockCtx::heard_at`]),
//!   so a video can follow the sound it actually plays (see the server's `playback` module), and
//!   taps stamp program audio with it ([`timecode`]).
//! - [`device`]: the sound card (`idn-audio`: WASAPI on Windows, ALSA on Linux), the null
//!   output, device listing and probing, and the migration of device names stored as ALSA
//!   card indexes (`hw:1,0` → `hw:CARD=<card ID>,DEV=0`, see [`device::resolve`]).
//! - **Health.** `idn-audio` reports a sound card as running, failing (recent errors: shown in
//!   the status `detail`) or lost (unplugged, taken by another program: the engine falls
//!   back to the null output and tries the device again every 10 s, backing off to 5 min,
//!   on a thread of its own). A device that plays far from real time (HDMI without a monitor)
//!   is dropped the same way.
//!
//! On Linux ALSA is the system's `libasound` (linked dynamically). The server user needs to
//! be in the `audio` group to open hardware devices; without it the engine reports the error
//! and runs the null output.

#![warn(missing_docs)]

pub mod device;
pub mod dsp;
pub mod engine;
pub mod mixer;
pub mod tone;

pub use engine::{AudioEngine, EngineConfig, EngineStatus, OutputState, TapBlock, TapReceiver};
pub use mixer::{BlockCtx, Group, Mixer, Route, Source, SourceParams};

use std::sync::OnceLock;
use std::time::Instant;

/// The bus sample rate, Hz.
pub const SAMPLE_RATE: u32 = 48_000;

/// Default bus channel count (stereo).
pub const DEFAULT_CHANNELS: usize = 2;

/// Largest supported bus / device channel count.
pub const MAX_CHANNELS: usize = 16;

/// The process-wide reference instant for timecodes (the first call).
pub fn epoch() -> Instant {
    static E: OnceLock<Instant> = OnceLock::new();
    *E.get_or_init(Instant::now)
}

/// An instant as a timecode: 100 ns units since [`epoch`] (the network video unit). Video frames and
/// program audio use the same scale, so equal timecodes mean "presented together".
pub fn timecode(at: Instant) -> i64 {
    let e = epoch();
    if at >= e {
        (at - e).as_nanos().min(i64::MAX as u128) as i64 / 100
    } else {
        -(((e - at).as_nanos().min(i64::MAX as u128)) as i64 / 100)
    }
}

/// Seconds from `a` to `b` (negative when `b` is earlier).
pub fn secs_between(a: Instant, b: Instant) -> f64 {
    if b >= a {
        (b - a).as_secs_f64()
    } else {
        -(a - b).as_secs_f64()
    }
}

/// `at` moved by `secs` (either direction; `at` itself when that is not a representable
/// instant). No other limit: the engine moves [`epoch`] by the whole uptime.
pub fn offset(at: Instant, secs: f64) -> Instant {
    let Ok(d) = std::time::Duration::try_from_secs_f64(secs.abs()) else {
        return at;
    };
    if secs >= 0.0 {
        at.checked_add(d).unwrap_or(at)
    } else {
        at.checked_sub(d).unwrap_or(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn offset_spans_weeks_of_uptime() {
        // A03-1: the shift used to stop at 1e6 s (11.6 days of uptime), freezing the instant
        // every block is heard at.
        let e = Instant::now();
        let two_weeks = 14.0 * 86_400.0;
        assert_eq!(offset(e, two_weeks) - e, Duration::from_secs(14 * 86_400));
        let later = offset(e, two_weeks + 0.25);
        assert_eq!(later - offset(e, two_weeks), Duration::from_millis(250));
        assert_eq!(offset(later, -0.25), offset(e, two_weeks));
        // Not representable: unchanged.
        assert_eq!(offset(e, f64::NAN), e);
        assert_eq!(offset(e, f64::INFINITY), e);
        assert_eq!(offset(e, 1e300), e);
        assert_eq!(offset(e, -1e300), e);
    }
}
