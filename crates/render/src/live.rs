//! Live data in slides: data-link text, visibility conditions and timer colour triggers,
//! resolved against a [`DataContext`] for one frame.
//!
//! The renderer itself draws slides as they are; a caller that shows **live** content (the
//! server's screens) passes every slide through [`resolve_slide`] with the frame's context
//! first. Thumbnails and the editor skip this step, so they show the design-time text (the
//! element's own paragraphs) and every element.
//!
//! ## What is resolved
//! - **Data links** of text elements: the element keeps its text box styling (font, size,
//!   colour, alignment, fit); its paragraphs are replaced by the live text, one paragraph per
//!   line, each in the style of the design-time paragraph at that position (the first run's
//!   character style; lines past the end take the last paragraph's). Implemented kinds:
//!   timer ([`TimerFormat`]), clock ([`ClockFormat`]), current / next slide text (all text, a
//!   slot, a named element; `keepFormatting` is rendered as plain text), slide notes, slide
//!   label, section name (current / next slide, next section), slide count (current /
//!   remaining / total), playlist item names (current / next, the headers, the playlist), the
//!   stage message ([`DataContext::message`]; a link with `flash` fills its element with the
//!   flash colour during the "on" phases of a flashing message), video and audio countdowns,
//!   the live presentation's name (or the announcements layer's) and notes, the CCLI
//!   copyright line ([`DataContext::ccli`]). A timer link's `show` picks the timer's value,
//!   the time elapsed or the time remaining ([`TimerValue::shown`]).
//!   **Song artwork** (a picture link): the element's fill becomes the artwork image (fitted);
//!   without artwork its fill is off.
//!   **Not here**: the slide image link is drawn by the caller (the server renders the slide
//!   into the element; the element's own text stays as authored here). RSS and other
//!   unsupported imported kinds (authored text) are not rendered.
//! - **Visibility conditions** (all must hold): timer `hasTimeRemaining` / `hasExpired` /
//!   `isRunning` / `notRunning`, `elementHasText` (on the resolved text of the other element),
//!   video input live, a message on the messages layer (any or one), a stage message, a show
//!   layer with content, the playlist's current / next item or header exists, the
//!   presentation video's or the audio layer's state (`hasTimeRemaining`, `hasExpired`,
//!   `isRunning`, `notRunning`, `looping`, `notLooping`; nothing live = expired, not running,
//!   not looping). Unsupported and unknown conditions hold.
//! - **Colour triggers** of timer links: the text takes the colour of the trigger with the
//!   smallest `remaining` that is ≥ the timer's value (e.g. red from 60 s down).
//!
//! A timer link or condition without a timer means the **most recently started** timer
//! ([`DataContext::last_started`]); with none started the link renders empty and the timer
//! counts as stopped with time remaining.
//!
//! ## Formats
//! [`format_timer`]: units are hours / minutes / seconds / milliseconds, each `hidden`, `short`,
//! `long`, `shortOmitZero` or `longOmitZero`. A hidden unit's amount carries into the next
//! smaller shown unit (hours hidden: `754:56`). The *leading* shown unit is padded to two
//! digits only when `long`; the units after it are always two digits. `…OmitZero` units are
//! left out while they are zero and leading; the smallest shown unit of hours / minutes /
//! seconds is always shown. Milliseconds: `short` = tenths, `long` = hundredths (after a
//! `.`). Countdowns round **up** to the shown precision (a countdown shows `0:00` only once it
//! has expired), elapsed timers round down. Negative values get a leading `-`.
//!
//! [`format_clock`]: en-US style. Time `short` = `9:30 AM` (24-hour: `09:30`), `medium` and
//! longer add seconds (`9:30:15 AM`); date `short` = `10/19/26`, `medium` = `Oct 19, 2026`,
//! `long` = `October 19, 2026`, `full` = `Monday, October 19, 2026`; the date comes first.
//! 12-hour unless `use24Hour` is set. Time zones and other locales are not rendered.
//!
//! Templates (`"We begin in ${timer}"`, `"${clock}"`) wrap the value.

use model::{
    ChordView, ClockFormat, CountdownSource, DataLink, DateTimeStyle, ElementId, Layer, Link,
    MediaAssetId, MediaCondition, MessageId, Paragraph, PlaylistItemWhich, SectionWhich, Slide,
    SlideCountKind, SlideTextSource, SlideWhich, TextBox, TextRun, TimeUnitStyle, TimerFormat,
    TimerId, TimerShow, TimerState, VideoInputId, VisibilityCondition,
};
use std::borrow::Cow;
use std::collections::HashMap;

/// Local wall-clock time (what clocks show).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WallTime {
    /// Year.
    pub year: i32,
    /// Month, 1–12.
    pub month: u32,
    /// Day of the month, 1–31.
    pub day: u32,
    /// Day of the week, 0 = Monday.
    pub weekday: u32,
    /// Hour, 0–23.
    pub hour: u32,
    /// Minute.
    pub minute: u32,
    /// Second.
    pub second: u32,
}

/// A timer's value at the frame's instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimerValue {
    /// Seconds (negative = overrun).
    pub seconds: f64,
    /// Counts down (rounds up for display).
    pub countdown: bool,
    /// Counting (running or overrun).
    pub running: bool,
    /// Reached its end (expired, or overrun).
    pub expired: bool,
    /// The timer's warning threshold, seconds left (`None` = none).
    pub warning: Option<f64>,
    /// A countdown's duration, a count-up timer's end (`None` = none: a countdown to a time,
    /// a count-up timer without an end). Elapsed / remaining links derive from it.
    pub total: Option<f64>,
}

impl TimerValue {
    /// A stopped timer with time remaining (what an unknown timer counts as).
    pub const IDLE: TimerValue = TimerValue {
        seconds: 0.0,
        countdown: true,
        running: false,
        expired: false,
        warning: None,
        total: None,
    };

    /// The time a timer link with `show` displays, and whether it counts down (for rounding);
    /// `None` = nothing to show (see [`TimerShow`]).
    pub fn shown(&self, show: TimerShow) -> Option<(f64, bool)> {
        match (show, self.countdown) {
            (TimerShow::Elapsed, true) => self.total.map(|t| ((t - self.seconds).max(0.0), false)),
            (TimerShow::Elapsed, false) => Some((self.seconds, false)),
            (TimerShow::Remaining, false) => {
                self.total.map(|t| ((t - self.seconds).max(0.0), true))
            }
            _ => Some((self.seconds, self.countdown)),
        }
    }

    /// The value a link with `show` colours by (its colour triggers and the warning count the
    /// time remaining): a count-up timer shown as its time remaining counts as a countdown of
    /// that time; anything else as the timer's own value.
    pub fn coloured_as(&self, show: TimerShow) -> TimerValue {
        match (show, self.countdown, self.total) {
            (TimerShow::Remaining, false, Some(t)) => TimerValue {
                seconds: (t - self.seconds).max(0.0),
                countdown: true,
                ..*self
            },
            _ => *self,
        }
    }

