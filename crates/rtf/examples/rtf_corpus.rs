//! Runs the RTF parser over a corpus and prints statistics and failures.
//!
//! ```text
//! cargo run -p rtf --example rtf_corpus -- [--text OUT.jsonl] PATH...
//! ```
//!
//! Each PATH is a file or a directory (searched recursively):
//!
//! * `.json` files are searched for every `"rtfData"` string, which is
//!   decoded as base64 (the format of `pp-inspect dump`).
//! * `.rtf` files are parsed as they are.
//!
//! A blob fails when it is not valid base64, when parsing panics, or when the
//! parser reports a structural [`rtf::Warning`]. The exit status is 1 if any
//! blob failed.
//!
//! `--text OUT.jsonl` also writes one JSON line per blob,
//! `{"file", "pointer", "paragraphs": [..]}`, for cross-checking the extracted
//! text against another implementation.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::panic;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use base64::Engine;
use rtf::{Alignment, Diagnostics, Document, LineSpacing};
use serde_json::Value;

struct Blob {
    file: PathBuf,
    /// JSON pointer of the `rtfData` field, empty for `.rtf` files.
    pointer: String,
    data: Result<Vec<u8>, String>,
}

#[derive(Default)]
struct Stats {
    files: usize,
    blobs: usize,
    empty_input: usize,
    empty_text: usize,
    paragraphs: usize,
    runs: usize,
    chars: usize,
    non_ascii_chars: BTreeMap<char, usize>,
    paragraphs_per_doc: BTreeMap<usize, usize>,
    multi_run_paragraphs: usize,
    multi_style_docs: usize,
    list_paragraphs: usize,
    fonts: BTreeMap<String, usize>,
    alignments: BTreeMap<String, usize>,
    line_spacing: BTreeMap<String, usize>,
    scripts: BTreeMap<String, usize>,
    unknown_words: BTreeMap<String, usize>,
    ignored_words: BTreeMap<String, usize>,
    unknown_symbols: BTreeMap<char, usize>,
    warnings: BTreeMap<String, usize>,
    failures: Vec<String>,
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut roots = Vec::new();
    let mut text_out = None;
    while let Some(a) = args.next() {
        if a == "--text" {
            let Some(p) = args.next() else {
                eprintln!("--text needs a path");
                return ExitCode::from(2);
            };
            text_out = Some(p);
        } else {
            roots.push(PathBuf::from(a));
        }
    }
    if roots.is_empty() {
        eprintln!("usage: rtf_corpus [--text OUT.jsonl] PATH...");
        return ExitCode::from(2);
    }
    let mut text_out = match text_out.map(fs::File::create).transpose() {
        Ok(f) => f.map(std::io::BufWriter::new),
        Err(e) => {
            eprintln!("cannot create --text output: {e}");
            return ExitCode::from(2);
        }
    };

    let mut files = Vec::new();
    for root in &roots {
        collect_files(root, &mut files);
    }
    files.sort();

    // Panics are counted as failures; keep the default hook from spamming.
    panic::set_hook(Box::new(|_| {}));

    let mut stats = Stats::default();
    for file in &files {
        let blobs = match read_blobs(file) {
            Ok(b) => b,
            Err(e) => {
                stats
                    .failures
                    .push(format!("{}: unreadable: {e}", file.display()));
                continue;
            }
        };
        stats.files += 1;
        for blob in blobs {
            stats.blobs += 1;
            let at = format!("{}{}", blob.file.display(), blob.pointer);
            let data = match blob.data {
                Ok(d) => d,
                Err(e) => {
                    stats.failures.push(format!("{at}: bad base64: {e}"));
                    continue;
                }
            };
            if data.is_empty() {
                stats.empty_input += 1;
            }
            let (doc, diag) = match panic::catch_unwind(|| rtf::parse_with_diagnostics(&data)) {
                Ok(r) => r,
                Err(_) => {
                    stats.failures.push(format!("{at}: PANIC"));
                    continue;
                }
            };
            // Empty input is not RTF at all, so only judge non-empty blobs.
            if !data.is_empty() && !diag.is_clean() {
                stats.failures.push(format!("{at}: {:?}", diag.warnings));
            }
            record(&mut stats, &doc, &diag);
            if let Some(out) = text_out.as_mut() {
                let line = serde_json::json!({
                    "file": blob.file.display().to_string(),
                    "pointer": blob.pointer,
                    "paragraphs": doc.paragraphs.iter().map(|p| p.text()).collect::<Vec<_>>(),
                });
                if let Err(e) = writeln!(out, "{line}") {
                    eprintln!("write failed: {e}");
                    return ExitCode::from(2);
                }
            }
        }
    }
    if let Some(mut out) = text_out
        && let Err(e) = out.flush()
    {
        eprintln!("write failed: {e}");
        return ExitCode::from(2);
    }
    report(&stats);
    if stats.failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn collect_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            eprintln!("cannot read directory {}", path.display());
            return;
        };
        for e in entries.flatten() {
            collect_files(&e.path(), out);
        }
    } else if matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("json" | "rtf")
    ) {
        out.push(path.to_owned());
    }
}

fn read_blobs(file: &Path) -> Result<Vec<Blob>, String> {
    let bytes = fs::read(file).map_err(|e| e.to_string())?;
    if file.extension().is_some_and(|e| e == "rtf") {
        return Ok(vec![Blob {
            file: file.to_owned(),
            pointer: String::new(),
            data: Ok(bytes),
        }]);
    }
    let json: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let mut blobs = Vec::new();
    find_rtf_data(&json, &mut String::new(), file, &mut blobs);
    Ok(blobs)
}

