//! Converters from ProPresenter protobuf messages (`crate::pb::rv::data`, imported as `pp`) to
//! `model` documents, split into four areas that are implemented independently.
//!
//! ## Areas and file ownership
//!
//! | area | files (only this area edits them) | owns |
//! |---|---|---|
//! | **slides** | `slide.rs`, `element.rs`, `text.rs`, `fonts.rs` (+ maintains `common.rs` bodies) | cue → slide, actions → `SlideAction` dispatch, elements (geometry, shape, fill, stroke, shadow, feather, placeholders, data links, visibility), text boxes (RTF + attributes), notes, fonts (usage, report, substitution table) |
//! | **media** | `media.rs` | URLs → `MediaAsset` / `MediaRoot` (dedup, status, probe), media triggers, media fills, video-input triggers, media bins (`Playlists/Media`), missing-media report, media-root binding suggestions |
//! | **content** | `presentation.rs`, `library.rs`, `playlist.rs`, `theme.rs` | libraries, presentations (sections, arrangements, song info, scripture, group links), playlists (`Playlists/Library`, `Playlists/Audio`), themes (dedup, template slides) |
//! | **config** | `workspace.rs`, `config.rs` | screens, output bindings, looks, masks, video / audio inputs + bindings, audio device, global groups, timers, stage layouts, clear groups, props, messages, macros, CCLI → show settings, key mappings / calendar (report only), theme-link post-pass |
//!
//! Shared and frozen (scaffold): `crate::context` (ImportContext, lookups, ids, JSON dumps),
//! `crate::bundle`, `crate::report`, `crate::source`, `common.rs` (bodies maintained by slides).
//! A change to a shared file or to another area's *signature* must be agreed with that area
//! first; adding private helpers inside your own files is always fine.
//!
//! ## Cross-area calls (the complete allowed list)
//! - every area → `common::*`, `ImportContext` methods, `context::ids::*`
//! - slides → media: `media::media_action`, `media::media_fill`, `media::asset_for_url`
//! - slides → config: `config::clear_target`, `config::timer_kind`
//! - media → slides: none (media never builds slides)
//! - content → slides: `slide::slide_from_cue`, `slide::slide_from_pb`, `slide::slide_actions`,
//!   `text::rich_text_from_rtf` (presentation notes)
//! - content → media: `media::media_action` (inline playlist cues), `media::origin`
//!   (verbatim origin of an unresolved playlist presentation item)
//! - config → slides: `slide::slide_from_pb`, `slide::slide_actions`, `slide::slide_from_cue`
//!   (prop cues), `element::timer_format`, `element::clock_format`
//! - config → media: `media::asset_for_url` (input thumbnails, mask fills go through slides)
//! - config → content: none (theme links resolve through `ctx.lookups.themes` /
//!   `ctx.lookups.template_slides`)
//!
//! ## Conventions
//! - Every converter takes `ctx: &mut ImportContext` first and never panics.
//! - *Pure* converters (`fn element(ctx, &pp::…) -> Element`) return model values and do not
//!   push documents; `import_*` functions push into `ctx.out` and register lookups.
//! - Reuse source UUIDs (`ids::parse_uuid` + `XxxId::from_uuid`); mint with `ids::derived` /
//!   `ids::source_or_derived` when missing. Sibling order → `ids::sort_keys`.
//! - Provenance: document-level entities get `ctx.document_info()`; nested entities get an
//!   `ImportInfo` only when they carry something (original name, original id, unmapped), via
//!   `ctx.keep_unmapped`. Leave `import: None` otherwise (compact JSON).
//! - Media assets created while converting a theme are theme-scoped: the content area sets
//!   `ctx.current.asset_scope` / `ctx.current.resource_dir` for the theme; other areas pass
//!   `&ctx.current.asset_scope.clone()` where a scope is asked for.

pub mod common;

// ---- slides area ----
pub mod element;
pub mod fonts;
pub mod slide;
pub mod text;

// ---- media area ----
pub mod media;

// ---- content area ----
pub mod library;
pub mod playlist;
pub mod presentation;
pub mod theme;

// ---- config area ----
pub mod config;
pub mod workspace;
