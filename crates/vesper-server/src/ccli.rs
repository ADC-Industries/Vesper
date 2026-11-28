//! CCLI song usage (`docs/MEDIA-PREP-PLAN.md`, "CCLI usage reports"): each time a song goes
//! live, one use per service is logged, and reports count them per song in a date range.
//!
//! - A **song** is a presentation with song metadata (a CCLI number, a title or authors).
//! - A **service** is the playlist the song was shown from (by name), else the day: the same
//!   song again in the same service and day is the same use, however many slides or returns.
//! - A use says whether it was **projected** (audience screens on), **streamed** or **recorded**
//!   (a stream / the recording running) when it went live or later while it stayed live.
//!
//! The log is `reports/ccli-usage.jsonl` in the box's data folder, append-only: a use is written
//! when it starts and again when one of its flags turns on; readers merge the lines of a use.
//!
//! Beside it, `reports/ccli-imported-license.txt` keeps the church's licence number the
//! SongSelect files read last carried (`CCLI License # …`), for Settings ▸ CCLI to offer.

use model::{Presentation, PresentationId};
use protocol::{CcliReport, CcliReportRow, CcliUse};
use std::collections::{BTreeMap, HashMap};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use vesper_core::lock;

/// A use's identity: day, service, presentation.
type Key = (String, String, PresentationId);

fn key(u: &CcliUse) -> Key {
    (u.date.clone(), u.service.clone(), u.presentation)
}

/// What is live now, as far as the log cares.
pub struct Live<'a> {
    /// The presentation on the slide layer.
    pub presentation: &'a Presentation,
    /// The playlist it was shown from, by name.
    pub service: Option<String>,
    /// Audience screens are on.
    pub projected: bool,
    /// A stream is running.
    pub streamed: bool,
    /// The recording is running.
    pub recorded: bool,
}

/// The longest licence number kept (characters).
pub const MAX_LICENSE: usize = 40;

/// The usage log of a box.
pub struct CcliLog {
    path: PathBuf,
    /// `reports/ccli-imported-license.txt`.
    imported: PathBuf,
    /// Uses logged (merged), loaded on first use.
    uses: Mutex<Option<HashMap<Key, CcliUse>>>,
}

/// Song metadata worth reporting: a CCLI number, a title or authors.
fn is_song(p: &Presentation) -> bool {
    p.song_info.as_ref().is_some_and(|s| {
        [&s.ccli_number, &s.title, &s.authors]
            .iter()
            .any(|v| v.as_deref().is_some_and(|t| !t.trim().is_empty()))
    })
}

/// Merges `u` into `into` (flags or'ed, the earliest time kept).
fn merge(into: &mut CcliUse, u: &CcliUse) {
    into.projected |= u.projected;
    into.streamed |= u.streamed;
    into.recorded |= u.recorded;
    if u.at < into.at {
        into.at = u.at.clone();
    }
}

/// Reads a log's lines, merged per use (bad lines skipped).
pub fn read_uses(path: &Path) -> HashMap<Key, CcliUse> {
    let mut out: HashMap<Key, CcliUse> = HashMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return out;
    };
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(u) = serde_json::from_str::<CcliUse>(line) {
            match out.get_mut(&key(&u)) {
                Some(e) => merge(e, &u),
                None => {
                    out.insert(key(&u), u);
                }
            }
        }
    }
    out
}

impl CcliLog {
    /// The log in `data_dir`.
    pub fn new(data_dir: &Path) -> CcliLog {
        CcliLog {
            path: data_dir.join("reports").join("ccli-usage.jsonl"),
            imported: data_dir.join("reports").join("ccli-imported-license.txt"),
            uses: Mutex::new(None),
        }
    }

    /// The licence number the SongSelect files read last carried, if any (blocking).
    pub fn imported_license(&self) -> Option<String> {
        let t = std::fs::read_to_string(&self.imported).ok()?;
        let t = t.trim();
        (!t.is_empty() && t.chars().count() <= MAX_LICENSE).then(|| t.to_string())
    }