fn find_rtf_data(v: &Value, pointer: &mut String, file: &Path, out: &mut Vec<Blob>) {
    let len = pointer.len();
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                pointer.push('/');
                pointer.push_str(k);
                if k == "rtfData" {
                    let data = match child.as_str() {
                        Some(s) => base64::engine::general_purpose::STANDARD
                            .decode(s)
                            .map_err(|e| e.to_string()),
                        None => Err(format!("not a string: {child}")),
                    };
                    out.push(Blob {
                        file: file.to_owned(),
                        pointer: pointer.clone(),
                        data,
                    });
                } else {
                    find_rtf_data(child, pointer, file, out);
                }
                pointer.truncate(len);
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                pointer.push_str(&format!("/{i}"));
                find_rtf_data(child, pointer, file, out);
                pointer.truncate(len);
            }
        }
        _ => {}
    }
}

fn bump<K: Ord>(map: &mut BTreeMap<K, usize>, k: K, n: usize) {
    *map.entry(k).or_default() += n;
}

fn record(s: &mut Stats, doc: &Document, diag: &Diagnostics) {
    if doc.is_empty() {
        s.empty_text += 1;
    }
    s.paragraphs += doc.paragraphs.len();
    bump(&mut s.paragraphs_per_doc, doc.paragraphs.len(), 1);
    let mut first_style = None;
    let mut multi_style = false;
    for p in &doc.paragraphs {
        s.runs += p.runs.len();
        if p.runs.len() > 1 {
            s.multi_run_paragraphs += 1;
        }
        if p.style.list.is_some() {
            s.list_paragraphs += 1;
        }
        bump(
            &mut s.alignments,
            alignment_name(p.style.alignment).to_owned(),
            1,
        );
        bump(
            &mut s.line_spacing,
            line_spacing_name(p.style.line_spacing),
            1,
        );
        for r in &p.runs {
            let style = first_style.get_or_insert(&r.style);
            multi_style |= **style != r.style;
            for c in r.text.chars() {
                s.chars += 1;
                if !c.is_ascii() {
                    bump(&mut s.non_ascii_chars, c, 1);
                }
            }
            let font = r.style.font.clone().unwrap_or_else(|| "(none)".into());
            bump(&mut s.fonts, font, 1);
            bump(&mut s.scripts, format!("{:?}", r.style.script), 1);
        }
    }
    if multi_style {
        s.multi_style_docs += 1;
    }
    for (k, v) in &diag.unknown_words {
        bump(&mut s.unknown_words, k.clone(), *v);
    }
    for (k, v) in &diag.ignored_words {
        bump(&mut s.ignored_words, k.clone(), *v);
    }
    for (k, v) in &diag.unknown_symbols {
        bump(&mut s.unknown_symbols, *k, *v);
    }
    for w in &diag.warnings {
        bump(&mut s.warnings, format!("{w:?}"), 1);
    }
}

fn alignment_name(a: Alignment) -> &'static str {
    match a {
        Alignment::Natural => "natural",
        Alignment::Left => "left",
        Alignment::Center => "center",
        Alignment::Right => "right",
        Alignment::Justified => "justified",
    }
}

fn line_spacing_name(l: LineSpacing) -> String {
    match l {
        LineSpacing::Auto => "auto".into(),
        LineSpacing::Multiple(m) => format!("multiple {m}"),
        LineSpacing::AtLeast(p) => format!("at least {p}pt"),
        LineSpacing::Exactly(p) => format!("exactly {p}pt"),
    }
}

fn print_map<K: std::fmt::Debug>(title: &str, map: &BTreeMap<K, usize>) {
    println!("{title} ({} distinct):", map.len());
    let mut rows: Vec<_> = map.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    for (k, v) in rows {
        println!("  {v:>8}  {k:?}");
    }
}

fn report(s: &Stats) {
    println!("files scanned:            {}", s.files);
    println!("rtfData blobs:            {}", s.blobs);
    println!("  empty input (0 bytes):  {}", s.empty_input);
    println!("  no text (empty boxes):  {}", s.empty_text);
    println!("  with >1 resolved style: {}", s.multi_style_docs);
    println!("paragraphs:               {}", s.paragraphs);
    println!("  with >1 run:            {}", s.multi_run_paragraphs);
    println!("  in a list:              {}", s.list_paragraphs);
    println!("runs:                     {}", s.runs);
    println!("characters:               {}", s.chars);
    println!("parse failures:           {}", s.failures.len());
    println!();
    print_map("paragraphs per document", &s.paragraphs_per_doc);
    print_map("alignment (per paragraph)", &s.alignments);
    print_map("line spacing (per paragraph)", &s.line_spacing);
    print_map("script (per run)", &s.scripts);
    print_map("fonts (per run)", &s.fonts);
    print_map("non-ASCII characters", &s.non_ascii_chars);
    print_map("warnings (per blob)", &s.warnings);
    print_map("control words NOT handled (unknown)", &s.unknown_words);
    print_map("control symbols NOT handled (unknown)", &s.unknown_symbols);
    print_map("control words deliberately ignored", &s.ignored_words);
    if !s.failures.is_empty() {
        println!("\nfailures (first 50 of {}):", s.failures.len());
        for f in s.failures.iter().take(50) {
            println!("  {f}");
        }
    }
}
