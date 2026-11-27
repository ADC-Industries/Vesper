//! # `vesper-core` — what the Vesper server's parts share
//!
//! The server (`vesper-server`) is split into crates so that an edit rebuilds only its own
//! part: this crate at the bottom, the site (`vesper-site`) on top of it, and the server on
//! top of both. What lives here is what more than one of them needs:
//!
//! - [`http`]: API errors as HTTP responses and the small request/response helpers of every
//!   route module (`If-Match`, `ETag`, JSON bodies, query parameters).
//! - [`change`]: document revisions and a refused change ([`change::ConfigError`]).
//! - [`fs`]: files written atomically and durably (flushed before and after the rename).
//! - [`lock`]: locks a mutex, ignoring poisoning.

pub mod change;
pub mod fs;
pub mod http;

use std::sync::Mutex;

/// Locks a mutex, ignoring poisoning (a panicked holder leaves consistent data here).
pub fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