    /// Keeps `license`, the licence number a SongSelect file carries (blocking; written only
    /// when it changes; a failure is logged).
    pub fn note_imported_license(&self, license: &str) {
        let license = license.trim();
        if license.is_empty()
            || license.chars().count() > MAX_LICENSE
            || license.chars().any(char::is_control)
            || self.imported_license().as_deref() == Some(license)
        {
            return;
        }
        let written = self
            .imported
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&self.imported, format!("{license}\n")));
        if let Err(e) = written {
            tracing::warn!("the imported CCLI licence number isn't kept: {e}");
        }
    }

    /// Notes what is live now (at `now`, local time). Writes a line when a use starts or one
    /// of its flags turns on; blocking only for that append.
    pub fn observe(&self, live: Option<Live<'_>>, now: chrono::DateTime<chrono::Local>) {
        let Some(l) = live else { return };
        if !is_song(l.presentation) {
            return;
        }
        let song = l.presentation.song_info.clone().unwrap_or_default();
        let text = |v: Option<String>| v.map(|t| t.trim().to_string()).unwrap_or_default();
        let title = text(song.title).to_string();
        let u = CcliUse {
            date: now.format("%Y-%m-%d").to_string(),
            at: now
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            service: l.service.unwrap_or_default(),
            presentation: l.presentation.id,
            title: if title.is_empty() {
                l.presentation.name.clone()
            } else {
                title
            },
            ccli_number: text(song.ccli_number),
            authors: text(song.authors),
            projected: l.projected,
            streamed: l.streamed,
            recorded: l.recorded,
        };
        let mut uses = lock(&self.uses);
        let uses = uses.get_or_insert_with(|| read_uses(&self.path));
        let write = match uses.get_mut(&key(&u)) {
            None => {
                uses.insert(key(&u), u.clone());
                true
            }
            Some(e) => {
                let before = (e.projected, e.streamed, e.recorded);
                merge(e, &u);
                before != (e.projected, e.streamed, e.recorded)
            }
        };
        if write && let Err(e) = self.append(&u) {
            tracing::warn!("CCLI usage log {}: {e}", self.path.display());
        }
    }

    fn append(&self, u: &CcliUse) -> std::io::Result<()> {
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut line = serde_json::to_vec(u).map_err(std::io::Error::other)?;
        line.push(b'\n');
        f.write_all(&line)
    }

    /// The report for `from`..=`to` (`YYYY-MM-DD`).
    pub fn report(&self, from: &str, to: &str) -> CcliReport {
        let uses = read_uses(&self.path);
        report(uses.into_values().collect(), from, to)
    }
}

/// The report of `uses` for `from`..=`to`: rows per song (by CCLI number, else title and
/// authors), most used first; uses oldest first.
pub fn report(uses: Vec<CcliUse>, from: &str, to: &str) -> CcliReport {
    let mut uses: Vec<CcliUse> = uses
        .into_iter()
        .filter(|u| u.date.as_str() >= from && u.date.as_str() <= to)
        .collect();
    uses.sort_by(|a, b| a.at.cmp(&b.at).then(a.title.cmp(&b.title)));
    let mut rows: BTreeMap<(String, String, String), CcliReportRow> = BTreeMap::new();
    for u in &uses {
        let k = if u.ccli_number.is_empty() {
            (
                String::new(),
                u.title.to_lowercase(),
                u.authors.to_lowercase(),
            )
        } else {
            (u.ccli_number.clone(), String::new(), String::new())
        };
        let r = rows.entry(k).or_insert_with(|| CcliReportRow {
            title: u.title.clone(),
            ccli_number: u.ccli_number.clone(),
            authors: u.authors.clone(),
            ..CcliReportRow::default()
        });
        r.count += 1;
        r.projected += u.projected as u32;
        r.streamed += u.streamed as u32;
        r.recorded += u.recorded as u32;
        if u.date > r.last_used {
            r.last_used = u.date.clone();
        }
    }
    let mut rows: Vec<CcliReportRow> = rows.into_values().collect();
    rows.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    CcliReport {
        from: from.into(),
        to: to.into(),
        rows,
        uses,
    }
}

/// A CSV field, quoted when it needs to be (and never read as a formula by a spreadsheet).
fn csv_field(v: &str) -> String {
    let v = if v.starts_with(['=', '+', '-', '@']) {
        format!("'{v}")
    } else {
        v.to_string()
    };
    if v.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v
    }
}

