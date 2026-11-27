//! The process heap: its numbers for `GET /server/stats`, and giving freed memory back.
//!
//! **Why trim.** With glibc's allocator, memory a thread frees stays in that thread's arena
//! for reuse, and is handed back to the system only from the top of a heap. A server that
//! opens and closes video players (each with its decoder threads and frame queues), runs
//! whisper passes for captions and passes multi-megabyte frames between threads leaves free
//! holes all over its arenas: resident memory creeps up although what is in use does not (the
//! soak, `docs/SOAK-PLAN.md`: 106 MB/h over three hours, slowing as it went). The trimmer
//! thread calls `malloc_trim(0)` once a minute when more than [`TRIM_ABOVE`] is held free:
//! every whole free page of every arena goes back to the system (glibc ≥ 2.8). It runs at the
//! lowest priority, so it never takes CPU from the screens; it holds one arena's lock at a
//! time, briefly.
//!
//! Other allocators (Windows, musl) return memory on their own terms: there the trimmer does
//! nothing and the heap numbers are absent.
//!
//! **mimalloc for Rust (optional).** With the feature `mimalloc` (off by default: owner,
//! 2026-11-26) the `vesper` binary runs its Rust code on mimalloc (`main.rs` declares it and
//! calls [`rust_on_mimalloc`]). mimalloc
//! keeps small objects in per-thread pages and gives the pages a thread no longer uses back to
//! the system after a short delay, where glibc's arenas held them free for good. The C
//! libraries (FFmpeg, whisper/ggml, the GPU drivers) still call glibc's `malloc`: with mimalloc
//! the glibc numbers ([`usage`]) are theirs alone, and [`rust_heap`] is what Rust holds.
//!
//! **glibc for C.** The soak showed that the memory glibc held free kept growing with only the
//! C libraries on it: their big buffers (decoded frames, whisper's work buffers) came from the
//! arenas once glibc had raised its mmap threshold. [`tune_glibc`] fixes the threshold and
//! limits the arenas, first thing in `main` (so the child processes the `vesper` binary serves,
//! IDN's captions worker with whisper's buffers and `media-convert`, have it too). The trimmer
//! stays for what is left on glibc.
//!
//! **Not IDN's `idn_common::memory::tune()`** (IDN v0.3.8-Beta18): it makes the same two
//! settings (256 KiB, 4 arenas), but takes its values from its own switches (`IDN_MALLOC_TUNE`,
//! `IDN_MALLOC_MMAP_KB`, `IDN_MALLOC_ARENAS`) and doesn't leave a setting the environment makes
//! in `GLIBC_TUNABLES` or `MALLOC_MMAP_THRESHOLD_` / `MALLOC_ARENA_MAX` to the admin, as this
//! one does (an admin's service override, or a soak run measuring other values with
//! `GLIBC_TUNABLES`, would be overridden).
//!
//! **Switches.** Both are cargo features of `vesper-server`, independent: `glibc-tuning` (the C
//! side, on by default) and `mimalloc` (Rust's allocator, off by default). Changing either is a
//! one-line change of `default` in its `Cargo.toml` (or `--no-default-features --features
//! …`). The soak's numbers for each choice are in `docs/SOAK-PLAN.md`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// glibc's mmap threshold for the C libraries' allocations, bytes: larger blocks get their own
/// mapping, which goes back to the system the moment they are freed (see [`tune_glibc`]).
pub const MMAP_THRESHOLD: i32 = 256 << 10;
/// glibc arenas at most (instead of eight per CPU): see [`tune_glibc`].
pub const ARENA_MAX: i32 = 4;