    /// A countdown with time left at or below its warning threshold.
    pub fn warns(&self) -> bool {
        self.countdown
            && !self.expired
            && self.seconds > 0.0
            && self.warning.is_some_and(|w| self.seconds <= w)
    }
}

/// The box-wide clock format (`setClockFormat`): applies where a clock's own format leaves it
/// open (24-hour time when `use24Hour` is unset; seconds on a short time).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoxClock {
    /// 24-hour time.
    pub use_24_hour: bool,
    /// Show seconds.
    pub seconds: bool,
}

impl BoxClock {
    /// `format` with the box's choices filled in.
    pub fn apply(&self, format: &ClockFormat) -> ClockFormat {
        let mut f = format.clone();
        if f.use_24_hour.is_none() {
            f.use_24_hour = Some(self.use_24_hour);
        }
        if self.seconds && f.time == DateTimeStyle::Short {
            f.time = DateTimeStyle::Medium;
        }
        f
    }
}

/// The colour of a timer that warns ([`TimerValue::warns`]) when its link has no colour
/// triggers of its own: amber.
pub const WARNING_COLOR: model::Color = model::Color {
    r: 0.96,
    g: 0.65,
    b: 0.14,
    a: 1.0,
};

/// The texts of one slide that data links read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlideInfo {
    /// All text: the text-slot elements, or every text element without a data link when there
    /// are none, joined by line breaks.
    pub text: String,
    /// Text per placeholder slot name.
    pub slots: Vec<(String, String)>,
    /// Text per element name.
    pub named: Vec<(String, String)>,
    /// Plain notes.
    pub notes: String,
    /// Label.
    pub label: String,
    /// Section (group) name.
    pub section: String,
    /// The lyric lines of `text` (same elements) with their chords, unstyled: what chord-chart
    /// links show.
    pub chart: Vec<Paragraph>,
}

impl SlideInfo {
    /// Extracts the texts of `slide` (`section` = its section's name).
    pub fn of(slide: &Slide, section: &str) -> SlideInfo {
        let text_of = |e: &model::Element| e.text_box().map(|t| t.plain_text());
        let slotted: Vec<String> = slide
            .elements
            .iter()
            .filter(|e| e.placeholder.as_ref().is_some_and(|p| p.takes_text()))
            .filter_map(text_of)
            .collect();
        let texts: Vec<String> = if slotted.is_empty() {
            slide
                .elements
                .iter()
                .filter(|e| e.data_link.is_none())
                .filter_map(text_of)
                .collect()
        } else {
            slotted
        };
        let lyric_boxes: Vec<&TextBox> = {
            let slotted: Vec<&TextBox> = slide
                .elements
                .iter()
                .filter(|e| e.placeholder.as_ref().is_some_and(|p| p.takes_text()))
                .filter_map(|e| e.text_box())
                .collect();
            if slotted.is_empty() {
                slide
                    .elements
                    .iter()
                    .filter(|e| e.data_link.is_none())
                    .filter_map(|e| e.text_box())
                    .collect()
            } else {
                slotted
            }
        };
        let chart: Vec<Paragraph> = lyric_boxes
            .into_iter()
            .filter(|t| !t.plain_text().trim().is_empty())
            .flat_map(|t| {
                t.paragraphs.iter().map(|p| {
                    let text = p.plain_text();
                    Paragraph {
                        runs: if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![TextRun::plain(text)]
                        },
                        chords: p.chords.clone(),
                        ..Paragraph::default()
                    }
                })
            })
            .collect();
        SlideInfo {
            text: texts
                .into_iter()
                .filter(|t| !t.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
            slots: slide
                .elements
                .iter()
                .filter_map(|e| Some((e.placeholder.as_ref()?.slot.clone(), text_of(e)?)))
                .collect(),
            named: slide
                .elements
                .iter()
                .filter(|e| !e.name.is_empty())
                .filter_map(|e| Some((e.name.clone(), text_of(e)?)))
                .collect(),
            notes: slide.notes.plain_text(),
            label: slide
                .label
                .as_ref()
                .map(|l| l.text.clone())
                .unwrap_or_default(),
            section: section.to_string(),
            chart,
        }
    }
}

/// Names around the live playlist item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistInfo {
    /// The live item.
    pub current: Option<String>,
    /// The next item.
    pub next: Option<String>,
    /// The header above the live item.
    pub current_header: Option<String>,
    /// The next header.
    pub next_header: Option<String>,
    /// The playlist's name.
    pub playlist: Option<String>,
}

/// A live video or audio play, for the media countdown conditions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MediaStatus {
    /// Seconds left (0 = ended).
    pub remaining: f64,
    /// Playing (not paused, not ended).
    pub running: bool,
    /// Loops.
    pub looping: bool,
}

/// Everything live data may show in one frame.
#[derive(Clone, Debug, Default)]
pub struct DataContext {
    /// Timer values by timer.
    pub timers: HashMap<TimerId, TimerValue>,
    /// The most recently started timer (links without a timer show it).
    pub last_started: Option<TimerId>,
    /// Local time (`None` = clocks render empty).
    pub wall: Option<WallTime>,
    /// The live slide.
    pub current: Option<SlideInfo>,
    /// The slide `next` would trigger.
    pub next: Option<SlideInfo>,
    /// The name of the next section of the live presentation.
    pub next_section: Option<String>,
    /// (0-based index, total) of the live slide in its presentation.
    pub slide_count: Option<(usize, usize)>,
    /// Playlist names.
    pub playlist: PlaylistInfo,
    /// The stage message (`None` = none): what stage-message links show.
    pub message: Option<String>,
    /// The stage message is in an "on" phase of its flash: stage-message links with `flash`
    /// fill their element with the flash colour.
    pub stage_flash: bool,
    /// Seconds left of the presentation video (foreground, else background).
    pub video_remaining: Option<f64>,
    /// Seconds left of the announcements layer's video.
    pub announcement_video_remaining: Option<f64>,
    /// Seconds left of the audio layer.
    pub audio_remaining: Option<f64>,
    /// Video inputs on the video input layer.
    pub video_inputs: Vec<VideoInputId>,
    /// The box-wide clock format.
    pub clock: BoxClock,
    /// The live presentation's name (slide layer).
    pub presentation_name: Option<String>,
    /// The announcements layer's presentation name.
    pub announcement_name: Option<String>,
    /// The live presentation's notes (plain text).
    pub presentation_notes: Option<String>,
    /// The live song's copyright line (CCLI text links).
    pub ccli: Option<String>,
    /// The live song's artwork (artwork links).
    pub artwork: Option<MediaAssetId>,
    /// The live song's written key (`SongInfo.originalKey`), for chord charts and key lines.
    pub song_key: Option<String>,
    /// The live song's transposition (`SongInfo.transpose`).
    pub song_transpose: i32,
    /// The chord view of the stage layout being drawn (its transposition, capo, notation).
    pub chord_view: ChordView,
    /// The messages on the messages layer.
    pub messages: Vec<MessageId>,
    /// The show layers with content.
    pub layers: Vec<Layer>,
    /// The presentation video (foreground, else background), for media conditions.
    pub video: Option<MediaStatus>,
    /// The audio layer's play, for media conditions.
    pub audio: Option<MediaStatus>,
}

