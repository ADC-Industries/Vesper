//! Loading a data directory laid out like `pro-import`'s output:
//!
//! ```text
//! <dir>/portable/<kind>/<id>.json   portable documents (presentations, screens, looks, ...)
//! <dir>/machine/<kind>/<id>.json    machine-local documents (output bindings, media roots, ...)
//! <dir>/media/<sha256>.<ext>        managed media (optional)
//! <dir>/fonts/                      extra fonts (optional)
//! ```
//!
//! `<kind>` is the camelCase [`DocumentKind`] name. Every document is migrated with
//! [`model::migrate`] before it is decoded: documents from a newer schema are refused, broken
//! files are reported, and loading carries on with the rest. Directories whose kind the server
//! does not use yet are still validated (migrated) so the load report covers the whole folder.
//!
//! The data splits in two (see `docs/CODEX-PLAN.md`):
//! - [`CollectionContent`]: the documents of one Codex collection ([`COLLECTION_KINDS`]:
//!   presentations, libraries, playlists, themes, media, bins, global groups and the documents
//!   slide actions refer to), from its own root and media folder;
//! - [`ShowData`]: this box's documents (screens, looks, show settings, the machine-local ones)
//!   plus the active collection's content, shared as an `Arc`.

use model::{
    AssetLocation, AudioDeviceConfig, ClearGroup, Document, DocumentKind, FontSubstitutionTable,
    GlobalGroup, GlobalGroupId, Library, LibraryId, Look, Macro, Mask, MediaAsset, MediaAssetId,
    MediaBin, MediaBinId, MediaRootBinding, MediaRootId, Message, MigrationError, OutputBinding,
    PlaylistId, PlaylistNode, Presentation, PresentationId, Prop, Screen, ShowSettings,
    StageLayout, Theme, ThemeId, Timer,
};
use model::{InputBinding, InputSource, VideoInput, VideoInputId};
use protocol::{DocumentChange, EditOps};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Library documents changed by an edit (`None` = removed), for [`ShowData::with_content`].
#[derive(Clone, Debug, Default)]
pub struct ContentChange {
    /// Presentations.
    pub presentations: Vec<(PresentationId, Option<Arc<Presentation>>)>,
    /// Libraries.
    pub libraries: Vec<(LibraryId, Option<Library>)>,
    /// Playlist nodes.
    pub playlists: Vec<(PlaylistId, Option<PlaylistNode>)>,
    /// Media assets.
    pub assets: Vec<(MediaAssetId, Option<MediaAsset>)>,
    /// Media and audio bins.
    pub media_bins: Vec<(MediaBinId, Option<MediaBin>)>,
    /// Themes.
    pub themes: Vec<(ThemeId, Option<Theme>)>,
    /// Global groups.
    pub global_groups: Vec<(GlobalGroupId, Option<GlobalGroup>)>,
}

impl ContentChange {
    /// Only media assets, bins or global groups changed (an upload, a probe's results, a
    /// relink, a bin or group edit): nothing a screen draws.
    pub fn media_only(&self) -> bool {
        self.presentations.is_empty()
            && self.libraries.is_empty()
            && self.playlists.is_empty()
            && self.themes.is_empty()
    }

    /// The media assets it changes.
    pub fn asset_ids(&self) -> Vec<MediaAssetId> {
        self.assets.iter().map(|(id, _)| *id).collect()
    }
}

/// One `documentChanged` to send.
#[derive(Clone, Debug)]
pub struct ChangeEvent {
    /// Document kind.
    pub kind: DocumentKind,
    /// Document.
    pub id: uuid::Uuid,
    /// What happened.
    pub change: DocumentChange,
    /// New revision (not for removals).
    pub revision: Option<String>,
    /// Applied edit ops.
    pub ops: Option<EditOps>,
}

/// One document that could not be loaded.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadIssue {
    /// File path.
    pub file: String,
    /// What went wrong.
    pub error: String,
    /// The document comes from a newer schema (refused, never overwritten).
    pub too_new: bool,
}

/// Summary of a data directory load.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadReport {
    /// Documents loaded, per kind directory.
    pub loaded: std::collections::BTreeMap<String, usize>,
    /// Documents that failed.
    pub issues: Vec<LoadIssue>,
}

/// Content fingerprint of every decoded document, keyed by kind and ID: a reload compares
/// two of these to announce what changed.
pub type DocHashes = HashMap<(DocumentKind, uuid::Uuid), u64>;

/// The documents of one collection ([`COLLECTION_KINDS`]), indexed. Immutable: an edit builds a
/// new one ([`CollectionContent::with_change`], [`CollectionContent::with_show_docs`]) that
/// shares every list it does not touch.
#[derive(Clone)]
pub struct CollectionContent {
    /// Root of the collection's documents (`<root>/portable/<kind>/<id>.json`).
    pub root: PathBuf,
    /// Folder of the managed media of the old store (`<sha256>.<ext>`).
    pub media_dir: PathBuf,
    /// The Codex's media library (library locations; see [`crate::library`]).
    pub library: Arc<crate::library::MediaLibrary>,
    /// Libraries by ID.
    pub libraries: Arc<HashMap<LibraryId, Library>>,
    /// Presentations sorted by name (each shared: an edit replaces one `Arc`).
    pub presentations: Arc<Vec<Arc<Presentation>>>,
    /// Index into `presentations`.
    pub presentation_index: Arc<HashMap<PresentationId, usize>>,
    /// Themes by ID (template overrides, theme assets).
    pub themes: Arc<HashMap<ThemeId, Theme>>,
    /// Media assets by ID.
    pub assets: Arc<HashMap<MediaAssetId, MediaAsset>>,
    /// Media root → local folder, from this box's media root bindings (machine-local).
    pub media_roots: Arc<HashMap<MediaRootId, PathBuf>>,
    /// Playlist tree nodes (folders and playlists) by ID.
    pub playlists: Arc<HashMap<PlaylistId, PlaylistNode>>,
    /// Clear groups sorted by sort key.
    pub clear_groups: Arc<Vec<ClearGroup>>,
    /// Timers sorted by sort key.
    pub timers: Arc<Vec<Timer>>,
    /// Props sorted by sort key.
    pub props: Arc<Vec<Prop>>,
    /// Messages sorted by sort key.
    pub messages: Arc<Vec<Message>>,
    /// Macros sorted by sort key (run by macro actions; not editable through the API yet).
    pub macros: Arc<Vec<Macro>>,
    /// Stage layouts sorted by sort key (chosen by stage layout actions and stage screens).
    pub stage_layouts: Arc<Vec<StageLayout>>,
    /// Media and audio bins, sorted by folder path, then sort key, then name.
    pub media_bins: Arc<Vec<MediaBin>>,
    /// Global groups (the group pickers' list), sorted by sort key then name.
    pub global_groups: Arc<Vec<GlobalGroup>>,
    /// Per-document fingerprints of the collection's documents.
    pub hashes: DocHashes,
    /// What happened while loading the collection's documents.
    pub report: Arc<LoadReport>,
}

