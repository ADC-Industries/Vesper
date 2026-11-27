//! **Area: content.** Libraries (`Libraries/LibraryData` + `Libraries/<folder>/`).
//!
//! ## Owns
//! - [`import_libraries`]: one [`Library`] per `Libraries/<folder>` folder. Order: the
//!   `LibraryData` list first (its URLs' `local.path` = `Libraries/<folder>`, ROOT_SHOW), then
//!   folders not listed there, by name. ID = `ids::library_id(folder)`, name = folder name,
//!   `sortKey` from `ids::sort_keys`, `import.documentPath = "Libraries/<folder>"`. Listed
//!   libraries without a folder are warned (`other`) and skipped. Registers
//!   `ctx.lookups.libraries`. Returns `(folder, id)` in sidebar order; the orchestrator imports
//!   presentations in that order.
//! - [`library_folder_from_url`].
//!
//! ## Details
//! - `LibraryData` only orders libraries; the content of a library is always its folder on disk.
//!   Nested `libraryChildren` are flattened depth-first (unused in practice); `libraryItems`
//!   (never used) are ignored with a warning.
//! - A listed name matches a folder exactly, else case-insensitively (the source is often
//!   Windows, whose paths are case-insensitive). A folder listed twice is imported once.
//!
//! ## May call
//! `common::*`, `ImportContext` helpers.

use std::collections::BTreeSet;

use model::{ImportInfo, Library, LibraryId};

use crate::context::{ImportContext, ids};
use crate::pb::rv::data as pp;
use crate::report::Category;
use crate::source;

/// Creates the libraries (see the module docs). `folders` are the folder names found on disk
/// (`source::library_folders`), sorted.
pub fn import_libraries(
    ctx: &mut ImportContext,
    library_data: Option<&pp::Library>,
    folders: &[String],
) -> Vec<(String, LibraryId)> {
    // Folder names in sidebar order.
    let mut order: Vec<String> = Vec::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();

    let mut listed = Vec::new();
    if let Some(doc) = library_data {
        collect_listed(ctx, doc, &mut listed, 0);
    }
    for name in listed {
        let found = folders
            .iter()
            .find(|f| **f == name)
            .or_else(|| folders.iter().find(|f| f.eq_ignore_ascii_case(&name)));
        match found {
            Some(folder) => {
                if taken.insert(folder.clone()) {
                    order.push(folder.clone());
                }
            }
            None => ctx.warn(
                Category::Other,
                format!("library \"{name}\" is listed in LibraryData but has no folder; skipped"),
            ),
        }
    }
    for folder in folders {
        if taken.insert(folder.clone()) {
            order.push(folder.clone());
        }
    }

    let keys = ids::sort_keys(order.len());
    let mut out = Vec::with_capacity(order.len());
    for (folder, sort_key) in order.into_iter().zip(keys) {
        let id = ids::library_id(&folder);
        let import = ImportInfo {
            document_path: Some(format!("{}/{folder}", source::LIBRARIES)),
            imported_at: ctx.options.imported_at.clone(),
            ..ImportInfo::default()
        };
        ctx.out.libraries.push(Library {
            id,
            name: folder.clone(),
            sort_key,
            import: Some(import),
            ..Library::default()
        });
        ctx.lookups.libraries.insert(folder.clone(), id);
        out.push((folder, id));
    }
    out
}

/// Library folder names listed in `LibraryData`, depth-first.
fn collect_listed(ctx: &mut ImportContext, lib: &pp::Library, out: &mut Vec<String>, depth: usize) {
    use pp::library::ChildType;
    if depth > 0 {
        match lib.url.as_ref().and_then(library_folder_from_url) {
            Some(folder) => out.push(folder),
            None => ctx.warn(
                Category::Other,
                "a LibraryData entry has no usable folder path; skipped",
            ),
        }
    }
    match &lib.child_type {
        Some(ChildType::LibraryChildren(arr)) if depth < 16 => {
            for child in &arr.libraries {
                collect_listed(ctx, child, out, depth + 1);
            }
        }
        Some(ChildType::LibraryItems(items)) if !items.items.is_empty() => ctx.warn(
            Category::NotImported,
            format!(
                "LibraryData lists {} library items; library contents come from the folders",
                items.items.len()
            ),
        ),
        _ => {}
    }
}

