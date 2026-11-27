//! The committed TypeScript bindings (`crates/protocol/bindings/`) must match the Rust types.
//!
//! Regenerate after changing a wire type:
//! ```text
//! UPDATE_BINDINGS=1 cargo test -p protocol --test bindings
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn committed_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("bindings")
}

fn read_ts(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "ts") {
                let name = p.file_name().unwrap().to_string_lossy().to_string();
                out.insert(name, std::fs::read_to_string(&p).unwrap());
            }
        }
    }
    out
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("protocol-bindings-{tag}-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn bindings_are_up_to_date() {
    let tmp = fresh_dir("check");
    let names = protocol::bindings::export_all(&tmp).expect("export");
    assert!(names.contains(&"ShowState".to_string()));
    let fresh = read_ts(&tmp);

    if std::env::var_os("UPDATE_BINDINGS").is_some() {
        let dir = committed_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // Remove only stale generated .ts files, then write the fresh set.
        for name in read_ts(&dir).keys() {
            if !fresh.contains_key(name) {
                std::fs::remove_file(dir.join(name)).unwrap();
            }
        }
        for (name, text) in &fresh {
            std::fs::write(dir.join(name), text).unwrap();
        }
        eprintln!("wrote {} files to {}", fresh.len(), dir.display());
    }

    let committed = read_ts(&committed_dir());
    let missing: Vec<&String> = fresh
        .keys()
        .filter(|k| !committed.contains_key(*k))
        .collect();
    let extra: Vec<&String> = committed
        .keys()
        .filter(|k| !fresh.contains_key(*k))
        .collect();
    let changed: Vec<&String> = fresh
        .iter()
        .filter(|(k, v)| committed.get(*k).is_some_and(|c| c != *v))
        .map(|(k, _)| k)
        .collect();
    std::fs::remove_dir_all(&tmp).ok();
    assert!(
        missing.is_empty() && extra.is_empty() && changed.is_empty(),
        "TypeScript bindings are stale (missing {missing:?}, extra {extra:?}, changed {changed:?}).\n\
         Regenerate with: {}",
        protocol::bindings::REGENERATE
    );
}