impl std::fmt::Debug for CollectionContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollectionContent")
            .field("root", &self.root)
            .field("documents", &self.hashes.len())
            .finish_non_exhaustive()
    }
}

/// Everything the server reads from the data directory. Immutable: a reload or a configuration
/// edit builds a new one. The collection content is shared (`Arc`) between versions, so an edit
/// of screens, looks or output bindings ([`ShowData::with_config`]) copies only those, and a
/// content edit ([`ShowData::with_content`]) swaps in a new [`CollectionContent`].
#[derive(Clone)]
pub struct ShowData {
    /// Root of the data directory (this box's documents, `cache/`, `fonts/`).
    pub root: PathBuf,
    /// Changes whenever the collection content may have changed: render fingerprints include
    /// it.
    pub content_generation: u64,
    /// The active collection's documents.
    pub content: Arc<CollectionContent>,
    /// Screens sorted by sort key.
    pub screens: Vec<Screen>,
    /// Looks sorted by sort key.
    pub looks: Vec<Look>,
    /// Masks sorted by sort key.
    pub masks: Vec<Mask>,
    /// Output bindings (machine-local), sorted by screen order then name.
    pub outputs: Vec<OutputBinding>,
    /// Font substitutions (machine-local).
    pub font_substitutions: FontSubstitutionTable,
    /// The audio device configuration (machine-local).
    pub audio_device: AudioDeviceConfig,
    /// Logical video inputs ("Camera 1"), sorted by sort key.
    pub video_inputs: Vec<VideoInput>,
    /// Which source each video input is on this machine (machine-local).
    pub input_bindings: Vec<InputBinding>,
    /// Show settings singleton.
    pub settings: Arc<ShowSettings>,
    /// Per-document fingerprints of this box's documents (the collection's are in
    /// [`CollectionContent::hashes`]; [`ShowData::doc_hashes`] has both).
    pub hashes: DocHashes,
    /// What happened while loading, the collection's documents included.
    pub report: Arc<LoadReport>,
}

static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Why a video input has no network source to show ([`ShowData::input_source`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputProblem {
    /// No such input in this box's documents.
    Missing,
    /// Not bound to a source on this machine.
    Unbound,
    /// Bound to a kind of source this server doesn't receive.
    Unsupported(&'static str),
}

/// The fingerprint [`ShowData::hashes`] keeps for a document.
pub fn doc_hash<T: Serialize>(doc: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_value(doc)
        .map(|v| v.to_string())
        .unwrap_or_default()
        .hash(&mut h);
    h.finish()
}

/// Sorts presentations by name (case-insensitive), then ID.
pub fn sort_presentations(list: &mut [Arc<Presentation>]) {
    list.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.0.cmp(&b.id.0))
    });
}

/// Sorts media bins by folder path, then sort key, then name, then ID.
pub fn sort_media_bins(list: &mut [MediaBin]) {
    list.sort_by(|a, b| {
        a.folder
            .cmp(&b.folder)
            .then_with(|| a.sort_key.cmp(&b.sort_key))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.0.cmp(&b.id.0))
    });
}

/// Sorts global groups by sort key, then name, then ID.
pub fn sort_global_groups(list: &mut [GlobalGroup]) {
    list.sort_by(|a, b| {
        a.sort_key
            .cmp(&b.sort_key)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.0.cmp(&b.id.0))
    });
}

/// Sorts screens by sort key (then ID).
pub fn sort_screens(screens: &mut [Screen]) {
    screens.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.id.0.cmp(&b.id.0)));
}

/// Sorts looks by sort key (then ID).
pub fn sort_looks(looks: &mut [Look]) {
    looks.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.id.0.cmp(&b.id.0)));
}

/// Sorts masks by sort key, then ID.
pub fn sort_masks(masks: &mut [Mask]) {
    masks.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.id.0.cmp(&b.id.0)));
}

/// Sorts output bindings by their screen's position, then name, then ID.
pub fn sort_outputs(outputs: &mut [OutputBinding], screens: &[Screen]) {
    let pos = |id| {
        screens
            .iter()
            .position(|s| s.id == id)
            .unwrap_or(usize::MAX)
    };
    outputs.sort_by(|a, b| {
        pos(a.screen)
            .cmp(&pos(b.screen))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then(a.id.0.cmp(&b.id.0))
    });
}

/// Reads every `*.json` in `<root>/<section>/<kind>/`, migrates and decodes it.
fn load_kind<T: Document>(
    root: &Path,
    section: &str,
    report: &mut LoadReport,
    hashes: &mut DocHashes,
) -> Vec<T> {
    let dir = root.join(section).join(kind_dir(T::KIND));
    let mut out = Vec::new();
    for path in json_files(&dir) {
        match read_doc(&path, T::KIND).and_then(|v| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            v.to_string().hash(&mut h);
            serde_json::from_value::<T>(v)
                .map(|d| (d, h.finish()))
                .map_err(|e| (format!("invalid document: {e}"), false))
        }) {
            Ok((doc, h)) => {
                hashes.insert((T::KIND, doc.id()), h);
                out.push(doc)
            }
            Err((error, too_new)) => report_issue(report, &path, error, too_new),
        }
    }
    *report
        .loaded
        .entry(format!("{section}/{}", kind_dir(T::KIND)))
        .or_default() += out.len();
    out
}

fn report_issue(report: &mut LoadReport, path: &Path, error: String, too_new: bool) {
    tracing::warn!(file = %path.display(), too_new, "skipping document: {error}");
    report.issues.push(LoadIssue {
        file: path.display().to_string(),
        error,
        too_new,
    });
}

fn read_doc(path: &Path, kind: DocumentKind) -> Result<Value, (String, bool)> {
    let text = std::fs::read_to_string(path).map_err(|e| (format!("read failed: {e}"), false))?;
    let mut v: Value =
        serde_json::from_str(&text).map_err(|e| (format!("not valid JSON: {e}"), false))?;
    model::migrate(kind, &mut v).map_err(|e| {
        let too_new = matches!(e, MigrationError::TooNew { .. });
        (e.to_string(), too_new)
    })?;
    Ok(v)
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    paths
}

