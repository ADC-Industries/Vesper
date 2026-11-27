//! The bundled IDN Node is Vesper's to update (owner decision (a), 2026-11-26; IDN Beta14's
//! "updates managed by another product", `docs/LAUNCHER-PLAN.md`):
//!
//! - **The marker.** `idn-updates-managed-by` beside the bundled `idn-node`, its first line
//!   `Vesper`: the Node reports `<update managed_by="Vesper" managed_by_source="package"/>` and
//!   the site's Site Manager sends that machine no IDN Media updates. Vesper's update channel
//!   does (H04-2 / R3-2): on Linux `vesper-idn-node` is one of the Launcher's products (a
//!   package of programs, `launcher.d/vesper-idn-node.json`, shipped by `vesper-launcher`), so a
//!   newer package uploaded to Site ▸ Updates is installed like the server's; on Windows the
//!   Node is part of the Launcher's installer, updated with the Launcher. The `vesper-idn-node`
//!   package ships the marker (Windows: the Launcher's installer); the Launcher puts it back
//!   when it is missing, names another product or can't be read as text ([`ensure_marker`]).
//! - **The guard.** When the Launcher installed the system's IDN Node service (the display box
//!   role), that service must run the bundled `idn-node`. If it runs another one (an IDN Media
//!   install replaced it), the bundled one is installed as the service again (`idn-node
//!   install`, which on Windows points the existing service at this copy and restarts it) and a
//!   warning says so ([`guard_action`]). Linux reads the service's program from its systemd unit
//!   ([`service_program`]); Windows from the service manager ([`windows_service_image`], the
//!   service `idn-node`'s image path, parsed by [`windows_image_program`] and compared by
//!   [`windows_is_ours`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

/// The marker's name, beside `idn-node`.
pub const MARKER: &str = "idn-updates-managed-by";
/// The product named in it.
pub const PRODUCT: &str = "Vesper";

/// What [`ensure_marker`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Marker {
    /// No bundled `idn-node` in the folder: nothing to mark.
    NoNode,
    /// It was there, naming Vesper.
    Kept,
    /// It was missing or named another product (`Some`): written again.
    Restored(Option<String>),
}

/// The product a marker's text names: its first line, trimmed (`None`: empty).
pub fn named(text: &str) -> Option<String> {
    text.lines()
        .next()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
}

