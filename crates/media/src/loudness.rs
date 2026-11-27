//! Loudness of a file's sound (EBU R128 / ITU-R BS.1770-4 integrated loudness) and its sample
//! peak, for media preparation's gain to the house target.
//!
//! The sound is read through IDN's `AudioReader` at 48 kHz stereo (a mono file is copied into
//! both channels, as it plays), K-weighted (the two BS.1770 filters at 48 kHz), cut into 400 ms
//! blocks every 100 ms, gated at −70 LUFS and then 10 LU below the ungated mean.

use anyhow::Context as _;
use idn_ffmpeg::media as idn;
use std::path::Path;
use std::time::Duration;

/// The sample rate everything here runs at.
const RATE: u32 = 48_000;
/// 100 ms of samples (a quarter block).
const STEP: usize = RATE as usize / 10;

/// A file's measured loudness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loudness {
    /// Integrated loudness, LUFS (`None`: silence, nothing above the absolute gate).
    pub integrated: Option<f64>,
    /// The highest sample, dBFS (`None`: silence).
    pub peak: Option<f64>,
    /// Seconds of sound measured.
    pub seconds: f64,
}

/// One biquad section (direct form I).
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl Biquad {
    fn new(b: [f64; 3], a: [f64; 2]) -> Biquad {
        Biquad {
            b,
            a,
            ..Biquad::default()
        }
    }
    fn run(&mut self, x: f64) -> f64 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// BS.1770's K-weighting at 48 kHz: the high shelf, then the high pass.
fn k_weighting() -> [Biquad; 2] {
    [
        Biquad::new(
            [
                1.535_124_859_586_97,
                -2.691_696_189_406_38,
                1.198_392_810_852_85,
            ],
            [-1.690_659_293_182_41, 0.732_480_774_215_85],
        ),
        Biquad::new(
            [1.0, -2.0, 1.0],
            [-1.990_047_454_833_98, 0.990_072_250_366_21],
        ),
    ]
}

/// Integrated loudness, fed interleaved stereo 48 kHz samples in any slices.
pub struct Meter {
    filters: [[Biquad; 2]; 2],
    /// Sum of squares of the current 100 ms step, per channel.
    acc: [f64; 2],
    n: usize,
    /// Mean square (both channels summed) of each finished 100 ms step.
    steps: Vec<f64>,
    peak: f32,
    frames: u64,
}

impl Default for Meter {
    fn default() -> Self {
        Meter {
            filters: [k_weighting(), k_weighting()],
            acc: [0.0; 2],
            n: 0,
            steps: Vec::new(),
            peak: 0.0,
            frames: 0,
        }
    }
}

impl Meter {
    /// Adds interleaved stereo frames.
    pub fn push(&mut self, stereo: &[f32]) {
        for f in stereo.as_chunks::<2>().0 {
            for (c, &s) in f.iter().enumerate() {
                self.peak = self.peak.max(s.abs());
                let [hs, hp] = &mut self.filters[c];
                let y = hp.run(hs.run(s as f64));
                self.acc[c] += y * y;
            }
            self.n += 1;
            self.frames += 1;
            if self.n == STEP {
                self.steps.push((self.acc[0] + self.acc[1]) / STEP as f64);
                self.acc = [0.0; 2];
                self.n = 0;
            }
        }
    }

    /// The measurement so far.
    pub fn result(&self) -> Loudness {
        // 400 ms blocks, 75 % overlap: four consecutive steps each.
        let blocks: Vec<f64> = self
            .steps
            .windows(4)
            .map(|w| w.iter().sum::<f64>() / 4.0)
            .collect();
        let lufs = |z: f64| -0.691 + 10.0 * z.log10();
        let above = |gate: f64| -> Vec<f64> {
            blocks
                .iter()
                .copied()
                .filter(|&z| z > 0.0 && lufs(z) > gate)
                .collect()
        };
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        let abs = above(-70.0);
        let integrated = if abs.is_empty() {
            None
        } else {
            let rel = above(lufs(mean(&abs)) - 10.0);
            (!rel.is_empty()).then(|| lufs(mean(&rel)))
        };
        Loudness {
            integrated,
            peak: (self.peak > 0.0).then(|| 20.0 * (self.peak as f64).log10()),
            seconds: self.frames as f64 / RATE as f64,
        }
    }
}

/// Measures a file's sound (`None`: the file has no sound). `stop` is asked between reads.
pub fn measure_loudness(path: &Path, stop: &dyn Fn() -> bool) -> anyhow::Result<Option<Loudness>> {
    let m = crate::probe::probe_file(path)?;
    if m.audio.is_none() {
        return Ok(None);
    }
    let cfg = idn::AudioReaderConfig {
        sample_rate: RATE,
        channels: 2,
        in_point: Duration::ZERO,
        out_point: None,
        loops: Some(1),
    };
    let mut reader = idn::AudioReader::open(path, cfg)
        .with_context(|| format!("cannot read the sound of {}", path.display()))?;
    let total = reader.length();
    let mut meter = Meter::default();
    let mut buf = vec![0f32; STEP * 2 * 10];
    let mut pos = 0u64;
    loop {
        if stop() {
            anyhow::bail!("stopped");
        }
        let n = reader
            .read(pos, &mut buf)
            .with_context(|| format!("reading the sound of {}", path.display()))?;
        if n == 0 {
            break;
        }
        // `read` answers frames, filling `n * channels` samples.
        meter.push(&buf[..n * 2]);
        pos += n as u64;
        if n < buf.len() / 2 || total.is_some_and(|t| pos >= t) {
            break;
        }
    }
    Ok(Some(meter.result()))
}

/// The gain that brings `measured` loudness to `target`, kept within −20…+12 dB and so that
/// the peak stays at or below −1 dBFS (`None`: no loudness to go by).
pub fn gain_to_target(measured: &Loudness, target: f64) -> Option<f64> {
    let l = measured.integrated?;
    let mut g = (target - l).clamp(-20.0, 12.0);
    if let Some(p) = measured.peak {
        g = g.min(-1.0 - p);
    }
    // Tenths of a dB are enough.
    Some((g * 10.0).round() / 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(amplitude: f32, hz: f32, secs: f32) -> Vec<f32> {
        let n = (secs * RATE as f32) as usize;
        let mut v = Vec::with_capacity(n * 2);
        for i in 0..n {
            let s = amplitude * (2.0 * std::f32::consts::PI * hz * i as f32 / RATE as f32).sin();
            v.extend_from_slice(&[s, s]);
        }
        v
    }

    #[test]
    fn a_stereo_1khz_sine_at_minus_23_dbfs_reads_minus_23_lufs() {
        // EBU Tech 3341, test 1: 1 kHz, −23 dBFS in both channels → −23.0 LUFS (±0.1).
        let mut m = Meter::default();
        let a = 10f32.powf(-23.0 / 20.0);
        for chunk in sine(a, 1000.0, 20.0).chunks(9_999 * 2) {
            m.push(chunk);
        }
        let r = m.result();
        let l = r.integrated.unwrap();
        assert!((l + 23.0).abs() < 0.1, "{l}");
        assert!((r.peak.unwrap() + 23.0).abs() < 0.05);
        assert!((r.seconds - 20.0).abs() < 1e-6);
    }

    #[test]
    fn silence_is_gated_out_and_quiet_parts_do_not_drag_the_level_down() {
        let mut m = Meter::default();
        m.push(&vec![0.0; RATE as usize * 4]);
        assert_eq!(m.result().integrated, None);
        assert_eq!(m.result().peak, None);
        // 10 s at −23 dBFS then 10 s at −60 dBFS: the quiet part is below the relative gate.
        let mut m = Meter::default();
        m.push(&sine(10f32.powf(-23.0 / 20.0), 1000.0, 10.0));
        m.push(&sine(10f32.powf(-60.0 / 20.0), 1000.0, 10.0));
        let l = m.result().integrated.unwrap();
        assert!((l + 23.0).abs() < 0.2, "{l}");
    }

    #[test]
    fn gain_reaches_the_target_without_clipping() {
        let quiet = Loudness {
            integrated: Some(-30.0),
            peak: Some(-20.0),
            seconds: 10.0,
        };
        assert_eq!(gain_to_target(&quiet, -16.0), Some(12.0));
        let peaky = Loudness {
            integrated: Some(-24.0),
            peak: Some(-3.0),
            seconds: 10.0,
        };
        // +8 would put the peak at +5 dBFS: held to −1.
        assert_eq!(gain_to_target(&peaky, -16.0), Some(2.0));
        let loud = Loudness {
            integrated: Some(-8.0),
            peak: Some(0.0),
            seconds: 10.0,
        };
        assert_eq!(gain_to_target(&loud, -16.0), Some(-8.0));
        let none = Loudness {
            integrated: None,
            peak: None,
            seconds: 1.0,
        };
        assert_eq!(gain_to_target(&none, -16.0), None);
    }
}
