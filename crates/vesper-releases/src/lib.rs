//! # Vesper releases
//!
//! What a Vesper package is and where releases come from, shared by the Vesper server (the
//! site's update channel) and the Vesper Launcher:
//!
//! - [`sign`]: Ed25519 signatures in minisign's format; the compiled-in release keys.
//! - [`package`]: a package's signed product, version and platform; version order.
//! - [`github`]: Vesper's releases on GitHub (`docs/PACKAGING.md`, "Releases"): listing them
//!   (unauthenticated API, ETag, rate limits, offline), channels and tag prefixes, picking a
//!   platform's package, downloading it with its signature and verifying both.
//! - `fake` (feature `fake`): a fake GitHub for tests.

#![warn(missing_docs)]

#[cfg(feature = "fake")]
pub mod fake;
pub mod github;
pub mod package;
pub mod sign;
