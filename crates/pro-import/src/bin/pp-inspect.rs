//! Inspection tool for ProPresenter backups.
//!
//!   pp-inspect scan <backup-dir>            decode every known file, report schema coverage
//!   pp-inspect dump <file> [message-type]   print one file as JSON

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor, Value};
use walkdir::WalkDir;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pool = pro_import::descriptor_pool();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["scan", dir] => scan(&pool, Path::new(dir)),
        ["dump", file] => dump(&pool, Path::new(file), None),
        ["dump", file, ty] => dump(&pool, Path::new(file), Some(ty)),
        _ => bail!("usage: pp-inspect scan <backup-dir> | dump <file> [message-type]"),
    }
}

/// Best guess at the root message type of a file inside a ProPresenter data folder.
fn message_type_for(path: &Path) -> Option<&'static str> {
    if path.extension().is_some_and(|e| e == "pro") {
        return Some("rv.data.Presentation");
    }
    if path.extension().is_some() {
        return None; // media
    }
    let parent = path.parent()?.file_name()?.to_str()?;
    let name = path.file_name()?.to_str()?;
    Some(match (parent, name) {
        ("Playlists", _) => "rv.data.PlaylistDocument",
        (_, "Workspace") => "rv.data.ProPresenterWorkspace",
        (_, "Stage") => "rv.data.Stage.Document",
        (_, "Props") => "rv.data.PropDocument",
        (_, "Macros") => "rv.data.MacrosDocument",
        (_, "Messages") => "rv.data.MessageDocument",
        (_, "Timers") => "rv.data.TimersDocument",
        (_, "Groups") => "rv.data.ProGroupsDocument",
        (_, "ClearGroups") => "rv.data.ClearGroupsDocument",
        (_, "CCLI") => "rv.data.CCLIDocument",
        (_, "Calendar") => "rv.data.Calendar",
        (_, "KeyMappings") => "rv.data.KeyMappingDocument",
        (_, "LibraryData") => "rv.data.Library",
        (_, "Theme") => "rv.data.Template.Document",
        _ => return None,
    })
}

fn decode(pool: &DescriptorPool, path: &Path, ty: &str) -> Result<DynamicMessage> {
    let desc: MessageDescriptor = pool
        .get_message_by_name(ty)
        .with_context(|| format!("unknown message type {ty}"))?;
    let bytes = std::fs::read(path)?;
    Ok(DynamicMessage::decode(desc, bytes.as_slice())?)
}

/// Counts fields present in the file but missing from our schemas, recursively.
fn unknown_fields(msg: &DynamicMessage, path: &str, out: &mut Vec<String>) {
    for f in msg.unknown_fields() {
        out.push(format!("{path}#{}", f.number()));
    }
    for (field, value) in msg.fields() {
        let p = format!("{path}.{}", field.name());
        match value {
            Value::Message(m) => unknown_fields(m, &p, out),
            Value::List(items) => items.iter().for_each(|v| {
                if let Value::Message(m) = v {
                    unknown_fields(m, &p, out)
                }
            }),
            Value::Map(map) => map.values().for_each(|v| {
                if let Value::Message(m) = v {
                    unknown_fields(m, &p, out)
                }
            }),
            _ => {}
        }
    }
}

fn scan(pool: &DescriptorPool, dir: &Path) -> Result<()> {
    let (mut ok, mut failed, mut skipped) = (0, 0, 0);
    let mut unknown: std::collections::BTreeMap<String, usize> = Default::default();
    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        // Skip media/theme asset folders; their extension-less files aren't protobuf documents.
        let rel = path.strip_prefix(dir).unwrap_or(path);
        if rel.starts_with("Media") {
            skipped += 1;
            continue;
        }
        let Some(ty) = message_type_for(path) else {
            skipped += 1;
            continue;
        };
        match decode(pool, path, ty) {
            Ok(msg) => {
                ok += 1;
                let mut u = Vec::new();
                unknown_fields(&msg, ty, &mut u);
                for k in u {
                    *unknown.entry(k).or_default() += 1;
                }
            }
            Err(e) => {
                failed += 1;
                println!("FAIL {} ({ty}): {e:#}", rel.display());
            }
        }
    }
    println!("\ndecoded {ok}, failed {failed}, skipped (media/unknown) {skipped}");
    if unknown.is_empty() {
        println!("no unknown fields: schemas fully cover this backup");
    } else {
        println!("fields missing from schemas (path#field-number: occurrences):");
        for (k, n) in unknown {
            println!("  {k}: {n}");
        }
    }
    Ok(())
}

fn dump(pool: &DescriptorPool, file: &Path, ty: Option<&str>) -> Result<()> {
    let file = PathBuf::from(file);
    let ty = match ty {
        Some(t) => t,
        None => message_type_for(&file).context("can't infer message type; pass it explicitly")?,
    };
    let msg = decode(pool, &file, ty)?;
    println!("{}", serde_json::to_string_pretty(&msg)?);
    Ok(())
}
