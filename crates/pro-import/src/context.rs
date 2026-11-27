//! The import context: everything converters share while one backup is imported.
//!
//! [`ImportContext`] owns
//! - the backup root and [`ImportOptions`],
//! - the accumulating output ([`ImportBundle`], field `out`) and [`ImportReport`] (field `report`),
//! - [`Lookups`]: tables filled by earlier phases and read by later ones (see the phase order in
//!   the crate docs),
//! - [`CurrentDocument`]: provenance and resolution base of the file being converted,
//! - the protobuf descriptor pool used to dump any prost message as JSON ([`ImportContext::to_json`]).
//!
//! ## Rules for converters
//! - **Never panic.** Bad or surprising input becomes a warning ([`ImportContext::warn`]) and a
//!   best-effort value. The orchestrator additionally runs every file's conversion inside
//!   [`ImportContext::guarded`], which turns a panic into a `parseFailures` entry.
//! - **Deterministic.** Only `BTreeMap`/`BTreeSet` in lookups, IDs from [`ids`] (source UUIDs or
//!   UUIDv5), no clocks, no randomness. `ImportInfo::imported_at` stays `None` unless the caller
//!   sets [`ImportOptions::imported_at`].
//! - **Paths** handed around are relative to the backup root, `/`-separated, without a leading
//!   slash ([`crate::source::normalize_rel`]). Only the context and [`crate::source`] touch the
//!   file system, read-only.
//! - **Unmapped data** is kept with [`ImportContext::keep_unmapped`] (which also counts the feature
//!   for the report), never dropped silently.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use model::{
    ArrangementId, AssetScope, AudioInputId, ClearGroupId, GlobalGroupId, ImportInfo, LibraryId,
    LookId, MacroId, MaskId, MediaAssetId, MediaRootId, MessageId, PresentationId, PropId,
    ScreenId, StageLayoutId, TemplateSlideId, ThemeId, TimerId, VideoInputId,
};
use prost_reflect::{DescriptorPool, DynamicMessage};
use serde_json::Value;
use uuid::Uuid;

use crate::bundle::ImportBundle;
use crate::pb::rv::data as pp;
use crate::report::{Category, ImportReport, ParseFailure};

/// Knobs of one import run.
#[derive(Clone, Debug)]
pub struct ImportOptions {
    /// Probe media files that exist inside the backup (cheap header sniffing for image size).
    pub probe_media: bool,
    /// Written to `ImportInfo::imported_at` on document-level provenance. `None` (default) keeps
    /// the output byte-identical across runs.
    pub imported_at: Option<String>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            probe_media: true,
            imported_at: None,
        }
    }
}

/// The source document being converted: provenance and the base for relative references.
///
/// Set by [`ImportContext::guarded`] (path) and refined by the area converter
/// ([`ImportContext::set_writer`], `asset_scope`, `resource_dir`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CurrentDocument {
    /// Path relative to the backup root (`"Libraries/Music/Song.pro"`).
    pub path: Option<String>,
    /// Version of the app that last wrote it (`"21.2.0"`), from its `applicationInfo`.
    pub writer_version: Option<String>,
    /// Scope of media assets created while converting it (`Theme` inside a theme).
    pub asset_scope: AssetScope,
    /// Folder that `ROOT_CURRENT_RESOURCE` URLs are relative to (the theme folder, relative to
    /// the backup root). `None` outside themes.
    pub resource_dir: Option<String>,
}

/// An ID plus the source name it was imported with (for name hints on links).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedId<T> {
    /// The model ID.
    pub id: T,
    /// The source name.
    pub name: String,
}

/// A presentation registered by the content area (playlist resolution input).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresentationEntry {
    /// Its ID.
    pub id: PresentationId,
    /// Its library.
    pub library: LibraryId,
    /// `Presentation.name`.
    pub name: String,
    /// File name without `.pro` (library-scoped fallback key).
    pub basename: String,
    /// Arrangements that exist in the imported presentation.
    pub arrangements: Vec<ArrangementId>,
}

