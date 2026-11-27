//! Video playback runtime: one [`LiveVideo`] (a `media` player plus its media clock) per play
//! of a video, kept in the player registry ([`crate::Shared::videos`]) under its play ID.
//!
//! - **Opening.** The show actor opens a player for every play ID the show state references
//!   (off the async threads), waits up to [`FIRST_FRAME_WAIT`] for its first frame so the
//!   layer never shows an empty frame, then starts its clock and publishes the state. Players
//!   no longer referenced leave the registry; the render thread may still hold one for the
//!   outgoing side of a dissolve (it keeps playing until the dissolve ends).
//! - **Timeline.** The clock runs in timeline seconds (see the `media` crate): `repeat` maps
//!   to the player's passes (`once`, `loopCount`, `loop` / `loopFor` = forever, `loopFor`
//!   ending on the clock). `rate` is the clock's speed. Positions shown to operators count
//!   from the in point (`timeline mod length`), like the transport's scrub bar.
//! - **End.** At the end of a non-repeating play: `holdLastFrame` keeps the last frame
//!   (transport state `stopped`), `clear` empties the layer, `black` shows black,
//!   `fadeToClear` / `fadeToBlack` fade over the fade-out length (else the show's default
//!   media fade) before the end. The render thread applies this at frame accuracy; the actor
//!   follows within a quarter second (clearing the layer, marking the play ended).
//! - **Fades.** `fadeIn` ramps the layer in from the start; `fadeOut` ramps it out before the
//!   end of a finite play and is the length of the dissolve when the layer is cleared or
//!   stopped. `Fade { enabled, seconds: None }` uses `ShowSettings::default_media_fade`
//!   (default 0.5 s).
//! - **Loop on / off** (`mediaSetLoop`) while a video plays: the decoder's passes are fixed
//!   when it opens, so the play continues under a new play ID, opened at the position it had
//!   ([`LiveVideo::open`]'s `start`) with the new repeat; until it can show frames the new ID
//!   names the old player in the registry, so the picture and the sound never stop.
//! - **Soft loop** (`softLoop`, a cross-fade at the loop point) is not implemented yet: loops
//!   are hard cuts, which are seamless for footage made to loop (no gap, no black frame).
//! - **Sound.** A video with an audio stream plays it (rate 1 only: other rates play silent),
//!   and its time follows the sound actually heard (see [`crate::sound`]); the soundtrack
//!   fades with the video's fade-in / fade-out.

use crate::sound::{AvSync, SyncedClock};
use media::{AudioTap, Loops, MediaClock, Player, PlayerOptions, VideoFrame};
use model::{EndBehavior, Fade, Layer, MediaAssetId, Playback, Repeat};
use protocol::PlaybackState;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long opening a video waits for its first frame before going live anyway.
pub const FIRST_FRAME_WAIT: Duration = Duration::from_millis(400);

/// One play of a video on a layer.
pub struct LiveVideo {
    /// Play ID (registry key).
    pub play: u64,
    /// The asset.
    pub asset: MediaAssetId,
    /// Display name.
    pub name: String,
    /// `media` or `slide` (foreground).
    pub layer: Layer,
    /// The decoder.
    pub player: Player,
    clock: SyncedClock,
    /// Playback settings of the trigger.
    pub playback: Playback,
    /// The show's default fade length, seconds.
    pub default_fade: f64,
}

/// What a video layer shows at an instant.
pub struct VideoView {
    /// The frame (none before the first one is decoded, or after a `clear` end).
    pub frame: Option<Arc<VideoFrame>>,
    /// Layer opacity from fades.
    pub opacity: f32,
    /// The play ended: what to show instead of / under the last frame.
    pub ended: bool,
    /// Draw black (an `endBehavior` of `black` after the end, `fadeToBlack` while fading).
    pub black: bool,
}

