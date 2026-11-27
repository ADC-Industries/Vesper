//! Migration of a standalone install (the old layout: `portable/<kind>`, `media/`) into its own
//! Codex, on synthetic fixtures.

use crate::common::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The collection kinds the fixture writes.
const MOVED: &[&str] = &[
    "library",
    "presentation",
    "playlistNode",
    "theme",
    "mediaAsset",
    "clearGroup",
];

fn read(path: &Path) -> Value {
    serde_json::from_slice(
        &std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())),
    )
    .unwrap()
}

/// The single collection folder of a migrated data folder.
fn collection_dir(data: &Path) -> PathBuf {
    let dirs: Vec<PathBuf> = std::fs::read_dir(data.join("codex/collections"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(dirs.len(), 1, "one collection: {dirs:?}");
    dirs[0].clone()
}

/// Every file below `dir`, relative, with its bytes.
fn tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap().display().to_string();
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// What the API serves of the fixture's content.
async fn served(addr: std::net::SocketAddr, fx: &Fixture) -> Value {
    let song = fx.song.id.0.to_string();
    let slide0 = fx.song.sections[0].slides[0].id.0.to_string();
    let thumb = http(
        addr,
        "GET",
        &format!("/api/v1/presentations/{song}/slides/{slide0}/thumbnail?w=160"),
        &[],
        None,
    )
    .await;
    assert_eq!(
        thumb.status,
        200,
        "{}",
        String::from_utf8_lossy(&thumb.body)
    );
    assert_eq!(&thumb.body[..2], &[0xFF, 0xD8], "a JPEG thumbnail");
    let file = http(
        addr,
        "GET",
        &format!("/api/v1/media/{}/file", fx.asset.id.0),
        &[],
        None,
    )
    .await;
    assert_eq!(file.status, 200);
    let media = get_json(addr, &format!("/api/v1/media/{}", fx.asset.id.0)).await;
    let mut names: Vec<String> = get_json(addr, "/api/v1/presentations")
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap_or_default().to_string())
        .collect();
    names.sort();
    json!({
        "presentations": names,
        "song": get_json(addr, &format!("/api/v1/presentations/{song}")).await,
        "libraries": get_json(addr, "/api/v1/libraries").await,
        "playlists": get_json(addr, "/api/v1/playlists").await,
        "clearGroups": get_json(addr, "/api/v1/clear-groups").await,
        "themes": get_json(addr, "/api/v1/themes").await,
        "mediaOnline": media["online"],
        "mediaBytes": file.body.len(),
        "mediaEtag": file.header("etag"),
        "thumbnailEtag": thumb.header("etag"),
        "loadIssues": get_json(addr, "/api/v1/diagnostics").await["loadIssues"]
            .as_array()
            .unwrap()
            .len(),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_old_layout_migrates_and_serves_the_same_content() {
    let fx = fixture("codex-migrate");
    let media_before = std::fs::read(fx.dir.join("media/bg0001.png")).unwrap();
    let future_before = std::fs::read(fx.dir.join("portable/presentation/future.json")).unwrap();
    // The old layout as the server loaded it before the Codex (read only).
    let legacy = vesper_server::data::ShowData::load(&fx.dir).unwrap();
    let legacy_paths: Vec<_> = legacy
        .content
        .assets
        .keys()
        .map(|id| (*id, legacy.content.asset_path(*id).unwrap()))
        .collect();

    let server = start_server(&fx, true, false).await;
    let after = served(server.addr, &fx).await;
    let migrated = server.shared.data();

    // Every document loads with the same fingerprint (the media assets' locations aside: their
    // files are in the media library now), every media file resolves to the same bytes.
    let others = |h: vesper_server::data::DocHashes| -> vesper_server::data::DocHashes {
        h.into_iter()
            .filter(|(k, _)| k.0 != model::DocumentKind::MediaAsset)
            .collect()
    };
    assert_eq!(others(migrated.doc_hashes()), others(legacy.doc_hashes()));
    assert_eq!(migrated.report.issues.len(), legacy.report.issues.len());
    assert_eq!(migrated.content.assets.len(), legacy.content.assets.len());
    for (id, old) in legacy_paths {
        let new = migrated.content.asset_path(id).unwrap();
        assert_ne!(old, new, "the file moved");
        assert!(
            new.starts_with(fx.dir.join("codex/library")),
            "{}",
            new.display()
        );
        assert_eq!(
            new.file_name().unwrap(),
            "Blue background.png",
            "named after its asset"
        );
        assert_eq!(std::fs::read(new).unwrap(), media_before);
        let a = &migrated.content.assets[&id];
        match &a.location {
            model::AssetLocation::Library { id, sha256 } => {
                assert_eq!(id.len(), 16);
                assert_eq!(*sha256, crate::codex::media::sha_hex(&media_before));
            }
            l => panic!("a library location: {l:?}"),
        }
    }
    server.shutdown().await.unwrap();

    // The layout.
    let d = &fx.dir;
    let col = collection_dir(d);
    let id = col.file_name().unwrap().to_str().unwrap().to_string();
    assert_eq!(
        read(&d.join("codex/codex.json"))["schemaVersion"],
        model::SCHEMA_VERSION
    );
    let migration = read(&d.join("codex/migration.json"));
    assert_eq!(migration["state"], "done");
    assert_eq!(migration["collection"], id);
    let collection = read(&col.join("collection.json"));
    assert_eq!(collection["id"], id);
    assert!(
        !collection["name"].as_str().unwrap().is_empty(),
        "named after the server"
    );
    let config = read(&d.join("machine/codex.json"));
    assert_eq!(config["role"], "host");
    assert_eq!(config["collection"], id);
    for kind in MOVED {
        assert!(
            !d.join("portable").join(kind).exists(),
            "{kind} left the old layout"
        );
        assert!(col.join("portable").join(kind).is_dir(), "{kind} moved");
    }
    for kind in ["screen", "look"] {
        assert!(
            d.join("portable").join(kind).is_dir(),
            "{kind} stays with the box"
        );
        assert!(!col.join("portable").join(kind).exists());
    }
    assert!(!d.join("media").exists(), "media moved");
    assert!(
        !d.join("codex/media/bg0001.png").exists(),
        "into the media library"
    );
    assert_eq!(
        read(&d.join("codex/library-migration.json"))["state"],
        "done"
    );
    assert!(!d.join("codex/library-import").exists());
    assert_eq!(
        std::fs::read(col.join("portable/presentation/future.json")).unwrap(),
        future_before,
        "a too-new document moves unchanged"
    );

    // The content served.
    assert_eq!(
        after["presentations"],
        json!(["Second Song", "Test Song"]),
        "{after}"
    );
    assert_eq!(after["song"]["name"], "Test Song");
    assert_eq!(after["mediaOnline"], true);
    assert_eq!(after["mediaBytes"], media_before.len());
    assert_eq!(after["clearGroups"][0]["name"], "Clear Slide");
    assert_eq!(
        after["loadIssues"], 2,
        "the broken and the too-new documents"
    );
    std::fs::remove_dir_all(&fx.dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_start_is_idempotent() {
    let fx = fixture("codex-twice");
    let server = start_server(&fx, true, false).await;
    let first = served(server.addr, &fx).await;
    server.shutdown().await.unwrap();
    // (The library's own state aside: IDN keeps probing and hashing in the background.)
    let codex_tree = || -> Vec<(String, Vec<u8>)> {
        tree(&fx.dir.join("codex"))
            .into_iter()
            .filter(|(p, _)| !p.starts_with("library/.library"))
            .collect()
    };
    let codex_before = codex_tree();
    let config_before = std::fs::read(fx.dir.join("machine/codex.json")).unwrap();

    let server = start_server(&fx, true, false).await;
    let second = served(server.addr, &fx).await;
    server.shutdown().await.unwrap();

    assert_eq!(first, second, "the same content, revisions and ETags");
    assert_eq!(codex_tree(), codex_before, "nothing in the Codex changed");
    assert_eq!(
        std::fs::read(fx.dir.join("machine/codex.json")).unwrap(),
        config_before
    );
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// A crash in the middle of a migration: `migration.json` says `moving`, some kinds are in the
/// collection already, one is half moved, media not yet. The next start finishes the job into
/// the same collection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_interrupted_migration_resumes() {
    let fx = fixture("codex-resume");
    let d = &fx.dir;
    let id = uuid::Uuid::new_v4().to_string();
    let col = d.join("codex/collections").join(&id);
    std::fs::create_dir_all(col.join("portable")).unwrap();
    std::fs::write(
        d.join("codex/migration.json"),
        serde_json::to_vec(&json!({
            "schemaVersion": 2, "state": "moving", "collection": id,
            "startedAt": "2026-11-20T10:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();
    // Fully moved: presentations and themes.
    for kind in ["presentation", "theme"] {
        std::fs::rename(
            d.join("portable").join(kind),
            col.join("portable").join(kind),
        )
        .unwrap();
    }
    // Half moved: one of the playlist nodes.
    let nodes = d.join("portable/playlistNode");
    let first = std::fs::read_dir(&nodes)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .min()
        .unwrap();
    std::fs::create_dir_all(col.join("portable/playlistNode")).unwrap();
    std::fs::rename(
        &first,
        col.join("portable/playlistNode")
            .join(first.file_name().unwrap()),
    )
    .unwrap();
    assert!(nodes.is_dir(), "the other node is still in the old layout");

    let server = start_server(&fx, true, false).await;
    let after = served(server.addr, &fx).await;
    server.shutdown().await.unwrap();

    assert_eq!(collection_dir(d), col, "the same collection");
    let migration = read(&d.join("codex/migration.json"));
    assert_eq!(migration["state"], "done");
    assert_eq!(migration["startedAt"], "2026-11-20T10:00:00Z");
    assert!(col.join("collection.json").is_file());
    assert_eq!(read(&d.join("machine/codex.json"))["collection"], id);
    for kind in MOVED {
        assert!(!d.join("portable").join(kind).exists(), "{kind} moved");
        assert!(col.join("portable").join(kind).is_dir(), "{kind} moved");
    }
    assert_eq!(
        std::fs::read_dir(col.join("portable/playlistNode"))
            .unwrap()
            .count(),
        2,
        "both playlist nodes"
    );
    assert!(!d.join("codex/media/bg0001.png").exists(), "in the library");
    assert_eq!(after["presentations"], json!(["Second Song", "Test Song"]));
    assert!(
        after["playlists"].to_string().contains("Sunday"),
        "{}",
        after["playlists"]
    );
    assert_eq!(after["mediaOnline"], true);
    assert_eq!(after["loadIssues"], 2);
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// Documents written by a newer schema are never overwritten: an old-layout document whose ID
/// has a too-new copy in the collection stays where it is (and is reported), and a too-new
/// `machine/codex.json` is left as it is.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn too_new_documents_are_left_untouched() {
    let fx = fixture("codex-too-new");
    let server = start_server(&fx, true, false).await;
    server.shutdown().await.unwrap();
    let d = &fx.dir;
    let col = collection_dir(d);

    // A newer server rewrote the second song in the collection; old-layout output (an import)
    // brings an older copy of it, and a new presentation.
    let song_id = fx.second.id.0.to_string();
    let in_codex = col.join(format!("portable/presentation/{song_id}.json"));
    let mut newer = read(&in_codex);
    newer["schemaVersion"] = json!(999);
    newer["name"] = json!("Second Song (from a newer server)");
    let newer_bytes = serde_json::to_vec_pretty(&newer).unwrap();
    std::fs::write(&in_codex, &newer_bytes).unwrap();
    let mut older = fx.second.clone();
    older.name = "Second Song (re-imported)".into();
    write_doc(d, "portable", "presentation", &song_id, &older);
    let extra = model::Presentation::new("Third Song");
    write_doc(
        d,
        "portable",
        "presentation",
        &extra.id.0.to_string(),
        &extra,
    );
    // A newer server's machine/codex.json.
    let mut config = read(&d.join("machine/codex.json"));
    config["schemaVersion"] = json!(999);
    let config_bytes = serde_json::to_vec_pretty(&config).unwrap();
    std::fs::write(d.join("machine/codex.json"), &config_bytes).unwrap();

    let server = start_server(&fx, true, false).await;
    let after = served(server.addr, &fx).await;
    let diag = get_json(server.addr, "/api/v1/diagnostics").await;
    server.shutdown().await.unwrap();

    assert_eq!(
        std::fs::read(&in_codex).unwrap(),
        newer_bytes,
        "not overwritten"
    );
    let left = d.join(format!("portable/presentation/{song_id}.json"));
    assert!(left.is_file(), "the older copy stays in the old layout");
    assert_eq!(read(&left)["name"], "Second Song (re-imported)");
    assert!(
        col.join(format!("portable/presentation/{}.json", extra.id.0))
            .is_file(),
        "the new presentation moved in"
    );
    assert_eq!(
        std::fs::read(d.join("machine/codex.json")).unwrap(),
        config_bytes,
        "a too-new machine/codex.json is not rewritten"
    );
    assert_eq!(
        after["presentations"],
        json!(["Test Song", "Third Song"]),
        "{after}"
    );
    let issues = diag["loadIssues"].as_array().unwrap();
    let file_of = |i: &Value| i["file"].as_str().unwrap_or_default().to_string();
    assert!(
        issues
            .iter()
            .any(|i| file_of(i).starts_with("portable/presentation/") && i["tooNew"] == true),
        "the document left behind is reported: {diag}"
    );
    assert!(
        issues
            .iter()
            .any(|i| file_of(i) == "machine/codex.json" && i["tooNew"] == true),
        "{diag}"
    );
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// A media asset document with a managed location, in the old layout or a collection.
fn managed_asset(dir: &Path, name: &str, sha: &str, ext: &str, origin: Option<&str>) -> String {
    let a = model::MediaAsset {
        name: name.into(),
        location: model::AssetLocation::Managed {
            sha256: sha.into(),
            extension: ext.into(),
        },
        origin: origin.map(|p| model::MediaOrigin {
            relative_path: Some(p.into()),
            ..model::MediaOrigin::default()
        }),
        ..model::MediaAsset::default()
    };
    let id = a.id.0.to_string();
    write_doc(dir, "portable", "mediaAsset", &id, &a);
    id
}

fn png_bytes(seed: u8) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(8, 8, image::Rgb([seed, 40, 200]))
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

/// The old managed store moves into the media library: old-layout media that show up later,
/// a blob an interrupted run had staged, a picture ending the library takes since IDN Beta3
/// (`.tga`), a document of a newer schema (never rewritten) and an item trash entry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_old_media_store_moves_into_the_library_resumably() {
    use crate::codex::media::sha_hex;
    let fx = fixture("codex-library-migrate");
    let server = start_server(&fx, true, false).await;
    server.shutdown().await.unwrap();
    let d = &fx.dir;
    let col = collection_dir(d);
    let col_id: model::CollectionId = col.file_name().unwrap().to_str().unwrap().parse().unwrap();
    let col_name = read(&col.join("collection.json"))["name"]
        .as_str()
        .unwrap()
        .to_string();

    // Old-layout media that show up later (an import): moved into the Codex, then the library.
    let welcome = png_bytes(1);
    let sw = sha_hex(&welcome);
    std::fs::create_dir_all(d.join("media")).unwrap();
    std::fs::write(d.join("media").join(format!("{sw}.png")), &welcome).unwrap();
    let a_welcome = managed_asset(d, "Welcome", &sw, "png", Some("Stage/Welcome.png"));
    // Staged by a run that stopped.
    let hymn = png_bytes(2);
    let sh_ = sha_hex(&hymn);
    let staged = d.join(format!("codex/library-import/{sh_}.png"));
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::write(staged.join("Hymn.png"), &hymn).unwrap();
    let a_hymn = managed_asset(&col, "Hymn", &sh_, "png", None);
    std::fs::write(
        d.join("codex/library-migration.json"),
        serde_json::to_vec(&json!({"schemaVersion": 2, "state": "moving", "imported": {}}))
            .unwrap(),
    )
    .unwrap();
    // A Targa picture (taken since IDN Beta3).
    let tga = b"not really a targa".to_vec();
    let st = sha_hex(&tga);
    std::fs::write(d.join(format!("codex/media/{st}.tga")), &tga).unwrap();
    let a_tga = managed_asset(&col, "Old", &st, "tga", None);
    // A newer server's document.
    let newer = png_bytes(3);
    let sn = sha_hex(&newer);
    std::fs::write(d.join(format!("codex/media/{sn}.png")), &newer).unwrap();
    let a_newer = managed_asset(&col, "Newer", &sn, "png", None);
    let newer_path = col.join(format!("portable/mediaAsset/{a_newer}.json"));
    let mut v = read(&newer_path);
    v["schemaVersion"] = json!(999);
    let newer_doc = serde_json::to_vec_pretty(&v).unwrap();
    std::fs::write(&newer_path, &newer_doc).unwrap();
    // Names that aren't plain file names, with no original path (R2-5): made plain, kept in
    // the staging folder.
    let slash = png_bytes(5);
    let s5 = sha_hex(&slash);
    std::fs::write(d.join(format!("codex/media/{s5}.png")), &slash).unwrap();
    let a_slash = managed_asset(&col, "Intro/Outro", &s5, "png", None);
    let up = png_bytes(6);
    let s6 = sha_hex(&up);
    std::fs::write(d.join(format!("codex/media/{s6}.png")), &up).unwrap();
    let a_up = managed_asset(&col, "../../escape", &s6, "png", None);
    // A deleted media asset in the item trash.
    let trashed = png_bytes(4);
    let sx = sha_hex(&trashed);
    std::fs::write(d.join(format!("codex/media/{sx}.png")), &trashed).unwrap();
    let doc = model::MediaAsset {
        name: "Deleted".into(),
        location: model::AssetLocation::Managed {
            sha256: sx.clone(),
            extension: "png".into(),
        },
        ..model::MediaAsset::default()
    };
    let entry = model::TrashEntry {
        schema_version: model::SCHEMA_VERSION,
        id: uuid::Uuid::new_v4(),
        collection: col_id,
        kind: model::DocumentKind::MediaAsset,
        doc_id: doc.id.0,
        name: "Deleted".into(),
        deleted_at: "2026-11-22T10:00:00Z".into(),
        expires_at: "2099-01-01T00:00:00Z".into(),
        by: model::ChangeBy::default(),
        document: Some(serde_json::to_value(&doc).unwrap()),
        subtree: Vec::new(),
        subtree_count: 0,
    };
    let entry_path = d.join(format!("codex/trash/{}.json", entry.id));
    std::fs::create_dir_all(entry_path.parent().unwrap()).unwrap();
    std::fs::write(&entry_path, serde_json::to_vec(&entry).unwrap()).unwrap();
    let mut line = entry.clone();
    line.document = None;
    let mut index = std::fs::read(d.join("codex/trash.jsonl")).unwrap_or_default();
    index.extend(serde_json::to_vec(&line).unwrap());
    index.push(b'\n');
    std::fs::write(d.join("codex/trash.jsonl"), index).unwrap();

    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let lib = server.shared.library.clone();
    let location =
        |id: &str| read(&col.join(format!("portable/mediaAsset/{id}.json")))["location"].clone();
    let library_file = |loc: &Value| lib.resolve(loc["id"].as_str().unwrap()).unwrap();

    let l = location(&a_welcome);
    assert_eq!(l["type"], "library", "{l}");
    assert_eq!(l["sha256"], sw);
    assert_eq!(
        library_file(&l),
        d.join("codex/library").join(&col_name).join("Welcome.png"),
        "its original name, in the collection's folder"
    );
    let l = location(&a_hymn);
    assert_eq!(l["type"], "library", "{l}");
    assert_eq!(std::fs::read(library_file(&l)).unwrap(), hymn);
    assert_eq!(library_file(&l).file_name().unwrap(), "Hymn.png");
    // The Targa picture, under its asset's name; written as schema 3 (a library location).
    let l = location(&a_tga);
    assert_eq!(l["type"], "library", "{l}");
    assert_eq!(library_file(&l).file_name().unwrap(), "Old.tga");
    assert_eq!(
        read(&col.join(format!("portable/mediaAsset/{a_tga}.json")))["schemaVersion"],
        3
    );
    // Names made plain file names, in the collection's folder.
    for (id, file) in [(&a_slash, "Intro_Outro.png"), (&a_up, "_.._escape.png")] {
        let l = location(id);
        assert_eq!(l["type"], "library", "{l}");
        assert_eq!(
            library_file(&l),
            d.join("codex/library").join(&col_name).join(file)
        );
    }
    // The newer document: not rewritten (its file is in the library all the same).
    assert_eq!(std::fs::read(&newer_path).unwrap(), newer_doc);
    assert!(lib.by_sha(&sn).is_some());
    assert!(!d.join(format!("codex/media/{sn}.png")).exists());
    let issues = get_json(addr, "/api/v1/diagnostics").await["loadIssues"].clone();
    assert!(
        issues
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["file"].as_str().unwrap_or_default().contains(&a_newer)),
        "{issues}"
    );
    // The trash entry's document.
    let e = read(&entry_path);
    assert_eq!(e["document"]["location"]["type"], "library", "{e}");
    assert_eq!(e["document"]["location"]["sha256"], sx);
    // Every blob taken in, nothing staged; but the newer document isn't rewritten yet, so the
    // state stays `moving` and the next start tries it again (R2-2).
    let state = read(&d.join("codex/library-migration.json"));
    assert_eq!(state["state"], "moving", "{state}");
    assert_eq!(state["imported"].as_object().unwrap().len(), 7, "{state}");
    assert!(
        std::fs::read_dir(d.join("codex/library-import"))
            .map(|r| r.count() == 0)
            .unwrap_or(true)
    );
    assert!(!d.join("media").exists());
    let blobs: Vec<String> = std::fs::read_dir(d.join("codex/media"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(blobs.is_empty(), "{blobs:?}");
    assert_eq!(e["schemaVersion"], 3, "the trash entry too");
    server.shutdown().await.unwrap();

    // The server is updated (the newer document reads now); and a run whose state was lost
    // had taken a blob in (R2-1): the state doesn't name it, nothing is staged or left in the
    // old store, a document still names it as managed. Found again by its SHA-256.
    let mut v = read(&newer_path);
    v["schemaVersion"] = json!(2);
    std::fs::write(&newer_path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let state_path = d.join("codex/library-migration.json");
    let mut state = read(&state_path);
    let welcome_item = state["imported"][format!("{sw}.png")].clone();
    state["imported"]
        .as_object_mut()
        .unwrap()
        .remove(&format!("{sw}.png"));
    std::fs::write(&state_path, serde_json::to_vec(&state).unwrap()).unwrap();
    let a_again = managed_asset(&col, "Welcome again", &sw, "png", None);
    let server = start_server(&fx, true, false).await;
    let l = location(&a_newer);
    assert_eq!(
        l["type"], "library",
        "the newer document, once it reads: {l}"
    );
    assert_eq!(l["sha256"], sn);
    let l = location(&a_again);
    assert_eq!(l["type"], "library", "{l}");
    assert_eq!(l["id"], welcome_item["id"], "{l}");
    let state = read(&state_path);
    assert_eq!(state["state"], "done", "{state}");
    assert!(!d.join("codex/library-import").exists());
    // The rewrites are in the collection's change log (show boxes fetch them; R2-4). They are
    // logged one by one: wait for both (not just the first), however slow the machine.
    let mut noted = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        let log = get_json(
            server.addr,
            &format!("/api/v1/codex/collections/{col_id}/changes?since=0"),
        )
        .await;
        noted = log["changes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["by"]["client"] == "migration")
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect();
        if noted.contains(&a_again) && noted.contains(&a_newer) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(
        noted.contains(&a_again) && noted.contains(&a_newer),
        "{noted:?}"
    );
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// A library whose index can't be read: the Codex says so, its media are missing, uploads
/// can't finish (`503`), and nothing in the library is touched. Its index back, all of it is
/// there again with the same IDs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_library_that_cannot_be_opened_is_reported_and_left_alone() {
    use crate::codex::media::{create_upload, finish, put_chunk, sha_hex};
    let fx = fixture("codex-library-broken");
    let server = start_server(&fx, true, false).await;
    let before = server.shared.data().content.assets[&fx.asset.id]
        .location
        .clone();
    let st = get_json(server.addr, "/api/v1/codex").await;
    assert_eq!(
        st["mediaLibrary"],
        json!({"available": true, "served": false}),
        "{st}"
    );
    server.shutdown().await.unwrap();
    let index = fx.dir.join("codex/library/.library/index.json");
    let good = std::fs::read(&index).unwrap();
    std::fs::write(&index, b"{ not json").unwrap();
    let lib_tree = || -> Vec<(String, Vec<u8>)> { tree(&fx.dir.join("codex/library")) };
    let untouched = lib_tree();

    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let st = get_json(addr, "/api/v1/codex").await;
    assert_eq!(st["mediaLibrary"]["available"], false, "{st}");
    assert!(
        st["mediaLibrary"]["problem"]
            .as_str()
            .unwrap()
            .contains("can't be opened"),
        "{st}"
    );
    let a = get_json(addr, &format!("/api/v1/media/{}", fx.asset.id.0)).await;
    assert_eq!(a["online"], false, "{a}");
    assert_eq!(
        server.shared.data().content.assets[&fx.asset.id].location,
        before,
        "the document is unchanged"
    );
    let main = st["collection"].as_str().unwrap().to_string();
    let bytes = png_bytes(9);
    let r = create_upload(
        addr,
        json!({"collection": main, "name": "Later.png", "size": bytes.len(), "sha256": sha_hex(&bytes)}),
    )
    .await;
    assert_eq!(r.status, 201);
    let id = r.json()["id"].as_str().unwrap().to_string();
    assert_eq!(
        put_chunk(addr, &id, &bytes, 0, bytes.len() as u64)
            .await
            .status,
        200
    );
    let r = finish(addr, &id, None).await;
    assert_eq!(r.status, 503, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["error"]["code"], "unavailable");
    assert_eq!(
        get_json(addr, &format!("/api/v1/codex/uploads/{id}")).await["offset"],
        bytes.len(),
        "the upload stays"
    );
    server.shutdown().await.unwrap();
    assert_eq!(lib_tree(), untouched, "nothing in the library changed");

    // The index back: the library opens with the same items.
    std::fs::write(&index, &good).unwrap();
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let st = get_json(addr, "/api/v1/codex").await;
    assert_eq!(st["mediaLibrary"]["available"], true, "{st}");
    let a = get_json(addr, &format!("/api/v1/media/{}", fx.asset.id.0)).await;
    assert_eq!(a["online"], true, "{a}");
    let r = finish(addr, &id, None).await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// Media an earlier version left in the old store because IDN didn't take their ending then
/// (its migration said `done`) move into the library at the next start; their documents are
/// written as schema 3, others stay 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_left_managed_move_in_once_the_library_takes_them() {
    use crate::codex::media::sha_hex;
    let fx = fixture("codex-library-endings");
    let server = start_server(&fx, true, false).await;
    server.shutdown().await.unwrap();
    let d = &fx.dir;
    let col = collection_dir(d);
    let mut left = Vec::new();
    for (name, ext, bytes) in [
        ("Title card", "tga", b"targa bytes".to_vec()),
        ("Archive clip", "mpg", b"mpeg bytes".to_vec()),
        ("Walk in", "jpe", png_bytes(7)),
    ] {
        let sha = sha_hex(&bytes);
        std::fs::write(d.join(format!("codex/media/{sha}.{ext}")), &bytes).unwrap();
        left.push((
            managed_asset(&col, name, &sha, ext, None),
            format!("{name}.{ext}"),
            bytes,
        ));
    }
    let mut st = read(&d.join("codex/library-migration.json"));
    assert_eq!(st["state"], "done");
    st["finishedAt"] = json!("2026-11-22T09:00:00Z");
    std::fs::write(
        d.join("codex/library-migration.json"),
        serde_json::to_vec(&st).unwrap(),
    )
    .unwrap();

    let server = start_server(&fx, true, false).await;
    for (id, file, bytes) in &left {
        let doc = read(&col.join(format!("portable/mediaAsset/{id}.json")));
        assert_eq!(doc["location"]["type"], "library", "{doc}");
        assert_eq!(doc["schemaVersion"], 3);
        let p = server
            .shared
            .library
            .resolve(doc["location"]["id"].as_str().unwrap())
            .unwrap();
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), file);
        assert_eq!(&std::fs::read(&p).unwrap(), bytes);
    }
    assert_eq!(
        read(&d.join("codex/library-migration.json"))["state"],
        "done"
    );
    assert_eq!(
        std::fs::read_dir(d.join("codex/media")).unwrap().count(),
        0,
        "nothing left in the old store"
    );
    // A document without a library location stays schema 2.
    let song = read(&col.join(format!("portable/presentation/{}.json", fx.song.id.0)));
    assert_eq!(song["schemaVersion"], 2);
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}