/// A theme registered by the content area (look / message template resolution input).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeEntry {
    /// The kept theme's ID (a de-duplicated copy maps to the kept original).
    pub id: ThemeId,
    /// Display name.
    pub name: String,
    /// Path of the kept theme folder relative to the backup root.
    pub folder: String,
    /// Its template slides in order.
    pub slides: Vec<TemplateSlideId>,
}

/// Identity of a font as the source names it, for usage counting.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontKey {
    /// PostScript name (`attributes.font.name` / RTF `\fonttbl`), may be empty.
    pub postscript_name: String,
    /// Family (`attributes.font.family`), may be empty.
    pub family: String,
    /// Face (`attributes.font.face`), may be empty.
    pub face: String,
}

/// Lookup tables shared between phases. Each table names the area that fills it; any area may
/// read any table once its phase has run (see the phase order in the crate docs).
#[derive(Clone, Debug, Default)]
pub struct Lookups {
    // ---- content area (library.rs / presentation.rs / theme.rs) -------------------------------
    /// Library folder name (`"Music"`) → library. Filled by `library::import_libraries`.
    pub libraries: BTreeMap<String, LibraryId>,
    /// Normalized relative path (`"Libraries/Music/Song.pro"`) → presentation. Filled by
    /// `presentation::import_presentation`.
    pub presentations: BTreeMap<String, PresentationEntry>,
    /// Theme folder path relative to the root (`"Themes/Lower Thirds"`) → theme. Duplicates map
    /// to the kept theme. Filled by `theme::import_theme`.
    pub themes: BTreeMap<String, ThemeEntry>,
    /// Raw theme file bytes → folder of the kept theme (byte-identical de-duplication).
    pub theme_contents: BTreeMap<Vec<u8>, String>,
    /// Source template-slide UUID → (kept theme, template slide). Filled by `theme::import_theme`.
    pub template_slides: BTreeMap<Uuid, (ThemeId, TemplateSlideId)>,

    // ---- config area (config.rs / workspace.rs) -------------------------------------------------
    /// Global group UUID (`applicationGroupIdentifier`) → group. Filled by `config::import_groups`.
    pub global_groups: BTreeMap<Uuid, NamedId<GlobalGroupId>>,
    /// Timer UUID → timer. Filled by `config::import_timers`.
    pub timers: BTreeMap<Uuid, NamedId<TimerId>>,
    /// ProScreen UUID → screen. Filled by `workspace::import_workspace`.
    pub screens: BTreeMap<Uuid, NamedId<ScreenId>>,
    /// Look UUID → look. Filled by `workspace::import_workspace`.
    pub looks: BTreeMap<Uuid, NamedId<LookId>>,
    /// Mask UUID → mask. Filled by `workspace::import_workspace`.
    pub masks: BTreeMap<Uuid, NamedId<MaskId>>,
    /// Video inputs in ProPresenter list order (`liveVideoIndex` indexes this). Filled by
    /// `workspace::import_workspace`.
    pub video_inputs: Vec<NamedId<VideoInputId>>,
    /// Audio inputs in list order (`AudioInputType.index` indexes this).
    pub audio_inputs: Vec<NamedId<AudioInputId>>,
    /// Stage layout UUID → layout. Filled by `config::import_stage`.
    pub stage_layouts: BTreeMap<Uuid, NamedId<StageLayoutId>>,
    /// Clear group UUID → clear group. Filled by `config::import_clear_groups`.
    pub clear_groups: BTreeMap<Uuid, NamedId<ClearGroupId>>,
    /// Prop (cue) UUID → prop. Filled by `config::import_props`.
    pub props: BTreeMap<Uuid, NamedId<PropId>>,
    /// Message UUID → message. Filled by `config::import_messages`.
    pub messages: BTreeMap<Uuid, NamedId<MessageId>>,
    /// Macro UUID → macro. Filled by `config::import_macros`.
    pub macros: BTreeMap<Uuid, NamedId<MacroId>>,

