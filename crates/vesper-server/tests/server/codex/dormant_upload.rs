//! A show box uploads its own, dormant collection to the site's Codex (G15): a new collection
//! on the Codex with the box's documents (their IDs kept, logged as added by the box), every
//! media file it uses sent over the link and relinked (bytes the Codex's library has already
//! aren't sent), never a second collection for the same source; a document of a newer schema
//! stops it with the reason, and starting again resumes into the same collection. Two servers
//! in this process (A hosts the Codex, B is its show box), synthetic fixtures.

use super::showbox::{sha_hex, solid_png, until, wait_client};
use crate::common::*;
use crate::site::keys::two_boxes_prepared;
use crate::site::{call, ok};
use model::{AssetLocation, MediaAsset};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::time::Duration;
use vesper_server::codex::CodexOptions;

async fn dormant(addr: SocketAddr, key: &str) -> Value {
    ok(
        &call(addr, "GET", "/api/v1/codex/dormant/upload", Some(key), None).await,
        200,
        "GET /codex/dormant/upload",
    )
}

/// Starts the upload on `addr` and waits until it stops copying; its last state.
async fn upload_and_wait(addr: SocketAddr, key: &str) -> Value {
    let r = call(
        addr,
        "POST",
        "/api/v1/codex/dormant/upload",
        Some(key),
        Some(json!({})),
    )
    .await;
    let started = ok(&r, 202, "POST /codex/dormant/upload");
    // A job that fails at once (a newer-schema document) can already read "failed" here.
    assert!(
        ["copying", "done", "failed"].contains(&started["state"].as_str().unwrap()),
        "{started}"
    );
    until("the upload stops copying", 60, || async {
        dormant(addr, key).await["state"] != "copying"
    })
    .await;
    dormant(addr, key).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_show_box_uploads_its_dormant_collection_to_the_sites_codex() {
    // B's own Codex has the fixture's image (A's library has the same bytes) and one image
    // of its own.
    let green = solid_png([20, 200, 60]);
    let green_sha = sha_hex(&green);
    let green_asset = MediaAsset {
        name: "Green wall".into(),
        location: AssetLocation::Managed {
            sha256: green_sha.clone(),
            extension: "png".into(),
        },
        ..MediaAsset::default()
    };
    let (g, ga) = (green.clone(), green_asset.clone());
    let t = two_boxes_prepared(
        "cx-adopt",
        CodexOptions::default(),
        CodexOptions {
            mirror_poll: Duration::from_secs(2),
            ..CodexOptions::default()
        },
        |_| {},
        move |fb| {
            std::fs::write(
                fb.dir.join("media").join(format!("{}.png", sha_hex(&g))),
                &g,
            )
            .unwrap();
            write_doc(&fb.dir, "portable", "mediaAsset", &ga.id.0.to_string(), &ga);
        },
    )
    .await;
    let (a, b, key) = (t.a.addr, t.b.addr, t.key_a.clone());
    let own =
        t.b.shared
            .codex()
            .config
            .collection
            .expect("B's own collection");
    wait_client(&t).await;
    let collections_before = t.a.shared.codex().collections.len();

    // The offer: B's own collection, with its songs.
    let offer = dormant(b, &key).await;
    assert_eq!(offer["state"], "idle", "{offer}");
    assert_eq!(offer["source"], own.to_string(), "{offer}");
    assert!(offer["songs"].as_u64().unwrap() >= 1, "{offer}");
    // Only a show box has one.
    let r = call(a, "GET", "/api/v1/codex/dormant/upload", Some(&key), None).await;
    assert_eq!(r.status, 404, "{}", String::from_utf8_lossy(&r.body));

    // A document of a newer schema stops the upload with the reason.
    let dormant_dir = t.fb.dir.join(format!("codex/collections/{own}/portable"));
    let newer = uuid::Uuid::new_v4();
    let newer_file = dormant_dir.join(format!("presentation/{newer}.json"));
    std::fs::write(
        &newer_file,
        json!({"schemaVersion": 999, "id": newer.to_string(), "name": "From the future"})
            .to_string(),
    )
    .unwrap();
    let failed = upload_and_wait(b, &key).await;
    assert_eq!(failed["state"], "failed", "{failed}");
    assert!(
        failed["error"].as_str().unwrap().contains("newer server"),
        "{failed}"
    );
    let staged = failed["collection"].as_str().unwrap().to_string();

    // M-1: a start right after a client saw the job end starts a new job, even while the
    // old job's thread is still finishing (its tail held open for 1.5 s here).
    {
        use vesper_server::codex::host::Server as _;
        t.b.shared
            .adopt()
            .pause_tail_for_tests(Some(Duration::from_millis(1500)));
        let post = || async {
            call(
                b,
                "POST",
                "/api/v1/codex/dormant/upload",
                Some(key.as_str()),
                Some(json!({})),
            )
            .await
        };
        ok(&post().await, 202, "start");
        // (start() said "copying" before it answered: "failed" now is this job's end.)
        until("the job fails", 30, || async {
            dormant(b, &key).await["state"] == "failed"
        })
        .await;
        let seen = dormant(b, &key).await;
        let again = ok(&post().await, 202, "start again at once");
        assert_ne!(
            again["startedAt"], seen["startedAt"],
            "a new job, not the old one's state: {again} / {seen}"
        );
        t.b.shared.adopt().pause_tail_for_tests(None);
        until("the new job stops copying", 60, || async {
            dormant(b, &key).await["state"] != "copying"
        })
        .await;
        // (its tail may still be paused: let it end before the next start)
        tokio::time::sleep(Duration::from_millis(1700)).await;
    }
    assert_eq!(
        t.a.shared.codex().collections.len(),
        collections_before,
        "nothing in A's Codex yet"
    );

    std::fs::remove_file(&newer_file).unwrap();

    // A long job holding A's writer stops the finish before B gives up (D04-2): it is never
    // applied later, when B may be sending the documents again.
    let held = t.a.shared.clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let (got_tx, got_rx) = tokio::sync::oneshot::channel::<()>();
    let job = tokio::spawn(async move {
        let _w = held.codex_writer.exclusive().await;
        let _ = got_tx.send(());
        let _ = rx.await;
    });
    got_rx.await.unwrap();
    let started = std::time::Instant::now();
    let busy = upload_and_wait(b, &key).await;
    assert_eq!(busy["state"], "failed", "{busy}");
    assert!(busy["error"].as_str().unwrap().contains("busy"), "{busy}");
    assert!(
        started.elapsed() < Duration::from_secs(55),
        "before B gave up"
    );
    tx.send(()).unwrap();
    job.await.unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        t.a.shared.codex().collections.len(),
        collections_before,
        "the refused finish is never applied"
    );

    // Starting again resumes into the same collection and finishes.
    let done = upload_and_wait(b, &key).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["collection"], staged.as_str(), "{done}");
    assert_eq!(
        done["documents"]["done"], done["documents"]["total"],
        "{done}"
    );
    assert_eq!(done["media"], json!({"done": 2, "total": 2}), "{done}");
    assert_eq!(done["bytes"]["done"], done["bytes"]["total"], "{done}");

    // A has the collection, with B's documents and their IDs.
    let cid: model::CollectionId = staged.parse().unwrap();
    assert_eq!(t.a.shared.codex().collections.len(), collections_before + 1);
    let content = t.a.shared.collection_content(cid).expect("on A");
    assert!(
        content.presentations.iter().any(|p| p.id == t.fb.song.id),
        "B's song is in the new collection"
    );
    assert_eq!(
        t.a.shared.codex().collection(cid).unwrap().collection.name,
        done["collectionName"].as_str().unwrap()
    );
    // Its media are A's library items now; B's own image was sent byte for byte.
    let green_here = content
        .assets
        .get(&green_asset.id)
        .expect("the green asset");
    match &green_here.location {
        AssetLocation::Library { id, sha256 } => {
            assert_eq!(sha256, &green_sha);
            let path = t.a.shared.library.resolve(id).unwrap();
            assert_eq!(std::fs::read(path).unwrap(), green);
        }
        other => panic!("the green image isn't relinked: {other:?}"),
    }
    for a in content.assets.values() {
        assert!(
            matches!(a.location, AssetLocation::Library { .. }),
            "{} → {:?}",
            a.name,
            a.location
        );
    }
    // The change log names B as the box the documents came from.
    let changes = ok(
        &call(
            a,
            "GET",
            &format!("/api/v1/codex/collections/{cid}/changes?since=0"),
            Some(&key),
            None,
        )
        .await,
        200,
        "changes",
    );
    let first = &changes["changes"][0];
    assert_eq!(first["change"], "added", "{changes}");
    assert_eq!(
        first["by"]["machine"], "6a1d1c8e-0000-4000-8000-0000000000b0",
        "{changes}"
    );
    // ...added by its server, not by a client named after the collection (R3-5).
    assert_eq!(first["by"]["client"], "server machine", "{changes}");
    // Starting it is in B's audit log (R3-6).
    let audit = ok(
        &call(b, "GET", "/api/v1/site/audit?n=200", Some(&key), None).await,
        200,
        "audit",
    );
    assert!(
        audit
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["action"] == "codex.uploadDormant"
                && e["error"].is_null()
                && e["detail"]["collection"] == staged.as_str()),
        "{audit}"
    );

    // Uploading again makes nothing new.
    let again = upload_and_wait(b, &key).await;
    assert_eq!(again["state"], "done", "{again}");
    assert_eq!(again["collection"], staged.as_str(), "{again}");
    assert_eq!(t.a.shared.codex().collections.len(), collections_before + 1);

    // The collection is deleted on A: uploading again makes a new one, and sends its media
    // again (D04-1: what went to the old one doesn't count).
    let r = call(
        a,
        "DELETE",
        &format!("/api/v1/codex/collections/{cid}"),
        Some(&key),
        None,
    )
    .await;
    assert_eq!(r.status, 204, "{}", String::from_utf8_lossy(&r.body));
    let anew = upload_and_wait(b, &key).await;
    assert_eq!(anew["state"], "done", "{anew}");
    assert_ne!(anew["collection"], staged.as_str(), "{anew}");
    assert_eq!(anew["media"], json!({"done": 2, "total": 2}), "{anew}");
    let cid2: model::CollectionId = anew["collection"].as_str().unwrap().parse().unwrap();
    let content2 = t.a.shared.collection_content(cid2).expect("on A");
    for a in content2.assets.values() {
        assert!(
            matches!(a.location, AssetLocation::Library { .. }),
            "{} → {:?}",
            a.name,
            a.location
        );
    }
    // B's own Codex is intact.
    assert!(
        t.fb.dir
            .join(format!("codex/collections/{own}/collection.json"))
            .is_file()
    );

    // R3-1: B left the site and joined it again (its job forgotten, as waking does), and its
    // own collection changed meanwhile: the offer's upload makes a new collection with the
    // change, instead of finding the earlier one and skipping every document.
    {
        use vesper_server::codex::host::Server as _;
        t.b.shared.adopt().forget(&t.b.shared);
    }
    let mut awake = t.fb.song.clone();
    awake.id = model::PresentationId(uuid::Uuid::new_v4());
    awake.name = "Added while awake".into();
    std::fs::write(
        dormant_dir.join(format!("presentation/{}.json", awake.id.0)),
        serde_json::to_vec(&awake).unwrap(),
    )
    .unwrap();
    assert_eq!(dormant(b, &key).await["state"], "idle");
    let rejoined = upload_and_wait(b, &key).await;
    assert_eq!(rejoined["state"], "done", "{rejoined}");
    assert_ne!(rejoined["collection"], anew["collection"], "{rejoined}");
    let cid3: model::CollectionId = rejoined["collection"].as_str().unwrap().parse().unwrap();
    let content3 = t.a.shared.collection_content(cid3).expect("on A");
    assert!(
        content3.presentations.iter().any(|p| p.id == awake.id),
        "the change made meanwhile is in"
    );
    assert!(
        t.a.shared.collection_content(cid2).is_some(),
        "the earlier upload stays"
    );

    // B leaves the site: the job was that site's, its state goes (D04-1).
    let saved = t.fb.dir.join("machine/codex-adopt.json");
    assert!(saved.is_file());
    ok(
        &call(b, "POST", "/api/v1/site/leave", Some(&key), None).await,
        204,
        "B leaves",
    );
    until("B's own Codex wakes", 30, || async {
        t.b.shared.codex().config.role == model::CodexRole::Host
    })
    .await;
    assert!(!saved.exists(), "the upload's state is reset");
    t.end().await;
}
