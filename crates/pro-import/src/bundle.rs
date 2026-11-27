//! [`ImportBundle`]: every document an import produced, plus the report, and
//! [`ImportBundle::write_bundle`].
//!
//! ## Output layout
//! ```text
//! <out>/portable/<kind>/<id>.json   one portable model document per file
//! <out>/machine/<kind>/<id>.json    machine-local *suggestions* (output / input bindings, media
//!                                   root bindings, audio device, font substitutions)
//! <out>/report.json                 ImportReport (machine-readable)
//! <out>/report.md                   human summary
//! ```
//! `<kind>` is the [`DocumentKind`] route segment (`presentation`, `globalGroup`, ...), `<id>` the
//! document's [`Document::id`] (singletons use the nil UUID). Files are pretty JSON with a trailing
//! newline. Output is deterministic: the bundle is sorted by ID, `portable/` and `machine/` are
//! recreated from scratch on every write (no stale files), and nothing time-dependent is written,
//! so two imports of the same backup produce byte-identical trees.
//!
//! **Safety:** `write_bundle` refuses an output directory inside the backup it was imported from,
//! and a data folder that has moved into Vesper Codex (it holds `codex/`): there `portable/` and
//! `machine/` are the box's own documents and identity (site membership, screens, looks), which
//! recreating them would destroy.

use std::io;
use std::path::{Path, PathBuf};

use model::{
    AudioDeviceConfig, AudioInput, AudioInputBinding, ClearGroup, Document, DocumentKind,
    FontSubstitutionTable, GlobalGroup, InputBinding, Library, Look, Macro, Mask, MediaAsset,
    MediaBin, MediaRoot, MediaRootBinding, Message, OutputBinding, PlaylistNode, Presentation,
    Prop, Screen, ShowSettings, StageLayout, Theme, Timer, VideoInput,
};

use crate::report::ImportReport;

/// Everything one import produced. Vectors are in insertion order until [`ImportBundle::sort`]
/// (called by `ImportContext::finish`) sorts them by ID.
#[derive(Clone, Debug, Default)]
pub struct ImportBundle {
    // ---- portable ----------------------------------------------------------------------------
    /// Libraries.
    pub libraries: Vec<Library>,
    /// Presentations (with sections and slides).
    pub presentations: Vec<Presentation>,
    /// Global groups.
    pub global_groups: Vec<GlobalGroup>,
    /// Playlist folders and playlists (service and audio).
    pub playlist_nodes: Vec<PlaylistNode>,
    /// Media and audio bins.
    pub media_bins: Vec<MediaBin>,
    /// Media assets (one per distinct file).
    pub media_assets: Vec<MediaAsset>,
    /// Media roots (the backup itself and one per external volume).
    pub media_roots: Vec<MediaRoot>,
    /// Themes (de-duplicated).
    pub themes: Vec<Theme>,
    /// Screens.
    pub screens: Vec<Screen>,
    /// Looks.
    pub looks: Vec<Look>,
    /// Masks.
    pub masks: Vec<Mask>,
    /// Logical video inputs.
    pub video_inputs: Vec<VideoInput>,
    /// Logical audio inputs.
    pub audio_inputs: Vec<AudioInput>,
    /// Timers.
    pub timers: Vec<Timer>,
    /// Stage layouts.
    pub stage_layouts: Vec<StageLayout>,
    /// Clear groups.
    pub clear_groups: Vec<ClearGroup>,
    /// Props.
    pub props: Vec<Prop>,
    /// Messages.
    pub messages: Vec<Message>,
    /// Macros.
    pub macros: Vec<Macro>,
    /// Show settings singleton (CCLI, startup look, ...).
    pub show_settings: Option<ShowSettings>,

    // ---- machine-local suggestions -------------------------------------------------------------
    /// Suggested output bindings (network sources, displays) of the source machine.
    pub output_bindings: Vec<OutputBinding>,
    /// Suggested video input bindings.
    pub input_bindings: Vec<InputBinding>,
    /// Suggested audio input bindings.
    pub audio_input_bindings: Vec<AudioInputBinding>,
    /// Suggested media root bindings (the backup root → its local folder).
    pub media_root_bindings: Vec<MediaRootBinding>,
    /// Suggested audio device config.
    pub audio_device_config: Option<AudioDeviceConfig>,
    /// Suggested font substitutions.
    pub font_substitutions: Option<FontSubstitutionTable>,

    // ---- report ----------------------------------------------------------------------------------
    /// The import report.
    pub report: ImportReport,
    /// Backup the bundle was imported from (never written into). Not serialized.
    pub backup_root: Option<PathBuf>,
}