    // ---- media area (media.rs) -------------------------------------------------------------------
    /// Media root key (see `media.rs`) → root.
    pub media_roots: BTreeMap<String, MediaRootId>,
    /// Asset → index into `ctx.out.media_assets` (de-duplication).
    pub media_assets: BTreeMap<MediaAssetId, usize>,
    /// Asset → number of references (use sites) seen, for the missing-media report.
    pub asset_references: BTreeMap<MediaAssetId, usize>,

    // ---- slides area (fonts.rs) ------------------------------------------------------------------
    /// Font → number of uses (box default styles and RTF runs).
    pub fonts: BTreeMap<FontKey, usize>,
}

impl Lookups {
    /// Presentation by its path relative to the backup root (normalized; exact match).
    pub fn presentation_by_path(&self, rel_path: &str) -> Option<&PresentationEntry> {
        self.presentations
            .get(&crate::source::normalize_rel(rel_path))
    }

    /// Presentations of `library` whose file basename (without `.pro`) equals `basename`,
    /// compared case-insensitively. Fallbacks are library-scoped, never cross libraries.
    pub fn presentations_in_library(
        &self,
        library: LibraryId,
        basename: &str,
    ) -> Vec<&PresentationEntry> {
        self.presentations
            .values()
            .filter(|p| p.library == library && p.basename.eq_ignore_ascii_case(basename))
            .collect()
    }

    /// Video input by ProPresenter list index (`liveVideoIndex`).
    pub fn video_input_by_index(&self, index: i32) -> Option<&NamedId<VideoInputId>> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.video_inputs.get(i))
    }

    /// Theme by folder path relative to the root (`"Themes/Lower Thirds"`) or by the path of its
    /// `Theme` file (`"Themes/Lower Thirds/Theme"`).
    pub fn theme_by_path(&self, rel_path: &str) -> Option<&ThemeEntry> {
        let p = crate::source::normalize_rel(rel_path);
        self.themes.get(&p).or_else(|| {
            p.strip_suffix("/Theme")
                .and_then(|folder| self.themes.get(folder))
        })
    }
}

/// Shared state of one import run. See the module docs.
pub struct ImportContext {
    /// Backup root (read only).
    pub root: PathBuf,
    /// Options.
    pub options: ImportOptions,
    /// Accumulating output documents.
    pub out: ImportBundle,
    /// Accumulating report.
    pub report: ImportReport,
    /// Lookup tables.
    pub lookups: Lookups,
    /// The document being converted.
    pub current: CurrentDocument,
    pool: DescriptorPool,
    /// Real paths of discovered files whose text form is lossy ([`Self::found`]).
    real_paths: std::collections::HashMap<String, PathBuf>,
    /// Sorted entry names of the backup's folders, read once ([`Self::dir_names`]).
    dir_names: std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Arc<Vec<String>>>>,
}

impl ImportContext {
    /// A fresh context for the backup at `root`.
    pub fn new(root: &Path, options: ImportOptions) -> Self {
        ImportContext {
            root: root.to_path_buf(),
            options,
            out: ImportBundle {
                backup_root: Some(root.to_path_buf()),
                ..ImportBundle::default()
            },
            report: ImportReport::default(),
            lookups: Lookups::default(),
            current: CurrentDocument::default(),
            pool: crate::descriptor_pool(),
            real_paths: std::collections::HashMap::new(),
            dir_names: Default::default(),
        }
    }

    /// The entry names of a folder of the backup, sorted (byte order); `None` when it can't be
    /// read. Read once per run: media references look the same folders up again and again.
    pub fn dir_names(&self, dir: &Path) -> Option<std::sync::Arc<Vec<String>>> {
        let mut cache = self.dir_names.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = cache.get(dir) {
            return Some(n.clone());
        }
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let names = std::sync::Arc::new(names);
        cache.insert(dir.to_path_buf(), names.clone());
        Some(names)
    }

