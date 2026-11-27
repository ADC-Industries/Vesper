//! Vesper Codex, the site's content store: the `vesper-codex` crate (`crates/vesper-codex`),
//! re-exported here so its modules keep their paths (`codex::writer`, `codex::library`, …),
//! and the server's side of it:
//!
//! - [`api`]: the routes under `/api/v1/codex` (collections, trash, uploads, media, backups,
//!   exports, a show box's dormant upload) and a show box's write-through middleware.
//! - [`server`]: the server's state as the Codex's [`Server`], the show actor as its
//!   [`Show`], and the content and show-document writes the server plans for the Codex's
//!   writer.
//! - [`site_presence`]: presence and edit locks site-wide over the box-to-Codex link.
//! - [`pptx`]: PowerPoint import jobs (a `.pptx` into a collection).

pub use vesper_codex::*;

pub mod api;
pub mod pptx;
pub mod server;
pub mod site_presence;
