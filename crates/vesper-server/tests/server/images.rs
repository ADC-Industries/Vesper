//! Image decoding never holds up live output: decodes run on the renderer's worker pool,
//! thumbnails wait for theirs without blocking the render thread, a background that is still
//! decoding leaves the previous one on screen, and the next slide's images are prefetched.

use crate::common::*;
use model::{
    AssetLocation, Color, Library, MediaAsset, MediaAssetId, MediaLayer, MediaTrigger,
    Presentation, Rect, Section, Size, Slide, SlideAction,
};
use serde_json::json;
use std::path::Path;
use std::time::{Duration, Instant};
use vesper_server::RunningServer;

const GREEN: [u8; 3] = [0, 255, 0];
/// Inside both test backgrounds as fitted on the 640 × 360 screen (the 3:2 photos are
/// pillarboxed), below the text.
const PROBE: (u32, u32) = (60, 340);

/// Writes a `w × h` PNG of one colour (fast compression: cheap to write, slow to decode).
fn write_png(path: &Path, w: u32, h: u32, rgb: [u8; 3]) {
    use image::ImageEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let mut row = Vec::with_capacity(w as usize * 3);
    for _ in 0..w {
        row.extend_from_slice(&rgb);
    }
    let mut px = Vec::with_capacity(row.len() * h as usize);
    for _ in 0..h {
        px.extend_from_slice(&row);
    }
    let f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    PngEncoder::new_with_quality(f, CompressionType::Fast, FilterType::NoFilter)
        .write_image(&px, w, h, image::ExtendedColorType::Rgb8)
        .unwrap();
}