    /// Notes a discovered file: when its name is not valid UTF-8, [`Self::path`] then reads it
    /// by its real path, and the report says so.
    pub fn found(&mut self, f: &crate::source::Found) {
        if f.is_exact() {
            return;
        }
        self.real_paths
            .insert(crate::source::normalize_rel(&f.rel), f.path.clone());
        self.report.warn(
            Category::Other,
            Some(f.rel.clone()),
            "the file name is not valid UTF-8 (perhaps unpacked from a zip made without UTF-8 \
             names); imported, with \u{FFFD} for the characters that could not be read"
                .to_string(),
        );
    }

    /// Ends the run: computes the report counts, sorts every document list by ID and returns the
    /// bundle (with the report inside).
    pub fn finish(mut self) -> ImportBundle {
        self.out.sort();
        self.report.compute_counts(&self.out);
        self.out.report = self.report;
        self.out
    }

    // ---- files ------------------------------------------------------------------------------

    /// Absolute path of a backup-relative path.
    pub fn path(&self, rel_path: &str) -> PathBuf {
        let rel = crate::source::normalize_rel(rel_path);
        if let Some(p) = self.real_paths.get(&rel) {
            return p.clone();
        }
        let mut p = self.root.clone();
        for part in rel.split('/').filter(|s| !s.is_empty()) {
            p.push(part);
        }
        p
    }

    /// True when the backup-relative path is an existing file.
    pub fn file_exists(&self, rel_path: &str) -> bool {
        self.path(rel_path).is_file()
    }

    /// Reads a file of the backup. An I/O error is recorded as a parse failure.
    pub fn read_bytes(&mut self, rel_path: &str) -> Option<Vec<u8>> {
        match std::fs::read(self.path(rel_path)) {
            Ok(b) => Some(b),
            Err(e) => {
                self.parse_failure(rel_path, format!("read failed: {e}"));
                None
            }
        }
    }

    /// Decodes a protobuf document. A decode error is recorded as a parse failure.
    pub fn decode<M: prost::Message + Default>(
        &mut self,
        rel_path: &str,
        bytes: &[u8],
    ) -> Option<M> {
        match M::decode(bytes) {
            Ok(m) => Some(m),
            Err(e) => {
                self.parse_failure(rel_path, format!("decode failed: {e}"));
                None
            }
        }
    }

    /// Reads and decodes a protobuf document; returns the message and its raw bytes.
    pub fn read_document<M: prost::Message + Default>(
        &mut self,
        rel_path: &str,
    ) -> Option<(M, Vec<u8>)> {
        let bytes = self.read_bytes(rel_path)?;
        let msg = self.decode::<M>(rel_path, &bytes)?;
        Some((msg, bytes))
    }

    /// Records a file that could not be read, decoded or converted.
    pub fn parse_failure(&mut self, rel_path: &str, error: impl Into<String>) {
        self.report.parse_failures.push(ParseFailure {
            file: crate::source::normalize_rel(rel_path),
            error: error.into(),
        });
    }

    /// Runs `f` with `rel_path` as the current document. A panic inside `f` is caught and recorded
    /// as a parse failure of that file (the import continues; whatever `f` already pushed stays).
    /// The previous current document is restored afterwards.
    pub fn guarded<R>(&mut self, rel_path: &str, f: impl FnOnce(&mut Self) -> R) -> Option<R> {
        let previous = std::mem::replace(
            &mut self.current,
            CurrentDocument {
                path: Some(crate::source::normalize_rel(rel_path)),
                ..CurrentDocument::default()
            },
        );
        let result = catch_unwind(AssertUnwindSafe(|| f(self)));
        self.current = previous;
        match result {
            Ok(r) => Some(r),
            Err(panic) => {
                let msg = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".into());
                self.parse_failure(rel_path, format!("conversion panicked: {msg}"));
                None
            }
        }
    }

    /// Records the writer version of the current document from its `applicationInfo`.
    pub fn set_writer(&mut self, app: Option<&pp::ApplicationInfo>) {
        self.current.writer_version = crate::convert::common::writer_version(app);
    }

