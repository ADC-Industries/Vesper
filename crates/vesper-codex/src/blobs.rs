//! Media files of the Codex by their SHA-256 (`<sha256>.<ext>`): library items, and the old
//! store's managed files (`<media_dir>/<sha256>.<ext>`); their thumbnails (`codex/thumbs/`).

use crate::Shared;
use model::{AssetLocation, MediaKind};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Width of the stored media thumbnails, px.
pub const THUMB_WIDTH: u32 = 320;

/// JPEG quality of thumbnails (the stored media thumbnails and the server's slide thumbnails).
pub const THUMB_QUALITY: u8 = 82;

/// Picture endings Vesper plays (lowercase, without the dot).
pub const IMAGE_ENDINGS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "jfif", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif", "tga",
];
/// Video endings Vesper plays.
pub const VIDEO_ENDINGS: &[&str] = &[
    "mp4", "m4v", "mov", "avi", "wmv", "mkv", "webm", "mpg", "mpeg", "mts", "m2ts", "ts", "flv",
    "mxf", "ogv", "3gp",
];
/// Sound endings Vesper plays.
pub const AUDIO_ENDINGS: &[&str] = &[
    "mp3", "m4a", "wav", "aac", "aif", "aiff", "flac", "ogg", "wma", "opus",
];

/// Image, video or audio, from a file extension (lowercase, without the dot): Vesper's own
/// endings, else those the media library takes (`idn_open_protocol::library::kind_of`); `None`
/// for a file that isn't media.
pub fn kind_for(ext: &str) -> Option<MediaKind> {
    if IMAGE_ENDINGS.contains(&ext) {
        return Some(MediaKind::Image);
    }
    if VIDEO_ENDINGS.contains(&ext) {
        return Some(MediaKind::Video);
    }
    if AUDIO_ENDINGS.contains(&ext) {
        return Some(MediaKind::Audio);
    }
    if ext.is_empty() {
        return None;
    }
    Some(
        match idn_open_protocol::library::kind_of(&format!("x.{ext}"))? {
            "picture" => MediaKind::Image,
            "video" => MediaKind::Video,
            "sound" => MediaKind::Audio,
            _ => return None,
        },
    )
}

/// A file name's stem and lowercase extension (`Sunrise.JPG` → `Sunrise`, `jpg`).
pub fn split_name(name: &str) -> (String, String) {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_string(), ext.to_ascii_lowercase()),
        _ => (base.to_string(), String::new()),
    }
}

/// 64 lowercase hex digits.
pub fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The parts of a blob's file name (`<sha>.<ext>`), when it is one: letters, digits, `-`, `_`
/// before the dot, lowercase letters and digits after it (no paths).
pub fn parse_blob_name(file: &str) -> Option<(String, String)> {
    let (sha, ext) = file.rsplit_once('.')?;
    let ok_sha = !sha.is_empty()
        && sha.len() <= 128
        && sha
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    let ok_ext = !ext.is_empty()
        && ext.len() <= 16
        && ext
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    (ok_sha && ok_ext).then(|| (sha.to_string(), ext.to_string()))
}

/// A blob's file name.
pub fn blob_name(sha: &str, ext: &str) -> String {
    if ext.is_empty() {
        sha.to_string()
    } else {
        format!("{sha}.{ext}")
    }
}

/// The blob of a managed location (`sha256`, `extension`), as a file name.
pub fn managed_name(location: &AssetLocation) -> Option<String> {
    match location {
        AssetLocation::Managed { sha256, extension } => Some(blob_name(sha256, extension)),
        _ => None,
    }
}

/// The name a media file of the Codex is known by (`<sha256>.<ext>`: thumbnails, the blob
/// routes): a managed file's, or a library item's (its SHA-256, the ending of its file `path`).
pub fn stored_name(location: &AssetLocation, path: &Path) -> Option<String> {
    match location {
        AssetLocation::Library { sha256, .. } if is_sha256(sha256) => {
            let name = path.file_name()?.to_string_lossy().to_string();
            Some(blob_name(sha256, &split_name(&name).1))
        }
        _ => managed_name(location),
    }
}

/// Every collection's media folder (each once): where blobs live.
pub fn media_dirs(sh: &Shared) -> Vec<PathBuf> {
    let codex = sh.codex();
    let mut dirs = BTreeSet::new();
    dirs.insert(codex.layout.media());
    for c in &codex.collections {
        if let Some(content) = sh.collection_content(c.collection.id) {
            dirs.insert(content.media_dir.clone());
        }
    }
    dirs.into_iter().collect()
}

/// A media file of the Codex by its `<sha256>.<ext>` name: the library's item with that
/// SHA-256, else a managed file of the old store.
pub fn find_blob(sh: &Shared, file: &str) -> Option<PathBuf> {
    if let Some((sha, _)) = parse_blob_name(file)
        && let Some((_, p)) = sh.library().by_sha(&sha)
        && p.is_file()
    {
        return Some(p);
    }
    media_dirs(sh)
        .into_iter()
        .map(|d| d.join(file))
        .find(|p| p.is_file())
}

/// `codex/thumbs`.
pub fn thumbs_dir(codex_dir: &Path) -> PathBuf {
    codex_dir.join("thumbs")
}

/// A blob's stored thumbnail (`codex/thumbs/<sha>-<w>.jpg`).
pub fn thumb_path(codex_dir: &Path, sha: &str, width: u32) -> PathBuf {
    thumbs_dir(codex_dir).join(format!("{sha}-{width}.jpg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            split_name("Backgrounds/Sun rise.JPG"),
            ("Sun rise".into(), "jpg".into())
        );
        assert_eq!(split_name("noext"), ("noext".into(), String::new()));
        assert_eq!(split_name(".hidden"), (".hidden".into(), String::new()));
        assert_eq!(kind_for("mov"), Some(MediaKind::Video));
        assert_eq!(kind_for("txt"), None);
        // The media library's own endings too.
        assert_eq!(kind_for("avif"), Some(MediaKind::Image));
        assert_eq!(kind_for("caf"), Some(MediaKind::Audio));
        assert_eq!(kind_for(""), None);
        super::super::library::take_vesper_endings();
        assert!(
            super::super::library::takes("Walk in.jpe"),
            "Vesper's endings, taken"
        );
        let sha = "ab".repeat(32);
        assert!(is_sha256(&sha) && !is_sha256(&sha.to_uppercase()) && !is_sha256("ab"));
        assert_eq!(
            parse_blob_name(&format!("{sha}.png")),
            Some((sha.clone(), "png".into()))
        );
        for bad in ["../x.png", "a/b.png", "x.PNG", "x.", ".png", "x"] {
            assert_eq!(parse_blob_name(bad), None, "{bad}");
        }
    }
}
