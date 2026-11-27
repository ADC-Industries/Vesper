//! Which snapshots of a set are kept: the newest `last`, plus the newest of each of the
//! `daily` most recent days that have snapshots, plus the newest of each of the `weekly` most
//! recent weeks (Monday to Sunday, UTC) that have snapshots. Periods are counted among those
//! that have snapshots, not back from today, so a box that was switched off for a while keeps
//! its older snapshots.

use model::{SettingsSnapshot, SnapshotId};
use std::collections::HashSet;

/// How many snapshots are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retention {
    /// The newest ones (default 10).
    pub last: usize,
    /// One a day (default 14 days).
    pub daily: usize,
    /// One a week (default 8 weeks).
    pub weekly: usize,
}

impl Default for Retention {
    fn default() -> Self {
        Retention {
            last: 10,
            daily: 14,
            weekly: 8,
        }
    }
}

/// Days since 1970-01-01 of an RFC 3339 timestamp's date (`None` when it has none).
fn day_number(at: &str) -> Option<i64> {
    let y: i64 = at.get(0..4)?.parse().ok()?;
    let m: i64 = at.get(5..7)?.parse().ok()?;
    let d: i64 = at.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from civil (Howard Hinnant's algorithm).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// The snapshots to remove from `snapshots` (any order).
pub fn prune(snapshots: &[SettingsSnapshot], r: Retention) -> Vec<SnapshotId> {
    let mut sorted: Vec<&SettingsSnapshot> = snapshots.iter().collect();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.seq));
    let mut keep: HashSet<SnapshotId> = sorted.iter().take(r.last).map(|s| s.id).collect();
    let mut keep_newest_per = |period: &dyn Fn(i64) -> i64, count: usize| {
        let mut seen = HashSet::new();
        for s in &sorted {
            // A snapshot whose date can't be read is kept (never removed by mistake).
            let Some(day) = day_number(&s.taken_at) else {
                keep.insert(s.id);
                continue;
            };
            let p = period(day);
            if seen.contains(&p) {
                continue;
            }
            if seen.len() == count {
                continue;
            }
            seen.insert(p);
            keep.insert(s.id);
        }
    };
    keep_newest_per(&|d| d, r.daily);
    // 1970-01-01 was a Thursday: day + 3 counts weeks from Mondays.
    keep_newest_per(&|d| (d + 3).div_euclid(7), r.weekly);
    sorted
        .iter()
        .filter(|s| !keep.contains(&s.id))
        .map(|s| s.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{BackupSetId, SnapshotReason};

    fn snap(seq: u64, at: &str) -> SettingsSnapshot {
        SettingsSnapshot {
            schema_version: 1,
            id: SnapshotId::new(),
            set: BackupSetId(uuid::Uuid::nil()),
            seq,
            taken_at: at.into(),
            reason: SnapshotReason::Change,
            by: Default::default(),
            digest: String::new(),
            version: String::new(),
            items: Vec::new(),
        }
    }

    #[test]
    fn day_numbers() {
        assert_eq!(day_number("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(day_number("2026-11-22T10:00:00Z"), Some(20_779));
        assert_eq!(day_number("2000-03-01T00:00:00Z"), Some(11_017));
        assert_eq!(day_number("garbage"), None);
    }

    #[test]
    fn keeps_last_daily_and_weekly() {
        // Four snapshots a day for 120 days.
        let mut all = Vec::new();
        let mut seq = 0;
        for day in 0..120 {
            let date = 20_089 + day;
            for h in [6, 12, 18, 23] {
                seq += 1;
                let t = std::time::SystemTime::UNIX_EPOCH
                    + std::time::Duration::from_secs(date as u64 * 86_400 + h * 3600);
                all.push(snap(seq, &crate::auth::rfc3339(t, true)));
            }
        }
        let gone: HashSet<SnapshotId> = prune(&all, Retention::default()).into_iter().collect();
        let kept: Vec<&SettingsSnapshot> = all.iter().filter(|s| !gone.contains(&s.id)).collect();
        // The newest 10 (2.5 days), one per day for the 14 newest days (11 more days beyond
        // the 3 the newest 10 touch, each its 23:00 one), and one per week for 8 weeks.
        let newest10: Vec<u64> = (seq - 9..=seq).collect();
        for s in &newest10 {
            assert!(kept.iter().any(|k| k.seq == *s), "the newest 10 stay");
        }
        let days: HashSet<&str> = kept.iter().map(|s| &s.taken_at[..10]).collect();
        assert!(days.len() >= 14, "{days:?}");
        let weeks: HashSet<i64> = kept
            .iter()
            .map(|s| (day_number(&s.taken_at).unwrap() + 3).div_euclid(7))
            .collect();
        assert_eq!(weeks.len(), 8, "eight weeks: {weeks:?}");
        // 10 newest + 11 more days + the weeks beyond those the newest 14 days touch.
        let recent_weeks: HashSet<i64> = (20_089 + 106..20_089 + 120)
            .map(|d: i64| (d + 3).div_euclid(7))
            .collect();
        assert_eq!(kept.len(), 10 + 11 + (8 - recent_weeks.len()));
        // Daily ones are each day's newest.
        for s in kept.iter().filter(|s| s.seq <= seq - 10) {
            assert!(s.taken_at.ends_with("T23:00:00Z"), "{}", s.taken_at);
        }
    }

    #[test]
    fn few_snapshots_all_stay_and_odd_dates_stay() {
        let all = vec![
            snap(1, "2026-01-01T00:00:00Z"),
            snap(2, "not a date"),
            snap(3, "2026-01-01T01:00:00Z"),
        ];
        assert!(prune(&all, Retention::default()).is_empty());
        let r = Retention {
            last: 1,
            daily: 0,
            weekly: 0,
        };
        let gone = prune(&all, r);
        assert_eq!(
            gone,
            vec![all[0].id],
            "the odd date is kept, the newest too"
        );
    }
}
