//! Show-side gaps closed in batch 4, over the API: `next` / `prev` and the cursor follow the
//! playlist item's arrangement (repeats, the step into the next item), presentation summaries
//! carry the kind and song metadata, and a cleared layer comes back with `restoreLayer`; the
//! live occurrence of a repeated slide (`step`) and triggering one.

use crate::common::*;
use serde_json::{Value, json};

fn text(state: &Value, which: &str) -> String {
    let pos = if which == "cursor" {
        &state["cursor"]
    } else {
        &state["layers"][which]
    };
    pos["presentationName"].as_str().unwrap_or("-").to_string()
        + "/"
        + &pos["slideIndex"].to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn arrangements_summaries_and_restoring_a_cleared_layer() {
    let fx = fixture("showgaps");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let song = fx.song.id.0.to_string();
    let plid = fx.playlist.id.0.to_string();
    let items: Vec<String> = fx.items().iter().map(|i| i.id.0.to_string()).collect();
    let verse = fx.song.sections[0].id;
    let chorus = fx.song.sections[1].id;

    // G1: summaries carry the kind and (empty) song metadata.
    let list = get_json(addr, "/api/v1/presentations").await;
    let find = |list: &Value, id: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == id)
            .cloned()
            .unwrap()
    };
    let s = find(&list, &song);
    assert_eq!(s["kind"], "text", "{s}");
    assert!(s.get("authors").is_none() && s.get("ccliNumber").is_none());

    // An arrangement "Twice" = Chorus, Verse 1, Chorus (the song's first chorus slide is
    // disabled), chosen for the song's playlist item; song metadata on the song.
    let arr = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let r = http(addr, "POST", &format!("/api/v1/presentations/{song}/edits"), &[], Some(json!({"ops": [
        {"type": "setArrangements", "arrangements": [{"id": arr, "name": "Twice", "sections": [chorus, verse, chorus]}]},
        {"type": "setPresentation", "songInfo": {"authors": "A. Writer", "copyrightYear": 2001, "ccliNumber": "42"}}
    ]}))).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/playlists/{plid}/edits"),
        &[],
        Some(json!({"ops": [
            {"type": "setItem", "item": items[1], "arrangement": arr}
        ]})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let list = get_json(addr, "/api/v1/presentations").await;
    let s = find(&list, &song);
    assert_eq!(s["kind"], "song", "{s}");
    assert_eq!(s["authors"], "A. Writer");
    assert_eq!(s["copyrightYear"], 2001);
    assert_eq!(s["ccliNumber"], "42");

    // G6: the item plays its arrangement; the cursor (stage "next") follows it.
    let mut a = Client::connect(addr, "").await;
    a.ok(json!({"type": "triggerPlaylistItem", "playlist": plid, "item": items[1]}))
        .await;
    let mut seen = vec![text(&a.state, "slide")];
    let mut cursors = vec![text(&a.state, "cursor")];
    for _ in 0..3 {
        a.ok(json!({"type": "next"})).await;
        seen.push(text(&a.state, "slide"));
        cursors.push(text(&a.state, "cursor"));
    }
    assert_eq!(
        seen,
        ["Test Song/3", "Test Song/0", "Test Song/1", "Test Song/3"],
        "chorus (FOUR), verse (ONE, TWO), chorus again"
    );
    assert_eq!(
        cursors,
        ["Test Song/0", "Test Song/1", "Test Song/3", "Second Song/0"],
        "after the second chorus, the next item"
    );
    a.ok(json!({"type": "prev"})).await;
    assert_eq!(text(&a.state, "slide"), "Test Song/1");

    // The occurrence on the wire (`step`: the place in the play order 2 3 0 1 2 3), and a
    // trigger that picks one: the second chorus (FOUR at step 5) goes on to the next item.
    assert_eq!(a.state["layers"]["slide"]["step"], 3);
    assert_eq!(
        a.state["cursor"]["step"], 5,
        "the disabled THREE is skipped"
    );
    let item = json!({"playlist": plid, "item": items[1]});
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 3, "playlistItem": item, "step": 5}))
        .await;
    assert_eq!(a.state["layers"]["slide"]["step"], 5);
    assert_eq!(text(&a.state, "cursor"), "Second Song/0");
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 3, "playlistItem": item, "step": 1}))
        .await;
    assert_eq!(a.state["layers"]["slide"]["step"], 1);
    assert_eq!(text(&a.state, "cursor"), "Test Song/0");
    assert_eq!(a.state["cursor"]["step"], 2);
    // Without a step: the first occurrence at or after the position.
    a.ok(json!({"type": "triggerSlide", "presentation": song, "index": 1, "playlistItem": item}))
        .await;
    assert_eq!(a.state["layers"]["slide"]["step"], 3);
    assert_eq!(text(&a.state, "slide"), "Test Song/1");

    // G4: a cleared slide comes back while its layer is empty.
    assert_eq!(a.state["restorableLayers"], json!([]));
    a.ok(json!({"type": "clearLayer", "layer": "slide"})).await;
    assert!(a.state["layers"]["slide"].is_null());
    assert_eq!(a.state["restorableLayers"], json!(["slide"]));
    a.ok(json!({"type": "restoreLayer", "layer": "slide"}))
        .await;
    assert_eq!(text(&a.state, "slide"), "Test Song/1");
    assert_eq!(a.state["restorableLayers"], json!([]));
    let r = a
        .command(json!({"type": "restoreLayer", "layer": "slide"}))
        .await;
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["error"]["code"], "conflict", "{r}");
    // Clear All keeps the media layer too (the song's background image).
    a.ok(json!({"type": "clearAll"})).await;
    assert_eq!(a.state["restorableLayers"], json!(["media", "slide"]));
    a.ok(json!({"type": "restoreLayer", "layer": "media"}))
        .await;
    assert!(!a.state["layers"]["media"].is_null());
    assert_eq!(a.state["restorableLayers"], json!(["slide"]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn timer_warning_and_the_box_clock_format() {
    let fx = fixture("clockfmt");
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let mut a = Client::connect(addr, "").await;
    assert_eq!(
        a.state["clockFormat"],
        json!({"use24Hour": false, "seconds": false})
    );
    a.ok(json!({"type": "setClockFormat", "use24Hour": true}))
        .await;
    assert_eq!(
        a.state["clockFormat"],
        json!({"use24Hour": true, "seconds": false})
    );
    a.ok(json!({"type": "setClockFormat", "seconds": true}))
        .await;
    assert_eq!(
        a.state["clockFormat"],
        json!({"use24Hour": true, "seconds": true})
    );
    // Kept on the box.
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fx.dir.join("machine/clock-format.json")).unwrap())
            .unwrap();
    assert_eq!(saved, json!({"use24Hour": true, "seconds": true}));

    // A timer's warning time is stored and shown in the state.
    let r = http(
        addr,
        "POST",
        "/api/v1/timers",
        &[],
        Some(json!({"name": "Offering", "kind": {"type": "countdown", "seconds": 240}, "warningSeconds": 120})),
    )
    .await;
    assert_eq!(r.status, 201, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["warningSeconds"], 120.0);
    a.wait_state("the timer is listed", |_, s| {
        s["timers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "Offering" && t["warningSeconds"] == 120.0)
    })
    .await;
    server.shutdown().await.unwrap();
}
