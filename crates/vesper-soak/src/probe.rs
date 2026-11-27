//! The A/V probe: receives the first audience output (IDN Open Protocol, compatible with NDI®,
//! with the program audio embedded) as any receiver on the network would, counts its frames and
//! timecode gaps, and while the sync clip plays pairs every beep heard with the white flash
//! shown at the same time: the offset (audio − video timecode) is the A/V drift at the output.

use output::{IdnFinder, IdnReceiver, Received};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One play of the sync clip as received.
#[derive(Clone, Debug)]
pub struct AvPhase {
    /// Seconds since the soak started (at its end).
    pub t: f64,
    /// Beeps heard.
    pub beeps: usize,
    /// Flashes seen.
    pub flashes: usize,
    /// Offsets of the beeps paired with a flash, ms (audio − video; positive = sound late).
    pub offsets: Vec<f64>,
}

impl AvPhase {
    pub fn mean(&self) -> Option<f64> {
        (!self.offsets.is_empty())
            .then(|| self.offsets.iter().sum::<f64>() / self.offsets.len() as f64)
    }

    pub fn worst(&self) -> Option<f64> {
        self.offsets
            .iter()
            .copied()
            .max_by(|a, b| a.abs().total_cmp(&b.abs()))
    }
}

/// What the probe measured, read by the watcher.
#[derive(Default)]
pub struct ProbeStats {
    /// Video frames received.
    pub frames: AtomicU64,
    /// Gaps in the video timecodes (frames the output never sent), in frames.
    pub gaps: AtomicU64,
    /// Audio blocks received.
    pub audio_blocks: AtomicU64,
    /// Times the receiver had to connect again.
    pub reconnects: AtomicU64,
    /// Receiving now.
    pub connected: AtomicBool,
    /// Finished plays of the sync clip.
    pub phases: Mutex<Vec<AvPhase>>,
}

/// Runs the probe for `source` (the output's source name) until `stop`; `av` is set while the
/// sync clip plays.
pub fn spawn(
    source: String,
    t0: Instant,
    av: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    stats: Arc<ProbeStats>,
) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("probe".into())
        .spawn(move || {
            let finder = match IdnFinder::new() {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("probe: no finder: {e:#}");
                    return;
                }
            };
            let mut first = true;
            while !stop.load(Ordering::Relaxed) {
                let Some(found) = finder.find_by_name(&source, Duration::from_secs(5)) else {
                    continue;
                };
                let Ok(rx) = IdnReceiver::connect(&found.name) else {
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                };
                if !first {
                    stats.reconnects.fetch_add(1, Ordering::Relaxed);
                }
                first = false;
                stats.connected.store(true, Ordering::Relaxed);
                receive(rx, t0, &av, &stop, &stats);
                stats.connected.store(false, Ordering::Relaxed);
            }
        })
        .expect("probe thread")
}

fn receive(
    mut rx: IdnReceiver,
    t0: Instant,
    av: &AtomicBool,
    stop: &AtomicBool,
    stats: &ProbeStats,
) {
    let mut collecting = false;
    let (mut flashes, mut beeps): (Vec<i64>, Vec<i64>) = (Vec::new(), Vec::new());
    let mut was_white = false;
    let mut last_loud: Option<i64> = None;
    let mut last_tc: Option<(i64, i64)> = None; // (timecode, period)
    let mut silent_since = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let on = av.load(Ordering::Relaxed);
        if on && !collecting {
            flashes.clear();
            beeps.clear();
            collecting = true;
        } else if !on && collecting {
            collecting = false;
            let phase = AvPhase {
                t: t0.elapsed().as_secs_f64(),
                beeps: beeps.len(),
                flashes: flashes.len(),
                offsets: pair(&beeps, &flashes),
            };
            eprintln!(
                "  A/V: {} beeps, {} flashes, {} paired, mean {:?} ms, worst {:?} ms",
                phase.beeps,
                phase.flashes,
                phase.offsets.len(),
                phase.mean().map(|m| (m * 10.0).round() / 10.0),
                phase.worst().map(|m| (m * 10.0).round() / 10.0)
            );
            stats.phases.lock().unwrap().push(phase);
        }
        let got = match rx.capture(Duration::from_millis(200)) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("probe: {e:#}");
                return;
            }
        };
        match got {
            Some(Received::Video(f)) => {
                silent_since = Instant::now();
                stats.frames.fetch_add(1, Ordering::Relaxed);
                let period =
                    10_000_000i64 * f.frame_rate.1.max(1) as i64 / f.frame_rate.0.max(1) as i64;
                if let Some((tc, p)) = last_tc {
                    let steps = ((f.timecode - tc) as f64 / p as f64).round() as i64;
                    if steps > 1 && steps < 300 {
                        stats.gaps.fetch_add((steps - 1) as u64, Ordering::Relaxed);
                    }
                }
                last_tc = Some((f.timecode, period));
                let p = f.pixel_rgba(8, 8);
                let white = p[0] > 200 && p[1] > 200 && p[2] > 200;
                if collecting && white && !was_white {
                    flashes.push(f.timecode);
                }
                was_white = white;
            }
            Some(Received::Audio(a)) => {
                silent_since = Instant::now();
                stats.audio_blocks.fetch_add(1, Ordering::Relaxed);
                if a.channels == 0 || a.sample_rate == 0 {
                    continue;
                }
                for (i, v) in a.channel(0).iter().enumerate() {
                    if v.abs() > 0.05 {
                        let tc = a.timecode + (i as i64 * 10_000_000) / a.sample_rate as i64;
                        if last_loud.is_none_or(|l| tc - l > 5_000_000) && collecting {
                            beeps.push(tc);
                        }
                        last_loud = Some(tc);
                    }
                }
            }
            None => {
                if silent_since.elapsed() > Duration::from_secs(10) {
                    eprintln!("probe: nothing received for 10 s, connecting again");
                    return;
                }
            }
        }
    }
}

/// Offsets (ms) of each beep from the nearest flash within 300 ms.
fn pair(beeps: &[i64], flashes: &[i64]) -> Vec<f64> {
    beeps
        .iter()
        .filter_map(|b| {
            flashes
                .iter()
                .map(|f| (b - f) as f64 / 10_000.0)
                .min_by(|x, y| x.abs().total_cmp(&y.abs()))
                .filter(|o| o.abs() <= 300.0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beeps_pair_with_the_nearest_flash() {
        let s = 10_000_000;
        let flashes = [0, 2 * s, 4 * s];
        let beeps = [200_000, 2 * s - 100_000, 6 * s];
        assert_eq!(pair(&beeps, &flashes), vec![20.0, -10.0]);
    }
}