impl LiveVideo {
    /// Opens the player for a play (blocking: file open, decoder set-up, first frame).
    /// `audio`: decode the soundtrack to this many channels (only at rate 1).
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        play: u64,
        asset: MediaAssetId,
        name: String,
        layer: Layer,
        path: &Path,
        playback: &Playback,
        default_fade: f64,
        audio: Option<usize>,
        start: f64,
    ) -> anyhow::Result<LiveVideo> {
        let start = start.max(0.0);
        let loops = match playback.repeat {
            Repeat::Loop | Repeat::LoopFor { .. } => Loops::Forever,
            Repeat::LoopCount { count } => Loops::Count(count.max(1)),
            Repeat::Once | Repeat::Unknown => Loops::Once,
        };
        let t0 = Instant::now();
        let player = Player::open(
            path,
            PlayerOptions {
                in_point: playback.in_point.max(0.0),
                out_point: playback.out_point,
                loops,
                audio: audio.filter(|_| (playback.rate - 1.0).abs() < 1e-9),
                start,
                ..Default::default()
            },
        )?;
        let _ = player.wait_frame_for(start, FIRST_FRAME_WAIT);
        let info = player.info();
        tracing::info!(
            play,
            file = %path.display(),
            codec = %info.codec,
            size = %format!("{}x{}", info.width, info.height),
            fps = info.fps,
            hw = info.hw,
            sound = player.audio().is_some(),
            ms = t0.elapsed().as_millis() as u64,
            "video opened"
        );
        let sound = player.audio().is_some();
        Ok(LiveVideo {
            play,
            asset,
            name,
            layer,
            player,
            clock: SyncedClock::new(playback.rate, sound),
            playback: playback.clone(),
            default_fade,
        })
    }

    /// Starts the clock at timeline 0 at `at` (called when the play goes live).
    pub fn start(&self, at: Instant) {
        self.clock.start(at, self.playback.rate);
    }

    /// Starts the clock at timeline time `t` at `at` (a play opened with `start` = `t`).
    pub fn start_at(&self, at: Instant, t: f64) {
        self.clock.start_at(at, t, self.playback.rate);
    }

    /// A copy of the commanded clock.
    pub fn clock(&self) -> MediaClock {
        self.clock.snapshot()
    }

    /// Timeline time at `at` (the sound's, while it plays in step).
    pub fn time_at(&self, at: Instant) -> f64 {
        self.clock.time_at(at)
    }

    /// The soundtrack's reading end and sync handle (once; `None` without sound).
    pub fn take_audio(&self) -> Option<(AudioTap, Arc<AvSync>)> {
        let sync = self.clock.sync()?;
        self.player.take_audio().map(|t| (t, sync))
    }

    /// Fade-in length of the sound (the video's fade-in), s.
    pub fn sound_fade_in(&self) -> f64 {
        self.fade_len(&self.playback.fade_in).unwrap_or(0.0)
    }

    /// Fade-out of the sound before a finite play's end: (end, length), s.
    pub fn sound_fade_out(&self) -> Option<(f64, f64)> {
        let end = self.end()?;
        let len = match self.playback.end_behavior {
            EndBehavior::FadeToClear | EndBehavior::FadeToBlack => self
                .fade_len(&self.playback.fade_out)
                .or(Some(self.default_fade)),
            _ => self.fade_len(&self.playback.fade_out),
        }?;
        Some((end, len))
    }

    /// Length of one pass (in → out), when known or estimable.
    pub fn region_len(&self) -> Option<f64> {
        self.player.region_len().or_else(|| {
            let end = self
                .playback
                .out_point
                .or_else(|| self.player.info().duration)?;
            Some((end - self.playback.in_point.max(0.0)).max(0.0))
        })
    }

    /// Timeline time at which the play ends (`None` = loops forever). Exact once the decoder
    /// reached the end, estimated before.
    pub fn end(&self) -> Option<f64> {
        match self.playback.repeat {
            Repeat::Loop => None,
            Repeat::LoopFor { seconds } => Some(seconds.max(0.0)),
            Repeat::LoopCount { count } => self
                .player
                .end_time()
                .or_else(|| self.region_len().map(|l| l * count.max(1) as f64)),
            Repeat::Once | Repeat::Unknown => self.player.end_time().or_else(|| self.region_len()),
        }
    }

    /// Whether the play loops (`loop`, `loopCount` > 1, `loopFor`).
    pub fn looping(&self) -> bool {
        match self.playback.repeat {
            Repeat::Loop | Repeat::LoopFor { .. } => true,
            Repeat::LoopCount { count } => count > 1,
            _ => false,
        }
    }

    /// Whether the play has ended at timeline time `t`.
    pub fn ended_at_time(&self, t: f64) -> bool {
        match self.playback.repeat {
            Repeat::Loop => false,
            Repeat::LoopFor { seconds } => t >= seconds,
            // The decoder knows the exact end once it got there; until then it has not ended.
            _ => self.player.finished_at(t),
        }
    }

    fn fade_len(&self, f: &Fade) -> Option<f64> {
        f.effective(self.default_fade).filter(|s| *s > 0.0)
    }

    /// What the layer shows at presentation time `at`.
    pub fn view(&self, at: Instant) -> VideoView {
        let clock = self.clock();
        let t = self.time_at(at);
        let rate = clock.rate().max(0.01);
        let ended = self.ended_at_time(t);
        let end = self.end();
        // Hold inside the played range (`loopFor` stops mid-pass).
        let ft = match end {
            Some(e) => t.min((e - self.player.frame_duration() * 0.5).max(0.0)),
            None => t,
        };
        let mut opacity = 1.0f64;
        if let Some(f) = self.fade_len(&self.playback.fade_in) {
            opacity = opacity.min(t / rate / f);
        }
        let end_fade = match self.playback.end_behavior {
            EndBehavior::FadeToClear | EndBehavior::FadeToBlack => self
                .fade_len(&self.playback.fade_out)
                .or(Some(self.default_fade)),
            _ => self.fade_len(&self.playback.fade_out),
        };
        if let (Some(f), Some(e)) = (end_fade, end) {
            opacity = opacity.min((e - t) / rate / f);
        }
        let opacity = opacity.clamp(0.0, 1.0) as f32;
        let (frame, black) = match (ended, self.playback.end_behavior) {
            (true, EndBehavior::Clear | EndBehavior::FadeToClear) => (None, false),
            (true, EndBehavior::Black | EndBehavior::FadeToBlack) => (None, true),
            (false, EndBehavior::FadeToBlack) => (self.player.frame_for(ft), true),
            _ => (self.player.frame_for(ft), false),
        };
        // Holding the last frame: no fade-out to nothing (unless asked for).
        let opacity = if ended && frame.is_some() {
            1.0
        } else {
            opacity
        };
        VideoView {
            frame,
            opacity,
            ended,
            black,
        }
    }

    /// Transport state at `at`.
    pub fn state_at(&self, at: Instant) -> PlaybackState {
        let c = self.clock();
        if self.ended_at_time(c.time_at(at)) {
            PlaybackState::Stopped
        } else if c.is_paused() {
            PlaybackState::Paused
        } else {
            PlaybackState::Playing
        }
    }

    /// Position at `at`: seconds from the start of the played range (the in point), within
    /// the current pass of a loop.
    pub fn position_at(&self, at: Instant) -> f64 {
        let t = self.time_at(at);
        let t = self.end().map_or(t, |e| t.min(e));
        match self.region_len() {
            Some(len) if len > 0.0 => {
                let k = (t / len).floor();
                let off = t - k * len;
                // The very end of the last pass shows as the end, not as 0.
                if off < 1e-6 && t > 0.0 && self.end().is_some_and(|e| t >= e - 1e-6) {
                    len
                } else {
                    off
                }
            }
            _ => t,
        }
    }

    /// Length of the played range, seconds (for the transport bar).
    pub fn duration(&self) -> Option<f64> {
        self.region_len()
    }

    /// Resumes; a play that ended starts again from the in point.
    pub fn play(&self, at: Instant) {
        let t = self.time_at(at);
        if self.ended_at_time(t) {
            self.seek_timeline(0.0, at);
        }
        self.clock.modify(at, |c| c.resume(at));
    }

    /// Pauses (holds the frame).
    pub fn pause(&self, at: Instant) {
        self.clock.modify(at, |c| c.pause(at));
    }

    /// Jumps to timeline time `t` (clamped to the play).
    pub fn seek_timeline(&self, t: f64, at: Instant) {
        let mut t = t.max(0.0);
        if let Some(e) = self.end() {
            t = t.min((e - self.player.frame_duration()).max(0.0));
        }
        self.player.seek(t);
        self.clock.modify(at, |c| c.seek(t, at));
    }

    /// Jumps to `seconds` from the in point, staying in the current pass of a loop.
    pub fn seek_position(&self, seconds: f64, at: Instant) {
        let t = self.time_at(at);
        let (base, off) = match self.region_len() {
            Some(len) if len > 0.0 => ((t / len).floor().max(0.0) * len, seconds.clamp(0.0, len)),
            _ => (0.0, seconds.max(0.0)),
        };
        self.seek_timeline(base + off, at);
    }

    /// Jumps by `seconds` (±).
    pub fn skip(&self, seconds: f64, at: Instant) {
        let t = self.time_at(at);
        self.seek_timeline(t + seconds, at);
    }

    /// Back to the in point, playing.
    pub fn restart(&self, at: Instant) {
        self.seek_timeline(0.0, at);
        self.clock.modify(at, |c| c.resume(at));
    }
}