    // ---- report helpers ------------------------------------------------------------------------

    /// Adds a warning under `category`, tagged with the current document.
    pub fn warn(&mut self, category: Category, detail: impl Into<String>) {
        let document = self.current.path.clone();
        self.report.warn(category, document, detail.into());
    }

    /// Counts an unsupported action type (`"ACTION_TYPE_COMMUNICATION"`).
    pub fn count_unsupported_action(&mut self, source_type: &str) {
        *self
            .report
            .unsupported_actions
            .entry(source_type.to_string())
            .or_default() += 1;
    }

    /// Counts an unmapped feature without keeping data (`"element.textScroller"`).
    pub fn count_unmapped(&mut self, feature: &str) {
        *self
            .report
            .unmapped_features
            .entry(feature.to_string())
            .or_default() += 1;
    }

    /// Keeps unmodelled source data in `info.unmapped` and counts the feature.
    ///
    /// `feature` is a dotted source path (`"element.buildIn"`); the JSON key is its last segment
    /// (`"buildIn"`). `info` is created when `None`; `unmapped` becomes an object when null.
    pub fn keep_unmapped(&mut self, info: &mut Option<ImportInfo>, feature: &str, value: Value) {
        self.count_unmapped(feature);
        let key = feature.rsplit('.').next().unwrap_or(feature).to_string();
        let info = info.get_or_insert_with(ImportInfo::default);
        if !info.unmapped.is_object() {
            info.unmapped = Value::Object(Default::default());
        }
        if let Value::Object(map) = &mut info.unmapped {
            map.insert(key, value);
        }
    }

    // ---- provenance ----------------------------------------------------------------------------

    /// Document-level provenance: source app, current document path, writer version and (when
    /// configured) the import time. Use on presentations, themes, playlists (root nodes), screens,
    /// looks.
    pub fn document_info(&self) -> ImportInfo {
        ImportInfo {
            document_path: self.current.path.clone(),
            writer_version: self.current.writer_version.clone(),
            imported_at: self.options.imported_at.clone(),
            ..ImportInfo::default()
        }
    }

    // ---- JSON dumps ------------------------------------------------------------------------------

    /// Any prost message as JSON through prost-reflect (proto3 JSON names, camelCase, enum names,
    /// defaults omitted). Used for `import.unmapped` and `Unsupported { payload }`. Returns
    /// `Value::Null` if the message cannot be described (never panics).
    pub fn to_json<M: prost::Message + prost::Name>(&self, msg: &M) -> Value {
        message_to_json(&self.pool, msg)
    }

    /// The descriptor pool (for callers that need dynamic access).
    pub fn pool(&self) -> &DescriptorPool {
        &self.pool
    }
}

/// See [`ImportContext::to_json`].
pub fn message_to_json<M: prost::Message + prost::Name>(pool: &DescriptorPool, msg: &M) -> Value {
    let Some(desc) = pool.get_message_by_name(&M::full_name()) else {
        return Value::Null;
    };
    let bytes = msg.encode_to_vec();
    match DynamicMessage::decode(desc, bytes.as_slice()) {
        Ok(dm) => serde_json::to_value(&dm).unwrap_or(Value::Null),
        Err(_) => Value::Null,
    }
}

/// Deterministic identifiers.
///
/// - Content entities reuse source UUIDs ([`ids::parse_uuid`], then `XxxId::from_uuid`).
/// - When a source UUID is missing, nil or invalid, or an entity has none (libraries, themes,
///   media roots, playlist items, bindings), the ID is a UUIDv5 under [`ids::IMPORT_NAMESPACE`]
///   of a kind tag plus stable parts ([`ids::derived`]). Re-importing the same backup yields the
///   same IDs.
/// - Linked media assets use the model's own [`MediaAssetId::for_linked`] (root + path).
pub mod ids {
    use super::*;
    use model::{OutputId, PlaylistId, PlaylistItemId, SortKey};