impl DataContext {
    /// The value of `timer` (`None` = the most recently started one). `None` when there is
    /// no such timer.
    pub fn timer(&self, timer: Option<&Link<TimerId>>) -> Option<TimerValue> {
        let id = timer.map(|l| l.id).or(self.last_started)?;
        self.timers.get(&id).copied()
    }
}

/// What a resolved slide depended on (so a caller knows when to refresh it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveUse {
    /// Shows the wall clock.
    pub clock: bool,
    /// Shows (or is shown depending on) a timer.
    pub timers: bool,
    /// Shows a media countdown.
    pub media: bool,
    /// Shows the stage message with `flash` (re-render while it flashes).
    pub flash: bool,
}

impl LiveUse {
    /// Combines two uses.
    pub fn merge(&mut self, o: LiveUse) {
        self.clock |= o.clock;
        self.timers |= o.timers;
        self.media |= o.media;
        self.flash |= o.flash;
    }

    /// Anything time-dependent.
    pub fn any(&self) -> bool {
        self.clock || self.timers || self.media || self.flash
    }
}

// ---- formats ------------------------------------------------------------------------------

/// Formats a timer value (see the module docs).
pub fn format_timer(seconds: f64, countdown: bool, format: &TimerFormat) -> String {
    let value = format_duration(seconds, countdown, format);
    match &format.template {
        Some(t) if t.contains("${timer}") => t.replace("${timer}", &value),
        _ => value,
    }
}

fn unit(u: TimeUnitStyle) -> TimeUnitStyle {
    match u {
        TimeUnitStyle::Unknown => TimeUnitStyle::Long,
        other => other,
    }
}

