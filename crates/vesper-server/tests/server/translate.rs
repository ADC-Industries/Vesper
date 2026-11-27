//! Machine translation (`docs/DUAL-LANGUAGE-PLAN.md` §5) with a fake engine and model files
//! served from memory (no network): the models list, an admin's download (checked, ready),
//! previews (the model fetched first when missing; one proposal per slide with text, marked as
//! a machine translation, nothing saved), Keep as `setSlideTranslation`, cancelling, refusals,
//! and deleting a model. Synthetic text.
//!
//! ```text
//! sg render -c "cargo test -p vesper-server --test server translate::"
//! ```

use crate::common::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use vesper_server::translate::{
    ModelFile, Pair, Source, Target, TranslateOptions, Translator, sha256,
};

/// Upper-cases each line, with the target language in front.
struct Fake;

impl Translator for Fake {
    fn translate(&self, _: &str, to: &str, lines: &[String]) -> anyhow::Result<Vec<String>> {
        Ok(lines
            .iter()
            .map(|l| format!("{to}: {}", l.to_uppercase()))
            .collect())
    }
    fn engine(&self) -> String {
        "fake".into()
    }
}

/// Model files from memory, a little slowly (so a download can be seen running).
struct Mem(HashMap<String, Vec<u8>>);

struct Slow(std::io::Cursor<Vec<u8>>);

impl Read for Slow {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::thread::sleep(Duration::from_millis(1));
        let k = buf.len().min(4096);
        self.0.read(&mut buf[..k])
    }
}

impl Source for Mem {
    fn open(&self, url: &str) -> anyhow::Result<Box<dyn Read + Send>> {
        let name = url.rsplit('/').next().unwrap_or_default();
        let b = self
            .0
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("404 {url}"))?;
        Ok(Box::new(Slow(std::io::Cursor::new(b))))
    }
}

fn pair(id: &str, from: &str, to: &str, files: &[(&str, &[u8])]) -> Pair {
    Pair {
        id: id.into(),
        from: from.into(),
        to: vec![Target {
            language: to.into(),
            token: None,
        }],
        repo: format!("Example/opus-mt-{id}"),
        revision: "0123456789abcdef0123456789abcdef01234567".into(),
        licence: "Apache-2.0".into(),
        base_url: "http://192.0.2.1".into(),
        files: files
            .iter()
            .map(|(n, b)| ModelFile {
                name: (*n).into(),
                size: b.len() as u64,
                sha256: sha256(b),
            })
            .collect(),
    }
}

