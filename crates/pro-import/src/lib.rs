//! Reads ProPresenter 7+ files (Protocol Buffers) and converts them into our own `model`.
//!
//! The `.proto` schemas are community reverse-engineered and fetched at build time, not kept in
//! the repository (see `PROTO.md`: `scripts/fetch-propresenter-proto.sh`); [`pb`] holds the
//! prost types (every message implements `prost::Name`, so any sub-message can be dumped as JSON
//! through prost-reflect, see [`ImportContext::to_json`]).
//!
//! ## Importer
//! [`import_backup`] converts a whole ProPresenter data folder (backup) into an [`ImportBundle`]
//! (every model document plus the [`ImportReport`]); [`ImportBundle::write_bundle`] writes it out
//! (`pp-import <backup-dir> <out-dir> [--check]`). The importer never writes into the backup,
//! never copies media and never touches the network. Module map:
//!
//! | module | role |
//! |---|---|
//! | [`context`] | [`ImportContext`]: backup root, output, report, lookups, IDs, JSON dumps |
//! | [`source`] | backup layout and sorted, read-only file discovery |
//! | [`bundle`] | [`ImportBundle`] and the deterministic writer |
//! | [`report`] | [`ImportReport`] (categories) and `report.md` |
//! | [`convert`] | the converters, split into the slides / media / content / config areas |
//! | [`census`] | invariants of the reference backup (`--check`) |
//!
//! ## Phase order (lookups flow forward only)
//! 1. **Libraries** (`Libraries/LibraryData` + folders) → `lookups.libraries`.
//! 2. **Configuration**, in this order:
//!    Groups (→ `lookups.global_groups`, used by presentations) → Timers (→ `lookups.timers`, used
//!    by data links in themes / stage / slides) → Workspace (screens, inputs → `lookups.video_inputs`
//!    used by live-video actions everywhere later; looks with provisional theme links; masks) →
//!    Stage → ClearGroups → Props → Messages → Macros → CCLI (show settings) → KeyMappings and
//!    Calendar (report only).
//! 3. **Themes** (`Themes/**/Theme`, de-duplicated) → `lookups.themes`, `lookups.template_slides`.
//! 4. **Theme-link post-pass** (`workspace::resolve_theme_links`): look template overrides and
//!    message templates now resolve to the kept themes.
//! 5. **Presentations**, library by library in sidebar order, files sorted →
//!    `lookups.presentations` (needs groups, timers, video inputs).
//! 6. **Playlists**: `Playlists/Library` (service, needs presentations), `Playlists/Audio`
//!    (audio playlists), `Playlists/Media` (media bins).
//! 7. **Finalize**: missing-media report + root binding suggestions (`media::finalize_media`),
//!    font report + substitution table (`fonts::font_report`), counts, sort by ID.
//!
//! Every file is converted inside [`ImportContext::guarded`]: a decode error or a converter panic
//! becomes a `parseFailures` entry and the import continues.

pub mod bundle;
pub mod census;
pub mod context;
pub mod convert;
pub mod report;
pub mod source;

#[allow(clippy::all, non_camel_case_types, dead_code, missing_docs)]
pub mod pb {
    include!(concat!(env!("OUT_DIR"), "/_includes.rs"));
}

use std::path::Path;

use prost_reflect::DescriptorPool;

pub use bundle::{ImportBundle, WriteSummary};
pub use context::{ImportContext, ImportOptions};
pub use report::ImportReport;

use convert::{config, fonts, library, media, playlist, presentation, theme, workspace};
use pb::rv::data as pp;

/// Encoded `FileDescriptorSet` for every vendored ProPresenter schema.
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/propresenter.fds"));

/// Descriptor pool for dynamic decoding (e.g. dumping a file as JSON for inspection).
pub fn descriptor_pool() -> DescriptorPool {
    DescriptorPool::decode(FILE_DESCRIPTOR_SET).expect("embedded descriptor set is valid")
}

/// Imports the ProPresenter data folder at `root` with default options. Never panics; problems
/// are in `bundle.report`.
pub fn import_backup(root: &Path) -> ImportBundle {
    import_backup_with(root, ImportOptions::default())
}

