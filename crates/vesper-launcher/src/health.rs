//! Health: is a product up (its API answers), and the machine's numbers (CPU, memory, disk,
//! GPUs).

use protocol::MachineHealth;
use std::path::Path;
use std::time::{Duration, Instant};

/// What a product's health check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probe {
    /// It answered `200`, with this version if it reports one.
    Up(Option<String>),
    /// It didn't answer, or not with `200`.
    Down(String),
}

/// Asks a product's health URL once.
pub fn probe(url: &str, timeout: Duration) -> Probe {
    match crate::http::request("GET", url, &[], None, timeout) {
        Ok(r) if r.status == 200 => {
            let version = serde_json::from_slice::<serde_json::Value>(&r.body)
                .ok()
                .and_then(|v| v.get("version")?.as_str().map(str::to_string));
            Probe::Up(version)
        }
        Ok(r) => Probe::Down(format!("its health check answered {}", r.status)),
        Err(e) => Probe::Down(format!("its health check failed: {e}")),
    }
}

/// A reported version is the package's: the same, or the package's without a packaging
/// suffix (`0.2.0` for the `.deb` version `0.2.0-1`).
pub fn same_version(reported: &str, package: &str) -> bool {
    reported == package
        || package
            .strip_prefix(reported)
            .is_some_and(|rest| rest.starts_with('-') || rest.starts_with('+'))
}

/// Waits until `url` answers with `version` (or with no version), for at most `timeout`.
pub fn wait_healthy(url: &str, version: &str, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut last: String;
    loop {
        match probe(url, Duration::from_secs(2)) {
            Probe::Up(None) => return Ok(()),
            Probe::Up(Some(v)) if same_version(&v, version) => return Ok(()),
            Probe::Up(Some(v)) => last = format!("it still reports version {v}"),
            Probe::Down(why) => last = why,
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "version {version} wasn't healthy within {} s: {last}",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// The machine's numbers; `disk` is a path on the disk to measure.
pub fn machine(disk: &Path) -> MachineHealth {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let mut h = MachineHealth {
        cpus,
        ..Default::default()
    };
    #[cfg(target_os = "linux")]
    linux(&mut h, disk);
    #[cfg(windows)]
    windows(&mut h, disk);
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = disk;
    h
}

#[cfg(target_os = "linux")]
fn linux(h: &mut MachineHealth, disk: &Path) {
    if let Ok(t) = std::fs::read_to_string("/proc/loadavg")
        && let Some(l) = t
            .split_whitespace()
            .next()
            .and_then(|l| l.parse::<f32>().ok())
    {
        h.cpu_percent = Some((l / h.cpus.max(1) as f32 * 100.0).min(100.0));
    }
    if let Ok(t) = std::fs::read_to_string("/proc/meminfo") {
        let kib = |key: &str| {
            t.lines()
                .find_map(|l| l.strip_prefix(key))
                .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
                .map(|k| k * 1024)
        };
        h.memory_total = kib("MemTotal:");
        h.memory_available = kib("MemAvailable:");
    }
    if let Some((total, free)) = statvfs(disk) {
        h.disk_total = Some(total);
        h.disk_free = Some(free);
    }
    // GPUs: the kernel driver of each DRM card (amdgpu, i915, nouveau, …).
    if let Ok(dir) = std::fs::read_dir("/sys/class/drm") {
        let mut gpus: Vec<String> = dir
            .flatten()
            .filter(|e| {
                let n = e.file_name();
                let n = n.to_string_lossy();
                n.starts_with("card") && !n.contains('-')
            })
            .filter_map(|e| {
                let t = std::fs::read_to_string(e.path().join("device/uevent")).ok()?;
                t.lines()
                    .find_map(|l| l.strip_prefix("DRIVER="))
                    .map(str::to_string)
            })
            .collect();
        gpus.sort();
        h.gpus = gpus;
    }
}

#[cfg(unix)]
fn statvfs(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `s` a valid out-parameter.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let frsize = s.f_frsize as u64;
    Some((s.f_blocks as u64 * frsize, s.f_bavail as u64 * frsize))
}

/// Windows: the processor count only for now (memory, disk and GPUs come with the Windows
/// round, through the system's APIs).
#[cfg(windows)]
fn windows(h: &mut MachineHealth, disk: &Path) {
    let _ = (h, disk);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_versions_match_reported_ones() {
        assert!(same_version("0.2.0", "0.2.0"));
        assert!(same_version("0.2.0", "0.2.0-1"));
        assert!(!same_version("0.2.0", "0.2.01"));
        assert!(!same_version("0.1.0", "0.2.0"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_machine_is_measured() {
        let h = machine(Path::new("/"));
        assert!(h.cpus >= 1);
        assert!(h.memory_total.unwrap() > 0);
        assert!(h.disk_total.unwrap() >= h.disk_free.unwrap());
    }
}
