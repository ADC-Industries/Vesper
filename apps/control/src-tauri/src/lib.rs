//! Desktop shell of the control app. The UI is the web frontend in `../src`; this shell only
//! adds what a browser can't do: finding servers on the LAN over mDNS, and the SongSelect
//! Downloads watcher's look at the user's Downloads folder (listing it, and reading a file to
//! tell whether it is a SongSelect song, by its content).

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use tauri::Manager;

/// A server found by mDNS, as the frontend's `discoverServers()` expects it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Discovered {
    id: String,
    name: String,
    version: String,
    protocol_version: u32,
    /// `http://<address>:<port>`, or `None` when the record carried no address.
    base_url: Option<String>,
}

/// Browses `_vesper._tcp` for `timeout_ms` (default 1.5 s) and returns what answered.
#[tauri::command]
async fn discover_servers(timeout_ms: Option<u64>) -> Result<Vec<Discovered>, String> {
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(1500).clamp(200, 10_000));
    let found = tauri::async_runtime::spawn_blocking(move || protocol::discover::discover(timeout))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    Ok(found
        .into_iter()
        .map(|s| {
            let base_url = s.base_url();
            Discovered {
                id: s.id,
                name: s.name,
                version: s.version,
                protocol_version: s.protocol_version,
                base_url,
            }
        })
        .collect())
}

/// A file in the Downloads folder, as the frontend's `listDownloads()` expects it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadFile {
    name: String,
    size: u64,
    /// Last written, ms since the Unix epoch.
    modified_ms: u64,
}

/// A download recognised as a SongSelect song (the frontend's `downloadedSong()`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadedSong {
    name: String,
    title: Option<String>,
    data: Vec<u8>,
}

/// Most files listed (a Downloads folder can hold thousands; the newest are kept).
const MAX_LISTED: usize = 5000;

/// The regular files directly in `dir` (not folders, not links), newest first.
fn list_files(dir: &Path) -> Result<Vec<DownloadFile>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out: Vec<DownloadFile> = entries
        .flatten()
        .filter_map(|e| {
            // (`DirEntry::metadata` doesn't follow a link.)
            let m = e.metadata().ok().filter(|m| m.is_file())?;
            let name = e.file_name().into_string().ok()?;
            let modified_ms = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as u64);
            Some(DownloadFile {
                name,
                size: m.len(),
                modified_ms,
            })
        })
        .collect();
    out.sort_by_key(|f| std::cmp::Reverse(f.modified_ms));
    out.truncate(MAX_LISTED);
    Ok(out)
}

/// `name` in `dir` if it is a SongSelect song: a plain file name (no folders), a regular file
/// no larger than a song file can be, recognised by its content.
fn read_song(dir: &Path, name: &str) -> Result<Option<DownloadedSong>, String> {
    let plain = !name.is_empty()
        && Path::new(name).file_name().and_then(|n| n.to_str()) == Some(name)
        && name != "."
        && name != "..";
    if !plain {
        return Err("not a file name".into());
    }
    let path = dir.join(name);
    let Ok(m) = std::fs::symlink_metadata(&path) else {
        return Ok(None);
    };
    if !m.is_file() || m.len() > model::songselect::MAX_FILE_BYTES as u64 {
        return Ok(None);
    }
    let Ok(data) = std::fs::read(&path) else {
        return Ok(None);
    };
    Ok(model::songselect::recognise(&data).map(|r| DownloadedSong {
        name: name.to_string(),
        title: r.title,
        data,
    }))
}

fn downloads_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .download_dir()
        .map_err(|e| format!("no Downloads folder: {e}"))
}

/// The files in the user's Downloads folder.
#[tauri::command]
async fn downloads_list(app: tauri::AppHandle) -> Result<Vec<DownloadFile>, String> {
    let dir = downloads_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || list_files(&dir))
        .await
        .map_err(|e| e.to_string())?
}

/// A file of the Downloads folder if it is a SongSelect song, else `None`.
#[tauri::command]
async fn downloads_song(
    app: tauri::AppHandle,
    name: String,
) -> Result<Option<DownloadedSong>, String> {
    let dir = downloads_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || read_song(&dir, &name))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            discover_servers,
            downloads_list,
            downloads_song
        ])
        .run(tauri::generate_context!())
        .expect("error while running the control app");
}

#[cfg(test)]
mod tests {
    use super::*;

    const SONG: &str = "Paper Boats\n\nVerse 1\nPaper boats upon the stream\n\nRiley Placeholder | Casey Invented\nCCLI Song # 7000001\n\u{a9} 2004 Imaginary Hymns\nFor use solely with the SongSelect Terms of Use. All rights reserved. www.ccli.com\nCCLI License # 1112223\n";

    #[test]
    fn a_downloaded_songselect_file_is_listed_and_recognised() {
        let dir = std::env::temp_dir().join(format!("oriel-downloads-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a folder")).unwrap();
        std::fs::write(dir.join("download (3).txt"), SONG).unwrap();
        std::fs::write(dir.join("notes.txt"), "Milk\nBread\n").unwrap();
        let names: Vec<String> = list_files(&dir)
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert!(
            names.contains(&"download (3).txt".to_string())
                && !names.contains(&"a folder".to_string())
        );
        let s = read_song(&dir, "download (3).txt").unwrap().unwrap();
        assert_eq!(s.title.as_deref(), Some("Paper Boats"));
        assert!(read_song(&dir, "notes.txt").unwrap().is_none());
        assert!(read_song(&dir, "missing.txt").unwrap().is_none());
        assert!(read_song(&dir, "../notes.txt").is_err());
        assert!(read_song(&dir, "a folder").unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