/// The directory name of a document kind (its camelCase serde name).
pub fn kind_dir(kind: DocumentKind) -> String {
    match serde_json::to_value(kind) {
        Ok(Value::String(s)) => s,
        _ => "unknown".into(),
    }
}

/// Kinds decoded by [`ShowData::load`]; every other kind directory is only validated.
const USED: &[DocumentKind] = &[
    DocumentKind::Library,
    DocumentKind::Presentation,
    DocumentKind::Screen,
    DocumentKind::Look,
    DocumentKind::Mask,
    DocumentKind::Theme,
    DocumentKind::MediaAsset,
    DocumentKind::ShowSettings,
    DocumentKind::OutputBinding,
    DocumentKind::MediaRootBinding,
    DocumentKind::FontSubstitutionTable,
    DocumentKind::AudioDeviceConfig,
    DocumentKind::VideoInput,
    DocumentKind::InputBinding,
    DocumentKind::PlaylistNode,
    DocumentKind::ClearGroup,
    DocumentKind::Timer,
    DocumentKind::Prop,
    DocumentKind::Message,
    DocumentKind::Macro,
    DocumentKind::StageLayout,
    DocumentKind::MediaBin,
    DocumentKind::GlobalGroup,
];

/// The portable kinds a Codex collection holds. Every other portable kind (screens, looks,
/// show settings, masks, video and audio inputs) and everything machine-local stays with the
/// box.
pub const COLLECTION_KINDS: &[DocumentKind] = &[
    DocumentKind::Library,
    DocumentKind::Presentation,
    DocumentKind::PlaylistNode,
    DocumentKind::Theme,
    DocumentKind::MediaAsset,
    DocumentKind::MediaBin,
    DocumentKind::MediaRoot,
    DocumentKind::GlobalGroup,
    DocumentKind::Timer,
    DocumentKind::Message,
    DocumentKind::Prop,
    DocumentKind::Macro,
    DocumentKind::ClearGroup,
    DocumentKind::StageLayout,
];

/// The server's own state folders in `machine/`, next to the documents: the Site Manager's,
/// the site membership's and the box-to-Codex link's device identities.
const SERVER_STATE_DIRS: &[&str] = &["site-manager", "site-member", "codex-member"];

/// Whether `<section>/<name>` is one of the server's own state folders.
fn server_state_dir(section: &str, name: &str) -> bool {
    section == "machine" && SERVER_STATE_DIRS.contains(&name)
}

/// Which part of the documents a load covers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    /// The collection's portable kinds.
    Collection,
    /// The box's portable kinds and every machine-local kind.
    Box,
}

/// Migrates (but does not decode) the documents of kinds the server does not use yet.
fn validate_other_kinds(root: &Path, part: Part, report: &mut LoadReport) {
    let sections: &[&str] = match part {
        Part::Collection => &["portable"],
        Part::Box => &["portable", "machine"],
    };
    for section in sections {
        let Ok(rd) = std::fs::read_dir(root.join(section)) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let name = dir.file_name().unwrap_or_default().to_string_lossy();
            let kind: DocumentKind = serde_json::from_value(Value::String(name.to_string()))
                .unwrap_or(DocumentKind::Unknown);
            if USED.contains(&kind) {
                continue;
            }
            let collection_kind = *section == "portable" && COLLECTION_KINDS.contains(&kind);
            if collection_kind != (part == Part::Collection) {
                continue;
            }
            if kind == DocumentKind::Unknown {
                // The server's own state folders in `machine/` (device identities) aren't
                // documents: skipped without a word.
                if part == Part::Box && !server_state_dir(section, &name) {
                    tracing::warn!(dir = %dir.display(), "unknown document kind directory, skipped");
                }
                continue;
            }
            let mut ok = 0;
            for path in json_files(&dir) {
                match read_doc(&path, kind) {
                    Ok(_) => ok += 1,
                    Err((error, too_new)) => report_issue(report, &path, error, too_new),
                }
            }
            *report
                .loaded
                .entry(format!("{section}/{name}"))
                .or_default() += ok;
        }
    }
}

fn sort_by_sort_key<T>(list: &mut [T], key: impl Fn(&T) -> (&str, uuid::Uuid)) {
    list.sort_by(|a, b| key(a).cmp(&key(b)));
}

/// Next value of [`ShowData::content_generation`].
fn next_generation() -> u64 {
    GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl CollectionContent {
    /// Loads a collection's documents from `<root>/portable/<kind>/`. Bad documents are reported
    /// in [`CollectionContent::report`] and skipped; a missing folder is an empty collection.
    /// `media_roots` are this box's media root bindings, `media_dir` the managed media folder of
    /// the old store, `library` the Codex's media library.
    pub fn load(
        root: &Path,
        media_dir: PathBuf,
        library: Arc<crate::library::MediaLibrary>,
        media_roots: Arc<HashMap<MediaRootId, PathBuf>>,
    ) -> CollectionContent {
        let mut report = LoadReport::default();
        let mut h = DocHashes::new();
        let (r, h) = (&mut report, &mut h);
        let libraries: Vec<Library> = load_kind(root, "portable", r, h);
        let presentations: Vec<Presentation> = load_kind(root, "portable", r, h);
        let themes: Vec<Theme> = load_kind(root, "portable", r, h);
        let assets: Vec<MediaAsset> = load_kind(root, "portable", r, h);
        let playlists: Vec<PlaylistNode> = load_kind(root, "portable", r, h);
        let mut clear_groups: Vec<ClearGroup> = load_kind(root, "portable", r, h);
        let mut timers: Vec<Timer> = load_kind(root, "portable", r, h);
        let mut props: Vec<Prop> = load_kind(root, "portable", r, h);
        let mut messages: Vec<Message> = load_kind(root, "portable", r, h);
        let mut macros: Vec<Macro> = load_kind(root, "portable", r, h);
        let mut stage_layouts: Vec<StageLayout> = load_kind(root, "portable", r, h);
        let mut media_bins: Vec<MediaBin> = load_kind(root, "portable", r, h);
        let mut global_groups: Vec<GlobalGroup> = load_kind(root, "portable", r, h);
        validate_other_kinds(root, Part::Collection, r);
        let hashes = std::mem::take(h);
        sort_by_sort_key(&mut clear_groups, |x| (x.sort_key.as_str(), x.id.0));
        sort_by_sort_key(&mut timers, |x| (x.sort_key.as_str(), x.id.0));
        sort_by_sort_key(&mut props, |x| (x.sort_key.as_str(), x.id.0));
        sort_by_sort_key(&mut messages, |x| (x.sort_key.as_str(), x.id.0));
        sort_by_sort_key(&mut macros, |x| (x.sort_key.as_str(), x.id.0));
        sort_by_sort_key(&mut stage_layouts, |x| (x.sort_key.as_str(), x.id.0));
        sort_media_bins(&mut media_bins);
        sort_global_groups(&mut global_groups);
        let mut presentations: Vec<Arc<Presentation>> =
            presentations.into_iter().map(Arc::new).collect();
        sort_presentations(&mut presentations);
        let presentation_index = presentations
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id, i))
            .collect();
        CollectionContent {
            root: root.to_path_buf(),
            media_dir,
            library,
            libraries: Arc::new(libraries.into_iter().map(|l| (l.id, l)).collect()),
            presentations: Arc::new(presentations),
            presentation_index: Arc::new(presentation_index),
            themes: Arc::new(themes.into_iter().map(|t| (t.id, t)).collect()),
            assets: Arc::new(assets.into_iter().map(|a| (a.id, a)).collect()),
            media_roots,
            playlists: Arc::new(playlists.into_iter().map(|p| (p.id, p)).collect()),
            clear_groups: Arc::new(clear_groups),
            timers: Arc::new(timers),
            props: Arc::new(props),
            messages: Arc::new(messages),
            macros: Arc::new(macros),
            stage_layouts: Arc::new(stage_layouts),
            media_bins: Arc::new(media_bins),
            global_groups: Arc::new(global_groups),
            hashes,
            report: Arc::new(report),
        }
    }
}