/// What [`ImportBundle::write_bundle`] wrote.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteSummary {
    /// Document files written.
    pub documents: usize,
    /// `kind/id` pairs that occurred more than once (later ones overwrote earlier ones). Always
    /// a converter bug; listed so the CLI can fail loudly.
    pub duplicate_ids: Vec<String>,
    /// `kind/id: error` of documents whose written JSON does not read back (a non-finite float
    /// becomes `null`). Always a converter bug; the server cannot load them.
    pub unloadable: Vec<String>,
}

fn sort_by_id<T: Document>(v: &mut [T]) {
    v.sort_by_key(|d| d.id());
}

fn kind_segment(kind: DocumentKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

/// Writes `value` as pretty JSON plus a newline; returns the bytes written.
fn write_json(path: &Path, value: &impl serde::Serialize) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    std::fs::write(path, &bytes)?;
    Ok(bytes)
}

fn write_docs<T: Document>(out: &Path, docs: &[T], summary: &mut WriteSummary) -> io::Result<()> {
    if docs.is_empty() {
        return Ok(());
    }
    let kind = kind_segment(T::KIND);
    let dir = out
        .join(if T::portable() { "portable" } else { "machine" })
        .join(&kind);
    std::fs::create_dir_all(&dir)?;
    let mut refs: Vec<&T> = docs.iter().collect();
    refs.sort_by_key(|d| d.id());
    for w in refs.windows(2) {
        if w[0].id() == w[1].id() {
            summary.duplicate_ids.push(format!("{kind}/{}", w[0].id()));
        }
    }
    for d in refs {
        let bytes = write_json(&dir.join(format!("{}.json", d.id())), d)?;
        summary.documents += 1;
        if let Err(e) = serde_json::from_slice::<T>(&bytes) {
            summary.unloadable.push(format!("{kind}/{}: {e}", d.id()));
        }
    }
    Ok(())
}

/// Absolute, normalized form of `p` without requiring it to exist: the longest existing ancestor
/// is canonicalized and the rest appended.
fn absolute_lenient(p: &Path) -> PathBuf {
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    let mut existing = p.as_path();
    let mut rest = Vec::new();
    loop {
        if let Ok(c) = existing.canonicalize() {
            let mut out = c;
            for r in rest.iter().rev() {
                out.push(r);
            }
            return out;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return p,
        }
    }
}

/// True when `dir` is `root` or inside it.
pub fn is_inside(dir: &Path, root: &Path) -> bool {
    absolute_lenient(dir).starts_with(absolute_lenient(root))
}

impl ImportBundle {
    /// Links the imported presentations' slides to the bundle's themes **keeping their look**
    /// (`pp-import --link-themes`, see [`model::theme_link::link_to_closest`]): each slide
    /// follows the template slide it was made from (else the closest one) with every
    /// difference as an override, so nothing on screen changes; a slide that can't be linked
    /// that way stays a plain copy. Returns (slides linked, slides in all).
    pub fn link_themes_keeping_look(&mut self) -> (usize, usize) {
        let themes = &self.themes;
        let (mut linked, mut total) = (0, 0);
        for p in &mut self.presentations {
            for sec in &mut p.sections {
                for s in &mut sec.slides {
                    total += 1;
                    if let Some(k) = model::theme_link::link_to_closest(s, themes) {
                        *s = k.slide;
                        linked += 1;
                    }
                }
            }
        }
        (linked, total)
    }

    /// Sorts every document list by ID (deterministic output order).
    pub fn sort(&mut self) {
        sort_by_id(&mut self.libraries);
        sort_by_id(&mut self.presentations);
        sort_by_id(&mut self.global_groups);
        sort_by_id(&mut self.playlist_nodes);
        sort_by_id(&mut self.media_bins);
        sort_by_id(&mut self.media_assets);
        sort_by_id(&mut self.media_roots);
        sort_by_id(&mut self.themes);
        sort_by_id(&mut self.screens);
        sort_by_id(&mut self.looks);
        sort_by_id(&mut self.masks);
        sort_by_id(&mut self.video_inputs);
        sort_by_id(&mut self.audio_inputs);
        sort_by_id(&mut self.timers);
        sort_by_id(&mut self.stage_layouts);
        sort_by_id(&mut self.clear_groups);
        sort_by_id(&mut self.props);
        sort_by_id(&mut self.messages);
        sort_by_id(&mut self.macros);
        sort_by_id(&mut self.output_bindings);
        sort_by_id(&mut self.input_bindings);
        sort_by_id(&mut self.audio_input_bindings);
        sort_by_id(&mut self.media_root_bindings);
    }

