//! Timers: what each timer is doing and its value at any instant.
//!
//! The engine is pure: every operation takes a [`Clock`] (a monotonic instant plus the local
//! time of day), so values can be computed for any instant (the render thread computes them for
//! the frame it draws; the tests simulate minutes in microseconds). The show state holds a
//! [`Timers`] (it is cloned with the state); timer documents stay in the data.
//!
//! - **Countdown** counts `seconds` down. At zero it **expires** (holds 0) unless the timer has
//!   `allowOverrun`: then it keeps counting below zero (**overrun**).
//! - **Countdown to time** counts down to a time of day (server local time). The target is
//!   `secondsAfterMidnight` read with its `period` (`am` / `pm` turn a 12-hour value into 24-hour;
//!   `h24` is taken as is). When the time has passed today, it counts to tomorrow's. While
//!   stopped it shows the time left from *now*; once started it counts on the monotonic clock
//!   (a change of the wall clock does not move it).
//! - **Elapsed** counts up from `start` (default 0) and expires at `end` when it has one.
//!
//! Operations: start (resume; an expired timer starts over), stop (pause), reset (back to the
//! configuration, stopped), set (a value, or a time of day), adjust (± seconds, in any state).
//! Set and adjust are not stored: reset goes back to the configuration.
//!
//! Displayed whole seconds: countdowns round **up**, elapsed timers down (see
//! [`render::live::format_timer`]); [`next_change`](Timers::next_change) says when the shown
//! value changes next, so screens re-render exactly then.

use model::{TimePeriod, Timer, TimerFormat, TimerId, TimerKind};
use protocol::TimerRunState;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

/// Seconds in a day.
pub const DAY: f64 = 86_400.0;

/// Largest value a timer can be set or adjusted to, either side of zero: the documents' limit
/// (99 hours). A value that only grows while a timer runs stays finite for centuries.
pub const MAX_VALUE: f64 = crate::showdocs::MAX_TIMER_SECONDS;

/// `v` within ±[`MAX_VALUE`] (a non-finite value is 0).
fn bounded(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(-MAX_VALUE, MAX_VALUE)
    } else {
        0.0
    }
}

/// The instant an operation happens at: monotonic time plus the local time of day.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    /// Monotonic now.
    pub now: Instant,
    /// Local time of day, seconds after midnight.
    pub wall: f64,
}

impl Clock {
    /// The real clock.
    pub fn now() -> Clock {
        Clock {
            now: Instant::now(),
            wall: local_seconds(),
        }
    }

    /// A clock `secs` later (tests and look-ahead).
    pub fn after(&self, secs: f64) -> Clock {
        Clock {
            now: self.now + Duration::from_secs_f64(secs.max(0.0)),
            wall: (self.wall + secs).rem_euclid(DAY),
        }
    }
}

/// Local time of day now, seconds after midnight.
pub fn local_seconds() -> f64 {
    use chrono::Timelike;
    let t = chrono::Local::now();
    t.num_seconds_from_midnight() as f64 + t.nanosecond() as f64 / 1e9
}

/// Local date and time now, for clocks.
pub fn wall_time() -> render::live::WallTime {
    use chrono::{Datelike, Timelike};
    let t = chrono::Local::now();
    render::live::WallTime {
        year: t.year(),
        month: t.month(),
        day: t.day(),
        weekday: t.weekday().num_days_from_monday(),
        hour: t.hour(),
        minute: t.minute(),
        second: t.second(),
    }
}

/// What a timer is doing.
#[derive(Clone, Debug, PartialEq)]
pub enum Run {
    /// Not started: shows `value` (set while stopped), else its configured start.
    Stopped {
        /// A value set while stopped.
        value: Option<f64>,
    },
    /// Counting from `base` since `since`.
    Running {
        /// Value at `since`.
        base: f64,
        /// When it (re)started.
        since: Instant,
    },
    /// Paused at `value`.
    Paused {
        /// The value.
        value: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct TimerRun {
    run: Run,
    /// A time of day set with `timerSet` (countdown-to-time timers).
    target: Option<f64>,
}

impl Default for TimerRun {
    fn default() -> Self {
        TimerRun {
            run: Run::Stopped { value: None },
            target: None,
        }
    }
}

/// A timer's value at an instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// Seconds (negative = overrun).
    pub seconds: f64,
    /// Run state.
    pub state: TimerRunState,
    /// Counts down.
    pub countdown: bool,
}

impl Reading {
    /// The value formatted with the default timer format.
    pub fn display(&self) -> String {
        render::live::format_timer(self.seconds, self.countdown, &TimerFormat::default())
    }