async fn wait_job(addr: std::net::SocketAddr, id: &str) -> Value {
    for _ in 0..300 {
        let j = get_json(addr, &format!("/api/v1/translate/jobs/{id}")).await;
        if matches!(j["status"].as_str(), Some("done" | "failed" | "cancelled")) {
            return j;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the preview never ended");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn previews_fetch_their_model_and_save_nothing_until_kept() {
    let fx = fixture("translate");
    let weights = vec![9u8; 64 * 1024];
    let config = b"{\"made\": \"up\"}".to_vec();
    let files: HashMap<String, Vec<u8>> = [
        ("config.json".to_string(), config.clone()),
        ("weights.bin".to_string(), weights.clone()),
    ]
    .into();
    let pairs = vec![
        pair(
            "en-es",
            "en",
            "es",
            &[("config.json", &config), ("weights.bin", &weights)],
        ),
        pair(
            "en-fr",
            "en",
            "fr",
            &[("config.json", &config), ("weights.bin", &weights)],
        ),
    ];
    let server = start_server_with(&fx, |c| {
        c.translation = TranslateOptions {
            engine: Some(Arc::new(Fake)),
            pairs: Some(pairs),
            source: Some(Arc::new(Mem(files))),
        };
    })
    .await;
    let addr = server.addr;

    let info = get_json(addr, "/api/v1/server").await;
    assert!(
        info["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("translate"))
    );
    let m = get_json(addr, "/api/v1/translate/models").await;
    assert_eq!(m["engine"], "fake");
    assert!(
        m["notice"]
            .as_str()
            .unwrap()
            .starts_with("Machine translation: check it")
    );
    assert_eq!(m["models"][0]["id"], "en-es");
    assert_eq!(m["models"][0]["status"], "notDownloaded");
    assert_eq!(m["models"][0]["licence"], "Apache-2.0");
    assert_eq!(
        m["models"][0]["size"],
        (config.len() + weights.len()) as u64
    );

    // The admin (this machine) downloads en-es; it is checked and ready.
    let r = http(
        addr,
        "POST",
        "/api/v1/translate/models/en-es/download",
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let mut ready = false;
    for _ in 0..200 {
        let m = get_json(addr, "/api/v1/translate/models").await;
        if m["models"][0]["status"] == "ready" {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ready, "en-es never became ready");
    let r = http(
        addr,
        "POST",
        "/api/v1/translate/models/xx-yy/download",
        &[],
        None,
    )
    .await;
    assert_eq!(r.status, 404);

    // A preview of the whole presentation: one proposal per slide, nothing saved.
    let pid = fx.second.id.0.to_string();
    let before = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    let r = http(
        addr,
        "POST",
        "/api/v1/translate/preview",
        &[],
        Some(json!({"presentation": pid, "to": "es"})),
    )
    .await;
    assert_eq!(r.status, 202, "{}", String::from_utf8_lossy(&r.body));
    let id = r.json()["id"].as_str().unwrap().to_string();
    let j = wait_job(addr, &id).await;
    assert_eq!(j["status"], "done", "{j}");
    assert_eq!(j["total"], 2);
    assert_eq!(j["model"], "opus-mt-en-es");
    let slides = j["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 2);
    let s0 = &slides[0];
    assert_eq!(s0["slide"], before["sections"][0]["slides"][0]["id"]);
    let tr = &s0["translation"];
    assert_eq!(tr["language"], "es");
    assert_eq!(
        tr["machine"],
        json!({"engine": "fake", "model": "opus-mt-en-es"})
    );
    assert_eq!(
        tr["texts"][0]["element"],
        before["sections"][0]["slides"][0]["elements"][0]["id"]
    );
    assert_eq!(tr["texts"][0]["paragraphs"][0]["runs"][0]["text"], "es: B1");
    let after = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(before, after, "a preview saves nothing");

    // Keep: the proposal sent as the slide's layer, marked.
    let r = http(
        addr,
        "POST",
        &format!("/api/v1/presentations/{pid}/edits"),
        &[],
        Some(json!({"ops": [{"type": "setSlideTranslation", "slide": s0["slide"], "translation": tr}]})),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let kept = get_json(addr, &format!("/api/v1/presentations/{pid}")).await;
    assert_eq!(
        kept["sections"][0]["slides"][0]["translation"]["machine"]["model"],
        "opus-mt-en-es"
    );

    // One slide, into French: the model is fetched first (the job says so), then translated.
    let one = before["sections"][0]["slides"][1]["id"].clone();
    let r = http(
        addr,
        "POST",
        "/api/v1/translate/preview",
        &[],
        Some(json!({"presentation": pid, "slides": [one], "to": "fr", "from": "en"})),
    )
    .await;
    assert_eq!(r.status, 202);
    let id = r.json()["id"].as_str().unwrap().to_string();
    let mut saw_download = false;
    for _ in 0..100 {
        let j = get_json(addr, &format!("/api/v1/translate/jobs/{id}")).await;
        if j["status"] == "downloading" {
            saw_download = true;
            break;
        }
        if j["status"] == "done" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(saw_download, "the job reports the model download");
    let j = wait_job(addr, &id).await;
    assert_eq!(j["status"], "done", "{j}");
    assert_eq!(j["slides"].as_array().unwrap().len(), 1);
    assert_eq!(
        j["slides"][0]["translation"]["texts"][0]["paragraphs"][0]["runs"][0]["text"],
        "fr: B2"
    );
    let m = get_json(addr, "/api/v1/translate/models").await;
    assert_eq!(m["models"][1]["status"], "ready");

    // Refusals: no pair, a bad code, a missing presentation or slide, no such job.
    for (body, status) in [
        (json!({"presentation": pid, "to": "ko"}), 400),
        (json!({"presentation": pid, "to": "e s"}), 400),
        (
            json!({"presentation": uuid::Uuid::new_v4(), "to": "es"}),
            404,
        ),
        (
            json!({"presentation": pid, "slides": [uuid::Uuid::new_v4()], "to": "es"}),
            404,
        ),
    ] {
        let r = http(
            addr,
            "POST",
            "/api/v1/translate/preview",
            &[],
            Some(body.clone()),
        )
        .await;
        assert_eq!(
            r.status,
            status,
            "{body}: {}",
            String::from_utf8_lossy(&r.body)
        );
    }
    let r = http(addr, "GET", "/api/v1/translate/jobs/nope", &[], None).await;
    assert_eq!(r.status, 404);
    let r = http(addr, "DELETE", "/api/v1/translate/jobs/nope", &[], None).await;
    assert_eq!(r.status, 404);

    // Deleting a model: not downloaded again, its files gone.
    let r = http(addr, "DELETE", "/api/v1/translate/models/en-fr", &[], None).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["status"], "notDownloaded");
    assert!(!fx.dir.join("cache/translate/en-fr").exists());
    assert!(fx.dir.join("cache/translate/en-es/weights.bin").is_file());
}
