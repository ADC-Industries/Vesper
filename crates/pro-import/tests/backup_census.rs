//! Imports the real reference backup and checks the census invariants.
//!
//! Runs only when `PP_BACKUP` points at a ProPresenter data folder (the backup is private and
//! never committed):
//!
//! ```text
//! PP_BACKUP=/path/to/ProPresentor cargo test -p pro-import --test backup_census -- --nocapture
//! ```

use std::path::PathBuf;

#[test]
fn reference_backup_matches_census() {
    let Some(root) = std::env::var_os("PP_BACKUP").map(PathBuf::from) else {
        eprintln!("PP_BACKUP not set; skipping");
        return;
    };
    let bundle = pro_import::import_backup(&root);
    let bad = pro_import::census::check(&bundle);
    for e in &bad {
        eprintln!("{}: expected {}, got {}", e.name, e.expected, e.actual);
    }
    for f in bundle.report.parse_failures.iter().take(20) {
        eprintln!("parse failure {}: {}", f.file, f.error);
    }
    assert!(bad.is_empty(), "{} census expectations failed", bad.len());

    // Deterministic: a second import yields identical documents.
    let again = pro_import::import_backup(&root);
    assert_eq!(
        serde_json::to_string(&bundle.presentations).unwrap(),
        serde_json::to_string(&again.presentations).unwrap()
    );
    assert_eq!(bundle.report, again.report);
}
