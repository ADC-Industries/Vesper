//! Stopping a run early: the operator's Ctrl-C (SIGINT) or a SIGTERM to the harness ends the
//! simulated Sundays at the next check (a tenth of a second), and the harness then winds down
//! as at the planned end: a last sample, the recording stopped, `summary.txt` written (marked
//! "stopped early by operator"), the server stopped. A second signal stops at once: the server
//! is killed and the harness exits without a summary (`summary-so-far.txt` stays).
//!
//! The server runs in a process group of its own, so the terminal's Ctrl-C reaches only the
//! harness and the server answers until the summary is written.

use std::sync::atomic::{AtomicI32, Ordering};

/// The first stop signal received (0: none).
static SIGNAL: AtomicI32 = AtomicI32::new(0);
/// The server's process ID, killed on a second signal (0: not started).
static SERVER: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn on_signal(sig: libc::c_int) {
    // Only async-signal-safe calls here: atomics, kill, _exit.
    if SIGNAL
        .compare_exchange(0, sig, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        let pid = SERVER.load(Ordering::SeqCst);
        unsafe {
            if pid > 0 {
                libc::kill(pid, libc::SIGKILL);
            }
            libc::_exit(130);
        }
    }
}

/// Installs the SIGINT / SIGTERM handlers (Unix; elsewhere the default Ctrl-C ends the
/// harness at once).
pub fn install() {
    #[cfg(unix)]
    unsafe {
        let h = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, h);
        libc::signal(libc::SIGTERM, h);
    }
}

/// The server to kill on a second signal.
pub fn set_server(pid: u32) {
    SERVER.store(pid as i32, Ordering::SeqCst);
}

/// The operator asked the run to stop: the signal's name.
pub fn requested() -> Option<&'static str> {
    match SIGNAL.load(Ordering::SeqCst) {
        0 => None,
        2 => Some("SIGINT"),
        15 => Some("SIGTERM"),
        _ => Some("a signal"),
    }
}
