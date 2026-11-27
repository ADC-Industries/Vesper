//! Data feeds through IDN's `idn-feeds` (a Google Sheet's link or its published CSV, CSV, TSV,
//! JSON, XML or RSS, from an address or a file on the server): Vesper schedules the reads,
//! keeps the last good rows and maps columns to a feed slide's fields.
//!
//! A feed is read from an `http://` or `https://` address (not this machine's loopback or
//! link-local addresses), or from a file in the signage feeds folder `<data>/signage-feeds/`
//! ([`FOLDER`]), named by its name there: never from anywhere else on the server (H02-5).

use protocol::{SignageFeed, SignageMapping};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Component, Path};

/// The folder in the data directory feed files are read from.
pub const FOLDER: &str = "signage-feeds";

/// A row: its values by column name.
pub type Row = BTreeMap<String, String>;

/// Rows kept per feed (and answered).
pub const KEPT_ROWS: usize = 50;

/// The feed's format as IDN names it.
pub fn format(name: &str) -> Result<idn_feeds::Format, String> {
    idn_feeds::Format::from_name(name.trim())
        .ok_or_else(|| format!("unknown format {name:?}: auto, csv, tsv, json or xml"))
}

/// Where a feed's `url` is read from: the address, or the file's full path in `files` (the
/// signage feeds folder); refused when it is neither (see the module docs).
pub fn source(url: &str, files: &Path) -> Result<String, String> {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    if let Some(scheme) = ["http://", "https://"]
        .into_iter()
        .find(|s| lower.starts_with(s))
    {
        let rest = &u[scheme.len()..];
        let host = host_of(rest);
        if host.is_empty() {
            return Err("the address has no host".into());
        }
        if own_host(host) {
            return Err(format!(
                "{host} is this machine: a feed is read from elsewhere"
            ));
        }
        return Ok(format!("{scheme}{rest}"));
    }
    let outside = || {
        format!(
            "a feed is an http or https address, or a file in the {FOLDER} folder of the \
             server's data directory"
        )
    };
    if lower.contains("://") {
        return Err(outside());
    }
    let p = Path::new(u);
    let rel = if p.is_absolute() {
        let d = files.canonicalize().unwrap_or_else(|_| files.to_path_buf());
        let full = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        full.strip_prefix(&d)
            .or_else(|_| p.strip_prefix(files))
            .map_err(|_| outside())?
            .to_path_buf()
    } else {
        p.to_path_buf()
    };
    if rel.as_os_str().is_empty() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(outside());
    }
    let full = files.join(&rel);
    // A link inside the folder that leads out of it is outside too.
    if let (Ok(f), Ok(d)) = (full.canonicalize(), files.canonicalize())
        && !f.starts_with(&d)
    {
        return Err(outside());
    }
    Ok(full.display().to_string())
}

/// The host of an address after its scheme (without user, port, path).
fn host_of(rest: &str) -> &str {
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let auth = &rest[..end];
    let auth = auth.rsplit_once('@').map_or(auth, |(_, h)| h);
    if let Some(v6) = auth.strip_prefix('[') {
        return v6.split(']').next().unwrap_or_default();
    }
    auth.split(':').next().unwrap_or_default()
}

/// This machine's own addresses (loopback, link-local, unspecified, `localhost`), and
/// numbers that aren't written as a plain address (`127.1`, `2130706433`).
fn own_host(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") {
        return true;
    }
    match h.parse::<IpAddr>() {
        Ok(IpAddr::V4(a)) => a.is_loopback() || a.is_link_local() || a.is_unspecified(),
        Ok(IpAddr::V6(a)) => {
            a.is_loopback()
                || a.is_unspecified()
                || (a.segments()[0] & 0xffc0) == 0xfe80
                || a.to_ipv4_mapped()
                    .is_some_and(|v| v.is_loopback() || v.is_link_local() || v.is_unspecified())
        }
        Err(_) => {
            h.chars().all(|c| c.is_ascii_digit() || c == '.')
                || h.starts_with("0x")
                || h.contains(".0x")
        }
    }
}

/// Reads a feed now (blocking), its file from `files` (the signage feeds folder): its rows
/// (at most [`KEPT_ROWS`]) and columns.
pub fn read(feed: &SignageFeed, files: &Path) -> Result<(Vec<Row>, Vec<String>), String> {
    let f = format(&feed.format)?;
    let from = source(&feed.url, files)?;
    let (mut rows, mut columns) = idn_feeds::read_table(&from, f).map_err(|e| e.to_string())?;
    rows.truncate(KEPT_ROWS);
    if columns.is_empty() {
        // JSON and XML rows keep name order: the columns are every name seen.
        for r in &rows {
            for k in r.keys() {
                if !columns.contains(k) {
                    columns.push(k.clone());
                }
            }
        }
    }
    Ok((rows, columns))
}

