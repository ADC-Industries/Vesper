//! The WebSocket connection itself: commands the server cannot read still get their result,
//! and a client that stops answering pings is dropped while one that answers stays.

use crate::common::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreadable_command_still_gets_its_result() {
    let fx = fixture("ws-unreadable");
    let server = start_server(&fx, true, false).await;
    let mut c = Client::connect(server.addr, "protocolVersion=1").await;
    // A command type from a newer client: notImplemented, at once (not a 5 s client timeout).
    let r = c.command(json!({"type": "fromANewerRelease"})).await;
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["error"]["code"], "notImplemented", "{r}");
    // A known command with a bad field: badRequest.
    let r = c
        .command(json!({"type": "setVolume", "target": "master", "db": "loud"}))
        .await;
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["error"]["code"], "badRequest", "{r}");
    // Not a command: an id-less error.
    c.ws.send(Message::Text(r#"{"type":"nonsense"}"#.into()))
        .await
        .unwrap();
    loop {
        let m = c.next_any().await;
        if m["type"] == "error" {
            assert_eq!(m["error"]["code"], "badRequest", "{m}");
            break;
        }
    }
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_that_stops_answering_is_dropped() {
    let fx = fixture("ws-liveness");
    let server = start_server_with(&fx, |c| c.ws_ping = Duration::from_millis(100)).await;
    let url = format!("ws://{}/api/v1/ws?protocolVersion=1", server.addr);

    // Never reads, so never answers the server's pings: gone after three ping intervals.
    let (mut silent, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .unwrap();
    // Reads all the time (answering pings): stays.
    let mut live = Client::connect(server.addr, "protocolVersion=1").await;
    let _ = tokio::time::timeout(Duration::from_millis(800), async {
        loop {
            let _ = live.ws.next().await;
        }
    })
    .await;

    let closed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match silent.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "the silent client is still connected");

    live.ws
        .send(Message::Text(r#"{"type":"ping","id":5}"#.into()))
        .await
        .unwrap();
    loop {
        let m = live.next_any().await;
        assert!(m.get("close").is_none(), "the answering client was dropped");
        if m["type"] == "pong" {
            assert_eq!(m["id"], 5);
            break;
        }
    }
    let _ = std::fs::remove_dir_all(&fx.dir);
}
