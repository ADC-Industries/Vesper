//! PowerPoint import into the Codex (`/api/v1/codex/imports/pptx`): a synthetic `.pptx`
//! (built by `pptx_import::synthetic`) becomes a presentation in the collection's "Imported
//! Slides" library, its picture and the slide picture of a chart taken in as media (shown by
//! the slide thumbnails), with a report; refusals of what isn't a `.pptx`. LibreOffice is
//! stood in for by a script that hands back a picture.

use crate::common::*;
use pptx_import::synthetic::{self, DeckSpec, SlideSpec};
use serde_json::Value;
use std::time::{Duration, Instant};
use vesper_server::{RunningServer, ServerConfig, start};

async fn start_with(fx: &Fixture, pictures: pptx_import::render::Renderer) -> RunningServer {
    start(ServerConfig {
        listen: "127.0.0.1:0".parse().unwrap(),
        trust_localhost: true,
        mdns: false,
        site: vesper_server::site::SiteConfig::off(),
        slide_pictures: pictures,
        ..ServerConfig::new(&fx.dir)
    })
    .await
    .expect("server starts")
}

async fn import(addr: std::net::SocketAddr, query: &str, bytes: &[u8]) -> Value {
    let r = http_bytes(
        addr,
        "POST",
        &format!("/api/v1/codex/imports/pptx?{query}"),
        &[],
        bytes,
    )
    .await;
    assert_eq!(r.status, 202, "{}", String::from_utf8_lossy(&r.body));
    let id = r.json()["id"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let j = get_json(addr, &format!("/api/v1/codex/imports/pptx/{id}")).await;
        match j["state"].as_str() {
            Some("done") => return j,
            Some("failed") => panic!("import failed: {j}"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "import in time: {j}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn deck() -> Vec<u8> {
    synthetic::build(&DeckSpec {
        title: Some("Morning notices".into()),
        slides: vec![
            SlideSpec {
                shapes: format!(
                    "{}{}",
                    synthetic::title(2, "Welcome"),
                    // The right half of the slide, top half: a red picture.
                    synthetic::picture(
                        4,
                        "Photo",
                        "rIdP",
                        (6_096_000, 0, 6_096_000, 3_429_000),
                        ""
                    )
                ),
                media: vec![(
                    "rIdP".into(),
                    "image1.png".into(),
                    synthetic::png(64, 36, [230, 20, 20]),
                )],
                notes: Some("Say hello".into()),
                ..SlideSpec::default()
            },
            SlideSpec {
                shapes: format!(
                    "{}{}",
                    synthetic::chart(5, "Attendance", (0, 0, 6_096_000, 3_429_000)),
                    synthetic::title(2, "Numbers")
                ),
                ..SlideSpec::default()
            },
        ],
        ..DeckSpec::default()
    })
}

#[cfg(unix)]
fn near(p: &image::Rgb<u8>, c: [u8; 3]) -> bool {
    (0..3).all(|k| (p.0[k] as i32 - c[k] as i32).abs() <= 24)
}

#[cfg(unix)]
async fn thumbnail(addr: std::net::SocketAddr, pres: &str, slide: &str) -> image::RgbImage {
    let r = http(
        addr,
        "GET",
        &format!("/api/v1/presentations/{pres}/slides/{slide}/thumbnail?w=320"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200);
    image::load_from_memory(&r.body).unwrap().to_rgb8()
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_pptx_becomes_a_presentation_with_its_media_and_slide_pictures() {
    use std::os::unix::fs::PermissionsExt;
    let fx = fixture("pptx-import");
    // The stand-in for LibreOffice: a green slide picture for every package it gets.
    let pic = fx.dir.join("slide-picture.png");
    std::fs::write(&pic, synthetic::png(1920, 1080, [20, 200, 40])).unwrap();
    let script = fx.dir.join("soffice");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nout=\"\"\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    --outdir) out=\"$2\"; shift 2;;\n    *.pptx) cp '{}' \"$out/$(basename \"$1\" .pptx).png\"; shift;;\n    *) shift;;\n  esac\ndone\n",
            pic.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let server = start_with(&fx, pptx_import::render::Renderer::Program(script)).await;
    let addr = server.addr;

    let job = import(addr, "fileName=Notices.pptx", &deck()).await;
    let rep = &job["report"];
    assert_eq!(rep["name"], "Notices", "named after the file");
    assert_eq!(rep["slides"].as_array().unwrap().len(), 2);
    assert_eq!(rep["editableSlides"], 1);
    assert_eq!(rep["picturedSlides"], 1);
    assert_eq!(rep["media"], 2, "the photo and slide 2's picture");
    assert_eq!(rep["width"], 1920.0);
    let s2 = &rep["slides"][1];
    assert_eq!(s2["picture"], "libreOffice");
    assert_eq!(s2["pictured"][0]["kind"], "chart");
    assert_eq!(s2["title"], "Numbers");
    assert!(rep.get("rendererProblem").is_none(), "{rep}");

    // In the collection's "Imported Slides" library.
    let libs = get_json(addr, "/api/v1/libraries").await;
    let lib = libs
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == "Imported Slides")
        .expect("library made");
    assert_eq!(lib["id"], rep["library"]);
    let pid = rep["presentation"].as_str().unwrap();
    let p = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(p["import"]["source"], "powerPoint");
    assert_eq!(p["import"]["documentPath"], "Notices.pptx");
    let slides: Vec<&Value> = p["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["slides"].as_array().unwrap())
        .collect();
    assert_eq!(slides.len(), 2);
    assert_eq!(
        slides[0]["notes"]["paragraphs"][0]["runs"][0]["text"],
        "Say hello"
    );

    // Its media are the collection's, online.
    let media = get_json(
        addr,
        &format!(
            "/api/v1/codex/collections/{}/media",
            job["collection"].as_str().unwrap()
        ),
    )
    .await;
    let names: Vec<&str> = media
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["asset"]["name"].as_str().unwrap_or_default())
        .collect();
    assert!(names.iter().any(|n| n.contains("image1")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("slide 2")), "{names:?}");
    assert!(
        media
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["availability"] == "online"),
        "{media}"
    );

    // The thumbnails show them: the photo top right of slide 1, the chart's picture top left
    // of slide 2.
    let t1 = thumbnail(addr, pid, slides[0]["id"].as_str().unwrap()).await;
    assert!(
        near(t1.get_pixel(240, 40), [230, 20, 20]),
        "{:?}",
        t1.get_pixel(240, 40)
    );
    assert!(
        near(t1.get_pixel(240, 140), [255, 255, 255]),
        "white background"
    );
    let t2 = thumbnail(addr, pid, slides[1]["id"].as_str().unwrap()).await;
    assert!(
        near(t2.get_pixel(40, 30), [20, 200, 40]),
        "{:?}",
        t2.get_pixel(40, 30)
    );
    assert!(
        near(t2.get_pixel(280, 120), [255, 255, 255]),
        "{:?}",
        t2.get_pixel(280, 120)
    );

    // A second import of the same file uses the same library and the same media files.
    let again = import(addr, "fileName=Notices.pptx&name=Notices%20again", &deck()).await;
    assert_eq!(again["report"]["library"], rep["library"]);
    assert_ne!(again["report"]["presentation"], rep["presentation"]);
    let list = get_json(addr, "/api/v1/codex/imports/pptx").await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[0]["id"], again["id"], "newest first");
    server.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn without_slide_pictures_and_refusals() {
    let fx = fixture("pptx-refusals");
    let server = start_with(&fx, pptx_import::render::Renderer::Off).await;
    let addr = server.addr;
    let job = import(addr, "fileName=deck.pptx&name=Plain", &deck()).await;
    let rep = &job["report"];
    assert_eq!(rep["name"], "Plain");
    assert_eq!(rep["picturedSlides"], 0);
    assert_eq!(rep["media"], 1);
    assert_eq!(rep["rendererProblem"], "slide pictures are turned off");
    assert_eq!(rep["slides"][1]["approximated"][0]["kind"], "chart");

    let refuse = |q: &'static str, body: Vec<u8>| async move {
        let r = http_bytes(
            addr,
            "POST",
            &format!("/api/v1/codex/imports/pptx?{q}"),
            &[],
            &body,
        )
        .await;
        assert_eq!(r.status, 400, "{q}: {}", String::from_utf8_lossy(&r.body));
        r.json()
    };
    let e = refuse("fileName=talk.key", deck()).await;
    assert_eq!(e["error"]["field"], "fileName");
    assert!(e["error"]["message"].as_str().unwrap().contains("Keynote"));
    refuse("fileName=old.ppt", deck()).await;
    let e = refuse("fileName=x.pptx", b"plain text, not a package".to_vec()).await;
    assert_eq!(e["error"]["field"], "body");
    let e = refuse(
        "fileName=x.pptx&library=00000000-0000-0000-0000-000000000001",
        deck(),
    )
    .await;
    assert_eq!(e["error"]["field"], "library");
    // A zip that isn't a presentation fails as a job, with the reason.
    let mut w = pptx_import::zip::Writer::new();
    w.add("hello.txt", b"hi").unwrap();
    let r = http_bytes(
        addr,
        "POST",
        "/api/v1/codex/imports/pptx?fileName=x.pptx",
        &[],
        &w.finish().unwrap(),
    )
    .await;
    assert_eq!(r.status, 202);
    let id = r.json()["id"].as_str().unwrap().to_string();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let j = get_json(addr, &format!("/api/v1/codex/imports/pptx/{id}")).await;
        if j["state"] == "failed" {
            assert!(j["error"].as_str().unwrap().contains("presentation"), "{j}");
            break;
        }
        assert!(Instant::now() < deadline, "{j}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let r = http(addr, "GET", "/api/v1/codex/imports/pptx/nope", &[], None).await;
    assert_eq!(r.status, 404);
    server.shutdown().await.unwrap();
}

/// Each waiting import holds its whole file (up to 1 GiB): one imports, one waits, and a
/// third is refused until the waiting one has started.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn at_most_one_import_waits() {
    use std::os::unix::fs::PermissionsExt;
    let fx = fixture("pptx-import-queue");
    let pic = fx.dir.join("slide-picture.png");
    std::fs::write(&pic, synthetic::png(1920, 1080, [20, 200, 40])).unwrap();
    // A slow stand-in for LibreOffice.
    let script = fx.dir.join("soffice");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nsleep 2\nout=\"\"\nwhile [ $# -gt 0 ]; do\n  case \"$1\" in\n    --outdir) out=\"$2\"; shift 2;;\n    *.pptx) cp '{}' \"$out/$(basename \"$1\" .pptx).png\"; shift;;\n    *) shift;;\n  esac\ndone\n",
            pic.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let server = start_with(&fx, pptx_import::render::Renderer::Program(script)).await;
    let addr = server.addr;
    let post = |name: &'static str| async move {
        http_bytes(
            addr,
            "POST",
            &format!("/api/v1/codex/imports/pptx?fileName={name}"),
            &[],
            &deck(),
        )
        .await
    };
    let a = post("A.pptx").await;
    assert_eq!(a.status, 202);
    let b = post("B.pptx").await;
    assert_eq!(b.status, 202);
    let c = post("C.pptx").await;
    assert_eq!(c.status, 429, "{}", String::from_utf8_lossy(&c.body));
    assert_eq!(c.json()["error"]["code"], "rateLimited", "{}", c.json());
    // Once both are done, imports are taken again.
    let deadline = Instant::now() + Duration::from_secs(120);
    for r in [a, b] {
        let id = r.json()["id"].as_str().unwrap().to_string();
        loop {
            let j = get_json(addr, &format!("/api/v1/codex/imports/pptx/{id}")).await;
            if j["state"] == "done" {
                break;
            }
            assert_ne!(j["state"], "failed", "{j}");
            assert!(Instant::now() < deadline, "in time: {j}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    import(addr, "fileName=D.pptx", &deck()).await;
}