    /// As the renderer takes it.
    pub fn value(&self) -> render::live::TimerValue {
        render::live::TimerValue {
            seconds: self.seconds,
            countdown: self.countdown,
            running: matches!(self.state, TimerRunState::Running | TimerRunState::Overrun),
            expired: matches!(self.state, TimerRunState::Expired | TimerRunState::Overrun),
            warning: None,
            total: None,
        }
    }
}

/// The 24-hour target of a countdown-to-time timer (seconds after midnight).
pub fn target_of(seconds_after_midnight: f64, period: TimePeriod) -> f64 {
    let s = seconds_after_midnight.rem_euclid(DAY);
    match period {
        TimePeriod::Pm if s < DAY / 2.0 => s + DAY / 2.0,
        TimePeriod::Am if s >= DAY / 2.0 => s - DAY / 2.0,
        _ => s,
    }
}

/// Seconds from `wall` to the next occurrence of `target` (tomorrow's when it has passed).
pub fn until(target: f64, wall: f64) -> f64 {
    let r = target - wall;
    if r <= 0.0 { r + DAY } else { r }
}

fn counts_down(t: &Timer) -> bool {
    !matches!(t.kind, TimerKind::Elapsed { .. })
}

/// All timers' runtime state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Timers {
    runs: BTreeMap<TimerId, TimerRun>,
    /// The most recently started timer (data links without a timer show it).
    pub last_started: Option<TimerId>,
}

impl Timers {
    fn run(&self, id: TimerId) -> TimerRun {
        self.runs.get(&id).cloned().unwrap_or_default()
    }

    fn set_run(&mut self, id: TimerId, r: TimerRun) {
        if r == TimerRun::default() {
            self.runs.remove(&id);
        } else {
            self.runs.insert(id, r);
        }
    }

    fn start_value(t: &Timer, target: Option<f64>, wall: f64) -> f64 {
        match t.kind {
            TimerKind::Countdown { seconds } => seconds,
            TimerKind::CountdownToTime {
                seconds_after_midnight,
                period,
            } => until(
                target.unwrap_or_else(|| target_of(seconds_after_midnight, period)),
                wall,
            ),
            TimerKind::Elapsed { start, .. } => start.unwrap_or(0.0),
            TimerKind::Unknown => 0.0,
        }
    }

    /// The value and state of `t` at `c`.
    pub fn reading(&self, t: &Timer, c: Clock) -> Reading {
        let r = self.run(t.id);
        let countdown = counts_down(t);
        let (raw, state) = match r.run {
            Run::Stopped { value } => (
                value.unwrap_or_else(|| Self::start_value(t, r.target, c.wall)),
                TimerRunState::Stopped,
            ),
            Run::Paused { value } => (value, TimerRunState::Paused),
            Run::Running { base, since } => {
                let el = c.now.saturating_duration_since(since).as_secs_f64();
                (
                    if countdown { base - el } else { base + el },
                    TimerRunState::Running,
                )
            }
        };
        let (seconds, state) = match (&t.kind, state) {
            (TimerKind::Unknown, _) => (0.0, TimerRunState::Stopped),
            (TimerKind::Elapsed { end: Some(end), .. }, TimerRunState::Running) if raw >= *end => {
                (*end, TimerRunState::Expired)
            }
            (TimerKind::Elapsed { .. }, s) => (raw, s),
            (_, TimerRunState::Running) if raw <= 0.0 => {
                if t.allow_overrun {
                    (raw, TimerRunState::Overrun)
                } else {
                    (0.0, TimerRunState::Expired)
                }
            }
            (_, s) => (raw, s),
        };
        Reading {
            seconds,
            state,
            countdown,
        }
    }

