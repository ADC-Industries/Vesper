//! The schedule's rules (pure: the caller passes the local date and time).
//!
//! A slot plays on its days (none: every day) from its start to its end; an end at or before
//! the start runs past midnight, so the part after midnight belongs to the day it started on.
//! A date range (`from`, `until`, inclusive) limits the days it starts on. Of the slots playing
//! now, the highest priority wins; a tie goes to the one listed first. With none, the channel's
//! default playlist plays.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Timelike};
use protocol::{SignageServiceTime, SignageSlot};

/// Minutes in a day.
pub const DAY: u16 = 1440;

/// Monday = 0 … Sunday = 6.
pub fn weekday(d: NaiveDate) -> u8 {
    d.weekday().num_days_from_monday() as u8
}

fn date(s: &Option<String>) -> Option<NaiveDate> {
    s.as_deref()
        .and_then(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
}

/// The slot starts on `day` (its weekday and date range).
fn starts_on(slot: &SignageSlot, day: NaiveDate) -> bool {
    (slot.days.is_empty() || slot.days.contains(&weekday(day)))
        && date(&slot.from).is_none_or(|f| day >= f)
        && date(&slot.until).is_none_or(|u| day <= u)
}

/// The slot plays at `now`.
pub fn plays(slot: &SignageSlot, now: NaiveDateTime) -> bool {
    let m = (now.hour() * 60 + now.minute()) as u16;
    let today = now.date();
    let (start, end) = (slot.start.min(DAY - 1), slot.end.min(DAY));
    if end > start {
        starts_on(slot, today) && m >= start && m < end
    } else {
        // Past midnight: from the start to midnight today, or from midnight to the end on the
        // day after a day it starts on.
        (starts_on(slot, today) && m >= start)
            || (m < end && starts_on(slot, today - Duration::days(1)))
    }
}

/// The slot that plays at `now`: the highest priority, the first listed on a tie.
pub fn active(slots: &[SignageSlot], now: NaiveDateTime) -> Option<&SignageSlot> {
    let mut best: Option<&SignageSlot> = None;
    for s in slots.iter().filter(|s| plays(s, now)) {
        if best.is_none_or(|b| s.priority > b.priority) {
            best = Some(s);
        }
    }
    best
}

/// The playlist a channel plays at `now`, with the slot that picked it (`None`: the default).
pub fn pick<'a>(
    slots: &'a [SignageSlot],
    default: Option<&'a str>,
    now: NaiveDateTime,
) -> Option<(&'a str, Option<&'a str>)> {
    match active(slots, now) {
        Some(s) => Some((s.playlist.as_str(), Some(s.id.as_str()))),
        None => default.filter(|d| !d.is_empty()).map(|d| (d, None)),
    }
}

/// The next service after `now` (within a week), with its time.
pub fn next_service(
    services: &[SignageServiceTime],
    now: NaiveDateTime,
) -> Option<(NaiveDateTime, &SignageServiceTime)> {
    let mut best: Option<(NaiveDateTime, &SignageServiceTime)> = None;
    for s in services {
        let m = s.minute.min(DAY - 1);
        let ahead = (i64::from(s.day % 7) - i64::from(weekday(now.date()))).rem_euclid(7);
        let day = now.date() + Duration::days(ahead);
        let mut at = day.and_hms_opt(u32::from(m / 60), u32::from(m % 60), 0)?;
        if at <= now {
            at += Duration::days(7);
        }
        if best.is_none_or(|(b, _)| at < b) {
            best = Some((at, s));
        }
    }
    best
}

/// Seconds from `now` until the local time `at`, counted on the real clock: a change of
/// daylight saving time between them counts (H02-10). A time that happens twice is the first;
/// one skipped by the clocks going forward is just after the gap.
pub fn seconds_until<Tz: chrono::TimeZone>(now: &chrono::DateTime<Tz>, at: NaiveDateTime) -> i64 {
    let tz = now.timezone();
    let when = tz.from_local_datetime(&at).earliest().or_else(|| {
        (1..=4).find_map(|q| {
            tz.from_local_datetime(&(at + Duration::minutes(15 * q)))
                .earliest()
        })
    });
    match when {
        Some(w) => (w.naive_utc() - now.naive_utc()).num_seconds(),
        None => (at - now.naive_local()).num_seconds(),
    }
}

