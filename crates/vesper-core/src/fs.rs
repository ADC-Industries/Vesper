//! Files written so that a power cut leaves the old file or the new one, never a damaged one.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Writes `bytes` to `path` atomically and durably: a temporary file in the same folder,
/// flushed to the disk, renamed over `path`, then the folder flushed (Unix), so the rename
/// is on the disk too. The folder is made when missing.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write(path, bytes, false)
}

/// [`write_atomic`] for a secret (stream keys): on Unix the file is created readable and
/// writable by its owner only (`0600`), so the bytes never exist with wider permissions.
pub fn write_atomic_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write(path, bytes, true)
}

fn write(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other(format!("{} has no file name", path.display())))?
        .to_string_lossy()
        .to_string();
    let tmp = dir.join(format!(
        ".{name}.{}-{}.tmp",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let r = (|| {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut f = o.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r?;
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Moves a damaged file aside (`<name>.damaged-<unix seconds>`, next to it) so a fresh one can
/// take its place and the old bytes stay for a look; where it went.
pub fn keep_aside(path: &Path) -> std::io::Result<PathBuf> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let mut to = path.as_os_str().to_owned();
    to.push(format!(".damaged-{secs}"));
    let to = PathBuf::from(to);
    std::fs::rename(path, &to)?;
    Ok(to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_replaces_and_keeps_aside() {
        let dir = std::env::temp_dir().join(format!("vesper-core-fs-{}", std::process::id()));
        let p = dir.join("sub").join("site.json");
        write_atomic(&p, b"one").unwrap();
        write_atomic(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        let left: Vec<_> = std::fs::read_dir(p.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left.len(), 1, "no temporary file left: {left:?}");
        let aside = keep_aside(&p).unwrap();
        assert!(!p.exists());
        assert_eq!(std::fs::read(&aside).unwrap(), b"two");
        assert!(
            aside
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("site.json.damaged-")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A secret is never readable by others, not even while it is written (the temporary
    /// file is created `0600`), and a file that was readable before is replaced by a private
    /// one.
    #[cfg(unix)]
    #[test]
    fn a_private_file_is_private_from_the_first_byte() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("vesper-core-fs-private-{}", std::process::id()));
        let p = dir.join("streaming.json");
        write_atomic(&p, b"old").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic_private(&p, b"keys").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"keys");
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        // (Nothing changes the mode after the write: this is the temporary file's.)
        assert_eq!(mode & 0o077, 0, "{mode:o}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
