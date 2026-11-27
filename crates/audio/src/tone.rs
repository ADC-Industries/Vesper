//! A sine test tone source.

use crate::mixer::{BlockCtx, Source};

/// A sine wave on every channel for a fixed length, with 10 ms ramps at both ends.
pub struct Tone {
    freq: f64,
    amp: f32,
    channels: usize,
    total: usize,
    done: usize,
}

impl Tone {
    /// `freq` Hz at `dbfs` (peak) for `seconds` on `channels` channels.
    pub fn new(freq: f64, dbfs: f64, seconds: f64, channels: usize) -> Tone {
        Tone {
            freq: freq.clamp(20.0, 20_000.0),
            amp: crate::dsp::db_to_gain(dbfs),
            channels: channels.max(1),
            total: (seconds.clamp(0.0, 3600.0) * crate::SAMPLE_RATE as f64) as usize,
            done: 0,
        }
    }
}

impl Source for Tone {
    fn channels(&self) -> usize {
        self.channels
    }

    fn render(&mut self, out: &mut [f32], ctx: &BlockCtx) -> bool {
        let ramp = (crate::SAMPLE_RATE / 100) as f32;
        let w = 2.0 * std::f64::consts::PI * self.freq / crate::SAMPLE_RATE as f64;
        for f in 0..ctx.frames {
            let k = self.done + f;
            if k >= self.total {
                break;
            }
            let edge = (k as f32 / ramp)
                .min((self.total - k) as f32 / ramp)
                .min(1.0);
            let v = (w * k as f64).sin() as f32 * self.amp * edge;
            for c in 0..self.channels {
                out[f * self.channels + c] = v;
            }
        }
        self.done += ctx.frames;
        self.done < self.total
    }
}