    /// Starts or resumes `t`. An expired timer starts over. Returns whether anything changed.
    pub fn start(&mut self, t: &Timer, c: Clock) -> bool {
        let mut r = self.run(t.id);
        let reading = self.reading(t, c);
        let base = match (&r.run, reading.state) {
            (Run::Running { .. }, TimerRunState::Expired) => Self::start_value(t, r.target, c.wall),
            (Run::Running { .. }, _) => return false,
            (Run::Paused { value }, _) => *value,
            (Run::Stopped { value }, _) => {
                value.unwrap_or_else(|| Self::start_value(t, r.target, c.wall))
            }
        };
        r.run = Run::Running { base, since: c.now };
        self.set_run(t.id, r);
        self.last_started = Some(t.id);
        true
    }

    /// Pauses `t` at its current value.
    pub fn stop(&mut self, t: &Timer, c: Clock) -> bool {
        let mut r = self.run(t.id);
        if !matches!(r.run, Run::Running { .. }) {
            return false;
        }
        r.run = Run::Paused {
            value: self.reading(t, c).seconds,
        };
        self.set_run(t.id, r);
        true
    }

    /// Back to the configuration, stopped.
    pub fn reset(&mut self, t: &Timer) -> bool {
        self.runs.remove(&t.id).is_some()
    }

    /// Reset, then start.
    pub fn restart(&mut self, t: &Timer, c: Clock) -> bool {
        self.reset(t);
        self.start(t, c)
    }

    /// Sets the current value (the timer keeps its state).
    pub fn set(&mut self, t: &Timer, seconds: f64, c: Clock) -> bool {
        let seconds = bounded(seconds);
        let mut r = self.run(t.id);
        r.run = match r.run {
            Run::Running { .. } => Run::Running {
                base: seconds,
                since: c.now,
            },
            Run::Paused { .. } => Run::Paused { value: seconds },
            Run::Stopped { .. } => Run::Stopped {
                value: Some(seconds),
            },
        };
        self.set_run(t.id, r);
        true
    }

    /// Counts to a time of day (`sod`, 24-hour): a countdown-to-time timer takes it as its
    /// target; any other timer is set to the seconds until it.
    pub fn set_time_of_day(&mut self, t: &Timer, sod: f64, c: Clock) -> bool {
        let sod = sod.rem_euclid(DAY);
        let left = until(sod, c.wall);
        if !matches!(t.kind, TimerKind::CountdownToTime { .. }) {
            return self.set(t, left, c);
        }
        let mut r = self.run(t.id);
        r.target = Some(sod);
        r.run = match r.run {
            Run::Running { .. } => Run::Running {
                base: left,
                since: c.now,
            },
            Run::Paused { .. } => Run::Paused { value: left },
            Run::Stopped { .. } => Run::Stopped { value: None },
        };
        self.set_run(t.id, r);
        true
    }

    /// Adds `delta` seconds to the current value.
    pub fn adjust(&mut self, t: &Timer, delta: f64, c: Clock) -> bool {
        let reading = self.reading(t, c);
        let mut r = self.run(t.id);
        // Rebased on the value shown now, so an expired countdown given more time runs again.
        let raw = match &r.run {
            Run::Running { base, since } => {
                let el = c.now.saturating_duration_since(*since).as_secs_f64();
                if reading.countdown {
                    base - el
                } else {
                    base + el
                }
            }
            _ => reading.seconds,
        };
        let now_value = if reading.state == TimerRunState::Expired {
            reading.seconds
        } else {
            raw
        };
        r.run = match r.run {
            Run::Running { .. } => Run::Running {
                base: bounded(now_value + delta),
                since: c.now,
            },
            Run::Paused { value } => Run::Paused {
                value: bounded(value + delta),
            },
            // A stopped countdown-to-time timer keeps following the wall clock: its target
            // moves (unless that would put it at or past now: then it holds the value).
            Run::Stopped { value: None }
                if matches!(t.kind, TimerKind::CountdownToTime { .. })
                    && reading.seconds + delta > 0.0
                    && reading.seconds + delta < DAY =>
            {
                r.target = Some((c.wall + reading.seconds + delta).rem_euclid(DAY));
                Run::Stopped { value: None }
            }
            Run::Stopped { .. } => Run::Stopped {
                value: Some(bounded(reading.seconds + delta)),
            },
        };
        self.set_run(t.id, r);
        true
    }

