//! Codex missing media and relinking: availability worked out on request, the search under
//! the media roots bound on this box (root binding, then file name, then content hash, with
//! ambiguous and not-found cases), a dry run, stored statuses written only when they change,
//! manual relinks (a library item, a file by its SHA-256, a file under a root, an upload
//! replacing the file), relinks by content hash in the media library, the client
//! folder match and bulk re-roots. Synthetic files only.

use super::media::{noise_png, probed, sha_hex, upload_replacing};
use crate::common::*;
use model::{AssetLocation, MediaAsset, MediaOrigin, MediaRoot, MediaRootBinding, MediaRootId};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::Path;

fn ok(r: &Resp, status: u16, what: &str) -> Value {
    assert_eq!(
        r.status,
        status,
        "{what}: {}",
        String::from_utf8_lossy(&r.body)
    );
    r.json()
}

async fn post(addr: SocketAddr, path: &str, body: Value) -> Resp {
    http(addr, "POST", path, &[], Some(body)).await
}

fn put_file(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn root(fx: &Fixture, name: &str) -> MediaRootId {
    let r = MediaRoot {
        name: name.into(),
        ..MediaRoot::default()
    };
    write_doc(&fx.dir, "portable", "mediaRoot", &r.id.0.to_string(), &r);
    let b = MediaRootBinding {
        root: r.id,
        local_path: fx.dir.join("roots").join(name).display().to_string(),
        ..MediaRootBinding::default()
    };
    write_doc(
        &fx.dir,
        "machine",
        "mediaRootBinding",
        &r.id.0.to_string(),
        &b,
    );
    std::fs::create_dir_all(fx.dir.join("roots").join(name)).unwrap();
    r.id
}

fn asset(fx: &Fixture, name: &str, location: AssetLocation, origin: Option<MediaOrigin>) -> String {
    let a = MediaAsset {
        name: name.into(),
        location,
        origin,
        ..MediaAsset::default()
    };
    let id = a.id.0.to_string();
    write_doc(&fx.dir, "portable", "mediaAsset", &id, &a);
    id
}

fn relative(p: &str) -> Option<MediaOrigin> {
    Some(MediaOrigin {
        relative_path: Some(p.into()),
        ..MediaOrigin::default()
    })
}

fn hint(p: &str) -> Option<MediaOrigin> {
    Some(MediaOrigin {
        absolute_hint: Some(p.into()),
        ..MediaOrigin::default()
    })
}

fn ids(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            x.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| x["asset"].as_str().unwrap().to_string())
        })
        .collect();
    out.sort();
    out
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_media_are_found_and_relinked() {
    let fx = fixture("codex-relink");
    let shared = root(&fx, "Shared");
    let archive = root(&fx, "Archive");
    let unbound = MediaRootId::new();
    let roots = fx.dir.join("roots");
    let png = |seed| noise_png(8, 8, seed);

    // 1. Linked under a root that isn't bound here; the same path is under Shared.
    let lost = asset(
        &fx,
        "Sunrise",
        AssetLocation::Linked {
            root: unbound,
            path: "Backgrounds/Sunrise.png".into(),
        },
        None,
    );
    put_file(&roots.join("Shared/Backgrounds/Sunrise.png"), &png(1));
    // 2. Its folder was renamed: the name is found once, in Archive.
    let renamed = asset(
        &fx,
        "Loop",
        AssetLocation::Linked {
            root: shared,
            path: "Old/Loop.png".into(),
        },
        None,
    );
    put_file(&roots.join("Archive/Loops/Loop.png"), &png(2));
    // 3. Two files with its name.
    let twice = asset(
        &fx,
        "Twice",
        AssetLocation::Unresolved,
        relative("Twice.png"),
    );
    put_file(&roots.join("Shared/a/Twice.png"), &png(3));
    put_file(&roots.join("Archive/b/Twice.png"), &png(4));
    // 4. A managed file gone from the Codex; the same bytes under another name in Archive.
    let bytes = png(5);
    let managed = asset(
        &fx,
        "Kept",
        AssetLocation::Managed {
            sha256: sha_hex(&bytes),
            extension: "png".into(),
        },
        relative("Kept.png"),
    );
    put_file(&roots.join("Archive/Different.png"), &bytes);
    // 5. Nowhere.
    let nowhere = asset(
        &fx,
        "Nowhere",
        AssetLocation::Unresolved,
        relative("Nowhere.png"),
    );
    // 6. Moved with its folder: re-rooted.
    let moved = asset(
        &fx,
        "Moved",
        AssetLocation::Linked {
            root: shared,
            path: "Old/Moved.png".into(),
        },
        None,
    );
    put_file(&roots.join("Archive/New/Moved.png"), &png(6));
    // 7. Imported with an absolute path only: re-rooted by its original path.
    let hinted = asset(
        &fx,
        "Cross",
        AssetLocation::Unresolved,
        hint("X:\\Media\\Hymns\\Cross.png"),
    );
    put_file(&roots.join("Shared/Hymns2/Hymns/Cross.png"), &png(7));
    let gone = asset(
        &fx,
        "Gone",
        AssetLocation::Unresolved,
        hint("X:\\Media\\Gone.png"),
    );
    // 8, 9. Found in a client's folder.
    let intro = asset(
        &fx,
        "Intro",
        AssetLocation::Linked {
            root: shared,
            path: "Songs/Intro.png".into(),
        },
        None,
    );
    let logo = asset(&fx, "Logo", AssetLocation::Unresolved, relative("Logo.png"));
    // 10. A library item that is gone; the same bytes are in the library (the background).
    let bg_bytes = std::fs::read(fx.dir.join("media/bg0001.png")).unwrap();
    let vanished = asset(
        &fx,
        "Vanished",
        AssetLocation::Library {
            id: "00000000deadbeef".into(),
            sha256: sha_hex(&bg_bytes),
        },
        None,
    );

    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let base = format!("/api/v1/codex/collections/{main}/media");

    // Availability, worked out now.
    let all = get_json(addr, &base).await;
    assert_eq!(all.as_array().unwrap().len(), 12, "{all}");
    let missing = get_json(addr, &format!("{base}?status=missing")).await;
    let want_missing = sorted(vec![
        lost.clone(),
        renamed.clone(),
        twice.clone(),
        managed.clone(),
        nowhere.clone(),
        moved.clone(),
        hinted.clone(),
        gone.clone(),
        intro.clone(),
        logo.clone(),
        vanished.clone(),
    ]);
    let got: Vec<String> = sorted(
        missing
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["asset"]["id"].as_str().unwrap().to_string())
            .collect(),
    );
    assert_eq!(got, want_missing);
    assert!(
        missing
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["availability"] == "missing" && m["problem"].is_string())
    );
    let online = get_json(addr, &format!("{base}?status=online")).await;
    assert_eq!(
        online.as_array().unwrap().len(),
        1,
        "the background: {online}"
    );
    assert_eq!(
        http(addr, "GET", &format!("{base}?status=lost"), &[], None)
            .await
            .status,
        400
    );

    // Bulk re-root by original path: a dry run lists both, the real one moves the one whose
    // file exists.
    let reroot = format!("{base}/reroot");
    let body =
        json!({"from": {"prefix": "X:\\Media"}, "to": {"root": shared.0, "prefix": "Hymns2"}});
    let mut dry = body.clone();
    dry["dryRun"] = json!(true);
    dry["onlyExisting"] = json!(false);
    let r = ok(&post(addr, &reroot, dry).await, 200, "dry re-root");
    assert_eq!(
        ids(&r["relinked"]),
        sorted(vec![hinted.clone(), gone.clone()])
    );
    let a = get_json(addr, &format!("/api/v1/media/{hinted}")).await;
    assert!(
        a["asset"].get("location").is_none(),
        "a dry run writes nothing: {a}"
    );
    let r = ok(&post(addr, &reroot, body).await, 200, "re-root");
    assert_eq!(ids(&r["relinked"]), std::slice::from_ref(&hinted));
    assert_eq!(r["relinked"][0]["method"], "rootBinding");
    assert_eq!(
        r["relinked"][0]["location"],
        json!({"type": "linked", "root": shared.0, "path": "Hymns2/Hymns/Cross.png"})
    );
    assert_eq!(ids(&r["notFound"]), std::slice::from_ref(&gone));
    // By root and folder.
    let r = ok(
        &post(
            addr,
            &reroot,
            json!({"from": {"root": shared.0, "prefix": "Old"}, "to": {"root": archive.0, "prefix": "New"}}),
        )
        .await,
        200,
        "re-root a folder",
    );
    assert_eq!(ids(&r["relinked"]), std::slice::from_ref(&moved));
    assert_eq!(
        ids(&r["notFound"]),
        std::slice::from_ref(&renamed),
        "New/Loop.png isn't there"
    );
    let r = post(
        addr,
        &reroot,
        json!({"from": {"prefix": "a"}, "to": {"root": MediaRootId::new().0}}),
    )
    .await;
    assert_eq!(ok(&r, 400, "unknown root")["error"]["field"], "to.root");

    // The search: a dry run, then for real.
    let search = format!("{base}/search");
    let r = ok(
        &post(addr, &search, json!({"dryRun": true})).await,
        200,
        "dry search",
    );
    let by_method = |r: &Value, m: &str| -> Vec<String> {
        sorted(
            r["relinked"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|x| x["method"] == m)
                .map(|x| x["asset"].as_str().unwrap().to_string())
                .collect(),
        )
    };
    assert_eq!(
        by_method(&r, "rootBinding"),
        std::slice::from_ref(&lost),
        "{r}"
    );
    assert_eq!(by_method(&r, "fileName"), std::slice::from_ref(&renamed));
    assert_eq!(
        by_method(&r, "contentHash"),
        sorted(vec![managed.clone(), vanished.clone()])
    );
    assert_eq!(ids(&r["ambiguous"]), std::slice::from_ref(&twice));
    assert_eq!(r["ambiguous"][0]["candidates"].as_array().unwrap().len(), 2);
    assert_eq!(
        ids(&r["notFound"]),
        sorted(vec![
            nowhere.clone(),
            gone.clone(),
            intro.clone(),
            logo.clone()
        ])
    );
    assert_eq!(
        get_json(addr, &format!("{base}?status=missing"))
            .await
            .as_array()
            .unwrap()
            .len(),
        9
    );

    let r = ok(&post(addr, &search, json!({})).await, 200, "search");
    assert_eq!(r["relinked"].as_array().unwrap().len(), 4);
    assert!(r["statusChanged"].as_u64().unwrap() >= 5, "{r}");
    let a = get_json(addr, &format!("/api/v1/media/{lost}")).await;
    assert_eq!(a["online"], true);
    assert_eq!(
        a["asset"]["location"],
        json!({"type": "linked", "root": shared.0, "path": "Backgrounds/Sunrise.png"})
    );
    let rec = &a["asset"]["relinks"][0];
    assert_eq!(rec["method"], "rootBinding");
    assert_eq!(rec["previous"]["root"], unbound.0.to_string());
    assert!(rec["at"].is_string());
    assert_eq!(a["asset"]["status"], "online");
    let a = get_json(addr, &format!("/api/v1/media/{managed}")).await;
    assert_eq!(a["asset"]["location"]["path"], "Different.png");
    assert_eq!(a["asset"]["relinks"][0]["previous"]["type"], "managed");
    let a = get_json(addr, &format!("/api/v1/media/{nowhere}")).await;
    assert_eq!(a["asset"]["status"], "missing");
    let e = loop {
        let m = ws.next_any().await;
        if m["type"] == "documentChanged" && m["id"] == lost.as_str() {
            break m;
        }
    };
    assert_eq!(e["collection"], main.as_str());

    // Once the probes of the relinked media are in, a search that finds nothing new writes
    // nothing.
    let a = get_json(addr, &format!("/api/v1/media/{vanished}")).await;
    assert_eq!(a["online"], true);
    assert_eq!(a["asset"]["location"]["type"], "library");
    assert_eq!(a["asset"]["location"]["sha256"], sha_hex(&bg_bytes));
    assert_ne!(a["asset"]["location"]["id"], "00000000deadbeef");
    assert_eq!(a["asset"]["relinks"][0]["method"], "contentHash");
    for id in [&lost, &renamed, &managed, &hinted, &moved, &vanished] {
        probed(addr, id).await;
    }
    let log = format!("/api/v1/codex/collections/{main}/changes?since=0");
    let before = get_json(addr, &log).await["to"].clone();
    let r = ok(&post(addr, &search, json!({})).await, 200, "search again");
    assert!(r["relinked"].as_array().unwrap().is_empty());
    assert_eq!(r["statusChanged"], 0);
    assert_eq!(
        get_json(addr, &log).await["to"],
        before,
        "statuses written only when they change"
    );

    // Manual relinks: a file under a root, a blob of the Codex.
    let relink = |id: &str| format!("{base}/{id}/relink");
    let r = post(
        addr,
        &relink(&twice),
        json!({"type": "linked", "root": archive.0, "path": "b/Twice.png"}),
    )
    .await;
    let a = ok(&r, 200, "manual relink");
    assert_eq!(a["asset"]["location"]["path"], "b/Twice.png");
    assert!(
        a["asset"]["relinks"][0].get("method").is_none(),
        "manual, the default"
    );
    assert_eq!(a["asset"]["status"], "online");
    // An item of the media library, by its ID; a media file by its SHA-256 (the library's
    // item with those bytes).
    let bg = server.shared.data().content.assets[&fx.asset.id]
        .location
        .clone();
    let AssetLocation::Library { id: bg_item, .. } = &bg else {
        panic!("the background is in the library: {bg:?}")
    };
    let r = post(
        addr,
        &relink(&nowhere),
        json!({"type": "library", "id": bg_item}),
    )
    .await;
    let a = ok(&r, 200, "relink to a library item");
    assert_eq!(a["asset"]["location"], serde_json::to_value(&bg).unwrap());
    assert_eq!(a["asset"]["status"], "online");
    let r = post(
        addr,
        &relink(&twice),
        json!({"type": "blob", "sha256": sha_hex(&bg_bytes), "extension": "png"}),
    )
    .await;
    let a = ok(&r, 200, "relink to a file by its SHA-256");
    assert_eq!(a["asset"]["location"], serde_json::to_value(&bg).unwrap());
    for (body, field) in [
        (json!({"type": "library", "id": "00000000deadbeef"}), "id"),
        (
            json!({"type": "linked", "root": archive.0, "path": "../x.png"}),
            "path",
        ),
        (
            json!({"type": "linked", "root": MediaRootId::new().0, "path": "x.png"}),
            "root",
        ),
        (
            json!({"type": "blob", "sha256": "0".repeat(64), "extension": "png"}),
            "sha256",
        ),
    ] {
        let r = post(addr, &relink(&logo), body).await;
        assert_eq!(ok(&r, 400, "bad relink")["error"]["field"], field);
    }
    let r = post(
        addr,
        &relink("0b3c5a51-7f63-4d1c-9a33-3a7c0e1f8d2e"),
        json!({"type": "linked", "root": archive.0, "path": "b/Twice.png"}),
    )
    .await;
    assert_eq!(r.status, 404);

    // The client's folder: matched by the end of the path, then by a unique name.
    let files = json!([
        {"name": "Intro.png", "size": 10, "relativePath": "My Media/Songs/Intro.png"},
        {"name": "Intro.png", "size": 10, "relativePath": "Other/Intro.png"},
        {"name": "logo.PNG", "size": 20, "relativePath": "Brand/logo.PNG"},
        {"name": "Unrelated.png", "size": 5}
    ]);
    let r = ok(
        &post(addr, &format!("{base}/match"), json!({"files": files})).await,
        200,
        "match",
    );
    let m: Vec<(String, String, String)> = r["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["asset"].as_str().unwrap().to_string(),
                x["file"]["relativePath"].as_str().unwrap().to_string(),
                x["by"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(
        m.contains(&(
            intro.clone(),
            "My Media/Songs/Intro.png".into(),
            "relativePath".into()
        )),
        "{r}"
    );
    assert!(
        m.contains(&(logo.clone(), "Brand/logo.PNG".into(), "fileName".into())),
        "{r}"
    );
    assert_eq!(ids(&r["unmatched"]), std::slice::from_ref(&gone));

    // The client uploads its file in place of the missing one.
    let file = png(8);
    let done = upload_replacing(addr, &main, "Intro.png", &file, &intro).await;
    assert_eq!(done["relinked"], true);
    assert_eq!(done["asset"]["id"], intro.as_str());
    assert_eq!(done["asset"]["location"]["sha256"], sha_hex(&file));
    let recs = done["asset"]["relinks"].as_array().unwrap();
    assert!(recs.last().unwrap().get("method").is_none(), "manual");
    assert_eq!(recs.last().unwrap()["previous"]["path"], "Songs/Intro.png");
    assert_eq!(
        get_json(addr, &format!("/api/v1/media/{intro}")).await["online"],
        true
    );
    let r = http(
        addr,
        "POST",
        "/api/v1/codex/uploads",
        &[],
        Some(json!({"collection": main, "name": "x.png", "size": 3, "replaces": "0b3c5a51-7f63-4d1c-9a33-3a7c0e1f8d2e"})),
    )
    .await;
    assert_eq!(
        ok(&r, 400, "replaces an unknown asset")["error"]["field"],
        "replaces"
    );
    let _ = std::fs::remove_dir_all(&fx.dir);
}