/// The report as CSV, in the columns CCLI's reporting asks for.
pub fn csv(r: &CcliReport) -> String {
    let mut out = String::from(
        "Song Title,CCLI Song #,Authors,Times Used,Projected,Streamed,Recorded,From,To\r\n",
    );
    for row in &r.rows {
        let cells = [
            csv_field(&row.title),
            csv_field(&row.ccli_number),
            csv_field(&row.authors),
            row.count.to_string(),
            row.projected.to_string(),
            row.streamed.to_string(),
            row.recorded.to_string(),
            r.from.clone(),
            r.to.clone(),
        ];
        out.push_str(&cells.join(","));
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;
    use model::SongInfo;

    fn song(title: &str, ccli: &str) -> Presentation {
        Presentation {
            name: format!("{title} (arr.)"),
            song_info: Some(SongInfo {
                title: Some(title.into()),
                ccli_number: (!ccli.is_empty()).then(|| ccli.into()),
                authors: Some("A. Writer, B. Composer".into()),
                ..SongInfo::default()
            }),
            ..Presentation::default()
        }
    }

    fn at(d: u32, h: u32) -> chrono::DateTime<chrono::Local> {
        chrono::Local
            .with_ymd_and_hms(2026, 11, d, h, 0, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn one_use_per_service_with_its_flags() {
        let dir = std::env::temp_dir().join(format!("ccli-{}", uuid::Uuid::new_v4()));
        let log = CcliLog::new(&dir);
        let a = song("Morning Light", "1234567");
        let b = song("River Song", "");
        let plain = Presentation::default();
        fn live<'a>(p: &'a Presentation, svc: Option<&str>, streamed: bool) -> Option<Live<'a>> {
            Some(Live {
                presentation: p,
                service: svc.map(str::to_string),
                projected: true,
                streamed,
                recorded: false,
            })
        }
        // Sunday: A on many slides, the stream starts mid-song; B; A again; a non-song.
        for _ in 0..5 {
            log.observe(live(&a, Some("Sunday 9am"), false), at(1, 9));
        }
        log.observe(live(&a, Some("Sunday 9am"), true), at(1, 9));
        log.observe(live(&b, Some("Sunday 9am"), true), at(1, 9));
        log.observe(live(&a, Some("Sunday 9am"), true), at(1, 10));
        log.observe(live(&plain, Some("Sunday 9am"), true), at(1, 10));
        log.observe(None, at(1, 10));
        // The second service the same day, and a rehearsal outside a playlist the day after.
        log.observe(live(&a, Some("Sunday 11am"), false), at(1, 11));
        log.observe(live(&a, None, false), at(2, 19));
        // Lines: A start, A streamed, B, A 11am, A ad hoc.
        let text = std::fs::read_to_string(dir.join("reports/ccli-usage.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 5, "{text}");

        // A fresh log (a restart) reads the same and adds nothing for a known use.
        let log = CcliLog::new(&dir);
        log.observe(live(&a, Some("Sunday 11am"), false), at(1, 11));
        let r = log.report("2026-11-01", "2026-11-30");
        assert_eq!(r.uses.len(), 4);
        assert_eq!(r.rows.len(), 2);
        let ra = &r.rows[0];
        assert_eq!(
            (
                ra.title.as_str(),
                ra.ccli_number.as_str(),
                ra.count,
                ra.streamed,
                ra.projected
            ),
            ("Morning Light", "1234567", 3, 1, 3)
        );
        assert_eq!(ra.last_used, "2026-11-02");
        assert_eq!(r.rows[1].title, "River Song");
        assert_eq!(log.report("2026-11-02", "2026-11-02").rows[0].count, 1);
        assert!(log.report("2026-12-01", "2026-12-31").rows.is_empty());

        let c = csv(&log.report("2026-11-01", "2026-11-30"));
        let lines: Vec<&str> = c.lines().collect();
        assert_eq!(
            lines[0],
            "Song Title,CCLI Song #,Authors,Times Used,Projected,Streamed,Recorded,From,To"
        );
        assert_eq!(
            lines[1],
            "Morning Light,1234567,\"A. Writer, B. Composer\",3,3,1,0,2026-11-01,2026-11-30"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn csv_fields_are_quoted_and_never_formulas() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a \"b\", c"), "\"a \"\"b\"\", c\"");
        assert_eq!(csv_field("=SUM(A1)"), "'=SUM(A1)");
    }
}
