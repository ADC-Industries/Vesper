//! The layout of a ProPresenter 7 data folder (backup) and read-only file discovery.
//!
//! ```text
//! <root>/Libraries/LibraryData                  rv.data.Library (library order)
//! <root>/Libraries/<library>/<name>.pro         rv.data.Presentation
//! <root>/Playlists/{Library,Media,Audio}        rv.data.PlaylistDocument
//! <root>/Themes/[<folder>/...]<theme>/Theme     rv.data.Template.Document (+ sibling Assets/)
//! <root>/Configuration/<Name>                   Workspace, Groups, Timers, Stage, Props, ...
//! <root>/Media/{Assets,Import}/...              media files (never decoded, never copied)
//! ```
//! All discovery is sorted by file name (byte order) so imports are deterministic. Every returned
//! path is relative to the root, `/`-separated ([`normalize_rel`]), and comes with the real path
//! on disk ([`Found`]): a name that is not valid UTF-8 (a Windows zip without the UTF-8 flag,
//! extracted on Linux) only has a lossy text form, which can't be turned back into the path.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

/// `Libraries/`.
pub const LIBRARIES: &str = "Libraries";
/// `Libraries/LibraryData` (ordered library list).
pub const LIBRARY_DATA: &str = "Libraries/LibraryData";
/// `Themes/`.
pub const THEMES: &str = "Themes";
/// File name of a theme document inside its folder.
pub const THEME_FILE: &str = "Theme";
/// `Playlists/Library` (service playlists).
pub const PLAYLISTS_LIBRARY: &str = "Playlists/Library";
/// `Playlists/Media` (media bin).
pub const PLAYLISTS_MEDIA: &str = "Playlists/Media";
/// `Playlists/Audio` (audio bin / playlists).
pub const PLAYLISTS_AUDIO: &str = "Playlists/Audio";

/// `Configuration/` documents.
pub mod config {
    /// `rv.data.ProPresenterWorkspace`.
    pub const WORKSPACE: &str = "Configuration/Workspace";
    /// `rv.data.ProGroupsDocument`.
    pub const GROUPS: &str = "Configuration/Groups";
    /// `rv.data.TimersDocument`.
    pub const TIMERS: &str = "Configuration/Timers";
    /// `rv.data.Stage.Document`.
    pub const STAGE: &str = "Configuration/Stage";
    /// `rv.data.ClearGroupsDocument`.
    pub const CLEAR_GROUPS: &str = "Configuration/ClearGroups";
    /// `rv.data.PropDocument`.
    pub const PROPS: &str = "Configuration/Props";
    /// `rv.data.MessageDocument`.
    pub const MESSAGES: &str = "Configuration/Messages";
    /// `rv.data.MacrosDocument`.
    pub const MACROS: &str = "Configuration/Macros";
    /// `rv.data.CCLIDocument`.
    pub const CCLI: &str = "Configuration/CCLI";
    /// `rv.data.KeyMappingDocument` (report only).
    pub const KEY_MAPPINGS: &str = "Configuration/KeyMappings";
    /// `rv.data.Calendar` (report only).
    pub const CALENDAR: &str = "Configuration/Calendar";
}

/// Normalizes a relative path: `\` → `/`, no leading `./` or `/`, no empty or `.` segments.
pub fn normalize_rel(path: &str) -> String {
    path.replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// Minimal `%XX` decoding (invalid escapes are kept verbatim).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && b[i + 1].is_ascii_hexdigit()
            && b[i + 2].is_ascii_hexdigit()
            && let Ok(v) =
                u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The path a `file://` URL names, percent-decoded: `file:///C:/A%20B/x.pro` → `C:/A B/x.pro`,
/// `file:///Volumes/D/x.pro` → `/Volumes/D/x.pro`, `file://host/share/x` → `//host/share/x`.
/// Any other string is returned as it is.
pub fn file_url_path(s: &str) -> std::borrow::Cow<'_, str> {
    let t = s.trim();
    let Some(rest) = t
        .get(..7)
        .filter(|p| p.eq_ignore_ascii_case("file://"))
        .map(|_| &t[7..])
    else {
        return std::borrow::Cow::Borrowed(s);
    };
    let rest = rest
        .get(..9)
        .filter(|p| p.eq_ignore_ascii_case("localhost"))
        .map_or(rest, |_| &rest[9..]);
    let path = percent_decode(rest);
    let b = path.as_bytes();
    std::borrow::Cow::Owned(if b.first() != Some(&b'/') {
        // A host: a network share.
        format!("//{path}")
    } else if b.len() >= 3 && b[1].is_ascii_alphabetic() && b[2] == b':' {
        // `/C:/…`: a Windows drive.
        path[1..].to_string()
    } else {
        path
    })
}

