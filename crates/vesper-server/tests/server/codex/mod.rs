//! Vesper Codex: the store and its layout (`codex/`, `machine/codex.json`), one module per
//! area. All on synthetic fixtures (`crate::common`).

mod backups;
mod box_status;
mod collections;
mod dormant_upload;
mod export;
mod export_site;
mod history;
mod media;
mod media_convert;
mod media_prep;
mod migration;
mod pptx;
mod relink;
mod scripture;
mod search;
mod showbox;
mod showbox_backups;
mod site_presence;
mod write_through;
