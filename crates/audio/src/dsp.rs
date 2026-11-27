//! Signal helpers: levels, fade curves, routing, soft clipping. Pure functions on interleaved
//! `f32` buffers.

/// Gains at or below this many dB are silence.
pub const SILENCE_DB: f64 = -96.0;

/// Meter floor, dBFS (what silence reads).
pub const METER_FLOOR_DB: f64 = -60.0;

/// The amplitude factor of a level in dB (0 dB = 1.0; ≤ [`SILENCE_DB`] or non-finite = 0).
pub fn db_to_gain(db: f64) -> f32 {
    if !db.is_finite() || db <= SILENCE_DB {
        0.0
    } else {
        10f64.powf(db / 20.0) as f32
    }
}

/// The level of an amplitude factor in dB (`-inf` for 0).
pub fn gain_to_db(g: f32) -> f64 {
    if g <= 0.0 {
        f64::NEG_INFINITY
    } else {
        20.0 * (g as f64).log10()
    }
}

/// A peak amplitude as a meter reading: dBFS rounded to whole dB, clamped to
/// [`METER_FLOOR_DB`]..=+6.
pub fn meter_db(peak: f32) -> f64 {
    let db = gain_to_db(peak);
    if !db.is_finite() {
        return METER_FLOOR_DB;
    }
    db.round().clamp(METER_FLOOR_DB, 6.0)
}

/// Fade-in curve: equal power (`sin(p·π/2)`), `p` in 0..=1. With [`fade_out`] at the same `p`
/// the summed power of a cross-fade stays constant.
pub fn fade_in(p: f64) -> f32 {
    let p = p.clamp(0.0, 1.0);
    (p * std::f64::consts::FRAC_PI_2).sin() as f32
}

/// Fade-out curve: equal power (`cos(p·π/2)`), `p` in 0..=1 (1 at the start, 0 at the end).
pub fn fade_out(p: f64) -> f32 {
    let p = p.clamp(0.0, 1.0);
    (p * std::f64::consts::FRAC_PI_2).cos() as f32
}

/// Soft clipping: unity below the knee (−1 dBFS), then a tanh curve that approaches ±1 and
/// never exceeds it. Continuous with a continuous slope at the knee.
pub fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.891_250_9; // −1 dBFS
    let a = x.abs();
    if a <= KNEE || !a.is_finite() {
        return if x.is_finite() { x } else { 0.0 };
    }
    let range = 1.0 - KNEE;
    let y = KNEE + range * ((a - KNEE) / range).tanh();
    y.copysign(x)
}

/// One routing entry: source channel `src` → bus channel `dest` at `gain` (linear).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tap {
    /// Source channel.
    pub src: usize,
    /// Bus channel.
    pub dest: usize,
    /// Linear gain.
    pub gain: f32,
}

/// The routing matrix for `routes` (source, dest, gain dB, muted): muted routes and routes
/// to channels that do not exist are dropped. No routes = identity (source *n* → bus *n*), with
/// a mono source feeding every bus channel.
pub fn routing(
    routes: &[(u32, u32, f64, bool)],
    src_channels: usize,
    bus_channels: usize,
) -> Vec<Tap> {
    if routes.is_empty() {
        if src_channels == 1 {
            return (0..bus_channels)
                .map(|d| Tap {
                    src: 0,
                    dest: d,
                    gain: 1.0,
                })
                .collect();
        }
        return (0..src_channels.min(bus_channels))
            .map(|c| Tap {
                src: c,
                dest: c,
                gain: 1.0,
            })
            .collect();
    }
    routes
        .iter()
        .filter(|r| !r.3)
        .filter(|r| (r.0 as usize) < src_channels && (r.1 as usize) < bus_channels)
        .map(|r| Tap {
            src: r.0 as usize,
            dest: r.1 as usize,
            gain: db_to_gain(r.2),
        })
        .filter(|t| t.gain > 0.0)
        .collect()
}

/// Adds `input` (interleaved, `in_ch` channels, `frames` frames) routed through `taps` into
/// `bus` (interleaved, `bus_ch`), each frame scaled by `gain(i)` (frame index → factor).
pub fn mix_routed(
    input: &[f32],
    in_ch: usize,
    bus: &mut [f32],
    bus_ch: usize,
    frames: usize,
    taps: &[Tap],
    gain: impl Fn(usize) -> f32,
) {
    for i in 0..frames {
        let g = gain(i);
        if g == 0.0 {
            continue;
        }
        let src = &input[i * in_ch..(i + 1) * in_ch];
        let dst = &mut bus[i * bus_ch..(i + 1) * bus_ch];
        for t in taps {
            dst[t.dest] += src[t.src] * t.gain * g;
        }
    }
}

/// A gain that glides to its target over one block (no zipper noise on level changes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smoothed {
    /// Gain at the start of the next block.
    pub current: f32,
    /// Where it is going.
    pub target: f32,
}

