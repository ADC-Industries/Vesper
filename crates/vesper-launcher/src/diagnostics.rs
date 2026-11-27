//! The diagnostics page (`GET /launcher/v1/diagnostics`): the bundled `idn doctor` run and
//! explained in plain language, with the Launcher's and the products' status, and a text
//! report for support ("Copy report") with keys and tokens redacted.
//!
//! `idn doctor` prints sections (a line of its own: `Network`, `Sources`, …), each with lines
//! marked `✔` (fine), `⚠` (worth checking) or `✖` (a problem), indented notes, and a last
//! summary line. Its words stay as they are (they say what to do); each section gets a sentence
//! saying what it checks.

use crate::launcher::Launcher;
use serde::Serialize;
use std::time::Duration;

/// How long `idn doctor` listens (`--seconds`).
const DOCTOR_SECONDS: u32 = 3;
/// How long it may take in all.
const DOCTOR_TIMEOUT: Duration = Duration::from_secs(45);

/// A line of a doctor section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorItem {
    /// `ok`, `warning`, `problem` or `info`.
    pub level: &'static str,
    /// Its text.
    pub text: String,
}

/// A doctor section.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSection {
    /// Its title, as the doctor prints it.
    pub title: String,
    /// What it checks, in plain language (`None`: a section this page doesn't know).
    pub about: Option<&'static str>,
    /// Its lines.
    pub items: Vec<DoctorItem>,
}

/// What `idn doctor` found.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Doctor {
    /// It ran (the bundled `idn` is there and started).
    pub ran: bool,
    /// Its exit code.
    pub exit_code: Option<i32>,
    /// Its sections.
    pub sections: Vec<DoctorSection>,
    /// Its last line (`No problems, 1 thing(s) worth checking.`).
    pub summary: Option<String>,
    /// The headline for people.
    pub headline: String,
    /// `ok`, `warning` or `problem`.
    pub level: &'static str,
    /// Why it couldn't run.
    pub problem: Option<String>,
    /// Its whole output (redacted).
    pub output: String,
}

/// The diagnostics page.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    /// When it ran (RFC 3339).
    pub ran_at: String,
    /// `idn doctor`.
    pub doctor: Doctor,
    /// The Launcher and its products, in plain language: `(label, value, problem)`.
    pub launcher: Vec<(String, String, bool)>,
    /// The text report for support (keys and tokens redacted).
    pub report: String,
}

/// What a doctor section checks.
fn about(title: &str) -> Option<&'static str> {
    let t = title.to_ascii_lowercase();
    Some(if t.starts_with("network") {
        "Whether this machine has a working network connection IDN can use (a wired one is best) and a route for multicast."
    } else if t.starts_with("shared network") {
        "Whether other devices on this network (phones, TVs, smart speakers) could disturb video: many of them can flood it with announcements."
    } else if t.starts_with("sources") {
        "Which cameras, screens and senders this machine can see on the network."
    } else if t.contains("node") {
        "Whether this machine's IDN Node (the service that joins it to the site) runs and is in the site."
    } else if t.contains("site") || t.contains("discovery") {
        "Whether the site's server can be reached from here."
    } else if t.contains("clock") || t.contains("time") {
        "Whether this machine's clock agrees with the site's, so video and sound stay in step."
    } else if t.contains("firewall") || t.contains("port") {
        "Whether the network ports IDN needs are open on this machine."
    } else if t.contains("encrypt") || t.contains("passphrase") {
        "Whether encrypted video can be sent and received here."
    } else if t.contains("test") {
        "A test connection to a source."
    } else {
        return None;
    })
}

