use std::{env, fs, path::PathBuf};

use prost::Message;

/// Where the ProPresenter 7 schemas are: `$PROPRESENTER_PROTO_DIR`, else the workspace's
/// `target/propresenter-proto/`, where `scripts/fetch-propresenter-proto.sh` puts them (they
/// are not in the repository: see `PROTO.md`).
fn proto_dir() -> anyhow::Result<PathBuf> {
    if let Some(dir) = env::var_os("PROPRESENTER_PROTO_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    // `crates/pro-import` → the workspace's folder (no `..` in the path protox is given).
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let workspace = manifest
        .ancestors()
        .nth(2)
        .ok_or_else(|| anyhow::anyhow!("no workspace folder above {}", manifest.display()))?;
    Ok(workspace.join("target").join("propresenter-proto"))
}

fn main() -> anyhow::Result<()> {
    println!("cargo:rerun-if-env-changed=PROPRESENTER_PROTO_DIR");
    let proto_dir = proto_dir()?;
    println!("cargo:rerun-if-changed={}", proto_dir.display());

    let mut files: Vec<PathBuf> = fs::read_dir(&proto_dir)
        .map(|d| {
            d.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|ext| ext == "proto"))
                .collect()
        })
        .unwrap_or_default();
    if !files.iter().any(|p| p.ends_with("presentation.proto")) {
        anyhow::bail!(
            "the ProPresenter 7 schemas are missing in {}: run scripts/fetch-propresenter-proto.sh \
             first (it downloads and checks them; see crates/pro-import/PROTO.md), or set \
             PROPRESENTER_PROTO_DIR to a folder holding them",
            proto_dir.display()
        );
    }
    files.sort();

    // protox is a pure-Rust protobuf compiler, so no system `protoc` is required.
    let fds = protox::compile(&files, [&proto_dir])?;

    // Raw descriptor set, used at runtime by prost-reflect to dump any message as JSON.
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    fs::write(out_dir.join("propresenter.fds"), fds.encode_to_vec())?;

    // `enable_type_names` implements `prost::Name` for every message, so the importer can look up
    // a message's descriptor by its full name and dump any sub-message as JSON (`import.unmapped`,
    // `Unsupported` payloads) through prost-reflect.
    prost_build::Config::new()
        .enable_type_names()
        .include_file("_includes.rs")
        .compile_fds(fds)?;
    Ok(())
}
