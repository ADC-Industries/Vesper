//! Codex media: resumable uploads into the media library (chunks with `Content-Range`, a wrong
//! offset, a restart in the middle, the SHA-256 check, dedup by the library, a bin), probing
//! (shared with the library's own) and thumbnails on the media worker, serving by SHA-256 with
//! `Range` and `ETag`, the limits (size, free space, open uploads), stale uploads, storage, the
//! library's "in use" marks and missing library media. Media are synthetic: PNGs drawn here, a
//! clip and a tone from the system `ffmpeg` CLI.

use crate::common::*;
use model::MediaBin;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};
use vesper_server::codex::CodexOptions;
use vesper_server::{RunningServer, ServerConfig, start};

async fn start_with(fx: &Fixture, codex: CodexOptions) -> RunningServer {
    start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost: true,
        mdns: false,
        site: vesper_server::site::SiteConfig::off(),
        codex,
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts")
}

fn ok(r: &Resp, status: u16, what: &str) -> Value {
    assert_eq!(
        r.status,
        status,
        "{what}: {}",
        String::from_utf8_lossy(&r.body)
    );
    r.json()
}

pub fn sha_hex(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

/// A PNG of `w`×`h` noise (deterministic per `seed`): it barely compresses.
pub fn noise_png(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    let img = image::RgbImage::from_fn(w, h, |_, _| {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        let b = x.to_le_bytes();
        image::Rgb([b[0], b[1], b[2]])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

pub async fn create_upload(addr: SocketAddr, body: Value) -> Resp {
    http(addr, "POST", "/api/v1/codex/uploads", &[], Some(body)).await
}

pub async fn put_chunk(addr: SocketAddr, id: &str, bytes: &[u8], start: u64, total: u64) -> Resp {
    let end = start + bytes.len() as u64 - 1;
    let range = format!("bytes {start}-{end}/{total}");
    http_bytes(
        addr,
        "PUT",
        &format!("/api/v1/codex/uploads/{id}"),
        &[("Content-Range", &range)],
        bytes,
    )
    .await
}

pub async fn finish(addr: SocketAddr, id: &str, body: Option<Value>) -> Resp {
    http(
        addr,
        "POST",
        &format!("/api/v1/codex/uploads/{id}/finish"),
        &[],
        body,
    )
    .await
}

/// Uploads `bytes` as `name` in one chunk; the finish response.
pub async fn upload(addr: SocketAddr, collection: &str, name: &str, bytes: &[u8]) -> Value {
    let r = create_upload(
        addr,
        json!({"collection": collection, "name": name, "size": bytes.len(), "sha256": sha_hex(bytes)}),
    )
    .await;
    let id = ok(&r, 201, "create")["id"].as_str().unwrap().to_string();
    ok(
        &put_chunk(addr, &id, bytes, 0, bytes.len() as u64).await,
        200,
        "chunk",
    );
    ok(&finish(addr, &id, None).await, 201, "finish")
}

/// Uploads `bytes` in place of the file of the asset `replaces`; the finish response.
pub async fn upload_replacing(
    addr: SocketAddr,
    collection: &str,
    name: &str,
    bytes: &[u8],
    replaces: &str,
) -> Value {
    let r = create_upload(
        addr,
        json!({"collection": collection, "name": name, "size": bytes.len(),
               "sha256": sha_hex(bytes), "replaces": replaces}),
    )
    .await;
    let id = ok(&r, 201, "create")["id"].as_str().unwrap().to_string();
    ok(
        &put_chunk(addr, &id, bytes, 0, bytes.len() as u64).await,
        200,
        "chunk",
    );
    ok(&finish(addr, &id, None).await, 201, "finish")
}

/// Waits for the media worker to fill an asset's probe (or mark it unsupported).
pub async fn probed(addr: SocketAddr, asset: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let info = get_json(addr, &format!("/api/v1/media/{asset}")).await;
        let a = &info["asset"];
        if a["probe"]["codec"].is_string() || a["status"] == "unsupported" {
            return a.clone();
        }
        assert!(Instant::now() < deadline, "no probe: {info}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn ff(args: &[&str]) {
    let out = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .expect("system ffmpeg");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A 2 s, 25 fps, 320×180 test clip with a sine (H.264, AAC).
pub fn clip(path: &Path) {
    ff(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=s=320x180:r=25:d=2",
        "-f",
        "lavfi",
        "-i",
        "sine=f=440:d=2",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
        path.to_str().unwrap(),
    ]);
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn uploads_resume_check_the_sha_and_deduplicate() {
    let fx = fixture("codex-upload");
    let bin = MediaBin {
        name: "Backgrounds".into(),
        ..MediaBin::default()
    };
    write_doc(&fx.dir, "portable", "mediaBin", &bin.id.0.to_string(), &bin);
    let opts = CodexOptions {
        max_upload_bytes: 8 << 20,
        ..CodexOptions::default()
    };
    let server = start_with(&fx, opts.clone()).await;
    let addr = server.addr;
    let ws = Client::connect(addr, "protocolVersion=1").await;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();

    // A noise PNG over 1 MiB, sent in three chunks.
    let png = noise_png(700, 600, 7);
    let size = png.len() as u64;
    assert!(size > 1 << 20, "{size}");
    let sha = sha_hex(&png);
    let r = create_upload(
        addr,
        json!({"collection": main, "name": "Noise.PNG", "size": size, "bin": bin.id.0,
               "relativePath": "Backgrounds/Noise.PNG"}),
    )
    .await;
    let info = ok(&r, 201, "create");
    let id = info["id"].as_str().unwrap().to_string();
    assert_eq!(info["offset"], 0);
    assert_eq!(info["size"], size);
    assert!(info["expiresAt"].as_str().unwrap() > info["createdAt"].as_str().unwrap());
    assert!(fx.dir.join(format!("codex/uploads/{id}.part")).is_file());
    let list = get_json(addr, "/api/v1/codex/uploads").await;
    assert_eq!(list[0]["id"], id.as_str());

    let a = (size / 3) as usize;
    let r = put_chunk(addr, &id, &png[..a], 0, size).await;
    assert_eq!(ok(&r, 200, "first chunk")["offset"], a as u64);
    // A chunk from the wrong place: 409 with where the upload stands.
    let r = put_chunk(addr, &id, &png[..10], 0, size).await;
    let c = ok(&r, 409, "wrong offset");
    assert_eq!(c["offset"], a as u64);
    assert_eq!(c["error"]["code"], "conflict");
    // Without Content-Range, or longer than it says: 400, nothing kept.
    let r = http_bytes(
        addr,
        "PUT",
        &format!("/api/v1/codex/uploads/{id}"),
        &[],
        &png[a..a + 5],
    )
    .await;
    assert_eq!(
        ok(&r, 400, "no Content-Range")["error"]["field"],
        "Content-Range"
    );
    let range = format!("bytes {a}-{}/{size}", a + 9);
    let r = http_bytes(
        addr,
        "PUT",
        &format!("/api/v1/codex/uploads/{id}"),
        &[("Content-Range", &range)],
        &png[a..a + 20],
    )
    .await;
    assert_eq!(r.status, 400, "longer than its range");
    let at = get_json(addr, &format!("/api/v1/codex/uploads/{id}")).await;
    assert_eq!(at["offset"], a as u64, "{at}");
    // Finishing early: 409 with the offset.
    let r = finish(addr, &id, Some(json!({"sha256": sha}))).await;
    assert_eq!(ok(&r, 409, "finish early")["offset"], a as u64);

    // A restart in the middle: the upload goes on from where it was (the hash catches up).
    drop(ws);
    server.shutdown().await.unwrap();
    let server = start_with(&fx, opts.clone()).await;
    let addr = server.addr;
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let at = get_json(addr, &format!("/api/v1/codex/uploads/{id}")).await;
    assert_eq!(at["offset"], a as u64, "{at}");
    let b = 2 * a;
    ok(
        &put_chunk(addr, &id, &png[a..b], a as u64, size).await,
        200,
        "second chunk",
    );
    // Bytes past the offset (as a block that failed part-way leaves them): the next chunk
    // goes at the offset, over them, not after them.
    {
        use std::io::Write as _;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(fx.dir.join(format!("codex/uploads/{id}.part")))
            .unwrap();
        f.write_all(&[0xAB; 300]).unwrap();
    }
    let r = put_chunk(addr, &id, &png[b..], b as u64, size).await;
    assert_eq!(ok(&r, 200, "last chunk")["offset"], size);

    // Finished: checked, taken into the media library under its name in the collection's
    // folder, a library asset in the bin.
    let r = finish(addr, &id, Some(json!({"sha256": sha}))).await;
    let done = ok(&r, 201, "finish");
    assert_eq!(done["collection"], main.as_str());
    assert_eq!(done["deduplicated"], false);
    let asset = &done["asset"];
    let aid = asset["id"].as_str().unwrap().to_string();
    assert!(asset.get("kind").is_none(), "image, the default: {asset}");
    assert_eq!(asset["name"], "Noise");
    let item = asset["location"]["id"].as_str().unwrap().to_string();
    assert_eq!(item.len(), 16, "{asset}");
    assert_eq!(
        asset["location"],
        json!({"type": "library", "id": item, "sha256": sha})
    );
    assert_eq!(asset["origin"]["relativePath"], "Backgrounds/Noise.PNG");
    assert_eq!(asset["status"], "online");
    assert!(done["binItem"].is_string(), "{done}");
    let file = server.shared.library.resolve(&item).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), png);
    assert_eq!(file.file_name().unwrap(), "Noise.PNG");
    let folder = get_json(addr, "/api/v1/codex").await["collectionName"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        file.parent().unwrap(),
        fx.dir.join("codex/library").join(&folder),
        "the collection's folder"
    );
    assert!(files_in(&fx.dir.join("codex/uploads")).is_empty());
    assert_eq!(
        call_status(addr, "GET", &format!("/api/v1/codex/uploads/{id}")).await,
        404
    );
    let e = loop {
        let m = ws.next_any().await;
        if m["type"] == "documentChanged" && m["id"] == aid.as_str() {
            break m;
        }
    };
    assert_eq!(e["change"], "added");
    assert_eq!(e["collection"], main.as_str());
    let bins = get_json(addr, &format!("/api/v1/media-bins/{}", bin.id.0)).await;
    assert_eq!(bins["items"][0]["asset"], aid.as_str(), "{bins}");
    let changes = get_json(
        addr,
        &format!("/api/v1/codex/collections/{main}/changes?since=0"),
    )
    .await;
    assert!(
        changes["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == aid.as_str() && c["kind"] == "mediaAsset"),
        "{changes}"
    );

    // The media worker probes it and makes its thumbnail.
    let a = probed(addr, &aid).await;
    assert_eq!(a["probe"]["width"], 700, "{a}");
    assert_eq!(a["probe"]["height"], 600);
    assert_eq!(a["probe"]["codec"], "png");
    assert!(fx.dir.join(format!("codex/thumbs/{sha}-320.jpg")).is_file());
    let r = http(
        addr,
        "GET",
        &format!("/api/v1/codex/blobs/{sha}.png/thumbnail"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    let thumb = image::load_from_memory(&r.body).unwrap();
    assert_eq!(thumb.width(), 320);
    let etag = r.header("etag").unwrap().to_string();
    let r = http(
        addr,
        "GET",
        &format!("/api/v1/codex/blobs/{sha}.png/thumbnail"),
        &[("If-None-Match", &etag)],
        None,
    )
    .await;
    assert_eq!(r.status, 304);

    // The blob itself, with ETag and Range.
    let path = format!("/api/v1/codex/blobs/{sha}.png");
    let r = http(addr, "GET", &path, &[], None).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, png);
    assert_eq!(r.header("etag").unwrap(), format!("\"{sha}\""));
    assert_eq!(r.header("content-type").unwrap(), "image/png");
    let r = http(addr, "GET", &path, &[("Range", "bytes=10-19")], None).await;
    assert_eq!(r.status, 206);
    assert_eq!(r.body, &png[10..20]);
    let r = http(
        addr,
        "GET",
        &path,
        &[("If-None-Match", &format!("\"{sha}\""))],
        None,
    )
    .await;
    assert_eq!(r.status, 304);
    for (p, status) in [
        (format!("/api/v1/codex/blobs/{}.png", "0".repeat(64)), 404),
        ("/api/v1/codex/blobs/..%2Fcodex.json".to_string(), 400),
    ] {
        assert_eq!(call_status(addr, "GET", &p).await, status, "{p}");
    }

    // The same bytes again (SHA-256 given up front, one chunk): deduplicated by the library.
    // The collection has an asset for them: that one, no second asset.
    let again = upload(addr, &main, "Copy.png", &png).await;
    assert_eq!(again["deduplicated"], true);
    assert_eq!(again["existing"], true);
    assert_eq!(again["asset"]["id"], aid.as_str());
    assert_eq!(again["asset"]["location"], asset["location"]);
    assert!(again.get("binItem").is_none());
    assert!(
        !fx.dir
            .join("codex/library")
            .join(&folder)
            .join("Copy.png")
            .exists()
    );
    assert!(files_in(&fx.dir.join("codex/uploads")).is_empty());

    // Endings the library takes since IDN Beta3 (`.tga`, `.avif`), and one only Vesper knew
    // (`.jpe`, taken through the library's host endings): uploaded into the library.
    for (i, name) in ["Old.tga", "Still.avif", "Photo.jpe"]
        .into_iter()
        .enumerate()
    {
        let done = upload(addr, &main, name, &noise_png(4, 4, 40 + i as u32)).await;
        assert_eq!(done["asset"]["location"]["type"], "library", "{done}");
        assert!(done["asset"].get("kind").is_none(), "a picture: {done}");
        let id = done["asset"]["id"].as_str().unwrap();
        let file = server
            .shared
            .library
            .resolve(done["asset"]["location"]["id"].as_str().unwrap())
            .unwrap();
        assert_eq!(file.file_name().unwrap(), name);
        // Written as schema 3: an older server refuses it rather than lose the location.
        let doc: Value = serde_json::from_slice(
            &std::fs::read(fx.dir.join(format!(
                "codex/collections/{main}/portable/mediaAsset/{id}.json"
            )))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(doc["schemaVersion"], 3, "{doc}");
    }

    // A wrong SHA-256: refused, and the upload is gone.
    let small = noise_png(16, 16, 3);
    let r = create_upload(
        addr,
        json!({"collection": main, "name": "Small.png", "size": small.len()}),
    )
    .await;
    let bad = ok(&r, 201, "create")["id"].as_str().unwrap().to_string();
    ok(
        &put_chunk(addr, &bad, &small, 0, small.len() as u64).await,
        200,
        "chunk",
    );
    let r = finish(addr, &bad, None).await;
    assert_eq!(
        ok(&r, 400, "finish without a sha")["error"]["field"],
        "sha256"
    );
    let r = finish(addr, &bad, Some(json!({"sha256": "0".repeat(64)}))).await;
    assert_eq!(ok(&r, 400, "a wrong sha")["error"]["field"], "sha256");
    assert_eq!(
        call_status(addr, "GET", &format!("/api/v1/codex/uploads/{bad}")).await,
        404
    );
    assert!(server.shared.library.by_sha(&sha_hex(&small)).is_none());

    // Cancelled.
    let r = create_upload(
        addr,
        json!({"collection": main, "name": "Gone.png", "size": 100}),
    )
    .await;
    let gone = ok(&r, 201, "create")["id"].as_str().unwrap().to_string();
    assert_eq!(
        call_status(addr, "DELETE", &format!("/api/v1/codex/uploads/{gone}")).await,
        204
    );
    assert!(!fx.dir.join(format!("codex/uploads/{gone}.part")).exists());

    // Validation.
    for (body, field) in [
        (
            json!({"collection": main, "name": "Big.mp4", "size": 9u64 << 20}),
            "size",
        ),
        (
            json!({"collection": main, "name": "notes.txt", "size": 10}),
            "name",
        ),
        (
            json!({"collection": main, "name": "Clip.xyz", "size": 10}),
            "name",
        ),
        (
            json!({"collection": main, "name": "a.png", "size": 0}),
            "size",
        ),
        (
            json!({"collection": "0b3c5a51-7f63-4d1c-9a33-3a7c0e1f8d2e", "name": "a.png", "size": 10}),
            "collection",
        ),
        (
            json!({"collection": main, "name": "a.png", "size": 10, "sha256": "ABC"}),
            "sha256",
        ),
        (
            json!({"collection": main, "name": "a.png", "size": 10, "bin": "0b3c5a51-7f63-4d1c-9a33-3a7c0e1f8d2e"}),
            "bin",
        ),
    ] {
        let r = create_upload(addr, body.clone()).await;
        assert_eq!(ok(&r, 400, &body.to_string())["error"]["field"], field);
    }

    // An upload left alone past its expiry is dropped by the upkeep.
    let r = create_upload(
        addr,
        json!({"collection": main, "name": "Stale.png", "size": small.len()}),
    )
    .await;
    let stale = ok(&r, 201, "create")["id"].as_str().unwrap().to_string();
    ok(
        &put_chunk(addr, &stale, &small[..10], 0, small.len() as u64).await,
        200,
        "chunk",
    );
    std::fs::write(fx.dir.join("codex/uploads/leftover.part"), b"x").unwrap();
    let dropped = server
        .shared
        .uploads
        .clean(
            &server.shared,
            std::time::SystemTime::now() + Duration::from_secs(3600),
        )
        .await;
    assert_eq!(dropped, 0, "not stale an hour on");
    assert!(fx.dir.join("codex/uploads/leftover.part").exists());
    let later = std::time::SystemTime::now() + Duration::from_secs(2 * 86_400);
    let dropped = server.shared.uploads.clean(&server.shared, later).await;
    assert_eq!(dropped, 1);
    assert_eq!(
        call_status(addr, "GET", &format!("/api/v1/codex/uploads/{stale}")).await,
        404
    );
    assert!(files_in(&fx.dir.join("codex/uploads")).is_empty());
    let _ = std::fs::remove_dir_all(&fx.dir);
}

pub async fn call_status(addr: SocketAddr, method: &str, path: &str) -> u16 {
    http(addr, method, path, &[], None).await.status
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn video_and_audio_uploads_are_probed() {
    let fx = fixture("codex-probe");
    let server = start_with(&fx, CodexOptions::default()).await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let tmp = fx.dir.join("src");
    std::fs::create_dir_all(&tmp).unwrap();
    clip(&tmp.join("clip.mp4"));
    ff(&[
        "-f",
        "lavfi",
        "-i",
        "sine=f=660:d=1.5",
        tmp.join("tone.wav").to_str().unwrap(),
    ]);

    let mp4 = std::fs::read(tmp.join("clip.mp4")).unwrap();
    let done = upload(addr, &main, "Clip.mp4", &mp4).await;
    assert_eq!(done["asset"]["kind"], "video");
    let a = probed(addr, done["asset"]["id"].as_str().unwrap()).await;
    let p = &a["probe"];
    assert_eq!(
        (&p["width"], &p["height"]),
        (&json!(320), &json!(180)),
        "{a}"
    );
    assert_eq!(p["codec"], "h264");
    assert_eq!(p["frameRate"], json!({"num": 25, "den": 1}));
    assert_eq!(p["hasAudio"], true);
    assert!((p["duration"].as_f64().unwrap() - 2.0).abs() < 0.2, "{p}");
    let sha = sha_hex(&mp4);
    assert!(fx.dir.join(format!("codex/thumbs/{sha}-320.jpg")).is_file());
    // The library lists the same facts, from the same probe: the file was probed once.
    let item = done["asset"]["location"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let lib = server.shared.library.server().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let it = loop {
        let it = lib.item(&item).unwrap();
        if it.duration.is_some() {
            break it;
        }
        assert!(
            Instant::now() < deadline,
            "the library never probed it: {it:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!((it.width, it.height), (Some(320), Some(180)));
    assert_eq!(it.codec.as_deref(), Some("h264"));
    assert_eq!(it.audio_tracks, Some(1));
    let path = server.shared.library.resolve(&item).unwrap();
    assert_eq!(server.shared.library.probes.runs(&path), 1, "one probe");

    let wav = std::fs::read(tmp.join("tone.wav")).unwrap();
    let done = upload(addr, &main, "Tone.wav", &wav).await;
    assert_eq!(done["asset"]["kind"], "audio");
    let a = probed(addr, done["asset"]["id"].as_str().unwrap()).await;
    assert_eq!(a["probe"]["hasAudio"], true, "{a}");
    assert!((a["probe"]["duration"].as_f64().unwrap() - 1.5).abs() < 0.1);
    // Audio has no picture: the generic one, not stored.
    let r = http(
        addr,
        "GET",
        &format!("/api/v1/codex/blobs/{}.wav/thumbnail", sha_hex(&wav)),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    assert!(
        !fx.dir
            .join(format!("codex/thumbs/{}-320.jpg", sha_hex(&wav)))
            .exists()
    );
    // (Audio never has a picture: the generic one is kept like a thumbnail.)
    assert!(r.header("etag").is_some(), "{:?}", r.headers);

    // A file that isn't what its name says: unsupported.
    let done = upload(addr, &main, "Broken.mp4", b"not a video at all").await;
    let a = probed(addr, done["asset"]["id"].as_str().unwrap()).await;
    assert_eq!(a["status"], "unsupported", "{a}");
    // A video with no frame decoded (yet): the generic picture, kept only briefly and without
    // the real thumbnail's ETag.
    let r = http(
        addr,
        "GET",
        &format!(
            "/api/v1/codex/blobs/{}.mp4/thumbnail",
            sha_hex(b"not a video at all")
        ),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    assert!(r.header("etag").is_none(), "{:?}", r.headers);
    let cache = r.header("cache-control").unwrap_or_default();
    assert!(!cache.contains("immutable"), "{cache}");
    let _ = std::fs::remove_dir_all(&fx.dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_limits_free_space_and_open_uploads() {
    let fx = fixture("codex-limits");
    // A server that keeps more free than any disk has: 507.
    let server = start_with(
        &fx,
        CodexOptions {
            keep_free_bytes: u64::MAX / 4,
            ..CodexOptions::default()
        },
    )
    .await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let r = create_upload(
        addr,
        json!({"collection": main, "name": "a.png", "size": 10}),
    )
    .await;
    assert_eq!(
        ok(&r, 507, "no space")["error"]["code"],
        "insufficientStorage"
    );
    server.shutdown().await.unwrap();

    // One open upload at most: the second waits (429).
    let server = start_with(
        &fx,
        CodexOptions {
            max_open_uploads: 1,
            ..CodexOptions::default()
        },
    )
    .await;
    let addr = server.addr;
    let body = json!({"collection": main, "name": "a.png", "size": 10});
    let first = ok(&create_upload(addr, body.clone()).await, 201, "first");
    let r = create_upload(addr, body.clone()).await;
    assert_eq!(ok(&r, 429, "second")["error"]["code"], "rateLimited");
    let id = first["id"].as_str().unwrap();
    assert_eq!(
        call_status(addr, "DELETE", &format!("/api/v1/codex/uploads/{id}")).await,
        204
    );
    ok(&create_upload(addr, body).await, 201, "after cancelling");
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// Waits until the Codex marks exactly `ids` in use in the media library (IDN's read-only
/// `marked`).
async fn marked(server: &RunningServer, ids: &[&String]) {
    let mut want: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    want.sort();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let now = server.shared.library.marked();
        if now == want {
            return;
        }
        assert!(Instant::now() < deadline, "marked {now:?}, not {want:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn storage_usage_marks_and_missing_library_media() {
    let fx = fixture("codex-storage");
    let server = start_with(&fx, CodexOptions::default()).await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let r = http(
        addr,
        "POST",
        "/api/v1/codex/collections",
        &[],
        Some(json!({"name": "Youth"})),
    )
    .await;
    let youth = ok(&r, 201, "create")["collection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (a, b, c) = (
        noise_png(64, 64, 1),
        noise_png(80, 40, 2),
        noise_png(30, 30, 3),
    );
    let (sa, sb) = (sha_hex(&a), sha_hex(&b));
    let ua = upload(addr, &main, "A.png", &a).await;
    let shared = upload(addr, &youth, "A again.png", &a).await;
    assert_eq!(shared["deduplicated"], true);
    let ub = upload(addr, &youth, "B.png", &b).await;
    let item = |u: &Value| u["asset"]["location"]["id"].as_str().unwrap().to_string();
    let (ia, ib) = (item(&ua), item(&ub));
    assert!(
        fx.dir.join("codex/library/Youth/B.png").is_file(),
        "Youth's folder"
    );
    // A file put into the library by hand: no document uses it.
    let lib_dir = fx.dir.join("codex/library");
    std::fs::write(lib_dir.join("Loose.png"), &c).unwrap();
    let bg_asset = server
        .shared
        .data()
        .content
        .assets
        .get(&fx.asset.id)
        .cloned()
        .unwrap();
    let ibg = match &bg_asset.location {
        model::AssetLocation::Library { id, .. } => id.clone(),
        l => panic!("the fixture's background moved into the library: {l:?}"),
    };
    let bg = std::fs::metadata(server.shared.library.resolve(&ibg).unwrap())
        .unwrap()
        .len();
    let (la, lb, lc) = (a.len() as u64, b.len() as u64, c.len() as u64);

    let s = get_json(addr, "/api/v1/codex/storage").await;
    assert_eq!(
        s["media"],
        json!({"files": 4, "bytes": bg + la + lb + lc}),
        "{s}"
    );
    assert_eq!(s["shared"], json!({"files": 1, "bytes": la}));
    assert_eq!(s["unreferenced"], json!({"files": 1, "bytes": lc}));
    assert_eq!(s["managed"], json!({"files": 0, "bytes": 0}));
    assert_eq!(s["library"]["items"], 4, "{s}");
    assert_eq!(s["library"]["trashItems"], 0);
    assert_eq!(s["library"]["trashDays"], 30);
    assert!(s["library"]["bytes"].as_u64().unwrap() >= bg + la + lb + lc);
    let d = &s["disk"];
    assert!(d["total"].as_u64().unwrap() >= d["available"].as_u64().unwrap());
    assert!(d["keepFree"].as_u64().unwrap() >= 2 << 30);
    assert_eq!(s["uploads"]["files"], 0);
    let col = |s: &Value, id: &str| {
        s["collections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["collection"] == id)
            .cloned()
            .unwrap()
    };
    let m = col(&s, &main);
    assert_eq!(m["assets"], 2, "{m}");
    assert_eq!(m["mediaBytes"], bg + la);
    assert_eq!(m["uniqueMediaBytes"], bg);
    assert_eq!(m["missing"], 0);
    assert!(m["documents"].as_u64().unwrap() >= 5);
    assert!(m["documentBytes"].as_u64().unwrap() > 0);
    let y = col(&s, &youth);
    assert_eq!(y["name"], "Youth");
    assert_eq!(
        (&y["assets"], &y["mediaBytes"], &y["uniqueMediaBytes"]),
        (&json!(2), &json!(la + lb), &json!(lb))
    );

    // Marked in use: every item a document uses, nothing else. A marked item can't be
    // deleted from the library without its name typed back; the loose one can.
    marked(&server, &[&ibg, &ia, &ib]).await;
    let lib = server.shared.library.server().unwrap();
    let refused = lib.delete(&ia, "test", None).unwrap_err();
    assert!(refused.contains("vesper-codex"), "{refused}");
    let loose = lib.id_of("Loose.png").unwrap();
    lib.delete(&loose, "test", None).unwrap();

    // Youth into the collection trash: its media count as the trash's, still marked.
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/codex/collections/{youth}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 204);
    let s = get_json(addr, "/api/v1/codex/storage").await;
    assert_eq!(s["collections"].as_array().unwrap().len(), 1);
    assert_eq!(s["trash"]["collections"], 1);
    assert!(s["trash"]["collectionBytes"].as_u64().unwrap() > 0);
    assert_eq!(s["trash"]["mediaBytes"], lb, "{s}");
    assert_eq!(
        s["unreferenced"]["files"], 0,
        "the loose file is in the library's trash"
    );
    assert_eq!(s["library"]["trashItems"], 1);
    tokio::time::sleep(Duration::from_millis(800)).await;
    marked(&server, &[&ibg, &ia, &ib]).await;
    assert_eq!(
        server
            .shared
            .library
            .server()
            .unwrap()
            .marked("vesper-codex")
            .len(),
        3,
        "IDN's own view"
    );
    assert!(lib.delete(&ib, "test", None).is_err());

    // Deleted for good: B is no longer used, no longer marked; the Codex leaves it in the
    // library (deleting it is the owner's action). Its thumbnail stays while it is there.
    let r = http(
        addr,
        "DELETE",
        &format!("/api/v1/codex/collection-trash/{youth}"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 204);
    marked(&server, &[&ibg, &ia]).await;
    assert!(server.shared.library.resolve(&ib).is_ok(), "still there");
    let s = get_json(addr, "/api/v1/codex/storage").await;
    assert_eq!(s["unreferenced"], json!({"files": 1, "bytes": lb}), "{s}");
    let thumbs = fx.dir.join("codex/thumbs");
    std::fs::create_dir_all(&thumbs).unwrap();
    std::fs::write(thumbs.join(format!("{sb}-320.jpg")), b"jpeg").unwrap();
    std::fs::write(thumbs.join(format!("{}-320.jpg", "0".repeat(64))), b"jpeg").unwrap();
    let w = &server.shared.codex_writer;
    w.tick(&server.shared.codex_server(), std::time::SystemTime::now())
        .await;
    assert!(thumbs.join(format!("{sb}-320.jpg")).exists());
    assert!(
        !thumbs.join(format!("{}-320.jpg", "0".repeat(64))).exists(),
        "a thumbnail of media the Codex doesn't have"
    );
    // The owner deletes B in the library: its thumbnail goes at the next upkeep.
    lib.delete(&ib, "owner", None).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while server.shared.library.resolve(&ib).is_ok() {
        assert!(Instant::now() < deadline, "the Codex never saw it go");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    w.tick(&server.shared.codex_server(), std::time::SystemTime::now())
        .await;
    assert!(!thumbs.join(format!("{sb}-320.jpg")).exists());
    assert!(
        server.shared.library.by_sha(&sa).is_some(),
        "main still uses A"
    );

    // The owner deletes the main collection's background anyway (its name typed back): it is
    // missing at once, and the collection's media are announced.
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    let name = lib.item(&ibg).unwrap().name().to_string();
    let overridden = lib.delete(&ibg, "owner", Some(&name)).unwrap();
    assert_eq!(overridden, ["vesper-codex"]);
    let e = loop {
        let m = ws.next_any().await;
        if m["type"] == "codexChanged" && m["parts"] == json!(["media"]) {
            break m;
        }
    };
    assert_eq!(e["collection"], main.as_str());
    let s = get_json(addr, "/api/v1/codex/storage").await;
    assert_eq!(s["collections"][0]["missing"], 1);
    let media = get_json(
        addr,
        &format!("/api/v1/codex/collections/{main}/media?status=missing"),
    )
    .await;
    assert_eq!(
        media[0]["asset"]["id"],
        fx.asset.id.0.to_string(),
        "{media}"
    );
    // Several, as a client that encodes its query sends them (`,` as `%2C`).
    assert_eq!(
        get_json(
            addr,
            &format!("/api/v1/codex/collections/{main}/media?status=missing%2Cunsupported"),
        )
        .await,
        media
    );
    // Put back from the library's trash: available again (the same ID).
    lib.restore(&ibg).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while server.shared.library.resolve(&ibg).is_err() {
        assert!(Instant::now() < deadline, "never back");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let media = get_json(
        addr,
        &format!("/api/v1/codex/collections/{main}/media?status=missing"),
    )
    .await;
    assert_eq!(media, json!([]));
    server.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&fx.dir);
}

/// Asset-only writes in the active collection (an upload, its probe, a relink) keep
/// `documentChanged` and the change log, but re-render the screens only when something live
/// uses the asset: an upload while a slide is live leaves the content generation alone;
/// relinking the live slide's background renews it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_writes_re_render_only_when_live_uses_them() {
    let fx = fixture("codex-media-live");
    let server = start_with(&fx, CodexOptions::default()).await;
    let addr = server.addr;
    let collection = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let mut ws = Client::connect(addr, "protocolVersion=1").await;
    // The song's first slide is live, with its background (the fixture's asset).
    ws.ok(json!({"type": "triggerSlide", "presentation": fx.song.id.0.to_string(), "index": 0}))
        .await;
    let generation = || server.shared.data().content_generation;
    let before = generation();

    let png = noise_png(16, 16, 77);
    let up = upload(addr, &collection, "Unrelated.png", &png).await;
    let asset = up["asset"]["id"].as_str().unwrap().to_string();
    let probe = probed(addr, &asset).await;
    assert_eq!(probe["probe"]["width"], 16, "{probe}");
    assert_eq!(
        generation(),
        before,
        "an upload nothing live uses: no re-render"
    );
    assert!(
        server
            .shared
            .data()
            .content
            .assets
            .values()
            .any(|a| a.id.0.to_string() == asset),
        "the show's documents have it"
    );
    let is_it = |m: &Value| m["type"] == "documentChanged" && m["id"] == asset.as_str();
    let changed = match ws.events.iter().position(is_it) {
        Some(i) => ws.events.remove(i),
        None => loop {
            let m = ws.next_any().await;
            if is_it(&m) {
                break m;
            }
            ws.absorb(&m);
        },
    };
    assert_eq!(changed["collection"], collection.as_str(), "{changed}");
    let log = get_json(
        addr,
        &format!("/api/v1/codex/collections/{collection}/changes?since=0"),
    )
    .await;
    assert!(
        log["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == asset.as_str() && c["kind"] == "mediaAsset"),
        "{log}"
    );

    // Relinking the live background to the uploaded file: re-rendered.
    let r = http(
        addr,
        "POST",
        &format!(
            "/api/v1/codex/collections/{collection}/media/{}/relink",
            fx.asset.id.0
        ),
        &[],
        Some(json!({"type": "blob", "sha256": sha_hex(&png), "extension": "png"})),
    )
    .await;
    ok(&r, 200, "relink the live asset");
    assert_ne!(
        generation(),
        before,
        "the live slide's asset changed: re-rendered"
    );
    server.shutdown().await.unwrap();
    std::fs::remove_dir_all(&fx.dir).ok();
}

/// The ids of a collection's media assets.
async fn asset_ids(addr: SocketAddr, collection: &str) -> Vec<String> {
    let list = get_json(
        addr,
        &format!("/api/v1/codex/collections/{collection}/media"),
    )
    .await;
    let mut ids: Vec<String> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["asset"]["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_file_twice_is_one_asset_per_collection() {
    let fx = fixture("codex-upload-twice");
    let bin = MediaBin {
        name: "Backgrounds".into(),
        ..MediaBin::default()
    };
    write_doc(&fx.dir, "portable", "mediaBin", &bin.id.0.to_string(), &bin);
    let server = start_with(&fx, CodexOptions::default()).await;
    let addr = server.addr;
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    let before = asset_ids(addr, &main).await;
    let png = noise_png(48, 32, 11);

    // Twice into the same collection: one asset; the second answer says it was there.
    let first = upload(addr, &main, "x.png", &png).await;
    assert!(first.get("existing").is_none(), "{first}");
    let aid = first["asset"]["id"].as_str().unwrap().to_string();
    // (Its probe is written into it first, so its revision stays put from here.)
    probed(addr, &aid).await;
    let second = upload(addr, &main, "x.png", &png).await;
    assert_eq!(second["deduplicated"], true, "{second}");
    assert_eq!(second["existing"], true, "{second}");
    assert_eq!(second["asset"]["id"], aid.as_str());
    let list = get_json(addr, &format!("/api/v1/codex/collections/{main}/media")).await;
    let stored = list
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["asset"]["id"] == aid.as_str())
        .unwrap();
    assert_eq!(
        second["revision"], stored["revision"],
        "its revision as stored"
    );
    assert_eq!(asset_ids(addr, &main).await.len(), before.len() + 1);

    // Into a bin: the existing asset gets one bin item, and a third upload finds it there.
    let into_bin = |n: &'static str| {
        let png = png.clone();
        let main = main.clone();
        async move {
            let r = create_upload(
                addr,
                json!({"collection": main, "name": n, "size": png.len(),
                       "sha256": sha_hex(&png), "bin": bin.id.0}),
            )
            .await;
            let id = ok(&r, 201, "create")["id"].as_str().unwrap().to_string();
            ok(
                &put_chunk(addr, &id, &png, 0, png.len() as u64).await,
                200,
                "chunk",
            );
            ok(&finish(addr, &id, None).await, 201, "finish")
        }
    };
    let b1 = into_bin("x.png").await;
    assert_eq!(b1["existing"], true, "{b1}");
    assert_eq!(b1["asset"]["id"], aid.as_str());
    let item = b1["binItem"]
        .as_str()
        .expect("added to the bin")
        .to_string();
    let b2 = into_bin("x.png").await;
    assert_eq!(
        b2["binItem"],
        item.as_str(),
        "the item it has, not a second one"
    );
    let detail = get_json(addr, &format!("/api/v1/media-bins/{}", bin.id.0)).await;
    let items = detail["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{detail}");
    assert_eq!(items[0]["id"], item.as_str());
    assert_eq!(items[0]["asset"], aid.as_str());
    assert_eq!(asset_ids(addr, &main).await.len(), before.len() + 1);

    // Another collection: an asset of its own for the same file, once.
    let r = http(
        addr,
        "POST",
        "/api/v1/codex/collections",
        &[],
        Some(json!({"name": "Youth"})),
    )
    .await;
    let youth = ok(&r, 201, "create")["collection"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let y1 = upload(addr, &youth, "x.png", &png).await;
    assert!(y1.get("existing").is_none(), "{y1}");
    assert_eq!(y1["deduplicated"], true, "the library had the file");
    assert_ne!(y1["asset"]["id"], aid.as_str());
    let y2 = upload(addr, &youth, "x.png", &png).await;
    assert_eq!(y2["existing"], true, "{y2}");
    assert_eq!(y2["asset"]["id"], y1["asset"]["id"]);
    assert_eq!(
        asset_ids(addr, &youth).await,
        vec![y1["asset"]["id"].as_str().unwrap().to_string()]
    );
    assert_eq!(asset_ids(addr, &main).await.len(), before.len() + 1);

    // `replaces` keeps its meaning: that asset is relinked, even to bytes another asset has.
    let other = upload(addr, &youth, "y.png", &noise_png(20, 20, 12)).await;
    let oid = other["asset"]["id"].as_str().unwrap();
    let re = upload_replacing(addr, &youth, "x.png", &png, oid).await;
    assert_eq!(re["relinked"], true, "{re}");
    assert!(re.get("existing").is_none(), "{re}");
    assert_eq!(re["asset"]["id"], oid);
    assert_eq!(re["asset"]["location"], y1["asset"]["location"]);
    assert_eq!(asset_ids(addr, &youth).await.len(), 2);
    server.shutdown().await.unwrap();
}

/// "Used by": the documents that use an asset or a prop (a song's media action, a prop's
/// media element), and the media library's items for a relink to any of them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn used_by_and_the_library_list() {
    let fx = fixture("codex-used-by");
    let logo = model::Prop {
        name: "Corner logo".into(),
        slide: model::Slide {
            elements: vec![model::Element::media(
                model::Rect::new(0.0, 0.0, 100.0, 100.0),
                fx.asset.id,
            )],
            ..model::Slide::new()
        },
        ..model::Prop::default()
    };
    write_doc(&fx.dir, "portable", "prop", &logo.id.0.to_string(), &logo);
    let server = start_with(&fx, CodexOptions::default()).await;
    let addr = server.addr;

    let asset = fx.asset.id.0.to_string();
    let uses = get_json(addr, &format!("/api/v1/media/{asset}/used-by")).await;
    let uses = uses.as_array().unwrap();
    assert_eq!(uses.len(), 2, "{uses:?}");
    assert_eq!(uses[0]["kind"], "presentation");
    assert_eq!(uses[0]["name"], fx.song.name.as_str());
    assert_eq!(uses[0]["id"], fx.song.id.0.to_string());
    assert_eq!(uses[1]["kind"], "prop");
    assert_eq!(uses[1]["name"], "Corner logo");
    let prop = logo.id.0.to_string();
    assert_eq!(
        get_json(addr, &format!("/api/v1/props/{prop}/used-by")).await,
        json!([])
    );
    let missing = uuid::Uuid::new_v4();
    for path in [
        format!("/api/v1/media/{missing}/used-by"),
        format!("/api/v1/props/{missing}/used-by"),
    ] {
        assert_eq!(call_status(addr, "GET", &path).await, 404, "{path}");
    }

    // The library's items, by path; found by words of their path, of a kind.
    let main = get_json(addr, "/api/v1/codex").await["collection"]
        .as_str()
        .unwrap()
        .to_string();
    upload(addr, &main, "Welcome Loop.png", &noise_png(32, 18, 3)).await;
    upload(addr, &main, "Offering.png", &noise_png(32, 18, 4)).await;
    let all = get_json(addr, "/api/v1/codex/library").await;
    assert!(all["total"].as_u64().unwrap() >= 2, "{all}");
    let found = get_json(addr, "/api/v1/codex/library?q=welcome%20loop").await;
    assert_eq!(found["total"], 1, "{found}");
    let item = &found["items"][0];
    assert!(
        item["path"].as_str().unwrap().ends_with("Welcome Loop.png"),
        "{item}"
    );
    assert_eq!(item["kind"], "picture");
    assert!(item["id"].is_string() && item["size"].as_u64().unwrap() > 0);
    let pictures = get_json(addr, "/api/v1/codex/library?kind=picture&limit=1").await;
    assert_eq!(pictures["items"].as_array().unwrap().len(), 1);
    assert!(pictures["total"].as_u64().unwrap() >= 2);
    assert_eq!(
        get_json(addr, "/api/v1/codex/library?kind=video").await["total"],
        0
    );
    for bad in ["kind=film", "limit=0"] {
        let r = http(
            addr,
            "GET",
            &format!("/api/v1/codex/library?{bad}"),
            &[],
            None,
        )
        .await;
        assert_eq!(r.status, 400, "{bad}");
    }
    server.shutdown().await.unwrap();
}