/// Imports the ProPresenter data folder at `root` (see the phase order in the crate docs).
pub fn import_backup_with(root: &Path, options: ImportOptions) -> ImportBundle {
    let mut ctx = ImportContext::new(root, options);

    // 1. Libraries.
    let library_data = if ctx.file_exists(source::LIBRARY_DATA) {
        ctx.read_document::<pp::Library>(source::LIBRARY_DATA)
            .map(|(doc, _)| doc)
    } else {
        None
    };
    let found_folders = source::library_folders(root);
    let folders: Vec<String> = found_folders.iter().map(|f| f.rel.clone()).collect();
    let libraries = ctx
        .guarded(source::LIBRARY_DATA, |ctx| {
            library::import_libraries(ctx, library_data.as_ref(), &folders)
        })
        // If the library converter failed, still import every folder's presentations.
        .unwrap_or_else(|| {
            folders
                .iter()
                .map(|f| (f.clone(), context::ids::library_id(f)))
                .collect()
        });

    // 2. Configuration.
    use source::config as cfg;
    import_config(&mut ctx, cfg::GROUPS, config::import_groups);
    import_config(&mut ctx, cfg::TIMERS, config::import_timers);
    import_config(&mut ctx, cfg::WORKSPACE, workspace::import_workspace);
    import_config(&mut ctx, cfg::STAGE, config::import_stage);
    import_config(&mut ctx, cfg::CLEAR_GROUPS, config::import_clear_groups);
    import_config(&mut ctx, cfg::PROPS, config::import_props);
    import_config(&mut ctx, cfg::MESSAGES, config::import_messages);
    import_config(&mut ctx, cfg::MACROS, config::import_macros);
    import_config(&mut ctx, cfg::CCLI, config::import_ccli);
    import_config(&mut ctx, cfg::KEY_MAPPINGS, config::report_key_mappings);
    import_config(&mut ctx, cfg::CALENDAR, config::report_calendar);

    // 3. Themes.
    let theme_files = source::theme_files(root);
    let keys = context::ids::sort_keys(theme_files.len());
    for (f, key) in theme_files.iter().zip(keys) {
        ctx.found(f);
        let rel = &f.rel;
        if let Some((doc, raw)) = ctx.read_document::<pp::template::Document>(rel) {
            ctx.guarded(rel, |ctx| theme::import_theme(ctx, &doc, rel, &raw, key));
        }
    }

    // 4. Theme links of looks and messages.
    ctx.guarded(cfg::WORKSPACE, workspace::resolve_theme_links);

    // 5. Presentations.
    for (folder, library) in &libraries {
        // By the folder's real path: its name may not be valid UTF-8.
        let Some(dir) = found_folders.iter().find(|f| f.rel == *folder) else {
            continue;
        };
        for f in source::presentation_files(root, &dir.path) {
            ctx.found(&f);
            let rel = f.rel;
            if let Some((doc, _)) = ctx.read_document::<pp::Presentation>(&rel) {
                ctx.guarded(&rel, |ctx| {
                    presentation::import_presentation(ctx, &doc, &rel, *library)
                });
            }
        }
    }

    // 6. Playlists and bins.
    import_playlist_doc(&mut ctx, source::PLAYLISTS_LIBRARY, |ctx, doc, rel| {
        playlist::import_playlists(ctx, doc, rel, model::PlaylistPurpose::Service)
    });
    import_playlist_doc(&mut ctx, source::PLAYLISTS_AUDIO, |ctx, doc, rel| {
        playlist::import_playlists(ctx, doc, rel, model::PlaylistPurpose::Audio)
    });
    import_playlist_doc(&mut ctx, source::PLAYLISTS_MEDIA, media::import_media_bins);

    // 7. Finalize.
    ctx.guarded("Media", media::finalize_media);
    ctx.guarded("fonts", fonts::font_report);
    ctx.finish()
}

/// Reads, decodes and converts one optional `Configuration/` document.
fn import_config<M: prost::Message + Default>(
    ctx: &mut ImportContext,
    rel: &str,
    convert: impl FnOnce(&mut ImportContext, &M, &str),
) {
    if !ctx.file_exists(rel) {
        return;
    }
    if let Some((doc, _)) = ctx.read_document::<M>(rel) {
        ctx.guarded(rel, |ctx| convert(ctx, &doc, rel));
    }
}

/// Reads, decodes and converts one optional playlist document.
fn import_playlist_doc(
    ctx: &mut ImportContext,
    rel: &str,
    convert: impl FnOnce(&mut ImportContext, &pp::PlaylistDocument, &str),
) {
    import_config(ctx, rel, convert);
}