/// Sets glibc's `malloc` up for the C libraries that still use it (Linux with glibc and the
/// `glibc-tuning` feature, on by default; nothing otherwise). Call it first thing in `main`,
/// before any thread starts. Independent of the `mimalloc` feature: either can be turned off
/// on its own.
///
/// - **A fixed mmap threshold** ([`MMAP_THRESHOLD`]): by default glibc raises the threshold
///   to the size of every mapped block freed (up to 32 MiB), after which FFmpeg's frame
///   buffers and whisper's work buffers come from the arenas, and their holes stay there
///   ("held free" growing for good). Fixed, every block above it is its own mapping.
/// - **At most [`ARENA_MAX`] arenas**: the server has ~130 threads; with Rust on mimalloc the
///   C threads (decoders, captions, drivers) share a few arenas, so fewer half-empty heaps.
///
/// Same as `GLIBC_TUNABLES=glibc.malloc.mmap_threshold=262144:glibc.malloc.arena_max=4`
/// (the soak measured it that way). An admin's own setting wins: each of the two is left
/// alone when the environment sets it, in `GLIBC_TUNABLES` or with glibc's older variables
/// `MALLOC_MMAP_THRESHOLD_` and `MALLOC_ARENA_MAX` ([`glibc_tuning`] says which, for the log).
pub fn tune_glibc() {
    #[cfg(all(feature = "glibc-tuning", target_os = "linux", target_env = "gnu"))]
    {
        let (mmap, arenas) = glibc_choices(|name| std::env::var(name).ok());
        // SAFETY: plain settings of glibc's allocator, made before other threads exist.
        unsafe {
            if mmap {
                libc::mallopt(libc::M_MMAP_THRESHOLD, MMAP_THRESHOLD);
            }
            if arenas {
                libc::mallopt(libc::M_ARENA_MAX, ARENA_MAX);
            }
        }
        let value = |made: bool, v: i32| {
            if made {
                v.to_string()
            } else {
                "as the environment sets it".to_string()
            }
        };
        let _ = TUNED.set(format!(
            "mmap_threshold {}, arena_max {}",
            value(mmap, MMAP_THRESHOLD),
            value(arenas, ARENA_MAX)
        ));
    }
}

/// What [`tune_glibc`] did, for the log (`None`: nothing: not glibc, or built without it).
pub fn glibc_tuning() -> Option<&'static str> {
    TUNED.get().map(|s| s.as_str())
}

static TUNED: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Which of [`tune_glibc`]'s two settings it makes (the mmap threshold, the arenas), given the
/// environment (`env`): one the environment sets (`GLIBC_TUNABLES`, or glibc's older
/// `MALLOC_MMAP_THRESHOLD_` / `MALLOC_ARENA_MAX`) is the admin's.
#[cfg_attr(
    not(all(feature = "glibc-tuning", target_os = "linux", target_env = "gnu")),
    allow(dead_code)
)]
fn glibc_choices(env: impl Fn(&str) -> Option<String>) -> (bool, bool) {
    let set = |name: &str| env(name).is_some_and(|v| !v.trim().is_empty());
    let tunables = env("GLIBC_TUNABLES").unwrap_or_default();
    (
        !tunables.contains("glibc.malloc.mmap_threshold") && !set("MALLOC_MMAP_THRESHOLD_"),
        !tunables.contains("glibc.malloc.arena_max") && !set("MALLOC_ARENA_MAX"),
    )
}

/// Whether the process's Rust allocator is mimalloc (set by the binary).
static RUST_ON_MIMALLOC: AtomicBool = AtomicBool::new(false);

/// Called by a binary that declares mimalloc its `#[global_allocator]`: `/server/stats` then
/// reports mimalloc's memory ([`rust_heap`]).
pub fn rust_on_mimalloc() {
    RUST_ON_MIMALLOC.store(true, Ordering::Relaxed);
}

/// Memory mimalloc has committed for Rust, bytes (`None` when Rust isn't on mimalloc).
pub fn rust_heap() -> Option<u64> {
    #[cfg(feature = "mimalloc")]
    if RUST_ON_MIMALLOC.load(Ordering::Relaxed) {
        let mut n = [0usize; 8];
        let [
            elapsed,
            user,
            system,
            rss,
            peak_rss,
            commit,
            peak_commit,
            faults,
        ] = &mut n;
        // SAFETY: mimalloc fills the eight numbers it is given; nothing else is touched.
        unsafe {
            libmimalloc_sys::mi_process_info(
                elapsed,
                user,
                system,
                rss,
                peak_rss,
                commit,
                peak_commit,
                faults,
            )
        };
        return Some(n[5] as u64);
    }
    None
}

/// How often the trimmer looks.
pub const TRIM_EVERY: Duration = Duration::from_secs(60);
/// Free heap above which it trims, bytes.
pub const TRIM_ABOVE: u64 = 32 << 20;