/// One row through the mapping: (when, title, body). A mapping left empty takes the first
/// column for the title.
pub fn mapped(row: &Row, m: &SignageMapping, columns: &[String]) -> (String, String, String) {
    let get = |c: &str| {
        if c.is_empty() {
            String::new()
        } else {
            row.get(c).cloned().unwrap_or_default()
        }
    };
    let mut title = get(&m.title);
    if m.title.is_empty() && m.body.is_empty() {
        title = columns
            .first()
            .and_then(|c| row.get(c))
            .cloned()
            .unwrap_or_default();
    }
    (get(&m.when), title, get(&m.body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str, name: &str) -> (Vec<Row>, Vec<String>) {
        idn_feeds::parse_table(text, idn_feeds::Format::Auto, name).unwrap()
    }

    #[test]
    fn csv_tsv_json_and_rss_read_as_rows() {
        let (rows, cols) = parse("Date,Event,Room\nSun 9:00,Breakfast,Hall\n", "events.csv");
        assert_eq!(cols, ["Date", "Event", "Room"]);
        assert_eq!(rows[0]["Event"], "Breakfast");
        let (rows, _) = parse("Date\tEvent\nMon\tChoir\n", "x.tsv");
        assert_eq!(rows[0]["Event"], "Choir");
        let (rows, _) = parse(r#"[{"title":"Youth night","when":"Fri"}]"#, "x.json");
        assert_eq!(rows[0]["title"], "Youth night");
        let rss = r#"<?xml version="1.0"?><rss><channel><title>News</title>
            <item><title>Food drive</title><description>Bring cans</description></item>
            <item><title>Picnic</title></item></channel></rss>"#;
        let (rows, _) = parse(rss, "news.rss");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["title"], "Food drive");
    }

    #[test]
    fn columns_map_to_fields() {
        let (rows, cols) = parse("When,What,Notes\nSun,Picnic,Bring a chair\n", "e.csv");
        let m = SignageMapping {
            title: "What".into(),
            body: "Notes".into(),
            when: "When".into(),
        };
        assert_eq!(
            mapped(&rows[0], &m, &cols),
            ("Sun".into(), "Picnic".into(), "Bring a chair".into())
        );
        // Nothing mapped: the first column is the title.
        assert_eq!(mapped(&rows[0], &SignageMapping::default(), &cols).1, "Sun");
    }

    #[test]
    fn a_file_on_the_server_is_read_and_a_missing_one_fails() {
        let dir = std::env::temp_dir().join(format!("vesper-feed-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("events.csv");
        std::fs::write(&p, "Event,When\nPotluck,Sunday\n").unwrap();
        let mut feed = SignageFeed {
            id: "f".into(),
            name: "Events".into(),
            url: "events.csv".into(),
            format: "auto".into(),
            refresh: 60,
            mapping: SignageMapping::default(),
            columns: vec![],
            rows: vec![],
            read_at: None,
            problem: None,
        };
        let (rows, cols) = read(&feed, &dir).unwrap();
        assert_eq!((rows.len(), cols[0].as_str()), (1, "Event"));
        // Its full path in the folder works too.
        feed.url = p.display().to_string();
        assert_eq!(read(&feed, &dir).unwrap().0.len(), 1);
        feed.url = "gone.csv".into();
        assert!(read(&feed, &dir).is_err());
        feed.format = "pdf".into();
        assert!(read(&feed, &dir).unwrap_err().contains("unknown format"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H02-5: no file outside the signage feeds folder, no address of this machine.
    #[test]
    fn feeds_come_from_their_folder_or_from_elsewhere_on_the_network() {
        let root = std::env::temp_dir().join(format!("vesper-feeds-{}", uuid::Uuid::new_v4()));
        let dir = root.join(FOLDER);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(root.join("secret.json"), "[{\"a\":\"b\"}]").unwrap();
        std::fs::write(dir.join("ok.csv"), "A\n1\n").unwrap();
        let ok = |u: &str| source(u, &dir);
        assert!(ok("ok.csv").is_ok());
        assert!(ok("sub/ok.csv").is_ok(), "a sub-folder is inside");
        for bad in [
            "../secret.json",
            "sub/../../secret.json",
            &root.join("secret.json").display().to_string(),
            "/etc/passwd",
            "file:///etc/passwd",
            "ftp://example.org/a.csv",
            "http://localhost:7788/api/v1/site",
            "http://127.0.0.1/a.csv",
            "https://user@127.0.0.1:80/a.csv",
            "http://[::1]/a.csv",
            "http://169.254.169.254/latest",
            "http://0.0.0.0/",
            "http://127.1/",
            "http://2130706433/",
            "http://LOCALHOST./a",
            "http:///a.csv",
        ] {
            assert!(ok(bad).is_err(), "{bad} is refused");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("secret.json"), dir.join("link.json")).unwrap();
            assert!(ok("link.json").is_err(), "a link out of the folder");
        }
        assert_eq!(
            ok("HTTPS://docs.example.org/x.csv").unwrap(),
            "https://docs.example.org/x.csv"
        );
        assert!(
            ok("http://192.0.2.10:8080/events.json").is_ok(),
            "the LAN is fine"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
