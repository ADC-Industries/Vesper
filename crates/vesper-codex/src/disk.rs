//! Disk space and file moves for the Codex's media (Linux and Windows).

use std::path::Path;

/// A filesystem's size and the space this process may still use on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiskSpace {
    /// Bytes on the filesystem.
    pub total: u64,
    /// Bytes this process can still write (without reserved blocks).
    pub available: u64,
}

impl DiskSpace {
    /// What the Codex keeps free: 5 % of the disk plus 2 GiB (`percent`, `bytes` from
    /// [`super::CodexOptions`]).
    pub fn reserve(&self, percent: f64, bytes: u64) -> u64 {
        ((self.total as f64) * percent / 100.0) as u64 + bytes
    }
}

/// The filesystem holding `path` (an existing file or folder).
#[cfg(unix)]
pub fn space(path: &Path) -> std::io::Result<DiskSpace> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `s` a writable statvfs.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let unit = s.f_frsize as u64;
    Ok(DiskSpace {
        total: (s.f_blocks as u64).saturating_mul(unit),
        available: (s.f_bavail as u64).saturating_mul(unit),
    })
}

/// The filesystem holding `path` (an existing file or folder).
#[cfg(windows)]
pub fn space(path: &Path) -> std::io::Result<DiskSpace> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let (mut available, mut total, mut free) = (0u64, 0u64, 0u64);
    // SAFETY: `wide` is NUL-terminated; the three outputs are valid u64s.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            &mut total,
            &mut free,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(DiskSpace { total, available })
}

/// Lowers the calling thread's CPU priority (media probing and thumbnails, which must not
/// disturb the outputs' frame pacing). On Linux threads it starts afterwards inherit it; on
/// Windows they don't (a new thread starts at normal priority): lower each thread there.
pub fn lower_thread_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: plain syscalls on the calling thread.
    unsafe {
        let tid = libc::gettid();
        if libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, 19) != 0 {
            tracing::debug!(
                "lowering a media worker's priority: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    #[cfg(windows)]
    // SAFETY: the pseudo handle of the calling thread.
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_LOWEST,
        };
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
    }
}

/// Moves a file: a rename, or (onto another filesystem) a copy to a temporary file next to
/// `to`, synced and renamed into place, then the source removed.
pub fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() != std::io::ErrorKind::CrossesDevices => return Err(e),
        Err(_) => {}
    }
    let dir = to.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        to.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        uuid::Uuid::new_v4().simple()
    ));
    let copied = (|| {
        std::fs::copy(from, &tmp)?;
        std::fs::File::open(&tmp)?.sync_all()?;
        std::fs::rename(&tmp, to)
    })();
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::remove_file(from)
}

/// Bytes and number of the files below `dir` (recursively).
pub fn dir_usage(dir: &Path) -> (u64, u64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    let mut out = (0, 0);
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            let (b, n) = dir_usage(&e.path());
            out.0 += b;
            out.1 += n;
        } else if let Ok(m) = e.metadata() {
            out.0 += m.len();
            out.1 += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_of_the_temp_dir() {
        let s = space(&std::env::temp_dir()).unwrap();
        assert!(s.total > 0 && s.available <= s.total, "{s:?}");
        let r = DiskSpace {
            total: 100 << 30,
            available: 0,
        }
        .reserve(5.0, 2 << 30);
        assert_eq!(r, (5 << 30) + (2 << 30));
    }

    #[test]
    fn move_file_renames() {
        let d = std::env::temp_dir().join(format!("codex-move-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a"), b"abc").unwrap();
        move_file(&d.join("a"), &d.join("b")).unwrap();
        assert!(!d.join("a").exists());
        assert_eq!(std::fs::read(d.join("b")).unwrap(), b"abc");
        assert_eq!(dir_usage(&d), (3, 1));
        std::fs::remove_dir_all(&d).ok();
    }
}