    /// Writes the bundle to `dir` (see the module docs). Refuses a `dir` inside the backup, and
    /// a data folder that has moved into Vesper Codex (it holds `codex/`).
    pub fn write_bundle(&self, dir: &Path) -> io::Result<WriteSummary> {
        if let Some(root) = &self.backup_root
            && is_inside(dir, root)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "refusing to write import output inside the backup ({})",
                    dir.display()
                ),
            ));
        }
        // `portable/` and `machine/` are deleted below: never when the backup lives in one.
        if let Some(root) = &self.backup_root
            && ["portable", "machine"]
                .iter()
                .any(|sub| is_inside(root, &dir.join(sub)))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "refusing to write import output to {}: it would delete the backup",
                    dir.display()
                ),
            ));
        }
        // A data folder moved into Vesper Codex keeps the box's identity, site membership,
        // screens and looks in `portable/` and `machine/`: never recreate those.
        if dir.join("codex").exists() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "refusing to write import output to {}: this data folder has been moved into \
                     Vesper Codex; import into a fresh folder",
                    dir.display()
                ),
            ));
        }
        std::fs::create_dir_all(dir)?;
        for sub in ["portable", "machine"] {
            let p = dir.join(sub);
            if p.exists() {
                std::fs::remove_dir_all(&p)?;
            }
        }
        let mut s = WriteSummary::default();
        write_docs(dir, &self.libraries, &mut s)?;
        write_docs(dir, &self.presentations, &mut s)?;
        write_docs(dir, &self.global_groups, &mut s)?;
        write_docs(dir, &self.playlist_nodes, &mut s)?;
        write_docs(dir, &self.media_bins, &mut s)?;
        write_docs(dir, &self.media_assets, &mut s)?;
        write_docs(dir, &self.media_roots, &mut s)?;
        write_docs(dir, &self.themes, &mut s)?;
        write_docs(dir, &self.screens, &mut s)?;
        write_docs(dir, &self.looks, &mut s)?;
        write_docs(dir, &self.masks, &mut s)?;
        write_docs(dir, &self.video_inputs, &mut s)?;
        write_docs(dir, &self.audio_inputs, &mut s)?;
        write_docs(dir, &self.timers, &mut s)?;
        write_docs(dir, &self.stage_layouts, &mut s)?;
        write_docs(dir, &self.clear_groups, &mut s)?;
        write_docs(dir, &self.props, &mut s)?;
        write_docs(dir, &self.messages, &mut s)?;
        write_docs(dir, &self.macros, &mut s)?;
        write_docs(dir, self.show_settings.as_slice(), &mut s)?;
        write_docs(dir, &self.output_bindings, &mut s)?;
        write_docs(dir, &self.input_bindings, &mut s)?;
        write_docs(dir, &self.audio_input_bindings, &mut s)?;
        write_docs(dir, &self.media_root_bindings, &mut s)?;
        write_docs(dir, self.audio_device_config.as_slice(), &mut s)?;
        write_docs(dir, self.font_substitutions.as_slice(), &mut s)?;
        write_json(&dir.join("report.json"), &self.report)?;
        std::fs::write(dir.join("report.md"), self.report.render_markdown())?;
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_imported_slides_keeping_their_look() {
        use model::{
            Element, ElementId, Placeholder, Rect, Section, Slide, TemplateSlide, TextBox,
        };
        let mut slot = Element::text(Rect::new(10.0, 10.0, 600.0, 100.0), TextBox::plain(""));
        slot.placeholder = Some(Placeholder::text("lyrics"));
        let t = TemplateSlide {
            name: "Lyrics".into(),
            slide: Slide {
                elements: vec![slot],
                ..Slide::new()
            },
            ..TemplateSlide::default()
        };
        let tid = t.id;
        let theme = Theme {
            name: "Made-up Theme".into(),
            slides: vec![t],
            ..Theme::default()
        };
        let mut copied = theme.template(tid).unwrap().slide.clone();
        copied.id = model::SlideId::new();
        copied.elements[0].id = ElementId::new();
        copied.elements[0].text_box_mut().unwrap().paragraphs =
            TextBox::plain("Made-up line").paragraphs;
        copied.elements[0].bounds = Rect::new(0.0, 0.0, 300.0, 50.0);
        copied.template_origin = Some(theme.template_ref(tid));
        let plain = Slide {
            elements: vec![Element::text(
                Rect::new(0.0, 0.0, 10.0, 10.0),
                TextBox::plain("No theme"),
            )],
            ..Slide::new()
        };
        let mut sec = Section::new("Verse");
        sec.slides = vec![copied.clone(), plain.clone()];
        let mut b = ImportBundle {
            themes: vec![theme],
            presentations: vec![Presentation {
                sections: vec![sec],
                ..Presentation::new("Made-up Song")
            }],
            ..ImportBundle::default()
        };
        assert_eq!(b.link_themes_keeping_look(), (1, 2));
        let s = &b.presentations[0].sections[0].slides;
        assert!(s[0].theme_link.is_some());
        assert_eq!(
            s[0].elements[0].from_theme.as_ref().unwrap().overrides,
            vec!["bounds".to_string()]
        );
        assert_eq!(s[1], plain, "a slide without a matching template stays");
    }

    fn scratch(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "pro-import-bundle-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn writes_deterministic_tree_and_refuses_backup() {
        let backup = scratch("backup");
        std::fs::create_dir_all(&backup).unwrap();
        let mut b = ImportBundle {
            backup_root: Some(backup.clone()),
            ..ImportBundle::default()
        };
        b.libraries.push(Library {
            id: model::LibraryId::from_uuid(uuid::Uuid::from_u128(2)),
            name: "B".into(),
            ..Library::default()
        });
        b.libraries.push(Library {
            id: model::LibraryId::from_uuid(uuid::Uuid::from_u128(1)),
            name: "A".into(),
            ..Library::default()
        });
        b.show_settings = Some(ShowSettings::default());
        b.font_substitutions = Some(FontSubstitutionTable::default());

        assert!(b.write_bundle(&backup.join("out")).is_err());

        let out = scratch("out");
        let s = b.write_bundle(&out).unwrap();
        assert_eq!(s.documents, 4);
        assert!(s.duplicate_ids.is_empty());
        let lib = out
            .join("portable/library")
            .join(format!("{}.json", uuid::Uuid::from_u128(1)));
        let first = std::fs::read(&lib).unwrap();
        assert!(first.ends_with(b"\n"));
        assert!(out.join("portable/showSettings").is_dir());
        assert!(out.join("machine/fontSubstitutionTable").is_dir());
        assert!(out.join("report.json").is_file() && out.join("report.md").is_file());
        // Second write: identical bytes, stale files removed.
        std::fs::write(out.join("portable/library/stale.json"), b"{}").unwrap();
        b.write_bundle(&out).unwrap();
        assert_eq!(std::fs::read(&lib).unwrap(), first);
        assert!(!out.join("portable/library/stale.json").exists());
        let _ = std::fs::remove_dir_all(&out);
        let _ = std::fs::remove_dir_all(&backup);
    }

    #[test]
    fn unloadable_documents_are_reported() {
        // JSON has no NaN: serde_json writes a non-finite float as `null`, which the f64 field
        // then refuses to read. Such a document used to be written silently, and the server
        // could not load it.
        let out = scratch("nan");
        let mut b = ImportBundle::default();
        let mut bad = Screen::default();
        bad.size.width = f64::NAN;
        let good = Screen::default();
        b.screens = vec![bad.clone(), good];
        let s = b.write_bundle(&out).unwrap();
        assert_eq!(s.documents, 2);
        assert_eq!(s.unloadable.len(), 1, "{:?}", s.unloadable);
        assert!(s.unloadable[0].starts_with(&format!("screen/{}", bad.id)));
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn never_deletes_a_backup_inside_the_output_subfolders() {
        // Writing recreates <out>/portable and <out>/machine from scratch. A backup stored in
        // one of them used to be deleted with it.
        let out = scratch("parent");
        for sub in ["portable", "machine"] {
            let backup = out.join(sub).join("Backup");
            std::fs::create_dir_all(backup.join("Libraries")).unwrap();
            std::fs::write(backup.join("Libraries/Song.pro"), b"x").unwrap();
            let b = ImportBundle {
                backup_root: Some(backup.clone()),
                ..ImportBundle::default()
            };
            assert!(b.write_bundle(&out).is_err(), "{sub}");
            assert!(backup.join("Libraries/Song.pro").is_file(), "{sub}");
        }
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn never_writes_into_a_data_folder_moved_into_codex() {
        // A data folder the server has migrated into Vesper Codex: `portable/` and `machine/`
        // hold the box's own documents and identity, `codex/` the content.
        let out = scratch("codex");
        let keep = [
            "machine/site-member/identity.json",
            "machine/codex.json",
            "portable/screen/s.json",
            "portable/look/l.json",
            "codex/codex.json",
        ];
        for f in keep {
            std::fs::create_dir_all(out.join(f).parent().unwrap()).unwrap();
            std::fs::write(out.join(f), b"{}").unwrap();
        }
        let mut b = ImportBundle::default();
        b.screens.push(Screen::default());
        let e = b.write_bundle(&out).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
        assert!(e.to_string().contains("Vesper Codex"), "{e}");
        for f in keep {
            assert_eq!(std::fs::read(out.join(f)).unwrap(), b"{}", "{f}");
        }
        assert!(!out.join("report.json").exists(), "nothing written");
        let _ = std::fs::remove_dir_all(&out);
    }
}
