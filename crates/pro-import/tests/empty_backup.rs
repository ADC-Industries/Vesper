//! The orchestrator never lets a panic escape, even on an empty or bogus folder.

#[test]
fn empty_folder_imports_without_panicking() {
    let dir = std::env::temp_dir().join(format!("pro-import-empty-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("Libraries")).unwrap();
    std::fs::write(dir.join("Libraries/LibraryData"), b"\xff\xff not protobuf").unwrap();
    let bundle = pro_import::import_backup(&dir);
    assert!(bundle.presentations.is_empty());
    assert!(
        bundle
            .report
            .parse_failures
            .iter()
            .any(|f| f.file == "Libraries/LibraryData")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A library folder and a song whose names are not valid UTF-8 (a Windows zip without the
/// UTF-8 flag, unpacked on Linux) are imported by their real paths and reported (A11-1).
#[cfg(unix)]
#[test]
fn non_utf8_names_are_imported() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let dir = std::env::temp_dir().join(format!("pro-import-latin1-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let lib = dir
        .join("Libraries")
        .join(OsStr::from_bytes(b"Canci\xf3nes"));
    std::fs::create_dir_all(&lib).unwrap();
    // An empty file is a valid (empty) presentation document.
    std::fs::write(lib.join(OsStr::from_bytes(b"Canci\xf3n.pro")), b"").unwrap();
    let bundle = pro_import::import_backup(&dir);
    assert!(
        bundle.report.parse_failures.is_empty(),
        "{:?}",
        bundle.report.parse_failures
    );
    assert_eq!(bundle.presentations.len(), 1);
    assert!(
        bundle
            .report
            .warnings
            .iter()
            .any(|w| w.detail.contains("not valid UTF-8"))
    );
    let _ = std::fs::remove_dir_all(&dir);
}