/// This box's documents, before the collection content joins them.
struct BoxDocs {
    screens: Vec<Screen>,
    looks: Vec<Look>,
    masks: Vec<Mask>,
    outputs: Vec<OutputBinding>,
    font_substitutions: FontSubstitutionTable,
    audio_device: AudioDeviceConfig,
    video_inputs: Vec<VideoInput>,
    input_bindings: Vec<InputBinding>,
    settings: ShowSettings,
    media_roots: Arc<HashMap<MediaRootId, PathBuf>>,
    hashes: DocHashes,
    report: LoadReport,
}

impl BoxDocs {
    fn load(root: &Path) -> BoxDocs {
        let mut report = LoadReport::default();
        let mut h = DocHashes::new();
        let (r, h) = (&mut report, &mut h);
        let mut screens: Vec<Screen> = load_kind(root, "portable", r, h);
        let mut looks: Vec<Look> = load_kind(root, "portable", r, h);
        let mut masks: Vec<Mask> = load_kind(root, "portable", r, h);
        let settings: Vec<ShowSettings> = load_kind(root, "portable", r, h);
        let mut outputs: Vec<OutputBinding> = load_kind(root, "machine", r, h);
        let roots: Vec<MediaRootBinding> = load_kind(root, "machine", r, h);
        let subs: Vec<FontSubstitutionTable> = load_kind(root, "machine", r, h);
        let audio_devices: Vec<AudioDeviceConfig> = load_kind(root, "machine", r, h);
        let mut video_inputs: Vec<VideoInput> = load_kind(root, "portable", r, h);
        let input_bindings: Vec<InputBinding> = load_kind(root, "machine", r, h);
        validate_other_kinds(root, Part::Box, r);
        video_inputs.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.id.0.cmp(&b.id.0)));
        let hashes = std::mem::take(h);
        sort_screens(&mut screens);
        sort_looks(&mut looks);
        sort_masks(&mut masks);
        sort_outputs(&mut outputs, &screens);
        BoxDocs {
            screens,
            looks,
            masks,
            outputs,
            font_substitutions: subs.into_iter().next().unwrap_or_default(),
            audio_device: audio_devices.into_iter().next().unwrap_or_default(),
            video_inputs,
            input_bindings,
            settings: settings.into_iter().next().unwrap_or_default(),
            media_roots: Arc::new(
                roots
                    .into_iter()
                    .map(|b| (b.root, PathBuf::from(b.local_path)))
                    .collect(),
            ),
            hashes,
            report,
        }
    }
}

/// The two reports as one: the collection's first, then the box's.
fn merge_reports(content: &LoadReport, own: LoadReport) -> LoadReport {
    let mut merged = content.clone();
    for (k, n) in own.loaded {
        *merged.loaded.entry(k).or_default() += n;
    }
    merged.issues.extend(own.issues);
    merged
}

impl ShowData {
    /// Loads a data directory whose collection content lives in the directory itself
    /// (`<root>/portable/`, `<root>/media/`). Fails only when the directory does not exist; bad
    /// documents are reported in [`ShowData::report`] and skipped.
    pub fn load(root: &Path) -> anyhow::Result<ShowData> {
        Self::load_from(
            root,
            root,
            root.join("media"),
            crate::library::MediaLibrary::detached(),
        )
    }

    /// Loads this box's documents from the data directory `root` and the collection content
    /// from `content_root` (with its managed media in `media_dir`, its library media in
    /// `library`).
    pub fn load_from(
        root: &Path,
        content_root: &Path,
        media_dir: PathBuf,
        library: Arc<crate::library::MediaLibrary>,
    ) -> anyhow::Result<ShowData> {
        anyhow::ensure!(
            root.join("portable").is_dir(),
            "{} is not a data directory (no portable/ folder)",
            root.display()
        );
        let own = BoxDocs::load(root);
        let content =
            CollectionContent::load(content_root, media_dir, library, own.media_roots.clone());
        Ok(Self::assemble(root, own, Arc::new(content)))
    }

    fn assemble(root: &Path, own: BoxDocs, content: Arc<CollectionContent>) -> ShowData {
        let report = merge_reports(&content.report, own.report);
        ShowData {
            root: root.to_path_buf(),
            content_generation: next_generation(),
            content,
            screens: own.screens,
            looks: own.looks,
            masks: own.masks,
            outputs: own.outputs,
            font_substitutions: own.font_substitutions,
            audio_device: own.audio_device,
            video_inputs: own.video_inputs,
            input_bindings: own.input_bindings,
            settings: Arc::new(own.settings),
            hashes: own.hashes,
            report: Arc::new(report),
        }
    }

    /// The network source video input `input` is on this machine: its `InputBinding`'s IDN
    /// source name, or `omt:` and the OMT source's name or address (the live-source key of an
    /// OMT source; an `omt://host:port` address is its own key, as `output::omt::key` has it).
    pub fn input_source(&self, input: VideoInputId) -> Result<String, InputProblem> {
        if !self.video_inputs.iter().any(|v| v.id == input) {
            return Err(InputProblem::Missing);
        }
        match self
            .input_bindings
            .iter()
            .find(|b| b.input == input)
            .map(|b| &b.source)
        {
            Some(InputSource::Idn { source_name }) if !source_name.trim().is_empty() => {
                Ok(source_name.trim().to_string())
            }
            Some(InputSource::Omt { name }) if !name.trim().is_empty() => {
                let name = name.trim();
                Ok(if name.starts_with("omt://") {
                    name.to_string()
                } else {
                    format!("omt:{name}")
                })
            }
            Some(InputSource::Device { .. }) => Err(InputProblem::Unsupported("a capture device")),
            _ => Err(InputProblem::Unbound),
        }
    }