/// The heap's bytes in use and held free (glibc's `mallinfo2`, every arena; `None` elsewhere).
pub fn usage() -> Option<(u64, u64)> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: a plain query of the allocator; it takes each arena's lock briefly.
        let m = unsafe { libc::mallinfo2() };
        Some(((m.uordblks + m.hblkhd) as u64, m.fordblks as u64))
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        None
    }
}

/// Gives every whole free page back to the system; `true` when some was.
pub fn trim() -> bool {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: glibc's own maintenance call; safe from any thread.
        unsafe { libc::malloc_trim(0) == 1 }
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        false
    }
}

/// Starts the trimmer thread (once per process; nothing to do on other allocators).
pub fn start_trimmer() {
    static STARTED: std::sync::Once = std::sync::Once::new();
    if usage().is_none() {
        return;
    }
    STARTED.call_once(|| {
        let spawned = std::thread::Builder::new()
            .name("heap-trim".into())
            .spawn(|| {
                vesper_codex::disk::lower_thread_priority();
                loop {
                    std::thread::sleep(TRIM_EVERY);
                    let Some((_, free)) = usage() else { return };
                    if free <= TRIM_ABOVE {
                        continue;
                    }
                    let (rss0, t) = (rss_kb(), std::time::Instant::now());
                    let released = trim();
                    tracing::debug!(
                        free_mb = free >> 20,
                        released,
                        rss_mb_before = rss0 / 1024,
                        rss_mb_after = rss_kb() / 1024,
                        ms = format!("{:.1}", t.elapsed().as_secs_f64() * 1000.0),
                        "heap trimmed"
                    );
                }
            });
        if let Err(e) = spawned {
            tracing::warn!("no heap trimmer: {e}");
        }
    });
}

/// Resident memory, KiB (Linux; 0 elsewhere).
fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("VmRSS:")?
                    .split_whitespace()
                    .next()?
                    .parse()
                    .ok()
            })
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    /// An admin's own setting wins over the tuning: in `GLIBC_TUNABLES` or glibc's older
    /// variables (H05-2).
    #[test]
    fn the_environments_malloc_settings_win() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(super::glibc_choices(env(&[])), (true, true));
        assert_eq!(
            super::glibc_choices(env(&[("MALLOC_ARENA_MAX", "1")])),
            (true, false)
        );
        assert_eq!(
            super::glibc_choices(env(&[("MALLOC_MMAP_THRESHOLD_", "131072")])),
            (false, true)
        );
        assert_eq!(
            super::glibc_choices(env(&[(
                "GLIBC_TUNABLES",
                "glibc.malloc.arena_max=2:glibc.malloc.mmap_threshold=65536"
            )])),
            (false, false)
        );
        assert_eq!(
            super::glibc_choices(env(&[
                ("GLIBC_TUNABLES", "glibc.malloc.arena_max=2"),
                ("MALLOC_ARENA_MAX", "")
            ])),
            (true, false)
        );
    }

    /// Rust's heap is reported only once the binary says it runs on mimalloc.
    #[test]
    #[cfg(feature = "mimalloc")]
    fn rust_heap_is_reported_only_on_mimalloc() {
        assert_eq!(super::rust_heap(), None);
        super::rust_on_mimalloc();
        assert!(super::rust_heap().is_some());
    }

    /// The heap numbers count what this test holds (other tests allocate at the same time,
    /// so only a lower bound is certain), and trimming leaves memory in use alone.
    #[test]
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    fn heap_numbers_count_what_is_held_and_a_trim_keeps_it() {
        // 64 blocks of 1 MiB allocated on another thread (its own arena), each filled.
        let blocks = std::thread::spawn(|| {
            (0..64)
                .map(|i| vec![i as u8 | 1; 1 << 20])
                .collect::<Vec<_>>()
        })
        .join()
        .unwrap();
        let (used, _) = super::usage().unwrap();
        assert!(used >= 64 << 20, "{used}");
        // Freed memory beside what is held, then trimmed: what is held is untouched.
        drop(std::thread::spawn(|| vec![7u8; 48 << 20]).join().unwrap());
        super::trim();
        assert!(
            blocks
                .iter()
                .enumerate()
                .all(|(i, b)| b.iter().all(|x| *x == (i as u8 | 1)))
        );
        assert!(super::rss_kb() > 0);
    }
}