    /// Forgets timers that no longer exist.
    pub fn retain(&mut self, timers: &[Timer]) {
        self.runs.retain(|id, _| timers.iter().any(|t| t.id == *id));
        if let Some(id) = self.last_started
            && !timers.iter().any(|t| t.id == id)
        {
            self.last_started = None;
        }
    }

    /// Any timer counting.
    pub fn any_running(&self) -> bool {
        self.runs
            .values()
            .any(|r| matches!(r.run, Run::Running { .. }))
    }

    /// Whether `t`'s shown value changes by itself: it runs, or it is a stopped
    /// countdown-to-time timer (it shows the time left from now).
    pub fn moves(&self, t: &Timer) -> bool {
        match self.run(t.id).run {
            Run::Running { .. } => true,
            Run::Stopped { value: None } => {
                matches!(t.kind, TimerKind::CountdownToTime { .. })
            }
            _ => false,
        }
    }

    /// When the shown value or the state of a timer next changes (`None` = no timer's value
    /// moves, see [`moves`](Self::moves)). Whole seconds; a timer showing tenths still changes
    /// its screens once a second.
    pub fn next_change(&self, timers: &[Timer], c: Clock) -> Option<Instant> {
        let mut best: Option<Instant> = None;
        for t in timers {
            if !self.moves(t) {
                continue;
            }
            let r = self.reading(t, c);
            if r.state == TimerRunState::Expired || !r.seconds.is_finite() {
                continue;
            }
            let v = r.seconds;
            let f = if r.countdown {
                v - v.floor()
            } else {
                v.ceil() - v
            };
            let dt = if (1e-6..=1.0).contains(&f) { f } else { 1.0 };
            let at = c.now + Duration::from_secs_f64(dt);
            best = Some(best.map_or(at, |b| b.min(at)));
        }
        best
    }

    /// Values of every timer for the renderer.
    pub fn values(&self, timers: &[Timer], c: Clock) -> HashMap<TimerId, render::live::TimerValue> {
        timers
            .iter()
            .map(|t| {
                let mut v = self.reading(t, c).value();
                v.warning = t.warning_seconds;
                v.total = match t.kind {
                    TimerKind::Countdown { seconds } => Some(seconds),
                    TimerKind::Elapsed { end, .. } => end,
                    _ => None,
                };
                (t.id, v)
            })
            .collect()
    }