    /// Fingerprints of every document: this box's and the collection's.
    pub fn doc_hashes(&self) -> DocHashes {
        let mut all = self.content.hashes.clone();
        all.extend(self.hashes.iter().map(|(k, v)| (*k, *v)));
        all
    }

    /// A copy showing another collection's content. `content_generation` is new, so every
    /// screen re-renders.
    pub fn with_collection(&self, content: Arc<CollectionContent>) -> ShowData {
        ShowData {
            content,
            content_generation: next_generation(),
            ..self.clone()
        }
    }

    /// A copy with new screens, looks, masks and output bindings (sorted here); the collection content
    /// is shared and `content_generation` stays, so unchanged screens do not re-render.
    pub fn with_config(
        &self,
        mut screens: Vec<Screen>,
        mut looks: Vec<Look>,
        mut masks: Vec<Mask>,
        mut outputs: Vec<OutputBinding>,
    ) -> ShowData {
        sort_screens(&mut screens);
        sort_looks(&mut looks);
        sort_masks(&mut masks);
        sort_outputs(&mut outputs, &screens);
        // Documents that did not change keep the fingerprint they were loaded with.
        let mut hashes = DocHashes::new();
        fn carry<T: Document + PartialEq>(
            new: &[T],
            old: &[T],
            hashes: &mut DocHashes,
            prev: &DocHashes,
        ) {
            for d in new {
                let key = (T::KIND, d.id());
                let same = old.iter().any(|o| o.id() == d.id() && o == d);
                let h = match prev.get(&key) {
                    Some(h) if same => *h,
                    _ => doc_hash(d),
                };
                hashes.insert(key, h);
            }
        }
        for (k, v) in &self.hashes {
            if !matches!(
                k.0,
                DocumentKind::Screen
                    | DocumentKind::Look
                    | DocumentKind::Mask
                    | DocumentKind::OutputBinding
            ) {
                hashes.insert(*k, *v);
            }
        }
        carry(&screens, &self.screens, &mut hashes, &self.hashes);
        carry(&looks, &self.looks, &mut hashes, &self.hashes);
        carry(&masks, &self.masks, &mut hashes, &self.hashes);
        carry(&outputs, &self.outputs, &mut hashes, &self.hashes);
        ShowData {
            screens,
            looks,
            masks,
            outputs,
            hashes,
            ..self.clone()
        }
    }

    /// A copy with new logical video inputs and this machine's bindings of them (sorted
    /// here); fingerprints follow the documents, `content_generation` stays.
    pub fn with_inputs(
        &self,
        mut video_inputs: Vec<VideoInput>,
        input_bindings: Vec<InputBinding>,
    ) -> ShowData {
        video_inputs.sort_by(|a, b| a.sort_key.cmp(&b.sort_key).then(a.id.0.cmp(&b.id.0)));
        let mut hashes: DocHashes = self
            .hashes
            .iter()
            .filter(|(k, _)| !matches!(k.0, DocumentKind::VideoInput | DocumentKind::InputBinding))
            .map(|(k, v)| (*k, *v))
            .collect();
        for v in &video_inputs {
            hashes.insert((DocumentKind::VideoInput, v.id.0), doc_hash(v));
        }
        for b in &input_bindings {
            hashes.insert((DocumentKind::InputBinding, b.input.0), doc_hash(b));
        }
        ShowData {
            video_inputs,
            input_bindings,
            hashes,
            ..self.clone()
        }
    }

    /// A copy with collection content changed (edits and CRUD), see
    /// [`CollectionContent::with_change`]. `content_generation` is new, so every screen
    /// re-renders against the edited documents.
    pub fn with_content(&self, change: &ContentChange) -> ShowData {
        self.with_collection(Arc::new(self.content.with_change(change)))
    }

    /// A copy with media assets and bins changed ([`ContentChange::media_only`]) that nothing
    /// live uses: `content_generation` stays, so no screen re-renders (frame pacing during a
    /// show); what is drawn next finds the new documents.
    pub fn with_media(&self, change: &ContentChange) -> ShowData {
        ShowData {
            content: Arc::new(self.content.with_change(change)),
            ..self.clone()
        }
    }

    /// A copy with new lists of timers, props, messages or stage layouts, see
    /// [`CollectionContent::with_show_docs`]. `content_generation` is new, so every screen
    /// re-renders.
    pub fn with_show_docs(
        &self,
        timers: Option<Vec<Timer>>,
        props: Option<Vec<Prop>>,
        messages: Option<Vec<Message>>,
        stage_layouts: Option<Vec<StageLayout>>,
    ) -> ShowData {
        self.with_collection(Arc::new(self.content.with_show_docs(
            timers,
            props,
            messages,
            stage_layouts,
        )))
    }
}

impl CollectionContent {
    /// Presentation by ID.
    pub fn presentation(&self, id: PresentationId) -> Option<&Presentation> {
        self.presentation_index
            .get(&id)
            .map(|&i| self.presentations[i].as_ref())
    }