/// Makes sure the marker beside the bundled `idn-node` in `idn_dir` names Vesper.
pub fn ensure_marker(idn_dir: &Path) -> std::io::Result<Marker> {
    let node = idn_dir.join(format!("idn-node{}", std::env::consts::EXE_SUFFIX));
    if !node.is_file() {
        return Ok(Marker::NoNode);
    }
    let path = idn_dir.join(MARKER);
    // Read as bytes: a marker that isn't UTF-8 (UTF-16 from PowerShell 5.1) names another
    // product as far as Vesper can tell, and is written again (H04-3).
    let had = match std::fs::read(&path) {
        Ok(b) => named(&String::from_utf8_lossy(&b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    if had.as_deref() == Some(PRODUCT) {
        return Ok(Marker::Kept);
    }
    let line = if cfg!(windows) {
        "Vesper\r\n"
    } else {
        "Vesper\n"
    };
    std::fs::write(&path, line)?;
    Ok(Marker::Restored(had))
}

/// The program a systemd unit's `ExecStart` runs (its first word, a leading `-`, `@`, `+`
/// or `!` taken off); `None`: no unit, or no `ExecStart`.
pub fn service_program(unit: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(unit).ok()?;
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("ExecStart="))
        .map(|v| v.trim().trim_start_matches(['-', '@', '+', '!']))
        .find(|v| !v.is_empty())
        .and_then(|v| v.split_whitespace().next())
        .map(PathBuf::from)
}

/// Whether the service's program is the bundled `idn-node` in `idn_dir`.
pub fn is_ours(program: &Path, idn_dir: &Path) -> bool {
    program.parent() == Some(idn_dir)
        && program.file_stem().and_then(|s| s.to_str()) == Some("idn-node")
}

/// The Windows service the bundled `idn-node.exe install` registers (`idn-node`; its display
/// name is "IDN Node").
pub const WINDOWS_SERVICE: &str = "idn-node";

/// The program a Windows service's image path (`BINARY_PATH_NAME`, the registry's `ImagePath`)
/// runs: the quoted part when it starts with a quote, else everything up to the first `.exe`
/// that ends a word (an unquoted path may hold spaces, `C:\Program Files\x\idn-node.exe run`),
/// else its first word; a `\\?\` prefix taken off. `None`: empty.
pub fn windows_image_program(image: &str) -> Option<String> {
    let image = image.trim();
    let program = if let Some(rest) = image.strip_prefix('"') {
        rest.split('"').next().unwrap_or(rest)
    } else {
        let lower = image.to_ascii_lowercase();
        lower
            .match_indices(".exe")
            .map(|(i, _)| i + 4)
            .find(|&end| image[end..].chars().next().is_none_or(char::is_whitespace))
            .map_or_else(
                || image.split_whitespace().next().unwrap_or(""),
                |end| &image[..end],
            )
    };
    let program = program.trim();
    let program = program
        .strip_prefix(r"\\?\")
        .or_else(|| program.strip_prefix(r"\??\"))
        .unwrap_or(program);
    (!program.is_empty()).then(|| program.to_string())
}

/// A Windows path as Windows compares it: `/` as `\`, ASCII case folded, a `\\?\` prefix and
/// trailing separators taken off.
fn windows_folded(path: &str) -> String {
    let p = path.trim().replace('/', "\\");
    let p = p.strip_prefix(r"\\?\").unwrap_or(&p);
    p.trim_end_matches('\\').to_ascii_lowercase()
}

/// Whether the Windows service's `program` ([`windows_image_program`]) is the bundled
/// `idn-node.exe` in `idn_dir` (Windows paths: any case, either slash).
pub fn windows_is_ours(program: &str, idn_dir: &Path) -> bool {
    let program = windows_folded(program);
    let dir = windows_folded(&idn_dir.to_string_lossy());
    let Some((parent, name)) = program.rsplit_once('\\') else {
        return false;
    };
    parent == dir && matches!(name, "idn-node.exe" | "idn-node")
}

/// The image path of the Windows service `name` from the service manager (`Ok(None)`: no
/// such service).
#[cfg(windows)]
pub fn windows_service_image(name: &str) -> std::io::Result<Option<String>> {
    use windows_sys::Win32::Foundation::{
        ERROR_INSUFFICIENT_BUFFER, ERROR_SERVICE_DOES_NOT_EXIST, GetLastError,
    };
    use windows_sys::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, QUERY_SERVICE_CONFIGW,
        QueryServiceConfigW, SC_MANAGER_CONNECT, SERVICE_QUERY_CONFIG,
    };

    /// Closes a service manager handle when dropped.
    struct Handle(windows_sys::Win32::System::Services::SC_HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: a handle the service manager gave us, closed once.
            unsafe { CloseServiceHandle(self.0) };
        }
    }

    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: null machine and database name (this machine's active database); the handles
    // are checked before use and closed by `Handle`; the buffer is as large as the service
    // manager asked for, u64-aligned for QUERY_SERVICE_CONFIGW, and its strings point into it.
    unsafe {
        let scm = OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT);
        if scm.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let scm = Handle(scm);
        let svc = OpenServiceW(scm.0, wide.as_ptr(), SERVICE_QUERY_CONFIG);
        if svc.is_null() {
            if GetLastError() == ERROR_SERVICE_DOES_NOT_EXIST {
                return Ok(None);
            }
            return Err(std::io::Error::last_os_error());
        }
        let svc = Handle(svc);
        let mut needed = 0u32;
        if QueryServiceConfigW(svc.0, std::ptr::null_mut(), 0, &mut needed) == 0
            && GetLastError() != ERROR_INSUFFICIENT_BUFFER
        {
            return Err(std::io::Error::last_os_error());
        }
        let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(1)];
        let config = buf.as_mut_ptr().cast::<QUERY_SERVICE_CONFIGW>();
        if QueryServiceConfigW(svc.0, config, (buf.len() * 8) as u32, &mut needed) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let path = (*config).lpBinaryPathName;
        if path.is_null() {
            return Ok(Some(String::new()));
        }
        let len = (0..).take_while(|&i| *path.add(i) != 0).count();
        Ok(Some(String::from_utf16_lossy(std::slice::from_raw_parts(
            path, len,
        ))))
    }
}

/// What the guard found: the program the system's IDN Node service runs, as shown in the
/// warning, and whether it is the bundled `idn-node`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceProgram {
    /// The program, as the warning shows it.
    pub shown: String,
    /// It is the bundled `idn-node`.
    pub ours: bool,
}

/// What the guard does ([`guard_action`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardAction {
    /// Nothing: the service is fine, isn't the Launcher's, or can't be put back now.
    Leave,
    /// Run the bundled `idn-node install` again, and warn.
    PutBack,
}

/// The guard's decision. `launcher_installed`: the Launcher installed the IDN Node service (the
/// display box role, `roles.json`); `service`: the service's program (`None`: no service, or
/// it can't be read); `bundled`: the bundled `idn-node` is there; `since_put_back`: how long
/// ago the guard last put it back; `role_run`: a role run is under way (it installs and removes
/// the service). At most every `every`.
pub fn guard_action(
    launcher_installed: bool,
    service: Option<&ServiceProgram>,
    bundled: bool,
    since_put_back: Option<Duration>,
    every: Duration,
    role_run: bool,
) -> GuardAction {
    let replaced = service.is_some_and(|s| !s.ours);
    let waited = since_put_back.is_none_or(|t| t >= every);
    if launcher_installed && replaced && bundled && waited && !role_run {
        GuardAction::PutBack
    } else {
        GuardAction::Leave
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("vesper-idn-marker-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_marker_is_put_back() {
        let d = dir("put-back");
        assert_eq!(ensure_marker(&d).unwrap(), Marker::NoNode);
        assert!(!d.join(MARKER).exists());
        let node = d.join(format!("idn-node{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&node, b"").unwrap();
        assert_eq!(ensure_marker(&d).unwrap(), Marker::Restored(None));
        assert_eq!(
            named(&std::fs::read_to_string(d.join(MARKER)).unwrap()).as_deref(),
            Some("Vesper")
        );
        assert_eq!(ensure_marker(&d).unwrap(), Marker::Kept);
        std::fs::write(d.join(MARKER), "Other\nmore\n").unwrap();
        assert_eq!(
            ensure_marker(&d).unwrap(),
            Marker::Restored(Some("Other".into()))
        );
        assert_eq!(ensure_marker(&d).unwrap(), Marker::Kept);
        // H04-3: one that isn't UTF-8 (`'Other' > file` in Windows PowerShell 5.1: UTF-16LE
        // with a BOM) is written again too.
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("Other\r\n".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        std::fs::write(d.join(MARKER), utf16).unwrap();
        assert!(matches!(
            ensure_marker(&d).unwrap(),
            Marker::Restored(Some(_))
        ));
        assert_eq!(ensure_marker(&d).unwrap(), Marker::Kept);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_services_program() {
        let d = dir("unit");
        let unit = d.join("idn-node.service");
        assert_eq!(service_program(&unit), None);
        std::fs::write(
            &unit,
            "[Unit]\nDescription=IDN Node\n[Service]\nExecStart=/usr/lib/vesper-idn-node/idn-node run --service\n",
        )
        .unwrap();
        let p = service_program(&unit).unwrap();
        assert!(is_ours(&p, Path::new("/usr/lib/vesper-idn-node")));
        std::fs::write(&unit, "[Service]\nExecStart=-/opt/idn/bin/idn-node run\n").unwrap();
        let p = service_program(&unit).unwrap();
        assert_eq!(p, PathBuf::from("/opt/idn/bin/idn-node"));
        assert!(!is_ours(&p, Path::new("/usr/lib/vesper-idn-node")));
        assert!(!is_ours(
            Path::new("/usr/lib/vesper-idn-node/idn-session"),
            Path::new("/usr/lib/vesper-idn-node")
        ));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn windows_image_paths() {
        let p = |s: &str| windows_image_program(s);
        assert_eq!(
            p(r#""C:\Program Files\Vesper Launcher\idn-node\idn-node.exe" run --service"#)
                .as_deref(),
            Some(r"C:\Program Files\Vesper Launcher\idn-node\idn-node.exe")
        );
        // Unquoted, with spaces: up to the `.exe` that ends a word.
        assert_eq!(
            p(r"C:\Program Files\IDN Media\idn-node.exe run --service").as_deref(),
            Some(r"C:\Program Files\IDN Media\idn-node.exe")
        );
        assert_eq!(
            p(r"C:\Tools\my.exec\IDN-NODE.EXE").as_deref(),
            Some(r"C:\Tools\my.exec\IDN-NODE.EXE")
        );
        assert_eq!(
            p(r"\\?\C:\IDN\idn-node.exe run").as_deref(),
            Some(r"C:\IDN\idn-node.exe")
        );
        assert_eq!(
            p(r#""\\?\C:\IDN\idn-node.exe" run"#).as_deref(),
            Some(r"C:\IDN\idn-node.exe")
        );
        // No `.exe`: the first word.
        assert_eq!(
            p(r"C:\IDN\idn-node run").as_deref(),
            Some(r"C:\IDN\idn-node")
        );
        assert_eq!(p("  "), None);
        assert_eq!(p(r#""" run"#), None);
    }

    #[test]
    fn windows_programs_compared() {
        let dir = Path::new(r"C:\Program Files\Vesper Launcher\idn-node");
        assert!(windows_is_ours(
            r"C:\Program Files\Vesper Launcher\idn-node\idn-node.exe",
            dir
        ));
        assert!(windows_is_ours(
            r"c:\program files\VESPER LAUNCHER\idn-node\IDN-NODE.EXE",
            dir
        ));
        assert!(windows_is_ours(
            r"\\?\C:/Program Files/Vesper Launcher/idn-node/idn-node.exe",
            Path::new(r"C:\Program Files\Vesper Launcher\idn-node\")
        ));
        assert!(!windows_is_ours(
            r"C:\Program Files\IDN Media\idn-node.exe",
            dir
        ));
        assert!(!windows_is_ours(
            r"C:\Program Files\Vesper Launcher\idn-node\idn-session.exe",
            dir
        ));
        assert!(!windows_is_ours(
            r"C:\Program Files\Vesper Launcher\idn-node\sub\idn-node.exe",
            dir
        ));
        assert!(!windows_is_ours("idn-node.exe", dir));
    }

    #[test]
    fn the_guards_decision() {
        let every = Duration::from_secs(600);
        let other = ServiceProgram {
            shown: "/opt/idn/bin/idn-node".into(),
            ours: false,
        };
        let ours = ServiceProgram {
            shown: "/usr/lib/vesper-idn-node/idn-node".into(),
            ours: true,
        };
        let put = |installed, service, bundled, since, role_run| {
            guard_action(installed, service, bundled, since, every, role_run)
        };
        use GuardAction::{Leave, PutBack};
        // Replaced, never put back before: put back.
        assert_eq!(put(true, Some(&other), true, None, false), PutBack);
        // Ten minutes after the last time: again; before that: wait.
        assert_eq!(put(true, Some(&other), true, Some(every), false), PutBack);
        assert_eq!(
            put(
                true,
                Some(&other),
                true,
                Some(Duration::from_secs(599)),
                false
            ),
            Leave
        );
        // The service runs ours, there is none, or it can't be read: nothing to do.
        assert_eq!(put(true, Some(&ours), true, None, false), Leave);
        assert_eq!(put(true, None, true, None, false), Leave);
        // Not the Launcher's service (an admin's own IDN install): left alone.
        assert_eq!(put(false, Some(&other), true, None, false), Leave);
        // No bundled idn-node to put back.
        assert_eq!(put(true, Some(&other), false, None, false), Leave);
        // Never during a role run.
        assert_eq!(put(true, Some(&other), true, None, true), Leave);
    }

    #[test]
    fn marker_text() {
        assert_eq!(named("Vesper\n").as_deref(), Some("Vesper"));
        assert_eq!(named("  Vesper \r\nx").as_deref(), Some("Vesper"));
        assert_eq!(named(""), None);
        assert_eq!(named("\nVesper"), None);
    }
}