/// Adds a managed PNG asset to the fixture's data directory.
fn add_asset(fx: &Fixture, name: &str, w: u32, h: u32, rgb: [u8; 3]) -> MediaAssetId {
    write_png(&fx.dir.join("media").join(format!("{name}.png")), w, h, rgb);
    let asset = MediaAsset {
        name: name.into(),
        location: AssetLocation::Managed {
            sha256: name.into(),
            extension: "png".into(),
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
    asset.id
}

/// A slide with `text` in `color`, optionally triggering a background.
fn slide(text: &str, color: Color, background: Option<MediaAssetId>) -> Slide {
    let mut s = Slide {
        elements: vec![text_element(
            Rect::new(20.0, 80.0, W as f64 - 40.0, 200.0),
            text,
            color,
        )],
        ..Slide::new()
    };
    if let Some(bg) = background {
        s.actions.push(SlideAction::media(MediaTrigger::new(
            bg,
            MediaLayer::Background,
        )));
    }
    s
}

/// Adds a presentation with `slides` to the fixture's library.
fn add_presentation(fx: &Fixture, library: &Library, name: &str, slides: Vec<Slide>) -> String {
    let mut sec = Section::new("Verse");
    sec.slides = slides;
    let p = Presentation {
        library: Some(library.id),
        canvas_size: Size::new(W as f64, H as f64),
        sections: vec![sec],
        ..Presentation::new(name)
    };
    write_doc(&fx.dir, "portable", "presentation", &p.id.0.to_string(), &p);
    p.id.0.to_string()
}

async fn trigger(server: &RunningServer, presentation: &str, index: u32) {
    let r = http(
        server.addr,
        "POST",
        "/api/v1/commands",
        &[],
        Some(json!({ "type": "triggerSlide", "presentation": presentation, "index": index })),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
}

fn sequence(server: &RunningServer, i: usize) -> u64 {
    server.shared.screens()[i]
        .latest
        .lock()
        .unwrap()
        .as_ref()
        .map_or(0, |f| f.sequence)
}

/// Waits for screen `i` to publish a frame newer than `after`; returns how long it took.
async fn next_frame(server: &RunningServer, i: usize, after: u64, since: Instant) -> Duration {
    let deadline = Instant::now() + Duration::from_secs(60);
    while sequence(server, i) <= after {
        assert!(Instant::now() < deadline, "no new frame on screen {i}");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    since.elapsed()
}

fn decode_stats(server: &RunningServer) -> render::DecodeStats {
    server.shared.decoder.get().expect("renderer up").stats()
}

/// Waits until the decoder has nothing queued or running.
async fn decodes_idle(server: &RunningServer) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let s = decode_stats(server);
        if s.queued == 0 && s.running == 0 {
            return;
        }
        assert!(Instant::now() < deadline, "decodes never finished: {s:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn incomplete_renders(server: &RunningServer, i: usize) -> u64 {
    server.shared.screens()[i]
        .stats
        .lock()
        .unwrap()
        .incomplete_renders
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_trigger_is_not_blocked_by_a_huge_thumbnail_decode() {
    let fx = fixture("img-huge");
    let huge = add_asset(&fx, "huge", 8000, 8000, [200, 120, 40]);
    let photos = add_presentation(
        &fx,
        &fx.library,
        "Huge Photo",
        vec![slide("BIG", Color::WHITE, Some(huge))],
    );
    let server = start_server(&fx, true, false).await;
    let second = fx.second.id.0.to_string();
    wait_frame(&server, 0, "first frame", |_| true).await;

    // A cold thumbnail of the slide over the 8000 × 8000 background.
    let p = get_json(server.addr, &format!("/api/v1/presentations/{photos}")).await;
    let slide_id = p["sections"][0]["slides"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let addr = server.addr;
    let thumb = tokio::spawn(async move {
        http(
            addr,
            "GET",
            &format!("/api/v1/presentations/{photos}/slides/{slide_id}/thumbnail?w=480"),
            &[],
            None,
        )
        .await
    });
    // Its decode is running on a worker.
    let deadline = Instant::now() + Duration::from_secs(30);
    while decode_stats(&server).running == 0 {
        assert!(
            Instant::now() < deadline,
            "the thumbnail decode never started"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    // A text slide goes live right away anyway.
    let before = sequence(&server, 0);
    let t = Instant::now();
    trigger(&server, &second, 0).await;
    let took = next_frame(&server, 0, before, t).await;
    let still_decoding = decode_stats(&server).running > 0 && !thumb.is_finished();
    eprintln!(
        "live trigger during a huge decode: {took:?} (decode still running: {still_decoding})"
    );
    assert!(
        still_decoding,
        "the decode finished before the live frame: the test proves nothing on this machine"
    );
    assert!(
        took < Duration::from_millis(100),
        "live trigger took {took:?} while a thumbnail decode was running"
    );
    let img = latest(&server, 0).unwrap();
    assert!(count(&img, 80, 280, WHITE, 40) > 100, "B1 text is live");

    // The thumbnail completes once its image is decoded.
    let r = tokio::time::timeout(Duration::from_secs(300), thumb)
        .await
        .expect("thumbnail in time")
        .unwrap();
    assert_eq!(r.status, 200);
    let jpeg = image::load_from_memory(&r.body).unwrap().to_rgb8();
    assert_eq!(jpeg.width(), 480);
    // The square photo is fitted (pillarboxed): probe below the text, inside it.
    let p = jpeg.get_pixel(240, 262);
    assert!(
        (0..3).all(|k| (p.0[k] as i32 - [200, 120, 40][k]).abs() <= 12),
        "thumbnail shows the photo: {p:?}"
    );
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_stays_until_the_new_one_is_decoded() {
    let fx = fixture("img-keep");
    let red = add_asset(&fx, "red", 6000, 4000, [255, 0, 0]);
    // Slide 1 has no background, so triggering slide 0 prefetches nothing big.
    let photos = add_presentation(
        &fx,
        &fx.library,
        "Photos",
        vec![
            slide("P0", Color::WHITE, Some(fx.asset.id)),
            slide("P1", Color::WHITE, None),
            slide("P2", Color::rgb(0.0, 1.0, 0.0), Some(red)),
        ],
    );
    let server = start_server(&fx, true, false).await;
    trigger(&server, &photos, 0).await;
    wait_frame(&server, 0, "blue background", |img| {
        near(img.get_pixel(PROBE.0, PROBE.1), BLUE, 6)
    })
    .await;
    decodes_idle(&server).await;

    let before = sequence(&server, 0);
    let incomplete = incomplete_renders(&server, 0);
    let t = Instant::now();
    trigger(&server, &photos, 2).await;
    let took = next_frame(&server, 0, before, t).await;
    let img = latest(&server, 0).unwrap();
    eprintln!("slide with a cold 6000x4000 background went live in {took:?}");
    // The text is live, over the previous (blue) background, not black.
    assert!(count(&img, 80, 280, GREEN, 40) > 100, "P2 text is live");
    assert!(
        near(img.get_pixel(PROBE.0, PROBE.1), BLUE, 6),
        "previous background kept: {:?}",
        img.get_pixel(PROBE.0, PROBE.1)
    );
    assert!(incomplete_renders(&server, 0) > incomplete);
    assert!(took < Duration::from_millis(500), "{took:?}");

    // The new background replaces it once decoded, without another command.
    let img = wait_frame(&server, 0, "red background", |img| {
        near(img.get_pixel(PROBE.0, PROBE.1), RED, 6)
    })
    .await;
    assert!(count(&img, 80, 280, GREEN, 40) > 100, "P2 text still live");
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn next_slide_background_is_prefetched() {
    let fx = fixture("img-next");
    let red = add_asset(&fx, "red2", 3000, 2000, [255, 0, 0]);
    let photos = add_presentation(
        &fx,
        &fx.library,
        "Photos",
        vec![
            slide("N0", Color::WHITE, None),
            slide("N1", Color::WHITE, Some(red)),
        ],
    );
    let server = start_server(&fx, true, false).await;
    trigger(&server, &photos, 0).await;
    wait_frame(&server, 0, "N0", |img| count(img, 80, 280, WHITE, 40) > 100).await;
    // Triggering N0 queued N1's background; let it finish.
    decodes_idle(&server).await;
    assert!(decode_stats(&server).decoded >= 1);

    let incomplete = incomplete_renders(&server, 0);
    let before = sequence(&server, 0);
    trigger(&server, &photos, 1).await;
    next_frame(&server, 0, before, Instant::now()).await;
    let img = latest(&server, 0).unwrap();
    assert!(
        near(img.get_pixel(PROBE.0, PROBE.1), RED, 6),
        "the first frame of N1 has its background: {:?}",
        img.get_pixel(PROBE.0, PROBE.1)
    );
    assert_eq!(incomplete_renders(&server, 0), incomplete);
    server.shutdown().await.unwrap();
}