/// Parses `idn doctor`'s output.
pub fn parse_doctor(out: &str) -> (Vec<DoctorSection>, Option<String>) {
    let mut sections: Vec<DoctorSection> = Vec::new();
    let mut summary = None;
    let lines: Vec<&str> = out.lines().collect();
    let last = lines.iter().rposition(|l| !l.trim().is_empty());
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        let t = line.trim();
        let (level, text) = match t.chars().next() {
            Some('✔' | '✓') => ("ok", &t[t.chars().next().map_or(0, char::len_utf8)..]),
            Some('⚠') => ("warning", &t[t.chars().next().map_or(0, char::len_utf8)..]),
            Some('✖' | '✗' | '✘' | '❌' | '×') => {
                ("problem", &t[t.chars().next().map_or(0, char::len_utf8)..])
            }
            _ => ("info", t),
        };
        let text = text.trim_start_matches('\u{fe0f}').trim().to_string();
        if !indented && level == "info" {
            if Some(i) == last && !sections.is_empty() {
                summary = Some(text);
            } else {
                sections.push(DoctorSection {
                    about: about(&text),
                    title: text,
                    items: Vec::new(),
                });
            }
            continue;
        }
        if sections.is_empty() {
            sections.push(DoctorSection {
                title: "Checks".into(),
                about: None,
                items: Vec::new(),
            });
        }
        if let Some(s) = sections.last_mut() {
            s.items.push(DoctorItem { level, text });
        }
    }
    (sections, summary)
}

/// Hides keys, tokens and passphrases in text for a report: `Bearer …`, `token=…`,
/// `key: …`, `passphrase=…`, `password=…`, long runs of key-like characters, and the digits of
/// a join code after `join` (`join 482913`, `join 482 913`).
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (n, line) in text.lines().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let mut words: Vec<String> = Vec::new();
        let mut hide_next = false;
        let mut after_join = false;
        for w in line.split(' ') {
            let lower = w.to_ascii_lowercase();
            let digits = w.trim_matches(|c: char| !c.is_ascii_alphanumeric());
            if after_join && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
                words.push("[redacted]".into());
                continue;
            }
            after_join = lower.trim_matches(|c: char| !c.is_ascii_alphanumeric()) == "join";
            if hide_next && !w.is_empty() {
                words.push("[redacted]".into());
                hide_next = false;
                continue;
            }
            let secret_word = ["token", "key", "passphrase", "password", "secret", "bearer"]
                .iter()
                .any(|s| lower.trim_end_matches([':', '=']) == *s);
            if secret_word && (lower.ends_with(':') || lower == "bearer") {
                words.push(w.to_string());
                hide_next = true;
                continue;
            }
            if let Some((k, _)) = w.split_once('=')
                && ["token", "key", "passphrase", "password", "secret"]
                    .iter()
                    .any(|s| k.to_ascii_lowercase().ends_with(s))
            {
                words.push(format!("{k}=[redacted]"));
                continue;
            }
            let keyish = w.len() >= 32
                && w.chars().all(|c| {
                    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_' | '.')
                })
                && w.chars().any(|c| c.is_ascii_digit())
                && w.chars().any(|c| c.is_ascii_alphabetic())
                && !w.contains("://");
            if keyish {
                words.push("[redacted]".into());
            } else {
                words.push(w.to_string());
            }
        }
        out.push_str(&words.join(" "));
    }
    out
}

fn headline(sections: &[DoctorSection]) -> (String, &'static str) {
    let count = |l: &str| {
        sections
            .iter()
            .flat_map(|s| &s.items)
            .filter(|i| i.level == l)
            .count()
    };
    match (count("problem"), count("warning")) {
        (0, 0) => ("Everything IDN needs on this machine works.".into(), "ok"),
        (0, 1) => (
            "No problems; one thing is worth checking (below).".into(),
            "warning",
        ),
        (0, w) => (
            format!("No problems; {w} things are worth checking (below)."),
            "warning",
        ),
        (1, _) => (
            "One problem needs fixing before video works well here (below).".into(),
            "problem",
        ),
        (p, _) => (
            format!("{p} problems need fixing before video works well here (below)."),
            "problem",
        ),
    }
}