impl Smoothed {
    /// Starts at `g`.
    pub fn new(g: f32) -> Self {
        Smoothed {
            current: g,
            target: g,
        }
    }

    /// The gain at frame `i` of a block of `n` frames (linear from current to target).
    pub fn at(&self, i: usize, n: usize) -> f32 {
        if self.current == self.target || n == 0 {
            return self.target;
        }
        self.current + (self.target - self.current) * (i as f32 + 1.0) / n as f32
    }

    /// The block is done: the target is reached.
    pub fn advance(&mut self) {
        self.current = self.target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_conversions() {
        assert_eq!(db_to_gain(0.0), 1.0);
        assert!((db_to_gain(-6.0) - 0.501_187).abs() < 1e-5);
        assert!((db_to_gain(6.0) - 1.995_262).abs() < 1e-5);
        assert_eq!(db_to_gain(-96.0), 0.0);
        assert_eq!(db_to_gain(f64::NEG_INFINITY), 0.0);
        assert_eq!(db_to_gain(f64::NAN), 0.0);
        assert!((gain_to_db(0.5) + 6.0206).abs() < 1e-3);
        assert_eq!(meter_db(0.0), METER_FLOOR_DB);
        assert_eq!(meter_db(1.0), 0.0);
        assert_eq!(meter_db(0.125), -18.0);
        assert_eq!(meter_db(1e-5), METER_FLOOR_DB);
    }

    #[test]
    fn fade_curves_are_equal_power_and_monotonic() {
        assert_eq!(fade_in(0.0), 0.0);
        assert!((fade_in(1.0) - 1.0).abs() < 1e-6);
        assert!((fade_out(0.0) - 1.0).abs() < 1e-6);
        assert!(fade_out(1.0).abs() < 1e-6);
        let mut prev = -1.0;
        for i in 0..=100 {
            let p = i as f64 / 100.0;
            let (a, b) = (fade_in(p), fade_out(p));
            assert!((a * a + b * b - 1.0).abs() < 1e-5, "power at {p}");
            assert!(a >= prev);
            prev = a;
        }
        // Half way: −3 dB each.
        assert!((gain_to_db(fade_in(0.5)) + 3.01).abs() < 0.01);
        // Out of range clamps.
        assert_eq!(fade_in(-1.0), 0.0);
        assert!((fade_in(2.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn soft_clip_is_transparent_below_the_knee_and_bounded() {
        for x in [-0.89f32, -0.5, 0.0, 0.25, 0.8, 0.89] {
            assert_eq!(soft_clip(x), x);
        }
        let mut prev = 0.0;
        for i in 0..400 {
            let x = 0.85 + i as f32 * 0.01;
            let y = soft_clip(x);
            assert!(y <= 1.0 && y >= prev, "{x} → {y}");
            assert_eq!(soft_clip(-x), -y);
            prev = y;
        }
        assert!(soft_clip(100.0) <= 1.0);
        assert_eq!(soft_clip(f32::NAN), 0.0);
        // Continuous at the knee.
        let k = 0.891_250_9f32;
        assert!((soft_clip(k + 1e-4) - (k + 1e-4)).abs() < 1e-5);
    }

    #[test]
    fn routing_swaps_mutes_and_defaults() {
        // Identity.
        let r = routing(&[], 2, 2);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].src, r[0].dest), (0, 0));
        // Mono feeds every bus channel.
        let r = routing(&[], 1, 2);
        assert_eq!(r.iter().map(|t| t.dest).collect::<Vec<_>>(), vec![0, 1]);
        // Left/right swap, with a gain.
        let r = routing(&[(0, 1, 0.0, false), (1, 0, -6.0, false)], 2, 2);
        let input = [1.0f32, 0.5, 1.0, 0.5];
        let mut bus = [0.0f32; 4];
        mix_routed(&input, 2, &mut bus, 2, 2, &r, |_| 1.0);
        assert!((bus[0] - 0.5 * 0.501_187).abs() < 1e-5);
        assert_eq!(bus[1], 1.0);
        // Muted route and out-of-range channels are dropped.
        let r = routing(
            &[(0, 0, 0.0, true), (1, 1, 0.0, false), (5, 0, 0.0, false)],
            2,
            2,
        );
        assert_eq!(r.len(), 1);
        let mut bus = [0.0f32; 4];
        mix_routed(&input, 2, &mut bus, 2, 2, &r, |_| 1.0);
        assert_eq!(bus, [0.0, 0.5, 0.0, 0.5]);
    }

    #[test]
    fn smoothed_gain_ramps_over_a_block() {
        let mut s = Smoothed::new(0.0);
        s.target = 1.0;
        assert!((s.at(0, 4) - 0.25).abs() < 1e-6);
        assert_eq!(s.at(3, 4), 1.0);
        s.advance();
        assert_eq!(s.at(0, 4), 1.0);
    }
}