    /// Namespace of every UUIDv5 the importer mints. Never change it: IDs of re-imports would
    /// change.
    pub const IMPORT_NAMESPACE: Uuid = Uuid::from_u128(0x6f1c_2a4e_8d3b_4c57_9e21_5b7a_0c94_d3e8);

    /// A source UUID, or `None` when missing, unparsable or nil.
    pub fn parse_uuid(u: Option<&pp::Uuid>) -> Option<Uuid> {
        let s = u?.string.trim();
        Uuid::parse_str(s).ok().filter(|u| !u.is_nil())
    }

    /// UUIDv5 of `kind` and `parts` (joined with U+001F, which never occurs in names or paths).
    pub fn derived(kind: &str, parts: &[&str]) -> Uuid {
        let mut name = String::from(kind);
        for p in parts {
            name.push('\u{1f}');
            name.push_str(p);
        }
        Uuid::new_v5(&IMPORT_NAMESPACE, name.as_bytes())
    }

    /// The source UUID if valid, else [`derived`]`(kind, parts)`. `parts` must identify the entity
    /// within the backup (e.g. document path + index).
    pub fn source_or_derived(u: Option<&pp::Uuid>, kind: &str, parts: &[&str]) -> Uuid {
        parse_uuid(u).unwrap_or_else(|| derived(kind, parts))
    }

    /// Library of a `Libraries/<folder>` folder.
    pub fn library_id(folder: &str) -> LibraryId {
        LibraryId::from_uuid(derived("library", &[folder]))
    }

    /// Theme of a theme folder (path relative to the root, `"Themes/Lyric Styles/Lines"`).
    /// Template documents have no UUID of their own.
    pub fn theme_id(folder_rel_path: &str) -> ThemeId {
        ThemeId::from_uuid(derived(
            "theme",
            &[&crate::source::normalize_rel(folder_rel_path)],
        ))
    }

    /// Playlist item `index` of `playlist` (source item UUIDs collide with cue / presentation
    /// UUIDs, so they are never reused; keep the original in `import.originalId`).
    pub fn playlist_item_id(playlist: PlaylistId, index: usize) -> PlaylistItemId {
        PlaylistItemId::from_uuid(derived(
            "playlistItem",
            &[&playlist.to_string(), &index.to_string()],
        ))
    }

    /// Media root for a root key (see `media.rs` for the key scheme).
    pub fn media_root_id(key: &str) -> MediaRootId {
        MediaRootId::from_uuid(derived("mediaRoot", &[key]))
    }

    /// Asset inside a theme folder (`path` relative to the theme folder).
    pub fn theme_asset_id(theme: ThemeId, path: &str) -> MediaAssetId {
        MediaAssetId::from_uuid(derived("themeAsset", &[&theme.to_string(), path]))
    }

    /// Unresolved asset (`key` = the most specific source reference, e.g. the absolute string).
    pub fn unresolved_asset_id(key: &str) -> MediaAssetId {
        MediaAssetId::from_uuid(derived("unresolvedAsset", &[key]))
    }

    /// Output binding `index` of a screen.
    pub fn output_id(screen: ScreenId, index: usize) -> OutputId {
        OutputId::from_uuid(derived(
            "output",
            &[&screen.to_string(), &index.to_string()],
        ))
    }

    /// The key output of a Fill/Key pair whose fill is [`output_id`]`(screen, index)`.
    pub fn key_output_id(screen: ScreenId, index: usize) -> OutputId {
        OutputId::from_uuid(derived(
            "output",
            &[&screen.to_string(), &index.to_string(), "key"],
        ))
    }