/// The duration alone (no template).
pub fn format_duration(seconds: f64, countdown: bool, format: &TimerFormat) -> String {
    let (hu, mu, su, msu) = (
        unit(format.hours),
        unit(format.minutes),
        unit(format.seconds),
        unit(format.milliseconds),
    );
    let per_sec: i64 = match msu {
        TimeUnitStyle::Hidden => 1,
        TimeUnitStyle::Short | TimeUnitStyle::ShortOmitZero => 10,
        _ => 100,
    };
    let seconds = if seconds.is_finite() { seconds } else { 0.0 };
    let scaled = seconds * per_sec as f64;
    // Rounded to the shown precision: countdowns up, elapsed timers down (with a tolerance
    // for float noise at exact boundaries).
    let ticks = if countdown {
        (scaled - 1e-6).ceil()
    } else {
        (scaled + 1e-6).floor()
    } as i64;
    let negative = ticks < 0;
    let t = ticks.unsigned_abs() as i64;
    let frac = t % per_sec;
    let total = t / per_sec;
    let shown = |u: TimeUnitStyle| u != TimeUnitStyle::Hidden;
    let h = if shown(hu) { total / 3600 } else { 0 };
    let rest = total - h * 3600;
    let m = if shown(mu) { rest / 60 } else { 0 };
    let s = rest - m * 60;
    // Units in order with their values; the smallest shown of h/m/s is always kept.
    let units = [(hu, h), (mu, m), (su, s)];
    let last_shown = units.iter().rposition(|(u, _)| shown(*u));
    let mut parts: Vec<String> = Vec::new();
    for (i, (u, v)) in units.iter().enumerate() {
        if !shown(*u) {
            continue;
        }
        let leading = parts.is_empty();
        let omit_zero = matches!(
            u,
            TimeUnitStyle::ShortOmitZero | TimeUnitStyle::LongOmitZero
        );
        if leading && omit_zero && *v == 0 && Some(i) != last_shown {
            continue;
        }
        let long = matches!(u, TimeUnitStyle::Long | TimeUnitStyle::LongOmitZero);
        parts.push(if leading && !long {
            v.to_string()
        } else {
            format!("{v:02}")
        });
    }
    let mut out = parts.join(":");
    if per_sec > 1 {
        let omit = matches!(
            msu,
            TimeUnitStyle::ShortOmitZero | TimeUnitStyle::LongOmitZero
        ) && frac == 0;
        if !omit {
            if per_sec == 10 {
                out.push_str(&format!(".{frac}"));
            } else {
                out.push_str(&format!(".{frac:02}"));
            }
        }
    }
    if negative && (t > 0) {
        out.insert(0, '-');
    }
    out
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Formats the wall clock (see the module docs).
pub fn format_clock(w: &WallTime, format: &ClockFormat) -> String {
    let h24 = format.use_24_hour == Some(true);
    let time = match format.time {
        DateTimeStyle::None => None,
        style => {
            let secs = !matches!(style, DateTimeStyle::Short);
            let (h, suffix) = if h24 {
                (format!("{:02}", w.hour), "")
            } else {
                let h12 = match w.hour % 12 {
                    0 => 12,
                    h => h,
                };
                (h12.to_string(), if w.hour < 12 { " AM" } else { " PM" })
            };
            Some(if secs {
                format!("{h}:{:02}:{:02}{suffix}", w.minute, w.second)
            } else {
                format!("{h}:{:02}{suffix}", w.minute)
            })
        }
    };
    let month = MONTHS[(w.month.clamp(1, 12) - 1) as usize];
    let date = match format.date {
        DateTimeStyle::None => None,
        DateTimeStyle::Medium => Some(format!("{} {}, {}", &month[..3], w.day, w.year)),
        DateTimeStyle::Long => Some(format!("{month} {}, {}", w.day, w.year)),
        DateTimeStyle::Full => Some(format!(
            "{}, {month} {}, {}",
            WEEKDAYS[(w.weekday % 7) as usize],
            w.day,
            w.year
        )),
        _ => Some(format!(
            "{}/{}/{:02}",
            w.month,
            w.day,
            w.year.rem_euclid(100)
        )),
    };
    let value = match (date, time) {
        (Some(d), Some(t)) => format!("{d} {t}"),
        (Some(d), None) => d,
        (None, Some(t)) => t,
        (None, None) => String::new(),
    };
    match &format.template {
        Some(t) if t.contains("${clock}") => t.replace("${clock}", &value),
        _ => value,
    }
}

// ---- resolution ---------------------------------------------------------------------------

fn slide_text(info: Option<&SlideInfo>, source: &SlideTextSource) -> String {
    let Some(i) = info else {
        return String::new();
    };
    let find = |list: &[(String, String)], key: &str| {
        list.iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    match source {
        SlideTextSource::Slot { slot } => find(&i.slots, slot),
        SlideTextSource::ElementNamed { name } => find(&i.named, name),
        _ => i.text.clone(),
    }
}

/// The live text of a data link (`None` = keep the element's own text), and the timer value
/// it shows (for colour triggers).
pub fn link_text(
    ctx: &DataContext,
    link: &DataLink,
    used: &mut LiveUse,
) -> Option<(String, Option<TimerValue>)> {
    let which = |w: SlideWhich| match w {
        SlideWhich::Next => ctx.next.as_ref(),
        _ => ctx.current.as_ref(),
    };
    let media = |remaining: Option<f64>, format: &TimerFormat, used: &mut LiveUse| {
        used.media = true;
        remaining
            .map(|r| format_timer(r.max(0.0), true, format))
            .unwrap_or_default()
    };
    Some(match link {
        DataLink::Timer {
            timer,
            format,
            show,
            ..
        } => {
            used.timers = true;
            match ctx.timer(timer.as_ref()) {
                Some(v) => (
                    v.shown(*show)
                        .map(|(t, down)| format_timer(t, down, format))
                        .unwrap_or_default(),
                    Some(v),
                ),
                None => (String::new(), None),
            }
        }
        DataLink::Clock { format } => {
            used.clock = true;
            (
                ctx.wall
                    .as_ref()
                    .map(|w| format_clock(w, &ctx.clock.apply(format)))
                    .unwrap_or_default(),
                None,
            )
        }
        DataLink::SlideText {
            which: w, source, ..
        } => (slide_text(which(*w), source), None),
        DataLink::SlideNotes { which: w } => {
            (which(*w).map(|i| i.notes.clone()).unwrap_or_default(), None)
        }
        DataLink::SlideLabel { which: w } => {
            (which(*w).map(|i| i.label.clone()).unwrap_or_default(), None)
        }
        DataLink::SectionName { which: w } => (
            match w {
                SectionWhich::NextSlide => ctx.next.as_ref().map(|i| i.section.clone()),
                SectionWhich::NextSection => ctx.next_section.clone(),
                _ => ctx.current.as_ref().map(|i| i.section.clone()),
            }
            .unwrap_or_default(),
            None,
        ),
        DataLink::SlideCount { count } => (
            match (ctx.slide_count, count) {
                (None, _) => String::new(),
                (Some((_, total)), SlideCountKind::Total) => total.to_string(),
                (Some((i, total)), SlideCountKind::Remaining) => {
                    total.saturating_sub(i + 1).to_string()
                }
                (Some((i, _)), _) => (i + 1).to_string(),
            },
            None,
        ),
        DataLink::PlaylistItemName { which: w } => {
            let p = &ctx.playlist;
            (
                match w {
                    PlaylistItemWhich::Next => p.next.clone(),
                    PlaylistItemWhich::CurrentHeader => p.current_header.clone(),
                    PlaylistItemWhich::NextHeader => p.next_header.clone(),
                    PlaylistItemWhich::Playlist => p.playlist.clone(),
                    _ => p.current.clone(),
                }
                .unwrap_or_default(),
                None,
            )
        }
        DataLink::Message { flash, .. } => {
            used.flash |= *flash;
            (ctx.message.clone().unwrap_or_default(), None)
        }
        DataLink::VideoCountdown { format, source, .. } => {
            let r = match source {
                CountdownSource::Announcements => ctx.announcement_video_remaining,
                _ => ctx.video_remaining,
            };
            (media(r, format, used), None)
        }
        DataLink::AudioCountdown { format, .. } => (media(ctx.audio_remaining, format, used), None),
        DataLink::PresentationName { source } => (
            match source {
                CountdownSource::Announcements => ctx.announcement_name.clone(),
                _ => ctx.presentation_name.clone(),
            }
            .unwrap_or_default(),
            None,
        ),
        DataLink::PresentationNotes => (ctx.presentation_notes.clone().unwrap_or_default(), None),
        DataLink::CcliText => (ctx.ccli.clone().unwrap_or_default(), None),
        DataLink::SongKey => (
            ctx.chord_view
                .key_label(ctx.song_key.as_deref(), ctx.song_transpose),
            None,
        ),
        DataLink::ChordChart { which: w } => (
            which(*w)
                .map(|i| {
                    i.chart
                        .iter()
                        .map(Paragraph::plain_text)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default(),
            None,
        ),
        DataLink::SlideImage { .. }
        | DataLink::SongArtwork
        | DataLink::Unsupported { .. }
        | DataLink::Unknown => return None,
    })
}

/// The fill of a flashing stage-message element when its link names no colour.
pub const FLASH_COLOR: model::Color = model::Color {
    r: 0.86,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

/// Replaces a text box's paragraphs with `text` (one paragraph per line), keeping the style of
/// the paragraph at each position (lines past the end take the last one's).
pub fn replace_text(tb: &mut TextBox, text: &str) {
    let old = std::mem::take(&mut tb.paragraphs);
    let last = old.last().cloned().unwrap_or_default();
    tb.paragraphs = text
        .split('\n')
        .enumerate()
        .map(|(i, line)| {
            let base = old.get(i).unwrap_or(&last);
            let style = base
                .runs
                .first()
                .map(|r| r.style.clone())
                .unwrap_or_else(|| base.end_style.clone());
            Paragraph {
                style: base.style.clone(),
                runs: if line.is_empty() {
                    Vec::new()
                } else {
                    vec![TextRun {
                        text: line.to_string(),
                        style,
                    }]
                },
                end_style: base.end_style.clone(),
                chords: Vec::new(),
            }
        })
        .collect();
}

fn condition_holds(
    ctx: &DataContext,
    slide: &Slide,
    c: &VisibilityCondition,
    used: &mut LiveUse,
) -> bool {
    match c {
        VisibilityCondition::Timer { timer, state } => {
            used.timers = true;
            let v = ctx.timer(timer.as_ref()).unwrap_or(TimerValue::IDLE);
            match state {
                TimerState::HasTimeRemaining => !v.expired,
                TimerState::HasExpired => v.expired,
                TimerState::IsRunning => v.running,
                TimerState::NotRunning => !v.running,
                TimerState::Unknown => true,
            }
        }
        VisibilityCondition::ElementHasText { element, has_text } => {
            match element_text(slide, *element) {
                Some(t) => !t.trim().is_empty() == *has_text,
                None => true,
            }
        }
        VisibilityCondition::VideoInput { input, active } => {
            ctx.video_inputs.contains(&input.id) == *active
        }
        VisibilityCondition::Message { message, active } => {
            let on = match message {
                Some(m) => ctx.messages.contains(&m.id),
                None => !ctx.messages.is_empty(),
            };
            on == *active
        }
        VisibilityCondition::StageMessage { active } => ctx.message.is_some() == *active,
        VisibilityCondition::Layer { layer, active } => ctx.layers.contains(layer) == *active,
        VisibilityCondition::PlaylistItem { which, active } => {
            let p = &ctx.playlist;
            let item = match which {
                PlaylistItemWhich::Next => &p.next,
                PlaylistItemWhich::CurrentHeader => &p.current_header,
                PlaylistItemWhich::NextHeader => &p.next_header,
                PlaylistItemWhich::Playlist => &p.playlist,
                _ => &p.current,
            };
            item.is_some() == *active
        }
        VisibilityCondition::VideoCountdown { state } => {
            used.media = true;
            media_holds(ctx.video, *state)
        }
        VisibilityCondition::AudioCountdown { state } => {
            used.media = true;
            media_holds(ctx.audio, *state)
        }
        VisibilityCondition::Unsupported { .. } | VisibilityCondition::Unknown => true,
    }
}

/// A media countdown condition (nothing live: expired, not running, not looping).
fn media_holds(m: Option<MediaStatus>, state: MediaCondition) -> bool {
    let has_time = m.is_some_and(|m| m.remaining > 0.0);
    let running = m.is_some_and(|m| m.running && m.remaining > 0.0);
    let looping = m.is_some_and(|m| m.looping);
    match state {
        MediaCondition::HasTimeRemaining => has_time,
        MediaCondition::HasExpired => !has_time,
        MediaCondition::IsRunning => running,
        MediaCondition::NotRunning => !running,
        MediaCondition::Looping => looping,
        MediaCondition::NotLooping => !looping,
        MediaCondition::Unknown => true,
    }
}

fn element_text(slide: &Slide, id: ElementId) -> Option<String> {
    slide
        .elements
        .iter()
        .find(|e| e.id == id)
        .and_then(|e| e.text_box())
        .map(|t| t.plain_text())
}

/// Resolves a slide's live data for one frame (see the module docs): data-link text, colour
/// triggers, then visibility conditions (elements that fail become hidden). Returns the slide
/// unchanged (borrowed) when it has no live data, and what it depended on.
pub fn resolve_slide<'a>(slide: &'a Slide, ctx: &DataContext) -> (Cow<'a, Slide>, LiveUse) {
    let mut used = LiveUse::default();
    if !slide
        .elements
        .iter()
        .any(|e| e.data_link.is_some() || !e.visibility.is_empty())
    {
        return (Cow::Borrowed(slide), used);
    }
    let mut out = slide.clone();
    for e in out.elements.iter_mut() {
        let Some(link) = &e.data_link else { continue };
        if matches!(link, DataLink::SongArtwork) {
            // A picture link: the artwork fills the element (fitted); none: no fill.
            e.fill = match ctx.artwork {
                Some(asset) => model::Fill::enabled(model::FillKind::Media {
                    asset,
                    display: model::MediaDisplay::default(),
                    playback: Default::default(),
                    audio: Default::default(),
                }),
                None => model::Fill {
                    enabled: false,
                    ..e.fill.clone()
                },
            };
            continue;
        }
        let Some((text, timer)) = link_text(ctx, link, &mut used) else {
            continue;
        };
        let (triggers, show) = match link {
            DataLink::Timer {
                color_triggers,
                show,
                ..
            } => (color_triggers.as_slice(), *show),
            _ => (&[][..], TimerShow::default()),
        };
        let colour = timer.map(|v| v.coloured_as(show)).and_then(|v| {
            if triggers.is_empty() {
                return v.warns().then_some(WARNING_COLOR);
            }
            triggers
                .iter()
                .filter(|t| v.seconds <= t.remaining)
                .min_by(|a, b| a.remaining.total_cmp(&b.remaining))
                .map(|t| t.color)
        });
        if let DataLink::Message {
            flash: true,
            flash_color,
        } = link
            && ctx.stage_flash
            && ctx.message.is_some()
        {
            e.fill = model::Fill::color(flash_color.unwrap_or(FLASH_COLOR));
        }
        if let DataLink::ChordChart { which } = link {
            // The lines with their chords, as this layout's chord view shows them.
            let info = match which {
                SlideWhich::Next => ctx.next.as_ref(),
                _ => ctx.current.as_ref(),
            };
            let view = ctx
                .chord_view
                .context(ctx.song_key.as_deref(), ctx.song_transpose);
            if let Some(tb) = e.text_box_mut() {
                replace_text(tb, &text);
                if let Some(info) = info {
                    for (p, src) in tb.paragraphs.iter_mut().zip(&info.chart) {
                        p.chords = src
                            .chords
                            .iter()
                            .map(|c| model::ChordMark::new(c.at, view.chord(&c.chord)))
                            .collect();
                    }
                }
                tb.chords.show = true;
            }
            continue;
        }
        if let Some(tb) = e.text_box_mut() {
            replace_text(tb, &text);
            if let Some(c) = colour {
                tb.default_style.color = Some(c);
                for p in tb.paragraphs.iter_mut() {
                    p.end_style.color = Some(c);
                    for r in p.runs.iter_mut() {
                        r.style.color = Some(c);
                    }
                }
            }
        }
    }
    // Conditions see the resolved text of other elements.
    let snapshot = out.clone();
    for e in out.elements.iter_mut() {
        if e.hidden || e.visibility.is_empty() {
            continue;
        }
        let visible = e
            .visibility
            .iter()
            .all(|c| condition_holds(ctx, &snapshot, c, &mut used));
        if !visible {
            e.hidden = true;
        }
    }
    (Cow::Owned(out), used)
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{ColorTrigger, Element, Rect};

    fn fmt(hours: TimeUnitStyle, minutes: TimeUnitStyle, ms: TimeUnitStyle) -> TimerFormat {
        TimerFormat {
            hours,
            minutes,
            milliseconds: ms,
            ..TimerFormat::default()
        }
    }

    #[test]
    fn default_format_is_mm_ss_with_hours_when_needed() {
        let f = TimerFormat::default();
        assert_eq!(format_timer(300.0, true, &f), "05:00");
        assert_eq!(format_timer(299.2, true, &f), "05:00"); // countdowns round up
        assert_eq!(format_timer(299.0, true, &f), "04:59");
        assert_eq!(format_timer(3725.0, true, &f), "1:02:05");
        assert_eq!(format_timer(0.4, true, &f), "00:01");
        assert_eq!(format_timer(0.0, true, &f), "00:00");
        assert_eq!(format_timer(-61.0, true, &f), "-01:01");
        assert_eq!(format_timer(59.9, false, &f), "00:59"); // elapsed rounds down
    }

    #[test]
    fn hidden_hours_carry_into_minutes_and_templates_wrap() {
        use TimeUnitStyle::*;
        let mut f = fmt(Hidden, Short, Hidden);
        assert_eq!(format_timer(300.0, true, &f), "5:00");
        assert_eq!(format_timer(45296.0, true, &f), "754:56");
        f.template = Some(" We will begin in ${timer}".into());
        assert_eq!(format_timer(65.0, true, &f), " We will begin in 1:05");
    }

    #[test]
    fn omit_zero_and_milliseconds() {
        use TimeUnitStyle::*;
        let f = TimerFormat {
            hours: Hidden,
            minutes: ShortOmitZero,
            seconds: ShortOmitZero,
            milliseconds: Short,
            template: None,
        };
        assert_eq!(format_timer(900.0, true, &f), "15:00.0");
        assert_eq!(format_timer(9.45, true, &f), "9.5");
        assert_eq!(format_timer(0.0, true, &f), "0.0");
        let long_ms = TimerFormat {
            milliseconds: Long,
            ..TimerFormat::default()
        };
        assert_eq!(format_timer(61.234, false, &long_ms), "01:01.23");
        let long_h = fmt(Long, Long, Hidden);
        assert_eq!(format_timer(61.0, true, &long_h), "00:01:01");
        let omit_h = fmt(LongOmitZero, Long, Hidden);
        assert_eq!(format_timer(7200.0, true, &omit_h), "02:00:00");
    }

    #[test]
    fn clock_formats() {
        let w = WallTime {
            year: 2026,
            month: 10,
            day: 19,
            weekday: 0,
            hour: 21,
            minute: 5,
            second: 9,
        };
        let mut f = ClockFormat::default();
        assert_eq!(format_clock(&w, &f), "9:05 PM");
        f.use_24_hour = Some(true);
        assert_eq!(format_clock(&w, &f), "21:05");
        f.time = DateTimeStyle::Medium;
        assert_eq!(format_clock(&w, &f), "21:05:09");
        f.use_24_hour = None;
        f.date = DateTimeStyle::Full;
        assert_eq!(format_clock(&w, &f), "Monday, October 19, 2026 9:05:09 PM");
        f.time = DateTimeStyle::None;
        f.date = DateTimeStyle::Short;
        assert_eq!(format_clock(&w, &f), "10/19/26");
        f.date = DateTimeStyle::Medium;
        f.template = Some("Today: ${clock}".into());
        assert_eq!(format_clock(&w, &f), "Today: Oct 19, 2026");
        let midnight = WallTime { hour: 0, ..w };
        assert_eq!(format_clock(&midnight, &ClockFormat::default()), "12:05 AM");
    }

    fn timer_slide(timer: TimerId, cond: TimerState) -> Slide {
        let mut s = Slide::new();
        let mut e = Element::text(Rect::new(0.0, 0.0, 100.0, 50.0), TextBox::plain("12:34"));
        e.data_link = Some(DataLink::Timer {
            timer: Some(Link::to(timer)),
            format: TimerFormat::default(),
            color_triggers: vec![ColorTrigger {
                remaining: 60.0,
                color: model::Color::rgb(1.0, 0.0, 0.0),
            }],
            show: TimerShow::Value,
        });
        e.visibility = vec![VisibilityCondition::Timer {
            timer: Some(Link::to(timer)),
            state: cond,
        }];
        s.elements.push(e);
        s
    }

    fn ctx_with(timer: TimerId, v: TimerValue) -> DataContext {
        let mut c = DataContext::default();
        c.timers.insert(timer, v);
        c
    }

    #[test]
    fn timer_links_resolve_with_colour_triggers_and_visibility() {
        let id = TimerId::new();
        let slide = timer_slide(id, TimerState::HasTimeRemaining);
        let running = TimerValue {
            seconds: 125.0,
            countdown: true,
            running: true,
            expired: false,
            warning: None,
            total: None,
        };
        let (s, used) = resolve_slide(&slide, &ctx_with(id, running));
        assert!(used.timers && !used.clock);
        let e = &s.elements[0];
        assert_eq!(e.text_box().unwrap().plain_text(), "02:05");
        assert!(!e.hidden);
        assert_eq!(
            e.text_box().unwrap().paragraphs[0].runs[0].style.color,
            None
        );
        // Under a minute: red.
        let low = TimerValue {
            seconds: 59.5,
            ..running
        };
        let (s, _) = resolve_slide(&slide, &ctx_with(id, low));
        assert_eq!(
            s.elements[0].text_box().unwrap().paragraphs[0].runs[0]
                .style
                .color,
            Some(model::Color::rgb(1.0, 0.0, 0.0))
        );
        // Expired: hidden.
        let expired = TimerValue {
            seconds: 0.0,
            running: false,
            expired: true,
            ..running
        };
        let (s, _) = resolve_slide(&slide, &ctx_with(id, expired));
        assert!(s.elements[0].hidden);
        assert_eq!(s.elements[0].text_box().unwrap().plain_text(), "00:00");
    }

    #[test]
    fn visibility_conditions() {
        let id = TimerId::new();
        let v = |running: bool, expired: bool| TimerValue {
            seconds: 10.0,
            countdown: true,
            running,
            expired,
            warning: None,
            total: None,
        };
        for (state, value, visible) in [
            (TimerState::HasTimeRemaining, v(true, false), true),
            (TimerState::HasTimeRemaining, v(false, true), false),
            (TimerState::HasExpired, v(false, true), true),
            (TimerState::HasExpired, v(true, false), false),
            (TimerState::IsRunning, v(true, false), true),
            (TimerState::IsRunning, v(false, false), false),
            (TimerState::NotRunning, v(false, false), true),
            (TimerState::NotRunning, v(true, true), false),
        ] {
            let slide = timer_slide(id, state);
            let ctx = ctx_with(id, value);
            let (s, _) = resolve_slide(&slide, &ctx);
            assert_eq!(!s.elements[0].hidden, visible, "{state:?} {value:?}");
        }
        // No such timer: stopped with time remaining.
        let slide = timer_slide(id, TimerState::HasTimeRemaining);
        let ctx = DataContext::default();
        let (s, _) = resolve_slide(&slide, &ctx);
        assert!(!s.elements[0].hidden);
        assert_eq!(s.elements[0].text_box().unwrap().plain_text(), "");

        // elementHasText follows the other element's text.
        let mut slide = Slide::new();
        let point = Element::text(Rect::new(0.0, 0.0, 10.0, 10.0), TextBox::plain(""));
        let mut bullet = Element::new(Rect::new(0.0, 0.0, 5.0, 5.0));
        bullet.visibility = vec![VisibilityCondition::ElementHasText {
            element: point.id,
            has_text: true,
        }];
        let mut empty = bullet.clone();
        empty.id = ElementId::new();
        empty.visibility = vec![VisibilityCondition::ElementHasText {
            element: point.id,
            has_text: false,
        }];
        slide.elements = vec![point, bullet, empty];
        let (s, _) = resolve_slide(&slide, &DataContext::default());
        assert!(s.elements[1].hidden && !s.elements[2].hidden);
        slide.elements[0].text_box_mut().unwrap().paragraphs = Paragraph::from_lines("Point");
        let (s, _) = resolve_slide(&slide, &DataContext::default());
        assert!(!s.elements[1].hidden && s.elements[2].hidden);
    }

    #[test]
    fn slides_without_live_data_are_borrowed() {
        let s = Slide::new();
        let ctx = DataContext::default();
        assert!(matches!(resolve_slide(&s, &ctx).0, Cow::Borrowed(_)));
    }

    #[test]
    fn last_started_timer_and_slide_links() {
        let id = TimerId::new();
        let mut ctx = ctx_with(
            id,
            TimerValue {
                seconds: 42.0,
                countdown: false,
                running: true,
                expired: false,
                warning: None,
                total: None,
            },
        );
        ctx.last_started = Some(id);
        ctx.current = Some(SlideInfo {
            text: "Morning light".into(),
            notes: "Slow".into(),
            section: "Verse 1".into(),
            ..SlideInfo::default()
        });
        ctx.slide_count = Some((2, 10));
        let mut used = LiveUse::default();
        let t = |l: DataLink, used: &mut LiveUse| link_text(&ctx, &l, used).unwrap().0;
        assert_eq!(
            t(
                DataLink::Timer {
                    timer: None,
                    format: TimerFormat::default(),
                    color_triggers: vec![],
                    show: TimerShow::Value,
                },
                &mut used
            ),
            "00:42"
        );
        assert_eq!(
            t(
                DataLink::SlideText {
                    which: SlideWhich::Current,
                    source: SlideTextSource::AllText,
                    keep_formatting: false
                },
                &mut used
            ),
            "Morning light"
        );
        assert_eq!(
            t(
                DataLink::SlideText {
                    which: SlideWhich::Next,
                    source: SlideTextSource::AllText,
                    keep_formatting: false
                },
                &mut used
            ),
            ""
        );
        assert_eq!(
            t(
                DataLink::SlideCount {
                    count: SlideCountKind::Remaining
                },
                &mut used
            ),
            "7"
        );
        assert_eq!(
            t(
                DataLink::SectionName {
                    which: SectionWhich::CurrentSlide
                },
                &mut used
            ),
            "Verse 1"
        );
        // CCLI text: the live song's copyright line (empty without one); song artwork is a
        // picture link (no text).
        assert_eq!(t(DataLink::CcliText, &mut used), "");
        assert!(link_text(&ctx, &DataLink::SongArtwork, &mut used).is_none());
    }

    #[test]
    fn stage_message_links_flash() {
        let mut slide = Slide::new();
        let mut e = Element::text(Rect::new(0.0, 0.0, 100.0, 50.0), TextBox::plain("msg"));
        e.data_link = Some(DataLink::Message {
            flash: true,
            flash_color: None,
        });
        slide.elements.push(e);
        let mut ctx = DataContext {
            message: Some("Wrap up".into()),
            ..DataContext::default()
        };
        let (s, used) = resolve_slide(&slide, &ctx);
        assert!(used.flash && used.any());
        assert_eq!(s.elements[0].text_box().unwrap().plain_text(), "Wrap up");
        assert!(!s.elements[0].fill.enabled);
        ctx.stage_flash = true;
        let (s, _) = resolve_slide(&slide, &ctx);
        assert_eq!(s.elements[0].fill, model::Fill::color(FLASH_COLOR));
        // No message: nothing flashes, the text is empty.
        ctx.message = None;
        let (s, _) = resolve_slide(&slide, &ctx);
        assert!(!s.elements[0].fill.enabled);
        assert_eq!(s.elements[0].text_box().unwrap().plain_text(), "");
    }

    #[test]
    fn the_box_clock_fills_in_open_formats_and_timers_warn_in_amber() {
        let w = WallTime {
            year: 2026,
            month: 10,
            day: 19,
            weekday: 0,
            hour: 21,
            minute: 5,
            second: 9,
        };
        let b = BoxClock {
            use_24_hour: true,
            seconds: true,
        };
        assert_eq!(
            format_clock(&w, &b.apply(&ClockFormat::default())),
            "21:05:09"
        );
        let own = ClockFormat {
            use_24_hour: Some(false),
            ..ClockFormat::default()
        };
        assert_eq!(format_clock(&w, &b.apply(&own)), "9:05:09 PM");
        assert_eq!(
            format_clock(&w, &BoxClock::default().apply(&ClockFormat::default())),
            "9:05 PM"
        );
        let v = |seconds: f64, warning: Option<f64>, expired: bool| TimerValue {
            seconds,
            countdown: true,
            running: true,
            expired,
            warning,
            total: None,
        };
        assert!(v(60.0, Some(60.0), false).warns());
        assert!(!v(61.0, Some(60.0), false).warns());
        assert!(!v(30.0, None, false).warns());
        assert!(!v(0.0, Some(60.0), true).warns());
        let id = TimerId::new();
        let mut el = Element::text(Rect::new(0.0, 0.0, 100.0, 50.0), TextBox::plain("x"));
        el.data_link = Some(DataLink::Timer {
            timer: Some(Link::to(id)),
            format: TimerFormat::default(),
            color_triggers: vec![],
            show: TimerShow::Value,
        });
        let slide = Slide {
            elements: vec![el],
            ..Slide::new()
        };
        let (s, _) = resolve_slide(&slide, &ctx_with(id, v(30.0, Some(60.0), false)));
        assert_eq!(
            s.elements[0].text_box().unwrap().default_style.color,
            Some(WARNING_COLOR)
        );
        let (s, _) = resolve_slide(&slide, &ctx_with(id, v(90.0, Some(60.0), false)));
        assert_ne!(
            s.elements[0].text_box().unwrap().default_style.color,
            Some(WARNING_COLOR)
        );
    }

    fn text_of(s: &Slide, i: usize) -> String {
        s.elements[i].text_box().unwrap().plain_text()
    }

    fn linked(link: DataLink) -> Element {
        let mut e = Element::text(Rect::new(0.0, 0.0, 100.0, 50.0), TextBox::plain("design"));
        e.data_link = Some(link);
        e
    }

    #[test]
    fn timer_links_show_elapsed_or_remaining() {
        let id = TimerId::new();
        let link = |show| {
            linked(DataLink::Timer {
                timer: Some(Link::to(id)),
                format: TimerFormat::default(),
                color_triggers: vec![],
                show,
            })
        };
        let slide = Slide {
            elements: vec![
                link(TimerShow::Value),
                link(TimerShow::Elapsed),
                link(TimerShow::Remaining),
            ],
            ..Slide::new()
        };
        let texts = |v: TimerValue| {
            let (s, used) = resolve_slide(&slide, &ctx_with(id, v));
            assert!(used.timers);
            (0..3).map(|i| text_of(&s, i)).collect::<Vec<_>>()
        };
        // A 10-minute countdown with 4:05 left: 5:55 elapsed.
        let countdown = TimerValue {
            seconds: 245.0,
            countdown: true,
            running: true,
            expired: false,
            warning: None,
            total: Some(600.0),
        };
        assert_eq!(texts(countdown), ["04:05", "05:55", "04:05"]);
        // A count-up timer at 1:30 that ends at 5:00: 3:30 remaining.
        let up = TimerValue {
            seconds: 90.0,
            countdown: false,
            total: Some(300.0),
            ..countdown
        };
        assert_eq!(texts(up), ["01:30", "01:30", "03:30"]);
        // No end / no duration: nothing to show for the derived time.
        assert_eq!(
            texts(TimerValue { total: None, ..up }),
            ["01:30", "01:30", ""]
        );
        assert_eq!(
            texts(TimerValue {
                total: None,
                ..countdown
            }),
            ["04:05", "", "04:05"]
        );
    }

    /// F03-3: a count-up timer with an end, shown as its time remaining, takes its colour
    /// triggers and its warning from the time it shows; shown as its value, from its value.
    #[test]
    fn a_count_up_shown_as_remaining_colours_by_the_time_left() {
        let id = TimerId::new();
        let red = model::Color::rgb(1.0, 0.0, 0.0);
        let link = |show, triggers: Vec<ColorTrigger>| {
            linked(DataLink::Timer {
                timer: Some(Link::to(id)),
                format: TimerFormat::default(),
                color_triggers: triggers,
                show,
            })
        };
        let trig = || {
            vec![ColorTrigger {
                remaining: 60.0,
                color: red,
            }]
        };
        let slide = Slide {
            elements: vec![
                link(TimerShow::Remaining, trig()),
                link(TimerShow::Remaining, vec![]),
                link(TimerShow::Value, trig()),
            ],
            ..Slide::new()
        };
        let colours = |seconds: f64| {
            let v = TimerValue {
                seconds,
                countdown: false,
                running: true,
                expired: false,
                warning: Some(60.0),
                total: Some(300.0),
            };
            let (s, _) = resolve_slide(&slide, &ctx_with(id, v));
            (0..3)
                .map(|i| s.elements[i].text_box().unwrap().default_style.color)
                .collect::<Vec<_>>()
        };
        // 0:30 in, 4:30 left: no colour on the remaining links; the value link (0:30) as before.
        let start = colours(30.0);
        assert_ne!(start[0], Some(red));
        assert_ne!(start[1], Some(WARNING_COLOR));
        assert_eq!(start[2], Some(red));
        // 4:30 in, 0:30 left: the remaining links show the trigger and the warning.
        let end = colours(270.0);
        assert_eq!(end[0], Some(red));
        assert_eq!(end[1], Some(WARNING_COLOR));
        assert_ne!(end[2], Some(red));
    }

    #[test]
    fn presentation_links_and_song_artwork() {
        let art = MediaAssetId::new();
        let mut frame = Element::new(Rect::new(0.0, 0.0, 100.0, 100.0));
        frame.data_link = Some(DataLink::SongArtwork);
        frame.fill = model::Fill::color(model::Color::rgb(0.0, 0.0, 1.0));
        let slide = Slide {
            elements: vec![
                linked(DataLink::PresentationName {
                    source: CountdownSource::Presentation,
                }),
                linked(DataLink::PresentationName {
                    source: CountdownSource::Announcements,
                }),
                linked(DataLink::PresentationNotes),
                linked(DataLink::CcliText),
                frame,
            ],
            ..Slide::new()
        };
        let ctx = DataContext {
            presentation_name: Some("Morning Song".into()),
            announcement_name: Some("Welcome Loop".into()),
            presentation_notes: Some("Capo 2".into()),
            ccli: Some("Morning Song\nA. Writer".into()),
            artwork: Some(art),
            ..DataContext::default()
        };
        let (s, _) = resolve_slide(&slide, &ctx);
        assert_eq!(text_of(&s, 0), "Morning Song");
        assert_eq!(text_of(&s, 1), "Welcome Loop");
        assert_eq!(text_of(&s, 2), "Capo 2");
        assert_eq!(text_of(&s, 3), "Morning Song\nA. Writer");
        let f = &s.elements[4].fill;
        assert!(f.enabled);
        assert!(matches!(f.kind, model::FillKind::Media { asset, .. } if asset == art));
        // Nothing live: empty texts, no artwork fill.
        let (s, _) = resolve_slide(&slide, &DataContext::default());
        assert!((0..4).all(|i| text_of(&s, i).is_empty()));
        assert!(!s.elements[4].fill.enabled);
    }

    #[test]
    fn message_layer_playlist_and_media_conditions() {
        let msg = MessageId::new();
        let other = MessageId::new();
        let cond = |c: VisibilityCondition| {
            let mut e = Element::new(Rect::new(0.0, 0.0, 10.0, 10.0));
            e.visibility = vec![c];
            Slide {
                elements: vec![e],
                ..Slide::new()
            }
        };
        let shown = |c: VisibilityCondition, ctx: &DataContext| {
            let slide = cond(c);
            let (s, used) = resolve_slide(&slide, ctx);
            (!s.elements[0].hidden, used.media)
        };
        let mut ctx = DataContext::default();
        let any = VisibilityCondition::Message {
            message: None,
            active: true,
        };
        let this = VisibilityCondition::Message {
            message: Some(Link::to(msg)),
            active: true,
        };
        let not_this = VisibilityCondition::Message {
            message: Some(Link::to(msg)),
            active: false,
        };
        assert!(!shown(any.clone(), &ctx).0 && !shown(this.clone(), &ctx).0);
        assert!(shown(not_this.clone(), &ctx).0);
        ctx.messages = vec![other];
        assert!(shown(any.clone(), &ctx).0 && !shown(this.clone(), &ctx).0);
        ctx.messages.push(msg);
        assert!(shown(this, &ctx).0 && !shown(not_this, &ctx).0);

        let stage = VisibilityCondition::StageMessage { active: true };
        assert!(!shown(stage.clone(), &ctx).0);
        ctx.message = Some("Wrap up".into());
        assert!(shown(stage, &ctx).0);

        let props = VisibilityCondition::Layer {
            layer: Layer::Props,
            active: true,
        };
        let no_media = VisibilityCondition::Layer {
            layer: Layer::Media,
            active: false,
        };
        assert!(!shown(props.clone(), &ctx).0 && shown(no_media.clone(), &ctx).0);
        ctx.layers = vec![Layer::Props, Layer::Media];
        assert!(shown(props, &ctx).0 && !shown(no_media, &ctx).0);

        let next = VisibilityCondition::PlaylistItem {
            which: PlaylistItemWhich::Next,
            active: true,
        };
        let last = VisibilityCondition::PlaylistItem {
            which: PlaylistItemWhich::Next,
            active: false,
        };
        assert!(!shown(next.clone(), &ctx).0 && shown(last.clone(), &ctx).0);
        ctx.playlist.next = Some("Offering".into());
        assert!(shown(next, &ctx).0 && !shown(last, &ctx).0);

        // Media: nothing live counts as expired, not running, not looping.
        let video = |state| VisibilityCondition::VideoCountdown { state };
        let audio = |state| VisibilityCondition::AudioCountdown { state };
        use MediaCondition as M;
        for (state, nothing) in [
            (M::HasTimeRemaining, false),
            (M::HasExpired, true),
            (M::IsRunning, false),
            (M::NotRunning, true),
            (M::Looping, false),
            (M::NotLooping, true),
        ] {
            assert_eq!(shown(video(state), &ctx), (nothing, true), "{state:?}");
            assert_eq!(shown(audio(state), &ctx).0, nothing, "{state:?}");
        }
        ctx.video = Some(MediaStatus {
            remaining: 12.0,
            running: true,
            looping: true,
        });
        ctx.audio = Some(MediaStatus {
            remaining: 0.0,
            running: false,
            looping: false,
        });
        for (state, v, a) in [
            (M::HasTimeRemaining, true, false),
            (M::HasExpired, false, true),
            (M::IsRunning, true, false),
            (M::NotRunning, false, true),
            (M::Looping, true, false),
            (M::NotLooping, false, true),
        ] {
            assert_eq!(shown(video(state), &ctx).0, v, "video {state:?}");
            assert_eq!(shown(audio(state), &ctx).0, a, "audio {state:?}");
        }
        // Paused with time left: has time remaining, not running.
        ctx.video = Some(MediaStatus {
            remaining: 12.0,
            running: false,
            looping: false,
        });
        assert!(shown(video(M::HasTimeRemaining), &ctx).0);
        assert!(shown(video(M::NotRunning), &ctx).0);
    }
}
