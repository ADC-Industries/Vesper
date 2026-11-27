//! The real engine on a real model (ignored: it downloads the English → Spanish model,
//! about 315 MB, from its publisher on first run). Run it with
//!
//! ```text
//! VESPER_TRANSLATE_DIR=/some/scratch cargo test -p vesper-translate --release --test real_model -- --ignored --nocapture
//! ```
//!
//! It prints each translated line and the time per slide. Synthetic lyrics.

use std::sync::Arc;
use std::time::{Duration, Instant};
use vesper_translate::catalogue::catalogue;
use vesper_translate::marian::Marian;
use vesper_translate::store::{Https, State, Store};
use vesper_translate::{Translator, translate_lines};

#[test]
#[ignore = "downloads a 315 MB model"]
fn english_to_spanish_on_a_real_model() {
    let dir = std::env::var_os("VESPER_TRANSLATE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("vesper-translate-models"));
    let store = Arc::new(Store::open(&dir, catalogue(), Arc::new(Https::default())));
    let t0 = Instant::now();
    store.start("en-es").unwrap();
    loop {
        match store.state("en-es") {
            State::Ready => break,
            State::Failed(why) => panic!("download failed: {why}"),
            State::NotDownloaded => panic!("download stopped"),
            State::Downloading { done, total } => {
                eprintln!(
                    "downloading {} of {} MB",
                    done / 1_000_000,
                    total / 1_000_000
                );
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
    eprintln!("model ready after {:.1} s", t0.elapsed().as_secs_f64());
    let m = Marian::new(store).unwrap();
    eprintln!("engine threads: {}", m.threads());
    let slides: Vec<Vec<String>> = [
        "Morning light is breaking\nover the quiet hills",
        "Sing a song of welcome\n\nto the friends who gather here",
        "Carry the light, carry it high\nOver the water, under the sky",
        "Grace and peace to all of you",
    ]
    .iter()
    .map(|s| s.split('\n').map(String::from).collect())
    .collect();
    for (i, lines) in slides.iter().enumerate() {
        let t = Instant::now();
        let out = translate_lines(&m, "en", "es", lines).unwrap();
        let ms = t.elapsed().as_millis();
        eprintln!(
            "slide {} ({ms} ms{}):",
            i + 1,
            if i == 0 { ", with loading" } else { "" }
        );
        for (a, b) in lines.iter().zip(&out) {
            eprintln!("  {a:?} → {b:?}");
        }
        assert_eq!(out.len(), lines.len());
        for (a, b) in lines.iter().zip(&out) {
            assert_eq!(a.trim().is_empty(), b.is_empty());
        }
    }
    let out = m
        .translate("en", "es", &["Good morning".to_string()])
        .unwrap();
    assert!(
        out[0].to_lowercase().contains("buen"),
        "a known phrase: {out:?}"
    );
    assert!(m.unload_idle(Duration::ZERO));
    assert_eq!(m.loaded(), None);
}

/// More pairs: `VESPER_TRANSLATE_PAIRS="en-pt:pt-BR,en-zh:zh-Hant"` (each downloads its model).
#[test]
#[ignore = "downloads the models named in VESPER_TRANSLATE_PAIRS"]
fn other_pairs_on_real_models() {
    let dir = std::env::var_os("VESPER_TRANSLATE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("vesper-translate-models"));
    let pairs = std::env::var("VESPER_TRANSLATE_PAIRS").unwrap_or_default();
    let store = Arc::new(Store::open(&dir, catalogue(), Arc::new(Https::default())));
    let m = Marian::new(store.clone()).unwrap();
    let lines: Vec<String> = [
        "Morning light is breaking",
        "",
        "Grace and peace to all of you",
    ]
    .map(String::from)
    .to_vec();
    for spec in pairs.split(',').filter(|s| !s.is_empty()) {
        let (id, to) = spec.split_once(':').expect("pair:language");
        store.start(id).unwrap();
        loop {
            match store.state(id) {
                State::Ready => break,
                State::Downloading { .. } => std::thread::sleep(Duration::from_millis(500)),
                other => panic!("{id}: {other:?}"),
            }
        }
        let from = id.split('-').next().unwrap();
        let t = Instant::now();
        let out = translate_lines(&m, from, to, &lines).unwrap();
        eprintln!(
            "{id} → {to} ({} ms, with loading): {out:?}",
            t.elapsed().as_millis()
        );
        assert_eq!(out.len(), 3);
        assert!(out[1].is_empty() && !out[0].is_empty() && !out[2].is_empty());
    }
}