/// Runs the bundled `idn doctor`.
pub async fn doctor(l: &Launcher) -> Doctor {
    let idn = l.config().system.idn_program("idn");
    if !idn.is_file() {
        return Doctor {
            headline: "The network check can't run here.".into(),
            level: "warning",
            problem: Some(if cfg!(windows) {
                format!(
                    "the IDN tools aren't installed with the Launcher ({} is missing): run the Vesper Launcher installer again",
                    idn.display()
                )
            } else {
                format!(
                    "the IDN tools aren't installed ({} is missing): install the vesper-idn-node package",
                    idn.display()
                )
            }),
            ..Default::default()
        };
    }
    let run = tokio::time::timeout(
        DOCTOR_TIMEOUT,
        tokio::process::Command::new(&idn)
            .args(["doctor", "--seconds", &DOCTOR_SECONDS.to_string()])
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    let out = match run {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => {
            return Doctor {
                headline: "The network check didn't start.".into(),
                level: "warning",
                problem: Some(format!("{} didn't start: {e}", idn.display())),
                ..Default::default()
            };
        }
        Err(_) => {
            return Doctor {
                headline: "The network check didn't finish.".into(),
                level: "warning",
                problem: Some(format!(
                    "idn doctor didn't finish within {} s",
                    DOCTOR_TIMEOUT.as_secs()
                )),
                ..Default::default()
            };
        }
    };
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        text.push_str(&err);
    }
    let text = redact(&text);
    let (sections, summary) = parse_doctor(&text);
    let (mut headline, mut level) = headline(&sections);
    if !out.status.success() && level == "ok" {
        headline = "The network check ended with an error (its output is below).".into();
        level = "warning";
    }
    Doctor {
        ran: true,
        exit_code: out.status.code(),
        sections,
        summary,
        headline,
        level,
        problem: None,
        output: text,
    }
}

fn state_words(s: protocol::ProductState) -> &'static str {
    use protocol::ProductState as S;
    match s {
        S::Running => "running",
        S::Stopped => "stopped",
        S::Starting => "starting",
        S::Updating => "updating",
        S::RollingBack => "rolling back",
        S::Failed => "not running (it failed to start or stopped by itself)",
    }
}

