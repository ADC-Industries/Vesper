//! Soak test of the server's render path under churn, the way a service runs: one server (one
//! renderer on the render thread), slide changes, videos started, swapped and stopped, image
//! backgrounds, layer clears, slide text edits and a steady stream of thumbnails at changing sizes, while the
//! screen keeps producing frames. Ignored by default; run it with
//! `SOAK_SECS=600 cargo test -p vesper-server --test server soak:: -- --ignored --nocapture`.

use crate::common::*;
use model::{AssetLocation, MediaAsset, MediaKind};
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// A `secs` long 640×360 H.264 clip of ffmpeg's test source at `path`.
fn clip(path: &Path, secs: u32) {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", &format!("testsrc2=s=640x360:r=30:d={secs}")])
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(path)
        .output()
        .expect("system ffmpeg");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn add_video(fx: &Fixture, name: &str, secs: u32) -> MediaAsset {
    clip(&fx.dir.join("media").join(format!("{name}.mp4")), secs);
    let asset = MediaAsset {
        name: name.into(),
        kind: MediaKind::Video,
        location: AssetLocation::Managed {
            sha256: name.into(),
            extension: "mp4".into(),
        },
        ..MediaAsset::default()
    };
    write_doc(
        &fx.dir,
        "portable",
        "mediaAsset",
        &asset.id.0.to_string(),
        &asset,
    );
    asset
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "soak: minutes long, run explicitly"]
async fn soak_server_under_churn() {
    let secs: u64 = std::env::var("SOAK_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let fx = fixture("soak");
    let videos = [add_video(&fx, "loop-a", 6), add_video(&fx, "loop-b", 3)];
    let server = start_server(&fx, true, false).await;
    let addr = server.addr;
    let song = fx.song.id.0.to_string();
    let slides: Vec<String> = fx
        .song
        .sections
        .iter()
        .flat_map(|s| s.slides.iter().map(|sl| sl.id.0.to_string()))
        .collect();
    let end = Instant::now() + Duration::from_secs(secs);

    // Thumbnails: a separate client at changing widths, so most are rendered, not cached.
    let stop = Arc::new(AtomicBool::new(false));
    let thumbs = Arc::new(AtomicU64::new(0));
    let thumb_task = {
        let (stop, thumbs, song, slides) =
            (stop.clone(), thumbs.clone(), song.clone(), slides.clone());
        tokio::spawn(async move {
            let mut i = 0usize;
            while !stop.load(Ordering::Relaxed) {
                let w = 96 + (i * 37) % 400;
                let path = format!(
                    "/api/v1/presentations/{song}/slides/{}/thumbnail?w={w}",
                    slides[i % slides.len()]
                );
                let r = http(addr, "GET", &path, &[], None).await;
                assert_eq!(r.status, 200, "thumbnail {path}");
                thumbs.fetch_add(1, Ordering::Relaxed);
                i += 1;
            }
        })
    };

    let mut a = Client::connect(addr, "").await;
    let (mut commands, mut edits, mut i) = (0u64, 0u64, 0usize);
    let mut last_seq = None;
    let mut next_check = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        let cmd = match i % 12 {
            0 | 3 | 6 | 9 => {
                json!({"type": "triggerSlide", "presentation": song, "index": (i / 3) % slides.len()})
            }
            1 => json!({"type": "triggerMedia", "asset": videos[0].id.0.to_string()}),
            4 => json!({"type": "triggerMedia", "asset": videos[1].id.0.to_string()}),
            7 => json!({"type": "mediaStop"}),
            8 => json!({"type": "triggerMedia", "asset": fx.asset.id.0.to_string()}),
            10 => json!({"type": "clearLayer", "layer": "media"}),
            11 => json!({"type": "clearLayer", "layer": "slide"}),
            2 => json!({"type": "mediaPause"}),
            _ => json!({"type": "mediaPlay"}),
        };
        // Transport commands with no video showing are refused (conflict), not failures.
        let r = a.command(cmd.clone()).await;
        assert!(
            r["error"].is_null() || r["error"]["code"] == "conflict",
            "{cmd} → {r}"
        );
        commands += 1;
        if i % 6 == 5 {
            // An edit: the live slide and its thumbnails render again.
            let slide = &slides[(i / 6) % slides.len()];
            let r = http(
                addr,
                "POST",
                &format!("/api/v1/presentations/{song}/edits"),
                &[],
                Some(json!({"ops": [{"type": "setSlideText", "slide": slide, "text": format!("Edit {i}")}]})),
            )
            .await;
            assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
            edits += 1;
        }
        i += 1;
        tokio::time::sleep(Duration::from_millis(40 + (i as u64 * 13) % 160)).await;
        if Instant::now() >= next_check {
            // The audience screen keeps publishing frames.
            let seq = server.shared.screens()[0]
                .latest
                .lock()
                .unwrap()
                .as_ref()
                .map(|f| f.sequence);
            assert!(
                seq.is_some() && seq != last_seq,
                "screen stalled at {seq:?}"
            );
            last_seq = seq;
            eprintln!(
                "soak: {:>4} s left, {commands} commands, {} thumbnails, frame {seq:?}",
                end.saturating_duration_since(Instant::now()).as_secs(),
                thumbs.load(Ordering::Relaxed)
            );
            next_check = Instant::now() + Duration::from_secs(5);
        }
    }
    stop.store(true, Ordering::Relaxed);
    thumb_task.await.unwrap();
    eprintln!(
        "soak: {secs} s, {commands} commands, {edits} edits, {} thumbnails, last frame {last_seq:?}",
        thumbs.load(Ordering::Relaxed)
    );
    server.shutdown().await.unwrap();
}
