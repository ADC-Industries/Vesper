//! Media clocks: which media time is shown at which wall-clock instant.

use std::time::Instant;

/// A media clock in timeline seconds: running at `rate` from `base` since `origin`, or paused
/// at `base`. `Copy`, so callers can snapshot it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaClock {
    origin: Instant,
    base: f64,
    rate: f64,
    paused: bool,
}

impl MediaClock {
    /// A clock running from `from` at `rate` since `at`.
    pub fn start(at: Instant, from: f64, rate: f64) -> Self {
        MediaClock {
            origin: at,
            base: from,
            rate: sane_rate(rate),
            paused: false,
        }
    }

    /// A paused clock at `t`.
    pub fn paused_at(t: f64, rate: f64) -> Self {
        MediaClock {
            origin: Instant::now(),
            base: t,
            rate: sane_rate(rate),
            paused: true,
        }
    }

    /// Media time at `at` (instants before the start give the start time).
    pub fn time_at(&self, at: Instant) -> f64 {
        if self.paused {
            return self.base;
        }
        let dt = at.saturating_duration_since(self.origin).as_secs_f64();
        self.base + dt * self.rate
    }

    /// Whether the clock is paused.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Playback rate (speed multiplier).
    pub fn rate(&self) -> f64 {
        self.rate
    }

    /// Pauses at the media time of `at`.
    pub fn pause(&mut self, at: Instant) {
        if !self.paused {
            self.base = self.time_at(at);
            self.origin = at;
            self.paused = true;
        }
    }

    /// Resumes from where it was paused.
    pub fn resume(&mut self, at: Instant) {
        if self.paused {
            self.origin = at;
            self.paused = false;
        }
    }

    /// Jumps to `t` (keeps running or paused).
    pub fn seek(&mut self, t: f64, at: Instant) {
        self.base = t;
        self.origin = at;
    }
}

fn sane_rate(r: f64) -> f64 {
    if r.is_finite() && r > 0.0 {
        r.clamp(0.05, 16.0)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn runs_pauses_seeks() {
        let t0 = Instant::now();
        let mut c = MediaClock::start(t0, 1.0, 2.0);
        assert_eq!(c.time_at(t0), 1.0);
        assert!((c.time_at(t0 + Duration::from_millis(500)) - 2.0).abs() < 1e-9);
        c.pause(t0 + Duration::from_secs(1));
        assert!((c.time_at(t0 + Duration::from_secs(5)) - 3.0).abs() < 1e-9);
        c.resume(t0 + Duration::from_secs(5));
        assert!((c.time_at(t0 + Duration::from_secs(6)) - 5.0).abs() < 1e-9);
        c.seek(10.0, t0 + Duration::from_secs(6));
        assert!((c.time_at(t0 + Duration::from_secs(7)) - 12.0).abs() < 1e-9);
        // Before the origin: the start time.
        let c = MediaClock::start(t0 + Duration::from_secs(1), 4.0, 1.0);
        assert_eq!(c.time_at(t0), 4.0);
    }
}
