//! The bus as a plain struct: sources → gain, fades, routing → groups → master → soft clip.

use crate::dsp::{self, Smoothed, Tap};
use std::time::Instant;

/// Timing of one block, handed to every source.
#[derive(Clone, Copy, Debug)]
pub struct BlockCtx {
    /// Bus sample index of the block's first frame (counts from the engine's start).
    pub sample: i64,
    /// When the block's first frame will be heard at the output.
    pub heard_at: Instant,
    /// Frames in the block.
    pub frames: usize,
}

/// Something that produces audio for the bus. Rendered on the engine thread once per block:
/// it must not block (read lock-free rings, `try_lock` at most).
pub trait Source: Send {
    /// Channels of what [`Source::render`] writes.
    fn channels(&self) -> usize;
    /// Writes `ctx.frames` frames (interleaved, [`Source::channels`]) into `out` (already
    /// zeroed). Returns `false` when the source has nothing more to play: the mixer removes it.
    fn render(&mut self, out: &mut [f32], ctx: &BlockCtx) -> bool;
}

/// A channel route of a source (model `ChannelRoute`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Route {
    /// Source channel, 0-based.
    pub source: u32,
    /// Bus channel, 0-based.
    pub dest: u32,
    /// Gain, dB.
    pub gain_db: f64,
    /// Muted.
    pub muted: bool,
}

/// A mixing group (a layer): every source belongs to one.
pub type Group = u32;

/// Level and routing of a source.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceParams {
    /// The group (layer) it belongs to.
    pub group: Group,
    /// Gain, dB.
    pub gain_db: f64,
    /// Muted (contributes nothing; the level and routes are kept).
    pub muted: bool,
    /// Channel routes (empty = identity).
    pub routes: Vec<Route>,
}