    /// A copy with library content changed (edits and CRUD): each `Some` replaces or inserts a
    /// document, each `None` removes it. Everything else is shared.
    pub fn with_change(&self, change: &ContentChange) -> CollectionContent {
        let mut d = self.clone();
        if !change.presentations.is_empty() {
            let mut list: Vec<Arc<Presentation>> = (*self.presentations).clone();
            for (id, p) in &change.presentations {
                list.retain(|x| x.id != *id);
                d.hashes.remove(&(DocumentKind::Presentation, id.0));
                if let Some(p) = p {
                    d.hashes
                        .insert((DocumentKind::Presentation, id.0), doc_hash(p.as_ref()));
                    list.push(p.clone());
                }
            }
            sort_presentations(&mut list);
            d.presentation_index =
                Arc::new(list.iter().enumerate().map(|(i, p)| (p.id, i)).collect());
            d.presentations = Arc::new(list);
        }
        if !change.libraries.is_empty() {
            let mut m = (*self.libraries).clone();
            for (id, l) in &change.libraries {
                d.hashes.remove(&(DocumentKind::Library, id.0));
                match l {
                    Some(l) => {
                        d.hashes.insert((DocumentKind::Library, id.0), doc_hash(l));
                        m.insert(*id, l.clone());
                    }
                    None => {
                        m.remove(id);
                    }
                }
            }
            d.libraries = Arc::new(m);
        }
        if !change.playlists.is_empty() {
            let mut m = (*self.playlists).clone();
            for (id, n) in &change.playlists {
                d.hashes.remove(&(DocumentKind::PlaylistNode, id.0));
                match n {
                    Some(n) => {
                        d.hashes
                            .insert((DocumentKind::PlaylistNode, id.0), doc_hash(n));
                        m.insert(*id, n.clone());
                    }
                    None => {
                        m.remove(id);
                    }
                }
            }
            d.playlists = Arc::new(m);
        }
        if !change.assets.is_empty() {
            let mut m = (*self.assets).clone();
            for (id, a) in &change.assets {
                d.hashes.remove(&(DocumentKind::MediaAsset, id.0));
                match a {
                    Some(a) => {
                        d.hashes
                            .insert((DocumentKind::MediaAsset, id.0), doc_hash(a));
                        m.insert(*id, a.clone());
                    }
                    None => {
                        m.remove(id);
                    }
                }
            }
            d.assets = Arc::new(m);
        }
        if !change.media_bins.is_empty() {
            let mut list: Vec<MediaBin> = (*self.media_bins).clone();
            for (id, b) in &change.media_bins {
                list.retain(|x| x.id != *id);
                d.hashes.remove(&(DocumentKind::MediaBin, id.0));
                if let Some(b) = b {
                    d.hashes.insert((DocumentKind::MediaBin, id.0), doc_hash(b));
                    list.push(b.clone());
                }
            }
            sort_media_bins(&mut list);
            d.media_bins = Arc::new(list);
        }
        if !change.global_groups.is_empty() {
            let mut list: Vec<GlobalGroup> = (*self.global_groups).clone();
            for (id, g) in &change.global_groups {
                list.retain(|x| x.id != *id);
                d.hashes.remove(&(DocumentKind::GlobalGroup, id.0));
                if let Some(g) = g {
                    d.hashes
                        .insert((DocumentKind::GlobalGroup, id.0), doc_hash(g));
                    list.push(g.clone());
                }
            }
            sort_global_groups(&mut list);
            d.global_groups = Arc::new(list);
        }
        if !change.themes.is_empty() {
            let mut m = (*self.themes).clone();
            for (id, t) in &change.themes {
                d.hashes.remove(&(DocumentKind::Theme, id.0));
                match t {
                    Some(t) => {
                        d.hashes.insert((DocumentKind::Theme, id.0), doc_hash(t));
                        m.insert(*id, t.clone());
                    }
                    None => {
                        m.remove(id);
                    }
                }
            }
            d.themes = Arc::new(m);
        }
        d
    }

    /// A copy with a new list of timers, props, messages or stage layouts (show documents edited
    /// through the API, sorted here).
    pub fn with_show_docs(
        &self,
        timers: Option<Vec<Timer>>,
        props: Option<Vec<Prop>>,
        messages: Option<Vec<Message>>,
        stage_layouts: Option<Vec<StageLayout>>,
    ) -> CollectionContent {
        fn sorted<T: Document>(mut list: Vec<T>, key: impl Fn(&T) -> String) -> Vec<T> {
            list.sort_by(|a, b| key(a).cmp(&key(b)).then(a.id().cmp(&b.id())));
            list
        }
        fn rehash<T: Document + Serialize>(hashes: &mut DocHashes, list: &[T]) {
            hashes.retain(|k, _| k.0 != T::KIND);
            for d in list {
                hashes.insert((T::KIND, d.id()), doc_hash(d));
            }
        }
        let mut d = self.clone();
        if let Some(t) = timers {
            let t = sorted(t, |x| x.sort_key.clone());
            rehash(&mut d.hashes, &t);
            d.timers = Arc::new(t);
        }
        if let Some(p) = props {
            let p = sorted(p, |x| x.sort_key.clone());
            rehash(&mut d.hashes, &p);
            d.props = Arc::new(p);
        }
        if let Some(m) = messages {
            let m = sorted(m, |x| x.sort_key.clone());
            rehash(&mut d.hashes, &m);
            d.messages = Arc::new(m);
        }
        if let Some(l) = stage_layouts {
            let l = sorted(l, |x| x.sort_key.clone());
            rehash(&mut d.hashes, &l);
            d.stage_layouts = Arc::new(l);
        }
        d
    }

    /// Local file of a media asset, or why it cannot be located. A prepared asset whose copy
    /// isn't on this box (yet: a show box copies a new library item from the Codex when it is
    /// wanted) plays its original, which it played before, when that one is here.
    pub fn asset_path(&self, id: MediaAssetId) -> Result<PathBuf, String> {
        let a = self
            .assets
            .get(&id)
            .ok_or_else(|| format!("media asset {} is not in the library", id.0))?;
        self.location_path(a, &a.location).or_else(|e| {
            match a.prep.as_ref().and_then(|p| p.original.as_ref()) {
                Some(o) if matches!(a.location, AssetLocation::Library { .. }) => {
                    self.location_path(a, &o.location).map_err(|_| e)
                }
                _ => Err(e),
            }
        })
    }

    /// The local file of `location` (asset `a`'s, or its original's).
    fn location_path(&self, a: &MediaAsset, location: &AssetLocation) -> Result<PathBuf, String> {
        let p = match location {
            AssetLocation::Linked { root, path } => {
                // A path that would leave its root (`..`, absolute, a drive) is never served.
                if !crate::relink::stays_below(path) {
                    return Err(format!("{}: path leaves its media root", a.name));
                }
                self.media_roots
                    .get(root)
                    .ok_or_else(|| format!("{}: media root {} is not bound", a.name, root.0))?
                    .join(path)
            }
            AssetLocation::ThemeAsset { theme, path } => {
                // ProPresenter keeps theme assets next to the theme document.
                let dir = self
                    .themes
                    .get(theme)
                    .and_then(|t| t.import.as_ref())
                    .and_then(|i| i.document_path.clone())
                    .and_then(|d| Path::new(&d).parent().map(Path::to_path_buf))
                    .ok_or_else(|| format!("{}: theme folder unknown", a.name))?;
                // Both halves come from documents: neither may leave the media root (an
                // absolute or `..` part would replace or climb out of the base).
                let below = |p: &Path| p.to_str().is_some_and(crate::relink::stays_below);
                if !below(&dir) || !crate::relink::stays_below(path) {
                    return Err(format!("{}: path leaves its media root", a.name));
                }
                self.media_roots
                    .values()
                    .map(|b| b.join(&dir).join(path))
                    .find(|p| p.exists())
                    .ok_or_else(|| format!("{}: theme asset not found", a.name))?
            }
            AssetLocation::Managed { sha256, extension } => {
                self.media_dir.join(format!("{sha256}.{extension}"))
            }
            AssetLocation::Library { id, .. } => self
                .library
                .resolve(id)
                .map_err(|e| format!("{}: {e}", a.name))?,
            _ => return Err(format!("{}: location unresolved", a.name)),
        };
        if p.exists() {
            Ok(p)
        } else {
            Err(format!("{}: file not found ({})", a.name, p.display()))
        }
    }

