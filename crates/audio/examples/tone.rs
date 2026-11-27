//! Plays a test tone on a device and reports underruns:
//! `cargo run -p audio --example tone -- hw:1,0 60 [freq]`.

use audio::{AudioEngine, EngineConfig, SourceParams, tone::Tone};
use std::time::{Duration, Instant};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let device = args.get(1).cloned().unwrap_or_default();
    let secs: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let freq: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1000.0);
    let e = AudioEngine::start(EngineConfig {
        device,
        ..EngineConfig::default()
    });
    println!("{:?}", e.status());
    e.add_source(
        1,
        Box::new(Tone::new(freq, -18.0, secs, 2)),
        SourceParams::default(),
        0.0,
    );
    let t0 = Instant::now();
    while t0.elapsed().as_secs_f64() < secs {
        std::thread::sleep(Duration::from_secs(1));
        println!(
            "t={:.0}s levels={:?} xruns={} blocks={}",
            t0.elapsed().as_secs_f64(),
            e.levels(),
            e.xruns(),
            e.blocks()
        );
    }
    println!("final: {:?} xruns={}", e.status(), e.xruns());
}