impl Default for SourceParams {
    fn default() -> Self {
        SourceParams {
            group: 0,
            gain_db: 0.0,
            muted: false,
            routes: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Envelope {
    /// Full level.
    Steady,
    /// Fading in: frames done, frames total.
    In(usize, usize),
    /// Fading out, then removed.
    Out(usize, usize),
}

impl Envelope {
    fn gain(&self, i: usize) -> f32 {
        match *self {
            Envelope::Steady => 1.0,
            Envelope::In(d, n) => dsp::fade_in((d + i) as f64 / n.max(1) as f64),
            Envelope::Out(d, n) => dsp::fade_out((d + i) as f64 / n.max(1) as f64),
        }
    }

    /// After a block of `frames`; `true` when a fade-out finished.
    fn advance(&mut self, frames: usize) -> bool {
        match *self {
            Envelope::In(d, n) => {
                *self = if d + frames >= n {
                    Envelope::Steady
                } else {
                    Envelope::In(d + frames, n)
                };
                false
            }
            Envelope::Out(d, n) => {
                if d + frames >= n {
                    true
                } else {
                    *self = Envelope::Out(d + frames, n);
                    false
                }
            }
            Envelope::Steady => false,
        }
    }
}

struct Slot {
    id: u64,
    src: Box<dyn Source>,
    params: SourceParams,
    taps: Vec<Tap>,
    gain: Smoothed,
    env: Envelope,
    buf: Vec<f32>,
}

#[derive(Clone, Copy)]
struct GroupLevel {
    group: Group,
    gain: Smoothed,
    db: f64,
    muted: bool,
}

/// Frames of the anti-click ramp used when a source starts or stops without a fade.
pub const DECLICK_FRAMES: usize = 240; // 5 ms

/// The bus (see the crate docs).
pub struct Mixer {
    channels: usize,
    slots: Vec<Slot>,
    groups: Vec<GroupLevel>,
    master: Smoothed,
    master_db: f64,
    muted: bool,
    /// Peak of each channel since the last [`Mixer::take_peaks`].
    peaks: Vec<f32>,
}

fn frames_of(secs: f64) -> usize {
    if secs.is_finite() && secs > 0.0 {
        (secs * crate::SAMPLE_RATE as f64).round() as usize
    } else {
        0
    }
}

impl Mixer {
    /// An empty bus of `channels` channels.
    pub fn new(channels: usize) -> Self {
        let channels = channels.clamp(1, crate::MAX_CHANNELS);
        Mixer {
            channels,
            slots: Vec::new(),
            groups: Vec::new(),
            master: Smoothed::new(1.0),
            master_db: 0.0,
            muted: false,
            peaks: vec![0.0; channels],
        }
    }

    /// Bus channels.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Sources playing (fading out included).
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// No source plays.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Whether source `id` is in the mix.
    pub fn contains(&self, id: u64) -> bool {
        self.slots.iter().any(|s| s.id == id)
    }

    fn taps(&self, src_channels: usize, params: &SourceParams) -> Vec<Tap> {
        let routes: Vec<(u32, u32, f64, bool)> = params
            .routes
            .iter()
            .map(|r| (r.source, r.dest, r.gain_db, r.muted))
            .collect();
        dsp::routing(&routes, src_channels, self.channels)
    }

    fn level(params: &SourceParams) -> f32 {
        if params.muted {
            0.0
        } else {
            dsp::db_to_gain(params.gain_db)
        }
    }

    /// Adds a source (replacing one with the same `id` at once), fading in over `fade_in`
    /// seconds (0 = a 5 ms anti-click ramp).
    pub fn add(&mut self, id: u64, src: Box<dyn Source>, params: SourceParams, fade_in: f64) {
        self.slots.retain(|s| s.id != id);
        let taps = self.taps(src.channels(), &params);
        let n = frames_of(fade_in).max(DECLICK_FRAMES);
        let ch = src.channels().max(1);
        self.slots.push(Slot {
            id,
            src,
            gain: Smoothed::new(Self::level(&params)),
            params,
            taps,
            env: Envelope::In(0, n),
            buf: vec![0.0; ch * 4096],
        });
    }

    /// Fades source `id` out over `fade` seconds (0 = 5 ms), then removes it.
    pub fn remove(&mut self, id: u64, fade: f64) {
        let n = frames_of(fade).max(DECLICK_FRAMES);
        if let Some(s) = self.slots.iter_mut().find(|s| s.id == id) {
            let start = match s.env {
                // Continue from the current level of a fade-in.
                Envelope::In(d, total) => {
                    let g = dsp::fade_in(d as f64 / total.max(1) as f64);
                    let p = (g as f64).acos() / std::f64::consts::FRAC_PI_2;
                    (p * n as f64) as usize
                }
                Envelope::Out(..) => return,
                Envelope::Steady => 0,
            };
            s.env = Envelope::Out(start.min(n), n);
        }
    }

    /// Brings source `id` back if it is still in the mix (cancels its fade-out: it fades in
    /// again from its current level over `fade_in` seconds; `src` is dropped) with `params`;
    /// otherwise adds `src` as `id` like [`Mixer::add`]. A source that keeps state across
    /// blocks (a live source's lane, locked on) goes on without starting over.
    pub fn resume(&mut self, id: u64, src: Box<dyn Source>, params: SourceParams, fade_in: f64) {
        let Some(s) = self.slots.iter_mut().find(|s| s.id == id) else {
            return self.add(id, src, params, fade_in);
        };
        if let Envelope::Out(d, total) = s.env {
            // Continue from the current level of the fade-out.
            let g = dsp::fade_out(d as f64 / total.max(1) as f64);
            let n = frames_of(fade_in).max(DECLICK_FRAMES);
            let p = (g as f64).clamp(0.0, 1.0).asin() / std::f64::consts::FRAC_PI_2;
            s.env = Envelope::In(((p * n as f64) as usize).min(n), n);
        }
        self.set_params(id, params);
    }

    /// Changes a source's level and routing (gain changes glide over one block).
    pub fn set_params(&mut self, id: u64, params: SourceParams) {
        let ch = self.channels;
        if let Some(s) = self.slots.iter_mut().find(|s| s.id == id) {
            let routes: Vec<(u32, u32, f64, bool)> = params
                .routes
                .iter()
                .map(|r| (r.source, r.dest, r.gain_db, r.muted))
                .collect();
            s.taps = dsp::routing(&routes, s.src.channels(), ch);
            s.gain.target = Self::level(&params);
            s.params = params;
        }
    }

    /// Sets a group's level.
    pub fn set_group(&mut self, group: Group, db: f64, muted: bool) {
        let g = if muted { 0.0 } else { dsp::db_to_gain(db) };
        match self.groups.iter_mut().find(|x| x.group == group) {
            Some(x) => {
                x.gain.target = g;
                x.db = db;
                x.muted = muted;
            }
            None => self.groups.push(GroupLevel {
                group,
                gain: Smoothed::new(g),
                db,
                muted,
            }),
        }
    }

    /// A group's level (dB, muted); unknown groups are at unity.
    pub fn group(&self, group: Group) -> (f64, bool) {
        self.groups
            .iter()
            .find(|g| g.group == group)
            .map(|g| (g.db, g.muted))
            .unwrap_or((0.0, false))
    }

    /// Sets the master level.
    pub fn set_master(&mut self, db: f64) {
        self.master_db = db;
        self.master.target = if self.muted { 0.0 } else { dsp::db_to_gain(db) };
    }

    /// Mutes the master.
    pub fn set_mute(&mut self, muted: bool) {
        self.muted = muted;
        self.set_master(self.master_db);
    }

    /// Master level (dB) and mute.
    pub fn master(&self) -> (f64, bool) {
        (self.master_db, self.muted)
    }

    /// Mixes one block of `ctx.frames` frames into `out` (interleaved, [`Mixer::channels`];
    /// overwritten): sources, groups, master, soft clip; updates the peaks.
    pub fn mix(&mut self, out: &mut [f32], ctx: &BlockCtx) {
        let n = ctx.frames;
        let ch = self.channels;
        let out = &mut out[..n * ch];
        out.fill(0.0);
        let mut i = 0;
        while i < self.slots.len() {
            let s = &mut self.slots[i];
            let sch = s.src.channels().max(1);
            if s.buf.len() < n * sch {
                s.buf.resize(n * sch, 0.0);
            }
            let buf = &mut s.buf[..n * sch];
            buf.fill(0.0);
            let alive = s.src.render(buf, ctx);
            let grp = self
                .groups
                .iter()
                .find(|g| g.group == s.params.group)
                .map(|g| g.gain)
                .unwrap_or(Smoothed::new(1.0));
            let (gain, env) = (s.gain, s.env);
            if gain.current != 0.0 || gain.target != 0.0 {
                dsp::mix_routed(buf, sch, out, ch, n, &s.taps, |f| {
                    gain.at(f, n) * env.gain(f) * grp.at(f, n)
                });
            }
            s.gain.advance();
            let faded_out = s.env.advance(n);
            if faded_out || !alive {
                self.slots.remove(i);
            } else {
                i += 1;
            }
        }
        for g in &mut self.groups {
            g.gain.advance();
        }
        let m = self.master;
        for f in 0..n {
            let g = m.at(f, n);
            for c in 0..ch {
                let v = dsp::soft_clip(out[f * ch + c] * g);
                out[f * ch + c] = v;
                let a = v.abs();
                if a > self.peaks[c] {
                    self.peaks[c] = a;
                }
            }
        }
        self.master.advance();
    }

    /// Peak amplitude per channel since the last call (then reset).
    pub fn take_peaks(&mut self) -> Vec<f32> {
        let p = self.peaks.clone();
        self.peaks.iter_mut().for_each(|x| *x = 0.0);
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A constant per channel, forever.
    struct Dc(Vec<f32>);
    impl Source for Dc {
        fn channels(&self) -> usize {
            self.0.len()
        }
        fn render(&mut self, out: &mut [f32], ctx: &BlockCtx) -> bool {
            for f in 0..ctx.frames {
                for (c, v) in self.0.iter().enumerate() {
                    out[f * self.0.len() + c] = *v;
                }
            }
            true
        }
    }

    fn ctx(frames: usize) -> BlockCtx {
        BlockCtx {
            sample: 0,
            heard_at: Instant::now(),
            frames,
        }
    }

    fn run(m: &mut Mixer, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * m.channels()];
        m.mix(&mut out, &ctx(frames));
        out
    }

    #[test]
    fn sums_sources_and_soft_clips() {
        let mut m = Mixer::new(2);
        m.add(
            1,
            Box::new(Dc(vec![0.25, 0.25])),
            SourceParams::default(),
            0.0,
        );
        m.add(
            2,
            Box::new(Dc(vec![0.25, -0.5])),
            SourceParams::default(),
            0.0,
        );
        run(&mut m, 480); // past the anti-click ramp
        let out = run(&mut m, 480);
        assert!((out[0] - 0.5).abs() < 1e-6);
        assert!((out[1] + 0.25).abs() < 1e-6);
        // Loud sum: clipped below 1.0, never above.
        m.add(
            3,
            Box::new(Dc(vec![0.9, 0.9])),
            SourceParams::default(),
            0.0,
        );
        run(&mut m, 480);
        let out = run(&mut m, 480);
        assert!(out[0] < 1.0 && out[0] > 0.95, "{}", out[0]);
        let peaks = m.take_peaks();
        assert!(peaks[0] < 1.0 && peaks[0] > 0.95);
        assert_eq!(m.take_peaks(), vec![0.0, 0.0]);
    }

    #[test]
    fn gain_mute_groups_and_master() {
        let mut m = Mixer::new(2);
        let p = SourceParams {
            group: 7,
            gain_db: -6.0,
            ..SourceParams::default()
        };
        m.add(1, Box::new(Dc(vec![0.5, 0.5])), p.clone(), 0.0);
        run(&mut m, 480);
        let out = run(&mut m, 480);
        assert!((out[0] - 0.5 * 0.501_187).abs() < 1e-4);
        m.set_group(7, -6.0, false);
        run(&mut m, 480);
        let out = run(&mut m, 480);
        assert!((out[0] - 0.5 * 0.501_187 * 0.501_187).abs() < 1e-4);
        m.set_master(6.0);
        run(&mut m, 480);
        let out = run(&mut m, 480);
        assert!((out[0] - 0.5 * 0.501_187).abs() < 1e-3);
        m.set_mute(true);
        run(&mut m, 480);
        assert!(run(&mut m, 480).iter().all(|v| *v == 0.0));
        assert_eq!(m.master(), (6.0, true));
        m.set_mute(false);
        m.set_group(7, 0.0, true);
        run(&mut m, 480);
        assert!(run(&mut m, 480).iter().all(|v| *v == 0.0));
        m.set_group(7, 0.0, false);
        m.set_params(1, SourceParams { muted: true, ..p });
        run(&mut m, 480);
        assert!(run(&mut m, 480).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn channel_routing_swaps_and_mutes() {
        let mut m = Mixer::new(2);
        let swap = SourceParams {
            routes: vec![
                Route {
                    source: 0,
                    dest: 1,
                    ..Route::default()
                },
                Route {
                    source: 1,
                    dest: 0,
                    ..Route::default()
                },
            ],
            ..SourceParams::default()
        };
        m.add(1, Box::new(Dc(vec![0.5, -0.25])), swap, 0.0);
        run(&mut m, 480);
        let out = run(&mut m, 480);
        assert_eq!((out[0], out[1]), (-0.25, 0.5));
        // Mute the right → left route only.
        let p = SourceParams {
            routes: vec![
                Route {
                    source: 0,
                    dest: 1,
                    ..Route::default()
                },
                Route {
                    source: 1,
                    dest: 0,
                    muted: true,
                    ..Route::default()
                },
            ],
            ..SourceParams::default()
        };
        m.set_params(1, p);
        let out = run(&mut m, 480);
        assert_eq!((out[0], out[1]), (0.0, 0.5));
    }

    #[test]
    fn fades_follow_equal_power_curves_and_remove() {
        let mut m = Mixer::new(1);
        // 0.1 s fade in = 4800 frames.
        m.add(1, Box::new(Dc(vec![1.0])), SourceParams::default(), 0.1);
        let out = run(&mut m, 4800);
        assert_eq!(out[0], 0.0);
        assert!((out[1] - dsp::fade_in(1.0 / 4800.0)).abs() < 1e-6);
        assert!(
            (out[2399] - dsp::fade_in(0.5)).abs() < 1e-3,
            "{}",
            out[2399]
        );
        let out = run(&mut m, 480);
        assert!(out.iter().all(|v| (*v - dsp::soft_clip(1.0)).abs() < 1e-6));
        // Fade out over 0.05 s, then gone.
        m.remove(1, 0.05);
        let out = run(&mut m, 2400);
        assert!(out[0] > 0.85);
        assert!(
            (out[1199] - dsp::fade_out(0.5)).abs() < 2e-3,
            "{}",
            out[1199]
        );
        assert!(out[2399] < 0.01);
        assert!(m.is_empty());
        assert!(run(&mut m, 480).iter().all(|v| *v == 0.0));
    }

    #[test]
    fn a_finished_source_is_removed() {
        struct Once(bool);
        impl Source for Once {
            fn channels(&self) -> usize {
                1
            }
            fn render(&mut self, _: &mut [f32], _: &BlockCtx) -> bool {
                std::mem::replace(&mut self.0, false)
            }
        }
        let mut m = Mixer::new(2);
        m.add(9, Box::new(Once(true)), SourceParams::default(), 0.0);
        run(&mut m, 10);
        assert!(m.contains(9));
        run(&mut m, 10);
        assert!(!m.contains(9));
    }

    /// A source brought back while it fades out keeps playing (the same source, from the
    /// level it had faded to, back up to full); one already gone is added anew.
    #[test]
    fn resume_cancels_a_fade_out() {
        let mut m = Mixer::new(1);
        m.add(1, Box::new(Dc(vec![0.5])), SourceParams::default(), 0.0);
        run(&mut m, 480);
        m.remove(1, 1.0);
        for _ in 0..30 {
            run(&mut m, 480); // 0.3 s into the 1 s fade
        }
        let faded = run(&mut m, 480)[479];
        assert!(faded > 0.2 && faded < 0.5, "{faded}");
        // The replacement is dropped: the source in the mix goes on (it plays 0.5, not 0.1).
        m.resume(1, Box::new(Dc(vec![0.1])), SourceParams::default(), 1.0);
        let first = run(&mut m, 480);
        assert!(
            (first[0] - faded).abs() < 0.02,
            "continues from {faded}: {}",
            first[0]
        );
        let mut last = first[479];
        for _ in 0..150 {
            let out = run(&mut m, 480);
            assert!(out[0] >= last - 1e-6, "rises again: {} < {last}", out[0]);
            last = out[479];
        }
        assert!((last - 0.5).abs() < 1e-6, "{last}");
        assert_eq!(m.len(), 1);
        // Gone: the replacement plays.
        m.remove(1, 0.0);
        run(&mut m, 480);
        assert!(m.is_empty());
        m.resume(1, Box::new(Dc(vec![0.1])), SourceParams::default(), 0.0);
        run(&mut m, 480);
        assert!((run(&mut m, 480)[0] - 0.1).abs() < 1e-6);
    }
}
