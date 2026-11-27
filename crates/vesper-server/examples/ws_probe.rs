//! `ws_probe`: a small command-line client of the control protocol, for smoke tests against a
//! running server (and an example of the protocol from Rust).
//!
//! ```text
//! cargo run -p vesper-server --example ws_probe -- [--addr 127.0.0.1:5555] [--token <token>]
//!     [--presentation <id or name substring>] [--thumbs 20] [--width 320]
//!     [--preview-seconds 3] [--change-ms 200] [--out-dir <dir>]
//! cargo run -p vesper-server --example ws_probe -- --discover
//! cargo run -p vesper-server --example ws_probe -- [--addr …] [--token …] --codex
//!     [--codex-search <query>] [--codex-collection <id or all>] [--watch-seconds <s>]
//! ```
//!
//! Connects to the WebSocket, prints the hello and state, triggers slides (reporting the
//! command → state latency), subscribes to a live preview (reporting frames per second),
//! fetches thumbnails cold, cached and with `If-None-Match` (reporting latencies), and with
//! `--out-dir` saves the last preview frame and the first two thumbnails as JPEG files.
//! `--discover` lists the servers found over mDNS (`protocol::discover`) and exits.
//!
//! `--codex` triggers nothing: it prints the box's Codex status (its role, active collection,
//! media library, and on a show box how its copy is syncing), the collections, with
//! `--codex-search` the hits of a search (this box's collection, or `--codex-collection`),
//! then prints the `codexChanged` and `documentChanged` events as they come (with their
//! collection), refetching the status when it changes, for `--watch-seconds` (default: until
//! the socket closes or Ctrl-C; `0`: not at all). `--codex-search` alone implies `--codex`.

use futures_util::{SinkExt, StreamExt};
use protocol::PreviewFrameHeader;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;