    /// Distinct media assets a presentation refers to (slide media actions, media fills,
    /// anything with an `asset` reference, then the song's artwork), in first-use order.
    pub fn presentation_assets(&self, p: &Presentation) -> Vec<MediaAssetId> {
        fn walk(v: &Value, out: &mut Vec<MediaAssetId>) {
            match v {
                Value::Object(m) => {
                    for (k, v) in m {
                        if k == "asset"
                            && let Some(id) = v.as_str().and_then(|s| s.parse::<uuid::Uuid>().ok())
                        {
                            let id = MediaAssetId::from(id);
                            if !out.contains(&id) {
                                out.push(id);
                            }
                        } else {
                            walk(v, out);
                        }
                    }
                }
                Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        if let Ok(v) = serde_json::to_value(&p.sections) {
            walk(&v, &mut out);
        }
        // The song's artwork (shown by artwork links).
        if let Some(a) = p.song_info.as_ref().and_then(|s| s.artwork)
            && !out.contains(&a)
        {
            out.push(a);
        }
        out
    }

    /// Display name of a media asset.
    pub fn asset_name(&self, id: MediaAssetId) -> String {
        self.assets
            .get(&id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| id.0.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{AssetLocation, Presentation, Screen, Timer};

    #[test]
    fn the_servers_own_state_folders_are_not_documents() {
        for d in ["site-manager", "site-member", "codex-member"] {
            assert!(server_state_dir("machine", d), "{d}");
            assert!(!server_state_dir("portable", d), "{d}");
        }
        assert!(!server_state_dir("machine", "mystery"));
    }

    fn write<T: Document>(root: &Path, doc: &T) {
        let w = crate::store::DocWrite::put(doc).unwrap();
        crate::store::apply(root, &[w]).unwrap();
    }

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("data-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("portable")).unwrap();
        dir
    }

    /// F02-1: an input bound to an OMT address resolves to the address itself (the source's
    /// live-source key), not `omt:omt://…`; a name gets the `omt:` prefix.
    #[test]
    fn an_omt_input_resolves_to_its_live_source_key() {
        let dir = temp("omt-key");
        let d = ShowData::load(&dir).unwrap();
        let (a, b, c) = (
            VideoInput::default(),
            VideoInput::default(),
            VideoInput::default(),
        );
        let bind = |v: &VideoInput, source| InputBinding {
            input: v.id,
            source,
            ..InputBinding::default()
        };
        let d = d.with_inputs(
            vec![a.clone(), b.clone(), c.clone()],
            vec![
                bind(
                    &a,
                    InputSource::Omt {
                        name: " omt://192.0.2.10:6400 ".into(),
                    },
                ),
                bind(
                    &b,
                    InputSource::Omt {
                        name: "STUDIO (Camera)".into(),
                    },
                ),
                bind(
                    &c,
                    InputSource::Idn {
                        source_name: "HOST (Cam)".into(),
                    },
                ),
            ],
        );
        assert_eq!(d.input_source(a.id).unwrap(), "omt://192.0.2.10:6400");
        assert_eq!(d.input_source(b.id).unwrap(), "omt:STUDIO (Camera)");
        assert_eq!(d.input_source(c.id).unwrap(), "HOST (Cam)");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_collection_and_the_box_split_the_documents() {
        let dir = temp("split");
        let p = Presentation::new("Song");
        let s = Screen::default();
        let t = Timer::default();
        write(&dir, &p);
        write(&dir, &s);
        write(&dir, &t);
        let d = ShowData::load(&dir).unwrap();
        assert_eq!(d.content.root, dir);
        assert_eq!(d.content.media_dir, dir.join("media"));
        assert!(d.content.presentation(p.id).is_some());
        assert_eq!(d.content.timers.len(), 1);
        assert_eq!(d.screens.len(), 1);
        // Fingerprints: each part keeps its own, `doc_hashes` has both.
        let pk = (DocumentKind::Presentation, p.id.0);
        let sk = (DocumentKind::Screen, s.id.0);
        assert!(d.content.hashes.contains_key(&pk) && !d.content.hashes.contains_key(&sk));
        assert!(d.hashes.contains_key(&sk) && !d.hashes.contains_key(&pk));
        let all = d.doc_hashes();
        assert!(all.contains_key(&pk) && all.contains_key(&sk));
        assert_eq!(all.len(), 3);
        // One report covers both.
        assert_eq!(d.report.loaded["portable/presentation"], 1);
        assert_eq!(d.report.loaded["portable/screen"], 1);
        assert!(!d.content.report.loaded.contains_key("portable/screen"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn content_can_live_in_its_own_root() {
        let dir = temp("own-root");
        let content = dir.join("collection");
        let media = dir.join("blobs");
        std::fs::create_dir_all(&media).unwrap();
        std::fs::write(media.join("abc.png"), b"png").unwrap();
        let p = Presentation::new("Song");
        let s = Screen::default();
        let a = MediaAsset {
            location: AssetLocation::Managed {
                sha256: "abc".into(),
                extension: "png".into(),
            },
            ..MediaAsset::default()
        };
        write(&content, &p);
        write(&content, &a);
        write(&dir, &s);
        // A presentation left in the box's folder is not the collection's.
        write(&dir, &Presentation::new("Elsewhere"));
        let d = ShowData::load_from(
            &dir,
            &content,
            media.clone(),
            crate::library::MediaLibrary::detached(),
        )
        .unwrap();
        assert_eq!(d.content.presentations.len(), 1);
        assert_eq!(d.content.presentations[0].id, p.id);
        assert_eq!(d.screens.len(), 1);
        assert_eq!(d.content.asset_path(a.id).unwrap(), media.join("abc.png"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A prepared asset whose copy isn't on this box yet (a show box's cache) plays its
    /// original meanwhile, instead of nothing.
    #[test]
    fn a_prepared_copy_not_here_yet_plays_the_original() {
        let dir = temp("prepared-copy");
        let content = dir.join("collection");
        let media = dir.join("blobs");
        std::fs::create_dir_all(&media).unwrap();
        std::fs::write(media.join("orig.mov"), b"mov").unwrap();
        let original = AssetLocation::Managed {
            sha256: "orig".into(),
            extension: "mov".into(),
        };
        let a = MediaAsset {
            location: AssetLocation::Library {
                id: "copy-item".into(),
                sha256: "copy".into(),
            },
            prep: Some(model::MediaPrep {
                original: Some(model::PreparedOriginal {
                    location: original.clone(),
                    probe: None,
                    reasons: Vec::new(),
                    format: String::new(),
                }),
                ..model::MediaPrep::default()
            }),
            ..MediaAsset::default()
        };
        // Without a preparation: missing.
        let plain = MediaAsset {
            location: AssetLocation::Library {
                id: "other-item".into(),
                sha256: "other".into(),
            },
            ..MediaAsset::default()
        };
        write(&content, &a);
        write(&content, &plain);
        let d = ShowData::load_from(
            &dir,
            &content,
            media.clone(),
            crate::library::MediaLibrary::detached(),
        )
        .unwrap();
        assert_eq!(d.content.asset_path(a.id).unwrap(), media.join("orig.mov"));
        assert!(d.content.asset_path(plain.id).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_linked_path_never_leaves_its_root() {
        let dir = temp("linked-escape");
        let root_dir = dir.join("bound");
        std::fs::create_dir_all(&root_dir).unwrap();
        std::fs::write(root_dir.join("ok.png"), b"png").unwrap();
        std::fs::write(dir.join("secret.txt"), b"secret").unwrap();
        let root = model::MediaRootId::new();
        let linked = |path: &str| MediaAsset {
            location: AssetLocation::Linked {
                root,
                path: path.into(),
            },
            ..MediaAsset::default()
        };
        let ok = linked("ok.png");
        let up = linked("../secret.txt");
        let abs = linked(dir.join("secret.txt").to_str().unwrap());
        for a in [&ok, &up, &abs] {
            write(&dir, a);
        }
        let d = ShowData::load(&dir).unwrap();
        let mut c = (*d.content).clone();
        c.media_roots = Arc::new(HashMap::from([(root, root_dir.clone())]));
        assert_eq!(c.asset_path(ok.id).unwrap(), root_dir.join("ok.png"));
        assert!(c.asset_path(up.id).is_err());
        assert!(c.asset_path(abs.id).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_theme_asset_never_leaves_its_root() {
        let dir = temp("theme-escape");
        let root_dir = dir.join("bound");
        std::fs::create_dir_all(root_dir.join("Themes/T")).unwrap();
        std::fs::write(root_dir.join("Themes/T/ok.png"), b"png").unwrap();
        std::fs::write(dir.join("secret.txt"), b"secret").unwrap();
        let theme = |doc: &str| model::Theme {
            import: Some(model::ImportInfo::propresenter(doc)),
            ..model::Theme::default()
        };
        let good = theme("Themes/T/t.pro");
        let abs_theme = theme(&format!("{}/t.pro", dir.display()));
        let up_theme = theme("../../x/t.pro");
        let asset = |theme: model::ThemeId, path: &str| MediaAsset {
            location: AssetLocation::ThemeAsset {
                theme,
                path: path.into(),
            },
            ..MediaAsset::default()
        };
        let ok = asset(good.id, "ok.png");
        let up = asset(good.id, "../../../secret.txt");
        let abs = asset(good.id, dir.join("secret.txt").to_str().unwrap());
        let via_abs = asset(abs_theme.id, "secret.txt");
        let via_up = asset(up_theme.id, "secret.txt");
        for t in [&good, &abs_theme, &up_theme] {
            write(&dir, t);
        }
        for a in [&ok, &up, &abs, &via_abs, &via_up] {
            write(&dir, a);
        }
        let d = ShowData::load(&dir).unwrap();
        let mut c = (*d.content).clone();
        let root = model::MediaRootId::new();
        c.media_roots = Arc::new(HashMap::from([(root, root_dir.clone())]));
        assert_eq!(
            c.asset_path(ok.id).unwrap(),
            root_dir.join("Themes/T").join("ok.png")
        );
        for a in [&up, &abs, &via_abs, &via_up] {
            assert!(c.asset_path(a.id).is_err(), "{:?}", a.location);
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edits_swap_the_content_and_share_the_rest() {
        let dir = temp("swap");
        let p = Presentation::new("Song");
        write(&dir, &p);
        write(&dir, &Screen::default());
        let d = ShowData::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        // A configuration edit keeps the content and its generation.
        let c = d.with_config(d.screens.clone(), Vec::new(), Vec::new(), Vec::new());
        assert!(Arc::ptr_eq(&c.content, &d.content));
        assert_eq!(c.content_generation, d.content_generation);

        // A content edit swaps in new content; the untouched lists stay shared.
        let mut renamed = p.clone();
        renamed.name = "Renamed".into();
        let change = ContentChange {
            presentations: vec![(p.id, Some(Arc::new(renamed)))],
            ..ContentChange::default()
        };
        let e = d.with_content(&change);
        assert!(!Arc::ptr_eq(&e.content, &d.content));
        assert_ne!(e.content_generation, d.content_generation);
        assert_eq!(e.content.presentation(p.id).unwrap().name, "Renamed");
        assert_eq!(d.content.presentation(p.id).unwrap().name, "Song");
        assert!(Arc::ptr_eq(&e.content.themes, &d.content.themes));
        assert!(Arc::ptr_eq(&e.settings, &d.settings));
        assert_ne!(
            e.content.hashes[&(DocumentKind::Presentation, p.id.0)],
            d.content.hashes[&(DocumentKind::Presentation, p.id.0)]
        );

        // Show documents too.
        let f = e.with_show_docs(Some(vec![Timer::default()]), None, None, None);
        assert_eq!(f.content.timers.len(), 1);
        assert!(Arc::ptr_eq(
            &f.content.presentations,
            &e.content.presentations
        ));
        assert_ne!(f.content_generation, e.content_generation);

        // Another collection's content.
        let other = Arc::new(CollectionContent::load(
            &dir.join("nothing"),
            dir.join("nothing/media"),
            crate::library::MediaLibrary::detached(),
            Arc::default(),
        ));
        let g = f.with_collection(other.clone());
        assert!(Arc::ptr_eq(&g.content, &other));
        assert!(g.content.presentations.is_empty());
        assert_eq!(g.screens.len(), 1);
    }
}