/// The library folder name a `LibraryData` URL points at (`local.path` `"Libraries/Music"` →
/// `"Music"`; falls back to the last segment of `absoluteString`).
pub fn library_folder_from_url(url: &pp::Url) -> Option<String> {
    use pp::url::{RelativeFilePath, Storage};
    let from_path = |p: &str| -> Option<String> {
        let norm = source::normalize_rel(p);
        let segs: Vec<&str> = norm.split('/').collect();
        // The segment after the last "Libraries" segment, else the last segment.
        let after = segs
            .iter()
            .rposition(|s| s.eq_ignore_ascii_case(source::LIBRARIES))
            .and_then(|i| segs.get(i + 1));
        after
            .or(segs.last())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let local = match &url.relative_file_path {
        Some(RelativeFilePath::Local(l)) => from_path(&l.path),
        Some(RelativeFilePath::External(e)) => from_path(&e.path),
        None => None,
    };
    local.or_else(|| match &url.storage {
        Some(Storage::AbsoluteString(s)) | Some(Storage::RelativePath(s)) => {
            let s = s.strip_prefix("file://").unwrap_or(s);
            from_path(s)
        }
        None => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImportOptions;
    use std::path::Path;

    fn url(local: &str, abs: &str) -> pp::Url {
        pp::Url {
            platform: pp::url::Platform::Win32 as i32,
            storage: (!abs.is_empty()).then(|| pp::url::Storage::AbsoluteString(abs.into())),
            relative_file_path: (!local.is_empty()).then(|| {
                pp::url::RelativeFilePath::Local(pp::url::LocalRelativePath {
                    root: pp::url::local_relative_path::Root::Show as i32,
                    path: local.into(),
                })
            }),
        }
    }

    fn data(urls: Vec<pp::Url>) -> pp::Library {
        pp::Library {
            url: None,
            child_type: Some(pp::library::ChildType::LibraryChildren(
                pp::library::LibraryArray {
                    libraries: urls
                        .into_iter()
                        .map(|u| pp::Library {
                            url: Some(u),
                            child_type: None,
                        })
                        .collect(),
                },
            )),
        }
    }

    #[test]
    fn folder_from_url() {
        assert_eq!(
            library_folder_from_url(&url("Libraries/Songs", "")).as_deref(),
            Some("Songs")
        );
        assert_eq!(
            library_folder_from_url(&url(
                "",
                "C:\\Users\\u\\Documents\\ProPresenter\\Libraries\\Talks"
            ))
            .as_deref(),
            Some("Talks")
        );
        assert_eq!(
            library_folder_from_url(&url("", "/Users/u/Library/X/")).as_deref(),
            Some("X")
        );
        assert_eq!(library_folder_from_url(&url("", "")), None);
    }

    #[test]
    fn order_listed_first_then_unlisted_by_name() {
        let mut ctx = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        let doc = data(vec![
            url("Libraries/Songs", ""),
            url("Libraries/talks", ""), // case differs from the folder
            url("Libraries/Gone", ""),  // no folder
            url("Libraries/Songs", ""), // listed twice
        ]);
        let folders: Vec<String> = ["Alpha", "Songs", "Talks", "Zeta"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let libs = import_libraries(&mut ctx, Some(&doc), &folders);
        let names: Vec<_> = libs.iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(names, ["Songs", "Talks", "Alpha", "Zeta"]);
        assert_eq!(ctx.out.libraries.len(), 4);
        let keys: Vec<_> = ctx
            .out
            .libraries
            .iter()
            .map(|l| l.sort_key.clone())
            .collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(ctx.out.libraries[0].id, ids::library_id("Songs"));
        assert_eq!(
            ctx.out.libraries[1]
                .import
                .as_ref()
                .unwrap()
                .document_path
                .as_deref(),
            Some("Libraries/Talks")
        );
        assert_eq!(ctx.lookups.libraries["Zeta"], ids::library_id("Zeta"));
        assert_eq!(ctx.report.warnings.len(), 1);
        assert!(ctx.report.warnings[0].detail.contains("Gone"));
    }

    #[test]
    fn without_library_data_folders_are_sorted() {
        let mut ctx = ImportContext::new(Path::new("/nonexistent"), ImportOptions::default());
        let folders = vec!["A".to_string(), "B".to_string()];
        let libs = import_libraries(&mut ctx, None, &folders);
        assert_eq!(libs.len(), 2);
        assert_eq!(libs[0].0, "A");
        assert!(ctx.report.warnings.is_empty());
    }
}