    /// The protocol view of every timer.
    pub fn states(&self, timers: &[Timer], c: Clock) -> Vec<protocol::TimerState> {
        timers
            .iter()
            .map(|t| {
                let r = self.reading(t, c);
                protocol::TimerState {
                    id: t.id,
                    name: t.name.clone(),
                    kind: t.kind,
                    allow_overrun: t.allow_overrun,
                    state: r.state,
                    current_seconds: (r.seconds * 1000.0).round() / 1000.0,
                    display: r.display(),
                    warning_seconds: t.warning_seconds,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn countdown(seconds: f64, overrun: bool) -> Timer {
        Timer {
            kind: TimerKind::Countdown { seconds },
            allow_overrun: overrun,
            ..Timer::default()
        }
    }

    fn clock() -> Clock {
        Clock {
            now: Instant::now(),
            wall: 10.0 * 3600.0,
        }
    }

    #[test]
    fn countdown_runs_and_expires() {
        let t = countdown(300.0, false);
        let mut ts = Timers::default();
        let c0 = clock();
        let r = ts.reading(&t, c0);
        assert_eq!((r.seconds, r.state), (300.0, TimerRunState::Stopped));
        assert_eq!(r.display(), "05:00");
        assert!(ts.start(&t, c0));
        assert_eq!(ts.last_started, Some(t.id));
        let r = ts.reading(&t, c0.after(0.25));
        assert_eq!(r.state, TimerRunState::Running);
        assert!((r.seconds - 299.75).abs() < 1e-9);
        assert_eq!(r.display(), "05:00"); // rounds up
        assert_eq!(ts.reading(&t, c0.after(1.0)).display(), "04:59");
        let r = ts.reading(&t, c0.after(300.0));
        assert_eq!((r.seconds, r.state), (0.0, TimerRunState::Expired));
        let r = ts.reading(&t, c0.after(400.0));
        assert_eq!((r.seconds, r.state), (0.0, TimerRunState::Expired));
        assert_eq!(r.display(), "00:00");
        assert!(r.value().expired && !r.value().running);
        // Starting an expired timer starts it over.
        let c1 = c0.after(400.0);
        assert!(ts.start(&t, c1));
        assert_eq!(ts.reading(&t, c1.after(10.0)).seconds, 290.0);
    }

    #[test]
    fn countdown_overruns_when_allowed() {
        let t = countdown(10.0, true);
        let mut ts = Timers::default();
        let c0 = clock();
        ts.start(&t, c0);
        let r = ts.reading(&t, c0.after(15.5));
        assert_eq!(r.state, TimerRunState::Overrun);
        assert!((r.seconds + 5.5).abs() < 1e-9);
        assert_eq!(r.display(), "-00:05");
        assert!(r.value().expired && r.value().running);
        // Pausing an overrun timer keeps the negative value.
        ts.stop(&t, c0.after(15.5));
        let r = ts.reading(&t, c0.after(99.0));
        assert_eq!(r.state, TimerRunState::Paused);
        assert!((r.seconds + 5.5).abs() < 1e-9);
    }

    #[test]
    fn pause_and_resume_stay_accurate_over_a_minute() {
        // 60 s simulated in 100 ms steps, paused for 10 s in the middle: the countdown must
        // have lost exactly the 50 s it ran.
        let t = countdown(120.0, false);
        let mut ts = Timers::default();
        let c0 = clock();
        ts.start(&t, c0);
        let mut c = c0;
        for step in 1..=600 {
            c = c0.after(step as f64 * 0.1);
            if step == 200 {
                assert!(ts.stop(&t, c));
            }
            if step == 300 {
                assert!(ts.start(&t, c));
            }
            let _ = ts.reading(&t, c);
        }
        let r = ts.reading(&t, c);
        assert_eq!(r.state, TimerRunState::Running);
        assert!((r.seconds - 70.0).abs() < 1e-6, "{}", r.seconds);
        // A second start does nothing; stop twice neither.
        assert!(!ts.start(&t, c));
        assert!(ts.stop(&t, c));
        assert!(!ts.stop(&t, c));
        assert!(ts.reset(&t));
        assert_eq!(ts.reading(&t, c).seconds, 120.0);
    }

    #[test]
    fn set_and_adjust() {
        let t = countdown(300.0, false);
        let mut ts = Timers::default();
        let c0 = clock();
        ts.set(&t, 90.0, c0);
        let r = ts.reading(&t, c0);
        assert_eq!((r.seconds, r.state), (90.0, TimerRunState::Stopped));
        ts.adjust(&t, 30.0, c0);
        assert_eq!(ts.reading(&t, c0).seconds, 120.0);
        ts.start(&t, c0);
        ts.adjust(&t, -20.0, c0.after(10.0));
        assert!((ts.reading(&t, c0.after(10.0)).seconds - 90.0).abs() < 1e-9);
        // An expired countdown given more time runs again.
        let c1 = c0.after(500.0);
        assert_eq!(ts.reading(&t, c1).state, TimerRunState::Expired);
        ts.adjust(&t, 60.0, c1);
        let r = ts.reading(&t, c1.after(1.0));
        assert_eq!(r.state, TimerRunState::Running);
        assert!((r.seconds - 59.0).abs() < 1e-9);
        // Reset forgets set values.
        ts.reset(&t);
        assert_eq!(ts.reading(&t, c1).seconds, 300.0);
    }

    #[test]
    fn countdown_to_time_across_midnight_and_periods() {
        // 12:05 PM stored 24-hour with its period.
        assert_eq!(target_of(43_500.0, TimePeriod::Pm), 43_500.0);
        // 7:30 entered as PM in 12-hour form.
        assert_eq!(target_of(7.5 * 3600.0, TimePeriod::Pm), 19.5 * 3600.0);
        // 12:15 AM entered as 12-hour (12:15) → 00:15.
        assert_eq!(target_of(12.25 * 3600.0, TimePeriod::Am), 0.25 * 3600.0);
        assert_eq!(target_of(9.0 * 3600.0, TimePeriod::H24), 9.0 * 3600.0);

        let t = Timer {
            kind: TimerKind::CountdownToTime {
                seconds_after_midnight: 0.5 * 3600.0, // 00:30
                period: TimePeriod::H24,
            },
            ..Timer::default()
        };
        let mut ts = Timers::default();
        // 23:50: 40 minutes to go, across midnight.
        let c0 = Clock {
            now: Instant::now(),
            wall: 23.0 * 3600.0 + 50.0 * 60.0,
        };
        let r = ts.reading(&t, c0);
        assert_eq!((r.seconds, r.state), (2400.0, TimerRunState::Stopped));
        // Stopped: follows the wall clock.
        assert_eq!(ts.reading(&t, c0.after(60.0)).seconds, 2340.0);
        ts.start(&t, c0);
        let r = ts.reading(&t, c0.after(2399.5));
        assert_eq!(r.state, TimerRunState::Running);
        assert_eq!(r.display(), "00:01");
        assert_eq!(
            ts.reading(&t, c0.after(2400.0)).state,
            TimerRunState::Expired
        );
        // Already passed today (00:40): counts to tomorrow's 00:30.
        let late = Clock {
            now: Instant::now(),
            wall: 0.0 + 40.0 * 60.0,
        };
        let r = Timers::default().reading(&t, late);
        assert_eq!(r.seconds, DAY - 600.0);
        // A time of day set at run time.
        let mut ts = Timers::default();
        ts.set_time_of_day(&t, 1.0 * 3600.0, late);
        assert_eq!(ts.reading(&t, late).seconds, 20.0 * 60.0);
        // On a plain countdown it sets the seconds until then.
        let cd = countdown(10.0, false);
        ts.set_time_of_day(&cd, 1.0 * 3600.0, late);
        assert_eq!(ts.reading(&cd, late).seconds, 1200.0);
    }

    #[test]
    fn elapsed_counts_up_to_its_end() {
        let t = Timer {
            kind: TimerKind::Elapsed {
                start: Some(10.0),
                end: Some(70.0),
            },
            ..Timer::default()
        };
        let mut ts = Timers::default();
        let c0 = clock();
        assert_eq!(ts.reading(&t, c0).seconds, 10.0);
        ts.start(&t, c0);
        let r = ts.reading(&t, c0.after(30.7));
        assert!((r.seconds - 40.7).abs() < 1e-9);
        assert_eq!(r.display(), "00:40"); // rounds down
        let r = ts.reading(&t, c0.after(61.0));
        assert_eq!((r.seconds, r.state), (70.0, TimerRunState::Expired));
        let open = Timer {
            kind: TimerKind::Elapsed {
                start: None,
                end: None,
            },
            ..Timer::default()
        };
        ts.start(&open, c0);
        assert_eq!(ts.reading(&open, c0.after(5000.0)).display(), "1:23:20");
    }

    #[test]
    fn next_change_is_the_next_shown_second() {
        let t = countdown(300.0, false);
        let mut ts = Timers::default();
        let c0 = clock();
        assert!(ts.next_change(std::slice::from_ref(&t), c0).is_none());
        ts.start(&t, c0);
        let c = c0.after(0.25);
        let next = ts.next_change(std::slice::from_ref(&t), c).unwrap();
        let dt = next.duration_since(c.now).as_secs_f64();
        assert!((dt - 0.75).abs() < 1e-3, "{dt}");
        assert!(!ts.any_running() || ts.any_running());
        ts.retain(&[]);
        assert!(ts.last_started.is_none() && !ts.any_running());
    }

    fn to_time(sod: f64) -> Timer {
        Timer {
            kind: TimerKind::CountdownToTime {
                seconds_after_midnight: sod,
                period: TimePeriod::H24,
            },
            ..Timer::default()
        }
    }

    #[test]
    fn huge_values_stay_bounded_and_next_change_never_panics() {
        // A01-3 / R4-1: set 1e308 then adjust 1e308 used to store +inf, and next_change
        // panicked on the render thread (Duration from NaN).
        for overrun in [false, true] {
            let t = countdown(300.0, overrun);
            let mut ts = Timers::default();
            let c0 = clock();
            ts.set(&t, 1e308, c0);
            ts.adjust(&t, 1e308, c0);
            assert_eq!(ts.reading(&t, c0).seconds, MAX_VALUE);
            ts.start(&t, c0);
            ts.adjust(&t, f64::MAX, c0);
            let r = ts.reading(&t, c0.after(0.5));
            assert!(
                r.seconds.is_finite() && r.seconds <= MAX_VALUE,
                "{}",
                r.seconds
            );
            assert!(
                ts.next_change(std::slice::from_ref(&t), c0.after(0.5))
                    .is_some()
            );
            ts.adjust(&t, -1e308, c0.after(1.0));
            ts.adjust(&t, -1e308, c0.after(1.0));
            let r = ts.reading(&t, c0.after(1.5));
            assert!(r.seconds.is_finite() && r.seconds >= -MAX_VALUE - 1.0);
            let _ = ts.next_change(std::slice::from_ref(&t), c0.after(1.5));
            ts.stop(&t, c0.after(2.0));
            ts.adjust(&t, -1e308, c0.after(2.0));
            assert_eq!(ts.reading(&t, c0.after(2.0)).seconds, -MAX_VALUE);
        }
    }

    #[test]
    fn adjusting_a_stopped_countdown_to_time_moves_its_target() {
        // R4-3: target 10:30; at 10:00 (1800 s left) +60 pushes it to 10:31; started at 10:15
        // it counts from 16:00, not from a frozen 31:00.
        let t = to_time(10.5 * 3600.0);
        let mut ts = Timers::default();
        let c0 = clock(); // 10:00
        assert_eq!(ts.reading(&t, c0).seconds, 1800.0);
        ts.adjust(&t, 60.0, c0);
        assert_eq!(ts.reading(&t, c0).seconds, 1860.0);
        let c1 = c0.after(15.0 * 60.0);
        let r = ts.reading(&t, c1);
        assert_eq!((r.seconds, r.state), (960.0, TimerRunState::Stopped));
        ts.start(&t, c1);
        assert_eq!(ts.reading(&t, c1).seconds, 960.0);
        assert_eq!(
            ts.reading(&t, c1.after(960.0)).state,
            TimerRunState::Expired
        );
    }

    #[test]
    fn a_stopped_countdown_to_time_changes_every_second() {
        // A01-6: it shows the time left from now, so screens re-render as the clock moves.
        let t = to_time(10.5 * 3600.0);
        let ts = Timers::default();
        let c = clock().after(0.25);
        assert!(ts.moves(&t));
        let next = ts.next_change(std::slice::from_ref(&t), c).unwrap();
        let dt = next.duration_since(c.now).as_secs_f64();
        assert!((dt - 0.75).abs() < 1e-3, "{dt}");
        // A set value holds still; a stopped plain countdown neither moves.
        let mut held = Timers::default();
        held.set(&t, 100.0, c);
        assert!(!held.moves(&t));
        assert!(held.next_change(std::slice::from_ref(&t), c).is_none());
        assert!(!ts.moves(&countdown(10.0, false)));
    }
}