struct Args {
    addr: SocketAddr,
    token: Option<String>,
    presentation: Option<String>,
    thumbs: usize,
    width: u32,
    preview_seconds: f64,
    change_ms: u64,
    out_dir: Option<PathBuf>,
    codex: bool,
    codex_search: Option<String>,
    codex_collection: Option<String>,
    watch_seconds: Option<f64>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut a = Args {
        addr: "127.0.0.1:5555".parse()?,
        token: None,
        presentation: None,
        thumbs: 20,
        width: 320,
        preview_seconds: 3.0,
        change_ms: 200,
        out_dir: None,
        codex: false,
        codex_search: None,
        codex_collection: None,
        watch_seconds: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut val = || {
            it.next()
                .ok_or_else(|| anyhow::anyhow!("{k} needs a value"))
        };
        match k.as_str() {
            "--addr" => a.addr = val()?.parse()?,
            "--token" => a.token = Some(val()?),
            "--presentation" => a.presentation = Some(val()?),
            "--thumbs" => a.thumbs = val()?.parse()?,
            "--width" => a.width = val()?.parse()?,
            "--preview-seconds" => a.preview_seconds = val()?.parse()?,
            "--change-ms" => a.change_ms = val()?.parse()?,
            "--out-dir" => a.out_dir = Some(val()?.into()),
            "--codex" => a.codex = true,
            "--codex-search" => {
                a.codex = true;
                a.codex_search = Some(val()?);
            }
            "--codex-collection" => a.codex_collection = Some(val()?),
            "--watch-seconds" => a.watch_seconds = Some(val()?.parse()?),
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    Ok(a)
}

struct Http {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

async fn http(
    addr: SocketAddr,
    token: Option<&str>,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
) -> anyhow::Result<Http> {
    let mut s = tokio::net::TcpStream::connect(addr).await?;
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: 0\r\n"
    );
    if let Some(t) = token {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    for (k, v) in extra {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await?;
    let mut resp = Vec::new();
    s.read_to_end(&mut resp).await?;
    let split = resp
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow::anyhow!("bad HTTP response"))?;
    let head = String::from_utf8_lossy(&resp[..split]).to_string();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let mut body = resp[split + 4..].to_vec();
    if headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.contains("chunked"))
    {
        let mut out = Vec::new();
        let mut b = &body[..];
        while let Some(nl) = b.windows(2).position(|w| w == b"\r\n") {
            let n = usize::from_str_radix(String::from_utf8_lossy(&b[..nl]).trim(), 16)?;
            if n == 0 {
                break;
            }
            out.extend_from_slice(&b[nl + 2..nl + 2 + n]);
            b = &b[nl + 2 + n + 2..];
        }
        body = out;
    }
    Ok(Http {
        status,
        headers,
        body,
    })
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 1000.0 * 10.0).round() / 10.0
}

fn stats(v: &[f64]) -> String {
    if v.is_empty() {
        return "-".into();
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let avg = s.iter().sum::<f64>() / s.len() as f64;
    format!(
        "avg {:.1} ms, median {:.1} ms, min {:.1} ms, max {:.1} ms (n={})",
        avg,
        s[s.len() / 2],
        s[0],
        s[s.len() - 1],
        s.len()
    )
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// What the probe has seen.
struct Live {
    revision: u64,
    state: Value,
    frames: Vec<(Instant, Vec<u8>)>,
}

/// Reads until `stop` accepts a message; states are tracked, binary frames collected.
async fn read_until(
    ws: &mut Ws,
    live: &mut Live,
    stop: impl Fn(&Value) -> bool,
) -> anyhow::Result<Value> {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(60), ws.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("socket closed"))??;
        match m {
            Message::Text(t) => {
                let v: Value = serde_json::from_str(t.as_str())?;
                if v["type"] == "state" {
                    live.revision = v["revision"].as_u64().unwrap_or(0);
                    live.state = v["state"].clone();
                } else if v["type"] == "notice" {
                    println!("  notice [{}]: {}", v["level"], v["message"]);
                }
                if stop(&v) {
                    return Ok(v);
                }
            }
            Message::Binary(b) => live.frames.push((Instant::now(), b.to_vec())),
            Message::Close(f) => anyhow::bail!("closed: {f:?}"),
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--discover") {
        let found =
            tokio::task::spawn_blocking(|| protocol::discover::discover(Duration::from_secs(3)))
                .await??;
        println!(
            "{} server(s) on {}",
            found.len(),
            protocol::MDNS_SERVICE_TYPE
        );
        for s in found {
            println!(
                "  {} id={} version={} protocol={} compatible={} {} addresses={:?}",
                s.name,
                s.id,
                s.version,
                s.protocol_version,
                s.compatible(),
                s.base_url().unwrap_or_default(),
                s.addresses
            );
        }
        return Ok(());
    }
    let a = parse_args()?;
    let tok = a.token.as_deref();
    if let Some(d) = &a.out_dir {
        std::fs::create_dir_all(d)?;
    }

    let info: Value =
        serde_json::from_slice(&http(a.addr, tok, "GET", "/api/v1/server", &[]).await?.body)?;
    println!(
        "server {} ({}), version {}, protocol {}, auth {}",
        info["name"], info["id"], info["version"], info["protocolVersion"], info["auth"]
    );
    if a.codex {
        return codex(&a, &info).await;
    }

    let mut url = format!(
        "ws://{}/api/v1/ws?protocolVersion={}",
        a.addr,
        protocol::PROTOCOL_VERSION
    );
    if let Some(t) = tok {
        url.push_str(&format!("&token={t}"));
    }
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await?;
    let mut live = Live {
        revision: 0,
        state: Value::Null,
        frames: Vec::new(),
    };
    let mut next_id = 1u32;

    let hello = read_until(&mut ws, &mut live, |v: &Value| v["type"] == "hello").await?;
    println!("hello: {}", hello["server"]);
    read_until(&mut ws, &mut live, |v: &Value| v["type"] == "state").await?;
    println!(
        "state r{}: {} screens, active look {}, slide layer {}",
        live.revision,
        live.state["screens"]
            .as_array()
            .map(|s| s.len())
            .unwrap_or(0),
        live.state["activeLook"],
        live.state["layers"]["slide"]["presentationName"]
    );
    for s in live.state["screens"].as_array().into_iter().flatten() {
        println!(
            "  screen {} {}x{} @{:.2} fps (measured {}), last render {} ms, outputs {}",
            s["name"],
            s["width"],
            s["height"],
            s["frameRate"]["num"].as_f64().unwrap_or(0.0)
                / s["frameRate"]["den"].as_f64().unwrap_or(1.0),
            s["measuredFps"],
            s["lastRenderMs"],
            s["outputs"]
                .as_array()
                .map(|o| o
                    .iter()
                    .map(|o| format!("{}={}", o["name"], o["status"]))
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default()
        );
    }

    // Pick a presentation.
    let list: Value = serde_json::from_slice(
        &http(a.addr, tok, "GET", "/api/v1/presentations", &[])
            .await?
            .body,
    )?;
    let list = list.as_array().cloned().unwrap_or_default();
    let pick = match &a.presentation {
        Some(q) => list.iter().find(|p| {
            p["id"] == q.as_str()
                || p["name"]
                    .as_str()
                    .is_some_and(|n| n.to_lowercase().contains(&q.to_lowercase()))
        }),
        None => list
            .iter()
            .filter(|p| p["destination"] == "slide")
            .max_by_key(|p| p["slideCount"].as_u64().unwrap_or(0).min(a.thumbs as u64)),
    }
    .ok_or_else(|| anyhow::anyhow!("no presentation found"))?
    .clone();
    let pid = pick["id"].as_str().unwrap_or_default().to_string();
    println!(
        "presentation {} ({} slides, {pid})",
        pick["name"], pick["slideCount"]
    );

    // Commands.
    let mut cmd_times = Vec::new();
    let mut send = |ws_cmd: Value| {
        let id = next_id;
        next_id += 1;
        (
            id,
            json!({"type": "command", "id": id, "command": ws_cmd}).to_string(),
        )
    };
    let n = pick["slideCount"].as_u64().unwrap_or(1).min(4);
    for i in 0..n {
        let (id, text) = send(json!({"type": "triggerSlide", "presentation": pid, "index": i}));
        let t = Instant::now();
        ws.send(Message::Text(text.into())).await?;
        let r = read_until(&mut ws, &mut live, |v: &Value| {
            v["type"] == "result" && v["id"] == id
        })
        .await?;
        cmd_times.push(ms(t.elapsed()));
        println!(
            "  triggerSlide {i}: ok={} r{} -> live {} #{} ({}), cursor #{}",
            r["ok"],
            live.revision,
            live.state["layers"]["slide"]["presentationName"],
            live.state["layers"]["slide"]["slideIndex"],
            live.state["layers"]["slide"]["sectionName"],
            live.state["cursor"]["slideIndex"]
        );
    }
    for c in ["prev", "next"] {
        let (id, text) = send(json!({"type": c}));
        let t = Instant::now();
        ws.send(Message::Text(text.into())).await?;
        let r = read_until(&mut ws, &mut live, |v: &Value| {
            v["type"] == "result" && v["id"] == id
        })
        .await?;
        cmd_times.push(ms(t.elapsed()));
        println!(
            "  {c}: ok={} -> #{}",
            r["ok"], live.state["layers"]["slide"]["slideIndex"]
        );
    }
    println!(
        "command round trip (result incl. state): {}",
        stats(&cmd_times)
    );

    // Live preview of the first screen.
    if let Some(screen) = live.state["screens"][0]["id"].as_str().map(str::to_owned) {
        let (id, text) =
            send(json!({"type": "subscribePreview", "screen": screen, "fps": 15, "width": 480}));
        ws.send(Message::Text(text.into())).await?;
        read_until(&mut ws, &mut live, |v: &Value| {
            v["type"] == "result" && v["id"] == id
        })
        .await?;
        live.frames.clear();
        let t0 = Instant::now();
        let mut k = 0u64;
        let mut trig_at = t0;
        while t0.elapsed().as_secs_f64() < a.preview_seconds {
            // Change the picture every `--change-ms` so frames keep coming.
            if trig_at.elapsed() >= Duration::from_millis(a.change_ms) {
                trig_at = Instant::now();
                let (_, text) =
                    send(json!({"type": "triggerSlide", "presentation": pid, "index": k % n}));
                k += 1;
                ws.send(Message::Text(text.into())).await?;
            }
            if let Ok(Some(Ok(m))) =
                tokio::time::timeout(Duration::from_millis(20), ws.next()).await
                && let Message::Binary(b) = m
            {
                live.frames.push((Instant::now(), b.to_vec()));
            }
        }
        let secs = t0.elapsed().as_secs_f64();
        let sizes: Vec<usize> = live.frames.iter().map(|f| f.1.len()).collect();
        println!(
            "preview: {} frames in {secs:.1} s = {:.1} fps while changing every {} ms ({k} triggers), avg {} KB",
            live.frames.len(),
            live.frames.len() as f64 / secs,
            a.change_ms,
            sizes.iter().sum::<usize>() / sizes.len().max(1) / 1024
        );
        if let Some((_, last)) = live.frames.last()
            && let Some((h, jpeg)) = PreviewFrameHeader::decode(last)
        {
            println!(
                "  last frame: screen {}, frame #{}, {}x{}",
                h.screen, h.frame, h.width, h.height
            );
            if let Some(d) = &a.out_dir {
                std::fs::write(d.join("preview.jpg"), jpeg)?;
            }
        }
        let (id, text) = send(json!({"type": "unsubscribePreview", "screen": null}));
        ws.send(Message::Text(text.into())).await?;
        read_until(&mut ws, &mut live, |v: &Value| {
            v["type"] == "result" && v["id"] == id
        })
        .await?;
    }

    // Thumbnails: cold, cached, conditional.
    let doc: Value = serde_json::from_slice(
        &http(
            a.addr,
            tok,
            "GET",
            &format!("/api/v1/presentations/{pid}"),
            &[],
        )
        .await?
        .body,
    )?;
    let slides: Vec<String> = doc["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|s| s["slides"].as_array().cloned().unwrap_or_default())
        .filter_map(|s| s["id"].as_str().map(str::to_owned))
        .take(a.thumbs)
        .collect();
    let mut cold = Vec::new();
    let mut warm = Vec::new();
    let mut cond = Vec::new();
    let mut etags = Vec::new();
    for (pass, times) in [(0, &mut cold), (1, &mut warm)] {
        for (i, s) in slides.iter().enumerate() {
            let path = format!(
                "/api/v1/presentations/{pid}/slides/{s}/thumbnail?w={}",
                a.width
            );
            let t = Instant::now();
            let r = http(a.addr, tok, "GET", &path, &[]).await?;
            times.push(ms(t.elapsed()));
            anyhow::ensure!(r.status == 200, "thumbnail {s}: HTTP {}", r.status);
            if pass == 0 {
                etags.push(
                    r.headers
                        .iter()
                        .find(|h| h.0 == "etag")
                        .map(|h| h.1.clone())
                        .unwrap_or_default(),
                );
                if i < 2
                    && let Some(d) = &a.out_dir
                {
                    std::fs::write(d.join(format!("thumb-{i}.jpg")), &r.body)?;
                }
            }
        }
    }
    for (s, etag) in slides.iter().zip(&etags) {
        let path = format!(
            "/api/v1/presentations/{pid}/slides/{s}/thumbnail?w={}",
            a.width
        );
        let t = Instant::now();
        let r = http(a.addr, tok, "GET", &path, &[("If-None-Match", etag)]).await?;
        cond.push(ms(t.elapsed()));
        anyhow::ensure!(r.status == 304, "expected 304, got {}", r.status);
    }
    println!("thumbnails ({} slides, w={}):", slides.len(), a.width);
    println!("  first request: {}", stats(&cold));
    println!("  cached:        {}", stats(&warm));
    println!("  304 (ETag):    {}", stats(&cond));
    let _ = ws.close(None).await;
    Ok(())
}

/// `GET` a JSON document; an error names the route and the server's answer.
async fn get_json(addr: SocketAddr, token: Option<&str>, path: &str) -> anyhow::Result<Value> {
    let r = http(addr, token, "GET", path, &[]).await?;
    anyhow::ensure!(
        r.status == 200,
        "GET {path}: HTTP {} {}",
        r.status,
        String::from_utf8_lossy(&r.body)
    );
    Ok(serde_json::from_slice(&r.body)?)
}

/// Percent-encodes a query parameter's value.
fn query_value(v: &str) -> String {
    v.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A collection's name by its ID, as the probe last listed them (else the ID).
fn collection_name(names: &[(String, String)], id: &Value) -> String {
    let Some(id) = id.as_str() else {
        return "-".into();
    };
    names
        .iter()
        .find(|(i, _)| i == id)
        .map(|(_, n)| format!("{n:?}"))
        .unwrap_or_else(|| id.to_string())
}

/// Lists the collections; returns their IDs and names.
async fn print_collections(
    addr: SocketAddr,
    token: Option<&str>,
) -> anyhow::Result<Vec<(String, String)>> {
    let list = get_json(addr, token, "/api/v1/codex/collections").await?;
    let list = list.as_array().cloned().unwrap_or_default();
    println!("collections: {}", list.len());
    let mut names = Vec::new();
    for c in &list {
        let doc = &c["collection"];
        let counts = &c["counts"];
        let total: u64 = counts
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, n)| n.as_u64())
            .sum();
        println!(
            "  {}{} {} ({}): {} documents ({} presentations, {} playlists, {} media), {} KB documents, {} MB media{}",
            if c["active"] == true { "*" } else { " " },
            doc["name"],
            doc["id"].as_str().unwrap_or_default(),
            doc["purpose"].as_str().unwrap_or("content"),
            total,
            counts["presentations"],
            counts["playlists"],
            counts["mediaAssets"],
            c["documentBytes"].as_u64().unwrap_or(0) / 1024,
            c["mediaBytes"].as_u64().unwrap_or(0) >> 20,
            if c["readOnly"] == true {
                ", read only (newer schema)"
            } else {
                ""
            }
        );
        if let (Some(id), Some(name)) = (doc["id"].as_str(), doc["name"].as_str()) {
            names.push((id.to_string(), name.to_string()));
        }
    }
    Ok(names)
}

/// Prints `GET /codex`: the role, the active collection, the media library and, on a show box,
/// the sync.
fn print_status(st: &Value) {
    println!(
        "codex {}: role {}, showing {} ({}), {} collections, {} documents, trash {}, deleted collections {}",
        st["id"].as_str().unwrap_or_default(),
        st["role"],
        st["collectionName"],
        st["collection"].as_str().unwrap_or_default(),
        st["collections"],
        st["documents"],
        st["trash"],
        st["deletedCollections"],
    );
    let lib = &st["mediaLibrary"];
    println!(
        "  media library: available {}, served {}{}",
        lib["available"],
        lib["served"],
        lib["problem"]
            .as_str()
            .map(|p| format!(", problem: {p}"))
            .unwrap_or_default()
    );
    if let Some(d) = st["dormantCollection"].as_str() {
        println!("  own Codex dormant (its collection {d})");
    }
    let sync = &st["sync"];
    if sync.is_object() {
        println!(
            "  sync: reachable {}, Codex box {} ({}), seq {} of {} (behind {}), last {}{}",
            sync["reachable"],
            sync["serverId"].as_str().unwrap_or("?"),
            sync["fingerprint"].as_str().unwrap_or("?"),
            sync["seq"],
            sync["codexSeq"],
            sync["behind"],
            sync["lastSync"].as_str().unwrap_or("never"),
            sync["problem"]
                .as_str()
                .map(|p| format!(", problem: {p}"))
                .unwrap_or_default()
        );
        let m = &sync["media"];
        println!(
            "  media cache: {} pinned, {} cached, {} copying, {} missing, {} failed, {} of {} MB",
            m["pinned"],
            m["cached"],
            m["copying"],
            m["missing"],
            m["failed"],
            m["bytes"].as_u64().unwrap_or(0) >> 20,
            m["capBytes"].as_u64().unwrap_or(0) >> 20
        );
    }
}

/// `--codex`: the Codex status, the collections, a search, then the Codex's events.
async fn codex(a: &Args, info: &Value) -> anyhow::Result<()> {
    let tok = a.token.as_deref();
    let caps = info["capabilities"].as_array().cloned().unwrap_or_default();
    if !caps.contains(&json!("codex")) {
        println!("the server doesn't announce the `codex` capability (an older server?)");
    }
    print_status(&get_json(a.addr, tok, "/api/v1/codex").await?);
    let mut names = print_collections(a.addr, tok).await?;

    if let Some(q) = &a.codex_search {
        let mut path = format!("/api/v1/codex/search?q={}&limit=20", query_value(q));
        if let Some(c) = &a.codex_collection {
            path.push_str(&format!("&collection={}", query_value(c)));
        }
        let t = Instant::now();
        let r = get_json(a.addr, tok, &path).await?;
        let hits = r["hits"].as_array().cloned().unwrap_or_default();
        println!(
            "search {q:?}: {} hits ({} shown) in {} ms",
            r["total"],
            hits.len(),
            ms(t.elapsed())
        );
        for h in &hits {
            let snippet: String = h["snippet"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    let text = p["text"].as_str().unwrap_or_default();
                    if p["mark"] == true {
                        format!("[{text}]")
                    } else {
                        text.to_string()
                    }
                })
                .collect();
            println!(
                "  {} {} in {}: {}: {}{}",
                h["kind"].as_str().unwrap_or_default(),
                h["title"],
                h["collectionName"],
                h["field"].as_str().unwrap_or_default(),
                snippet,
                h["slideIndex"]
                    .as_u64()
                    .map(|i| format!(" (slide #{i})"))
                    .unwrap_or_default()
            );
        }
    }

    if a.watch_seconds.is_some_and(|s| s <= 0.0) {
        return Ok(());
    }
    let mut url = format!(
        "ws://{}/api/v1/ws?protocolVersion={}",
        a.addr,
        protocol::PROTOCOL_VERSION
    );
    if let Some(t) = tok {
        url.push_str(&format!("&token={t}"));
    }
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await?;
    match a.watch_seconds {
        Some(s) => println!("watching the Codex's events for {s} s"),
        None => println!("watching the Codex's events (Ctrl-C to stop)"),
    }
    let t0 = Instant::now();
    loop {
        let wait = match a.watch_seconds {
            Some(s) => {
                let left = s - t0.elapsed().as_secs_f64();
                if left <= 0.0 {
                    break;
                }
                Duration::from_secs_f64(left)
            }
            None => Duration::from_secs(3600),
        };
        let m = match tokio::time::timeout(wait, ws.next()).await {
            Err(_) => continue,
            Ok(None) => anyhow::bail!("socket closed"),
            Ok(Some(m)) => m?,
        };
        let Message::Text(t) = m else {
            if let Message::Close(f) = m {
                anyhow::bail!("closed: {f:?}");
            }
            continue;
        };
        let v: Value = serde_json::from_str(t.as_str())?;
        let at = format!("{:>7.1} s", t0.elapsed().as_secs_f64());
        match v["type"].as_str() {
            Some("hello") => println!("{at} hello: {}", v["server"]["name"]),
            Some("codexChanged") => {
                let parts: Vec<&str> = v["parts"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                println!(
                    "{at} codexChanged [{}] collection {}",
                    parts.join(", "),
                    collection_name(&names, &v["collection"])
                );
                if parts.contains(&"collections") {
                    names = print_collections(a.addr, tok).await?;
                }
                if parts.contains(&"status") {
                    print_status(&get_json(a.addr, tok, "/api/v1/codex").await?);
                }
            }
            Some("documentChanged") => println!(
                "{at} documentChanged {} {} {} collection {}{}{}",
                v["kind"].as_str().unwrap_or_default(),
                v["id"].as_str().unwrap_or_default(),
                v["change"].as_str().unwrap_or_default(),
                collection_name(&names, &v["collection"]),
                v["revision"]
                    .as_str()
                    .map(|r| format!(" revision {r}"))
                    .unwrap_or_default(),
                v["clientId"]
                    .as_str()
                    .map(|c| format!(" by client {c}"))
                    .unwrap_or_default()
            ),
            Some("notice") => println!("{at} notice [{}]: {}", v["level"], v["message"]),
            _ => {}
        }
    }
    let _ = ws.close(None).await;
    Ok(())
}