/// `abs` relative to `root` (normalized), if it is inside it.
pub fn rel_path(root: &Path, abs: &Path) -> Option<String> {
    let rel = abs.strip_prefix(root).ok()?;
    Some(normalize_rel(&rel.to_string_lossy()))
}

/// A discovered file or folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Its root-relative path as text (`/`-separated; for a library folder, its name). A name
    /// that is not valid UTF-8 has U+FFFD in it.
    pub rel: String,
    /// The real path, to read it by.
    pub path: PathBuf,
}

impl Found {
    fn new(root: &Path, path: &Path) -> Option<Found> {
        Some(Found {
            rel: rel_path(root, path)?,
            path: path.to_path_buf(),
        })
    }

    /// True when its path is valid UTF-8 (its text form is exact).
    pub fn is_exact(&self) -> bool {
        self.path.to_str().is_some()
    }
}

fn sorted_walk(dir: &Path, max_depth: usize) -> impl Iterator<Item = walkdir::DirEntry> {
    WalkDir::new(dir)
        .max_depth(max_depth)
        .sort_by_file_name()
        .into_iter()
        .filter_map(Result::ok)
}

/// Folders directly under `Libraries/` (`rel` is the folder's name), sorted.
pub fn library_folders(root: &Path) -> Vec<Found> {
    sorted_walk(&root.join(LIBRARIES), 1)
        .filter(|e| e.depth() == 1 && e.file_type().is_dir())
        .map(|e| Found {
            rel: e.file_name().to_string_lossy().into_owned(),
            path: e.path().to_path_buf(),
        })
        .collect()
}

/// `.pro` files of one library folder (its real path, [`library_folders`]; recursively, though
/// real libraries are flat), sorted.
pub fn presentation_files(root: &Path, library_folder: &Path) -> Vec<Found> {
    sorted_walk(library_folder, usize::MAX)
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("pro"))
        })
        .filter_map(|e| Found::new(root, e.path()))
        .collect()
}

/// Every `Theme` document under `Themes/` (any depth), sorted. Files inside a theme's `Assets/`
/// folder are never theme documents.
pub fn theme_files(root: &Path) -> Vec<Found> {
    sorted_walk(&root.join(THEMES), usize::MAX)
        .filter(|e| e.file_type().is_file() && e.file_name() == THEME_FILE)
        .filter_map(|e| Found::new(root, e.path()))
        .filter(|f| !f.rel.split('/').any(|seg| seg == "Assets"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls() {
        assert_eq!(
            file_url_path("file:///C:/Users/x/Libraries/My%20Songs/A%20B.pro"),
            "C:/Users/x/Libraries/My Songs/A B.pro"
        );
        assert_eq!(
            file_url_path("file:///Volumes/Data/Sunday%20Service/x.pro"),
            "/Volumes/Data/Sunday Service/x.pro"
        );
        assert_eq!(
            file_url_path("file://localhost/Users/x.pro"),
            "/Users/x.pro"
        );
        assert_eq!(file_url_path("FILE://host/share/a%2"), "//host/share/a%2");
        assert_eq!(file_url_path("C:\\A B\\x.pro"), "C:\\A B\\x.pro");
    }

    #[test]
    fn normalize() {
        assert_eq!(
            normalize_rel("Libraries\\Music\\A.pro"),
            "Libraries/Music/A.pro"
        );
        assert_eq!(normalize_rel("./Themes//X/Theme"), "Themes/X/Theme");
        assert_eq!(normalize_rel("/Media/Assets/a.jpg"), "Media/Assets/a.jpg");
    }

    /// A library folder and a song whose names are not valid UTF-8 are still found, by their
    /// real paths (A11-1).
    #[cfg(unix)]
    #[test]
    fn non_utf8_names_keep_their_real_paths() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let root = std::env::temp_dir().join(format!("pro-import-non-utf8-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // "Canci\xf3n" (Latin-1), as an old zip tool extracts it.
        let lib = root
            .join(LIBRARIES)
            .join(OsStr::from_bytes(b"Canci\xf3nes"));
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join(OsStr::from_bytes(b"Canci\xf3n.pro")), b"x").unwrap();
        let folders = library_folders(&root);
        assert_eq!(folders.len(), 1);
        assert!(!folders[0].is_exact());
        assert_eq!(folders[0].rel, "Canci\u{FFFD}nes");
        let files = presentation_files(&root, &folders[0].path);
        assert_eq!(files.len(), 1, "the song inside is found");
        assert!(std::fs::read(&files[0].path).is_ok());
        assert_eq!(
            files[0].rel,
            "Libraries/Canci\u{FFFD}nes/Canci\u{FFFD}n.pro"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