    /// `n` strictly increasing, evenly spaced, fixed-width sort keys (digits `0-9A-Za-z`, never
    /// ending in `'0'`), compatible with [`model::sort_key_between`]. Assign them to siblings in
    /// source order.
    pub fn sort_keys(n: usize) -> Vec<SortKey> {
        const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
        if n == 0 {
            return Vec::new();
        }
        // Width w with 62^w >= 2(n+1), so neighbouring keys differ by >= 2 and bumping a
        // trailing '0' by one keeps the order.
        let need = 2 * (n as u128 + 1);
        let (mut width, mut space) = (1u32, 62u128);
        while space < need {
            width += 1;
            space *= 62;
        }
        (1..=n as u128)
            .map(|i| {
                let mut v = i * space / (n as u128 + 1);
                if v.is_multiple_of(62) {
                    v += 1;
                }
                let mut digits = vec![b'0'; width as usize];
                for slot in digits.iter_mut().rev() {
                    *slot = DIGITS[(v % 62) as usize];
                    v /= 62;
                }
                String::from_utf8(digits).expect("ASCII digits")
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::ids::*;
    use super::*;

    #[test]
    fn sort_keys_are_ordered_and_compatible() {
        for n in [1usize, 2, 5, 61, 62, 200, 4000] {
            let keys = sort_keys(n);
            assert_eq!(keys.len(), n);
            for w in keys.windows(2) {
                assert!(w[0] < w[1], "{} !< {}", w[0], w[1]);
            }
            assert!(keys.iter().all(|k| !k.ends_with('0')));
            // Inserting between neighbours works with the model's helper.
            if n >= 2 {
                let mid = model::sort_key_between(Some(&keys[0]), Some(&keys[1]));
                assert!(keys[0] < mid && mid < keys[1]);
            }
        }
    }

    #[test]
    fn derived_ids_are_stable_and_distinct() {
        assert_eq!(
            derived("library", &["Music"]),
            derived("library", &["Music"])
        );
        assert_ne!(
            derived("library", &["Music"]),
            derived("library", &["Songs"])
        );
        assert_ne!(derived("a", &["bc"]), derived("ab", &["c"]));
        assert_eq!(library_id("Music"), library_id("Music"));
    }

    #[test]
    fn parse_uuid_rejects_nil_and_garbage() {
        let u = |s: &str| pp::Uuid { string: s.into() };
        assert!(parse_uuid(Some(&u("00000000-0000-0000-0000-000000000000"))).is_none());
        assert!(parse_uuid(Some(&u("not a uuid"))).is_none());
        assert!(parse_uuid(None).is_none());
        assert!(parse_uuid(Some(&u("5B2E36A8-2A7C-4E0A-9F1C-0D6C9E7F1A23"))).is_some());
    }

    #[test]
    fn to_json_dumps_nested_messages() {
        let ctx = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        let font = pp::Font {
            name: "Example-Bold".into(),
            size: 72.0,
            bold: true,
            ..Default::default()
        };
        let v = ctx.to_json(&font);
        assert_eq!(v["name"], "Example-Bold");
        assert_eq!(v["bold"], true);
        let t = pp::graphics::Text::default();
        assert!(ctx.to_json(&t).is_object());
    }

    #[test]
    fn keep_unmapped_creates_object_and_counts() {
        let mut ctx = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        let mut info = None;
        ctx.keep_unmapped(&mut info, "element.buildIn", serde_json::json!({"a": 1}));
        ctx.keep_unmapped(&mut info, "element.info", serde_json::json!(7));
        let info = info.unwrap();
        assert_eq!(info.unmapped["buildIn"]["a"], 1);
        assert_eq!(info.unmapped["info"], 7);
        assert_eq!(ctx.report.unmapped_features["element.buildIn"], 1);
    }

    #[test]
    fn guarded_turns_panics_into_parse_failures() {
        let mut ctx = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        let prev = ctx.current.clone();
        let r: Option<()> = ctx.guarded("Libraries/X/Y.pro", |ctx| {
            assert_eq!(ctx.current.path.as_deref(), Some("Libraries/X/Y.pro"));
            panic!("boom")
        });
        assert!(r.is_none());
        assert_eq!(ctx.current, prev);
        assert_eq!(ctx.report.parse_failures.len(), 1);
        assert!(ctx.report.parse_failures[0].error.contains("boom"));
    }
}
