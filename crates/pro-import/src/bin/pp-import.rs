//! Imports a ProPresenter 7 data folder (backup) into model documents.
//!
//! ```text
//! pp-import <backup-dir> <out-dir> [--check] [--link-themes]
//! ```
//!
//! Writes `<out-dir>/portable/…`, `<out-dir>/machine/…`, `report.json` and `report.md` (see
//! `pro_import::bundle`). The suggested default output directory is `data/import` (gitignored).
//! Never writes inside the backup, never copies media.
//!
//! `--link-themes` links the imported songs' slides to their themes **keeping their look**
//! (every difference becomes an override, so nothing on screen changes; slides that can't be
//! linked that way stay plain copies): editing the theme then restyles them.
//!
//! `--check` additionally compares the result with the census of the reference backup
//! (`pro_import::census`) and exits with status 1 on any mismatch.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let check = if let Some(i) = args.iter().position(|a| a == "--check") {
        args.remove(i);
        true
    } else {
        false
    };
    let link_themes = if let Some(i) = args.iter().position(|a| a == "--link-themes") {
        args.remove(i);
        true
    } else {
        false
    };
    let [backup, out] = args.as_slice() else {
        eprintln!("usage: pp-import <backup-dir> <out-dir> [--check] [--link-themes]");
        return ExitCode::from(2);
    };
    let (backup, out) = (PathBuf::from(backup), PathBuf::from(out));
    if !backup.is_dir() {
        eprintln!("pp-import: {} is not a directory", backup.display());
        return ExitCode::from(2);
    }
    if pro_import::bundle::is_inside(&out, &backup) {
        eprintln!("pp-import: refusing to write output inside the backup folder");
        return ExitCode::from(2);
    }

    // Converter panics are caught per file and listed in the report; keep stderr readable.
    std::panic::set_hook(Box::new(|_| {}));
    let mut bundle = pro_import::import_backup(&backup);
    let _ = std::panic::take_hook();
    if link_themes {
        let (linked, total) = bundle.link_themes_keeping_look();
        println!("linked {linked} of {total} slides to their themes, keeping their look");
    }

    let summary = match bundle.write_bundle(&out) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("pp-import: writing {} failed: {e}", out.display());
            return ExitCode::FAILURE;
        }
    };

    let r = &bundle.report;
    println!("wrote {} documents to {}", summary.documents, out.display());
    println!(
        "presentations {}, slides {}, playlists {}, themes {}, media assets {} ({} missing)",
        r.counts.presentations,
        r.counts.slides,
        r.counts.playlists,
        r.counts.themes,
        r.counts.media_assets,
        r.counts.media_assets_missing
    );
    println!(
        "parse failures {}, warnings {}; see {}",
        r.parse_failures.len(),
        r.warnings.len(),
        out.join("report.md").display()
    );
    let mut failed = false;
    if !summary.duplicate_ids.is_empty() {
        failed = true;
        eprintln!(
            "pp-import: {} duplicate document IDs (converter bug):",
            summary.duplicate_ids.len()
        );
        for d in summary.duplicate_ids.iter().take(20) {
            eprintln!("  {d}");
        }
    }

    if !summary.unloadable.is_empty() {
        failed = true;
        eprintln!(
            "pp-import: {} documents do not read back (converter bug):",
            summary.unloadable.len()
        );
        for d in summary.unloadable.iter().take(20) {
            eprintln!("  {d}");
        }
    }

    if check {
        let all = pro_import::census::expectations(&bundle);
        let bad: Vec<_> = all.iter().filter(|e| !e.holds()).collect();
        println!(
            "\ncensus check: {}/{} hold",
            all.len() - bad.len(),
            all.len()
        );
        for e in &all {
            println!(
                "  {} {:<48} expected {:>5}  actual {:>5}",
                if e.holds() { "ok  " } else { "FAIL" },
                e.name,
                e.expected,
                e.actual
            );
        }
        failed |= !bad.is_empty();
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