/// The diagnostics: `idn doctor` and the Launcher's status.
pub async fn run(l: &std::sync::Arc<Launcher>) -> Diagnostics {
    let doctor = doctor(l).await;
    let l2 = l.clone();
    let o = tokio::task::spawn_blocking(move || l2.overview())
        .await
        .ok();
    let mut rows: Vec<(String, String, bool)> = Vec::new();
    rows.push((
        "Launcher".into(),
        format!("version {} on {}", crate::VERSION, l.platform()),
        false,
    ));
    if let Some(o) = &o {
        let s = &o.status;
        rows.push(("Machine".into(), s.machine_name.clone(), false));
        let roles = if s.roles.is_empty() {
            "none chosen".to_string()
        } else {
            s.roles.join(", ")
        };
        rows.push(("Roles".into(), roles, false));
        if let Some(r) = &o.setup.role_status.run {
            let failed = r.steps.iter().find(|x| x.state == "failed");
            let v = match (r.state.as_str(), failed) {
                ("done", _) => "made true on this machine".to_string(),
                ("applying", _) => "being set up now".to_string(),
                (_, Some(f)) => format!(
                    "{} failed: {}",
                    f.label,
                    f.message.clone().unwrap_or_default()
                ),
                _ => r.state.clone(),
            };
            rows.push(("Setting up the roles".into(), v, r.state == "failed"));
        }
        let (site, bad) = match &o.setup.join {
            crate::setup::JoinState::Joined {
                server_name,
                server,
                ..
            } => match &o.channel.problem {
                Some(p) => (
                    format!("in the site ({server_name}, {server}), but reports fail: {p}"),
                    true,
                ),
                None => (format!("in the site ({server_name}, {server})"), false),
            },
            crate::setup::JoinState::Waiting { server, .. } => (
                format!("waiting for an admin's approval at {server}"),
                false,
            ),
            crate::setup::JoinState::NotJoined { problem } => match problem {
                Some(p) => (format!("not in a site: {p}"), true),
                None => ("not in a site".into(), false),
            },
        };
        rows.push(("Site".into(), site, bad));
        for p in &s.products {
            let mut v = format!(
                "{}, {}",
                p.version
                    .as_deref()
                    .map_or("version not known".to_string(), |v| format!("version {v}")),
                state_words(p.state)
            );
            if p.restarts > 0 {
                v.push_str(&format!(", restarted {} times", p.restarts));
            }
            if let Some(u) = &p.last_update
                && u.result != protocol::UpdateResult::Installed
            {
                v.push_str(&format!("; the last update to {}: {}", u.to, u.message));
            }
            rows.push((p.name.clone(), v, p.state == protocol::ProductState::Failed));
        }
        if let Some((v, bad)) = l.idn_node_view() {
            rows.push(("IDN Node".into(), v, bad));
        }
        let h = &s.health;
        let gib = |b: u64| format!("{:.1} GB", b as f64 / 1e9);
        if let (Some(t), Some(f)) = (h.disk_total, h.disk_free) {
            rows.push((
                "Disk".into(),
                format!("{} free of {}", gib(f), gib(t)),
                t > 0 && f * 20 < t,
            ));
        }
        if let (Some(t), Some(a)) = (h.memory_total, h.memory_available) {
            rows.push((
                "Memory".into(),
                format!("{} available of {}", gib(a), gib(t)),
                false,
            ));
        }
    }
    let ran_at = crate::launcher::now();
    let mut report = format!("Vesper Launcher diagnostics, {ran_at}\n\n");
    for (k, v, bad) in &rows {
        report.push_str(&format!("{k}: {v}{}\n", if *bad { "  (!)" } else { "" }));
    }
    report.push_str(&format!("\nidn doctor: {}\n", doctor.headline));
    if let Some(p) = &doctor.problem {
        report.push_str(&format!("{p}\n"));
    }
    if !doctor.output.is_empty() {
        report.push('\n');
        report.push_str(&doctor.output);
        report.push('\n');
    }
    Diagnostics {
        ran_at,
        doctor,
        launcher: rows,
        report: redact(&report),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Network\n    2 virtual interface(s) ignored\n  ✔ eth0 192.0.2.20 (1000 Mbit/s): wired\n  ⚠ wifi0 192.0.2.21: wireless, video may stutter\n\nSources\n  ✖ no route for multicast\n\n1 problem, 1 thing(s) worth checking.\n";

    #[test]
    fn doctor_output_becomes_sections_and_a_headline() {
        let (s, summary) = parse_doctor(SAMPLE);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].title, "Network");
        assert!(s[0].about.is_some());
        assert_eq!(
            s[0].items.iter().map(|i| i.level).collect::<Vec<_>>(),
            ["info", "ok", "warning"]
        );
        assert_eq!(s[0].items[1].text, "eth0 192.0.2.20 (1000 Mbit/s): wired");
        assert_eq!(s[1].items[0].level, "problem");
        assert_eq!(
            summary.as_deref(),
            Some("1 problem, 1 thing(s) worth checking.")
        );
        let (h, level) = headline(&s);
        assert_eq!(level, "problem");
        assert!(h.starts_with("One problem"), "{h}");
    }

    #[test]
    fn reports_hide_keys_and_tokens() {
        let t = "Authorization: Bearer eyJhbGciOiJFZERTQSJ9.eyJzdWIiOiJrZXk6MTIzIn0.c2lnbmF0dXJl\npassphrase=hunter2 name=Booth\nkey: abc\nfingerprint 3f9a1c0d5e7b2a4f6c8e0d1b3a5f7c9e2b4d6f8a ok\nhttp://192.0.2.10:5555/api/v1/server";
        let r = redact(t);
        assert!(!r.contains("eyJ"), "{r}");
        assert!(!r.contains("hunter2"), "{r}");
        assert!(r.contains("name=Booth"), "{r}");
        assert!(r.contains("key: [redacted]"), "{r}");
        assert!(!r.contains("3f9a1c0d5e7b"), "{r}");
        assert!(r.contains("http://192.0.2.10:5555/api/v1/server"), "{r}");
        let r = redact("`idn-node join 482913` failed\nidn-node join 482 913 / joined 3 screens");
        assert!(!r.contains("482"), "{r}");
        assert!(!r.contains("913"), "{r}");
        assert!(r.contains("joined 3 screens"), "{r}");
    }
}