/// A countdown in words: "in 2 d 4 h", "in 3 h 05 min", "in 12:34" (under an hour), "now".
pub fn countdown(secs: i64) -> String {
    if secs <= 0 {
        return "now".into();
    }
    let (d, h, m, s) = (
        secs / 86_400,
        secs % 86_400 / 3600,
        secs % 3600 / 60,
        secs % 60,
    );
    if d > 0 {
        format!("in {d} d {h} h")
    } else if h > 0 {
        format!("in {h} h {m:02} min")
    } else {
        format!("in {m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn slot(id: &str, days: &[u8], start: u16, end: u16, priority: i32) -> SignageSlot {
        SignageSlot {
            id: id.into(),
            playlist: format!("p-{id}"),
            days: days.to_vec(),
            start,
            end,
            from: None,
            until: None,
            priority,
        }
    }

    // 2026-11-25 is a Wednesday (2).
    #[test]
    fn a_slot_plays_on_its_days_between_its_times() {
        let s = slot("a", &[2], 9 * 60, 12 * 60, 0);
        assert!(plays(&s, at("2026-11-25 09:00")));
        assert!(plays(&s, at("2026-11-25 11:59")));
        assert!(!plays(&s, at("2026-11-25 12:00")), "the end is exclusive");
        assert!(!plays(&s, at("2026-11-25 08:59")));
        assert!(!plays(&s, at("2026-11-26 10:00")), "Thursday");
        let every = slot("b", &[], 0, DAY, 0);
        assert!(plays(&every, at("2026-11-29 23:59")));
    }

    #[test]
    fn a_slot_past_midnight_belongs_to_the_day_it_starts() {
        let s = slot("late", &[4], 22 * 60, 2 * 60, 0); // Friday 22:00 → Saturday 02:00
        assert!(plays(&s, at("2026-11-27 23:00")));
        assert!(plays(&s, at("2026-11-28 01:59")));
        assert!(!plays(&s, at("2026-11-28 02:00")));
        assert!(
            !plays(&s, at("2026-11-28 23:00")),
            "Saturday doesn't start it"
        );
        assert!(!plays(&s, at("2026-11-27 01:00")), "nor does Thursday");
    }

    #[test]
    fn date_ranges_limit_the_days() {
        let mut s = slot("advent", &[], 0, DAY, 0);
        s.from = Some("2026-11-29".into());
        s.until = Some("2026-12-24".into());
        assert!(!plays(&s, at("2026-11-28 12:00")));
        assert!(plays(&s, at("2026-11-29 00:00")));
        assert!(plays(&s, at("2026-12-24 23:59")), "until is inclusive");
        assert!(!plays(&s, at("2026-12-25 00:00")));
    }

    #[test]
    fn the_highest_priority_wins_then_the_first_listed() {
        let slots = vec![
            slot("low", &[], 0, DAY, 0),
            slot("high", &[2], 9 * 60, 10 * 60, 5),
            slot("tie", &[2], 9 * 60, 10 * 60, 5),
        ];
        assert_eq!(active(&slots, at("2026-11-25 09:30")).unwrap().id, "high");
        assert_eq!(active(&slots, at("2026-11-25 10:30")).unwrap().id, "low");
        assert_eq!(
            pick(&slots, Some("d"), at("2026-11-25 09:30")),
            Some(("p-high", Some("high")))
        );
    }

    #[test]
    fn the_default_plays_when_no_slot_does() {
        let slots = vec![slot("a", &[0], 9 * 60, 10 * 60, 0)];
        let now = at("2026-11-25 09:30");
        assert_eq!(pick(&slots, Some("d"), now), Some(("d", None)));
        assert_eq!(pick(&slots, None, now), None);
        assert_eq!(pick(&slots, Some(""), now), None);
    }

    #[test]
    fn the_next_service_and_its_countdown() {
        let services = vec![
            SignageServiceTime {
                day: 6,
                minute: 10 * 60 + 30,
                name: "Sunday".into(),
            },
            SignageServiceTime {
                day: 2,
                minute: 19 * 60,
                name: "Midweek".into(),
            },
        ];
        let (t, s) = next_service(&services, at("2026-11-25 18:00")).unwrap();
        assert_eq!((t, s.name.as_str()), (at("2026-11-25 19:00"), "Midweek"));
        let (t, s) = next_service(&services, at("2026-11-25 19:00")).unwrap();
        assert_eq!((t, s.name.as_str()), (at("2026-11-29 10:30"), "Sunday"));
        let (t, _) = next_service(&services[..1], at("2026-11-29 10:31")).unwrap();
        assert_eq!(t, at("2026-12-06 10:30"), "a week on once it has started");
        assert_eq!(countdown(0), "now");
        assert_eq!(countdown(754), "in 12:34");
        assert_eq!(countdown(3 * 3600 + 5 * 60), "in 3 h 05 min");
        assert_eq!(countdown(2 * 86_400 + 4 * 3600 + 59), "in 2 d 4 h");
    }

    /// A made-up zone: UTC+1, and UTC+2 from 2026-03-29 01:00 UTC to 2026-10-25 01:00 UTC.
    #[derive(Clone, Copy, Debug)]
    struct Summer;

    impl Summer {
        fn offset_at(utc: &NaiveDateTime) -> chrono::FixedOffset {
            let (on, off) = (at("2026-03-29 01:00"), at("2026-10-25 01:00"));
            let h = if *utc >= on && *utc < off { 2 } else { 1 };
            chrono::FixedOffset::east_opt(h * 3600).unwrap()
        }
    }

    impl chrono::TimeZone for Summer {
        type Offset = chrono::FixedOffset;
        fn from_offset(_: &chrono::FixedOffset) -> Self {
            Summer
        }
        fn offset_from_local_date(
            &self,
            _: &NaiveDate,
        ) -> chrono::MappedLocalTime<chrono::FixedOffset> {
            unimplemented!()
        }
        fn offset_from_local_datetime(
            &self,
            local: &NaiveDateTime,
        ) -> chrono::MappedLocalTime<chrono::FixedOffset> {
            let fits: Vec<chrono::FixedOffset> = [1, 2]
                .into_iter()
                .map(|h| chrono::FixedOffset::east_opt(h * 3600).unwrap())
                .filter(|o| {
                    Self::offset_at(&(*local - Duration::seconds(i64::from(o.local_minus_utc()))))
                        == *o
                })
                .collect();
            match fits[..] {
                [] => chrono::MappedLocalTime::None,
                [o] => chrono::MappedLocalTime::Single(o),
                [a, b] => chrono::MappedLocalTime::Ambiguous(b, a),
                _ => unreachable!(),
            }
        }
        fn offset_from_utc_date(&self, _: &NaiveDate) -> chrono::FixedOffset {
            unimplemented!()
        }
        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> chrono::FixedOffset {
            Self::offset_at(utc)
        }
    }

    /// H02-10: the countdown counts the hour the clocks change.
    #[test]
    fn a_countdown_counts_a_change_of_the_clocks() {
        use chrono::TimeZone;
        // Saturday evening before the clocks go back one hour at 03:00 local: Sunday 10:30
        // is 16 h 30 min of local time away but 17 h 30 min of real time.
        let now = Summer.from_local_datetime(&at("2026-10-24 18:00")).unwrap();
        assert_eq!(
            seconds_until(&now, at("2026-10-25 10:30")),
            17 * 3600 + 1800
        );
        // In spring an hour less.
        let now = Summer.from_local_datetime(&at("2026-03-28 18:00")).unwrap();
        assert_eq!(
            seconds_until(&now, at("2026-03-29 10:30")),
            15 * 3600 + 1800
        );
        // No change: as on the wall clock.
        let now = Summer.from_local_datetime(&at("2026-11-21 18:00")).unwrap();
        assert_eq!(
            seconds_until(&now, at("2026-11-22 10:30")),
            16 * 3600 + 1800
        );
    }
}
