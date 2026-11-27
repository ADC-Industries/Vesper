//! IDN Open Protocol loopback: an `IdnSink` sends, an `IdnFinder` / `IdnReceiver` in the
//! same process receives, with synthetic frames at 1920×1080, 30 fps:
//! 1. **discovery**: time from creating the source until the finder lists it;
//! 2. **colour**: 75 % colour bars, centre of each bar compared with the sent value;
//! 3. **pacing + audio** (`IDN_LOOPBACK_SECS`, default 10 s): a 12-bit frame counter (white
//!    blocks across the top), a white flash in the lower half once a second, stereo audio
//!    (left: continuous 1 kHz at −20 dBFS peak; right: a 1 kHz beep during each flash frame):
//!    arrival jitter, repeated / skipped frames, tone frequency and level, audio continuity
//!    and the A/V offset (by timecode and by arrival time);
//! 4. **alpha**: a second source with alpha, a straight-alpha checker (alpha 0/64/128/192/255),
//!    alpha and colour errors after the round trip.
//!
//! It prints one result line; the asserts are loose bounds, the printed numbers are the
//! measurement. Checks against third-party receivers are manual: `docs/INTEROP-CHECKLIST.md`.
//!
//! ```text
//! cargo test --release -p output --test idn_loopback -- --nocapture
//! ```

use output::{
    AlphaMode, AudioSink, Frame, FrameRate, FrameSink, IdnFinder, IdnReceiver, IdnSenderConfig,
    IdnSink, Received, ReceivedFrame,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

const W: u32 = 1920;
const H: u32 = 1080;
const FPS: u32 = 30;
const RATE: u32 = 48_000;
const SPF: usize = (RATE / FPS) as usize;
const TC_PER_FRAME: i64 = 10_000_000 / FPS as i64;
const BITS: u32 = 12;

fn unique(tag: &str) -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_micros();
    format!("loopback-{tag}-{}-{n}", std::process::id())
}

// ---- patterns ---------------------------------------------------------------------------

/// 75 % bars (white, yellow, cyan, green, magenta, red, blue, black), straight sRGB.
const BARS: [[u8; 3]; 8] = [
    [191, 191, 191],
    [191, 191, 0],
    [0, 191, 191],
    [0, 191, 0],
    [191, 0, 191],
    [191, 0, 0],
    [0, 0, 191],
    [0, 0, 0],
];

fn bars() -> Vec<u8> {
    let mut v = Vec::with_capacity((W * H * 4) as usize);
    for _ in 0..H {
        for x in 0..W {
            let c = BARS[(x * 8 / W) as usize];
            v.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    v
}

fn bit_rect(bit: u32) -> (u32, u32, u32, u32) {
    let x0 = bit * 16 * W / 192 + 4;
    let x1 = (bit * 16 + 16) * W / 192 - 4;
    (x0, x1, H * 6 / 108, H * 18 / 108)
}

fn fill(px: &mut [u8], (x0, x1, y0, y1): (u32, u32, u32, u32), c: [u8; 4]) {
    for y in y0..y1 {
        let row = (y * W * 4) as usize;
        for x in x0..x1 {
            let i = row + (x * 4) as usize;
            px[i..i + 4].copy_from_slice(&c);
        }
    }
}

/// Counter frames: dark grey, the counter's 12 bits as white/black blocks, a white lower
/// half on flash frames.
struct CounterFrames {
    base: Vec<u8>,
    flash: Vec<u8>,
    buf: Vec<u8>,
}

impl CounterFrames {
    fn new() -> Self {
        let base = [40u8, 40, 40, 255].repeat((W * H) as usize);
        let mut flash = base.clone();
        fill(&mut flash, (0, W, H / 2, H), [255, 255, 255, 255]);
        CounterFrames {
            buf: base.clone(),
            base,
            flash,
        }
    }

    fn frame(&mut self, n: u32) -> &[u8] {
        let src = if is_flash(n) { &self.flash } else { &self.base };
        self.buf.copy_from_slice(src);
        for bit in 0..BITS {
            let on = (n >> (BITS - 1 - bit)) & 1 == 1;
            let c = if on { 255 } else { 0 };
            fill(&mut self.buf, bit_rect(bit), [c, c, c, 255]);
        }
        &self.buf
    }
}

fn is_flash(n: u32) -> bool {
    n % FPS == 5
}

fn counter(f: &ReceivedFrame) -> Option<u32> {
    let mut n = 0;
    for bit in 0..BITS {
        let (x0, x1, y0, y1) = bit_rect(bit);
        let p = f.pixel_rgba((x0 + x1) / 2 * f.width / W, (y0 + y1) / 2 * f.height / H);
        let on = match p[0] {
            0..=60 => false,
            195..=255 => true,
            _ => return None,
        };
        n = (n << 1) | on as u32;
    }
    Some(n)
}

const ALPHAS: [u8; 5] = [0, 64, 128, 192, 255];
const KEY_COLOURS: [[u8; 3]; 4] = [[230, 40, 40], [40, 230, 40], [40, 40, 230], [230, 230, 230]];
const TILE: u32 = 120;

fn tile(i: u32, j: u32) -> ([u8; 3], u8) {
    (
        KEY_COLOURS[((i / 2 + j) % 4) as usize],
        ALPHAS[((i + j) % 5) as usize],
    )
}

/// Straight-alpha checker, premultiplied for our sinks.
fn checker() -> Vec<u8> {
    let mut v = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        for x in 0..W {
            let (c, a) = tile(x / TILE, y / TILE);
            let pm = |v: u8| ((v as u32 * a as u32 + 127) / 255) as u8;
            v.extend_from_slice(&[pm(c[0]), pm(c[1]), pm(c[2]), a]);
        }
    }
    v
}

fn tone(n: u32) -> Vec<f32> {
    let mut v = Vec::with_capacity(SPF * 2);
    for i in 0..SPF {
        let t = (n as usize * SPF + i) as f64 / RATE as f64;
        let s = (2.0 * std::f64::consts::PI * 1000.0 * t).sin();
        v.push((0.1 * s) as f32);
        v.push(if is_flash(n) { (0.5 * s) as f32 } else { 0.0 });
    }
    v
}

// ---- senders and receivers ----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Bars = 0,
    Counter = 1,
    Checker = 2,
}

struct Sender {
    stop: Arc<AtomicBool>,
    mode: Arc<AtomicU8>,
    created: Instant,
    handle: Option<std::thread::JoinHandle<String>>,
}

impl Sender {
    fn start(name: &str, alpha: bool) -> Sender {
        let stop = Arc::new(AtomicBool::new(false));
        let mode = Arc::new(AtomicU8::new(if alpha {
            Mode::Checker as u8
        } else {
            Mode::Bars as u8
        }));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let handle = {
            let (stop, mode, name) = (stop.clone(), mode.clone(), name.to_string());
            std::thread::spawn(move || {
                let mut cfg = IdnSenderConfig::new(&name);
                cfg.alpha = alpha;
                let mut sink = IdnSink::new(cfg).expect("IdnSink");
                let mut audio = sink.audio_sender();
                ready_tx.send(Instant::now()).unwrap();
                let (bars, check) = (bars(), checker());
                let mut counter = CounterFrames::new();
                let start = Instant::now();
                let mut n: u32 = 0;
                let mut late = 0u32;
                while !stop.load(Ordering::Relaxed) {
                    let m = mode.load(Ordering::Relaxed);
                    let pixels: &[u8] = match m {
                        0 => &bars,
                        2 => &check,
                        _ => counter.frame(n),
                    };
                    let tc = n as i64 * TC_PER_FRAME;
                    if m == Mode::Counter as u8 {
                        audio.send(&tone(n), 2, RATE, tc).unwrap();
                    }
                    sink.send(&Frame {
                        width: W,
                        height: H,
                        pixels,
                        alpha: if alpha {
                            AlphaMode::Premultiplied
                        } else {
                            AlphaMode::Opaque
                        },
                        frame_rate: FrameRate { num: FPS, den: 1 },
                        timecode: tc,
                    })
                    .unwrap();
                    n += 1;
                    let due = start + Duration::from_nanos(n as u64 * 1_000_000_000 / FPS as u64);
                    match due.checked_duration_since(Instant::now()) {
                        Some(d) => std::thread::sleep(d),
                        None => late += 1,
                    }
                }
                format!("sent {n} frames, {late} late ticks")
            })
        };
        let created = ready_rx.recv().unwrap();
        Sender {
            stop,
            mode,
            created,
            handle: Some(handle),
        }
    }

    fn set(&self, m: Mode) {
        self.mode.store(m as u8, Ordering::Relaxed);
    }

    fn finish(mut self) -> String {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().unwrap().join().unwrap()
    }
}

/// Finds `name` with the IDN finder; returns the receiver and the discovery time measured
/// from `created`.
fn connect(name: &str, created: Instant) -> (IdnReceiver, f64) {
    let finder = IdnFinder::new().unwrap();
    let src = finder
        .find_by_name(name, Duration::from_secs(20))
        .unwrap_or_else(|| panic!("IDN finder did not list {name:?} within 20 s"));
    let t = created.elapsed().as_secs_f64();
    (IdnReceiver::connect(&src.name).unwrap(), t)
}

// ---- measurements -------------------------------------------------------------------------

/// A received audio block: arrival ms, timecode, sample rate, left and right channels.
type AudioRec = (f64, i64, u32, Vec<f32>, Vec<f32>);

#[derive(Debug, Default)]
struct Row {
    discovery_s: f64,
    first_frame_s: f64,
    size: (u32, u32),
    rate: (i32, i32),
    fourcc: String,
    bars_max_err: i32,
    fps: f64,
    jitter_sd_ms: f64,
    jitter_p99_ms: f64,
    jitter_max_ms: f64,
    frames: usize,
    repeats: usize,
    skips: usize,
    unreadable: usize,
    tone_hz: f64,
    tone_peak_dbfs: f64,
    tone_rms_dbfs: f64,
    audio_blocks: usize,
    audio_block_samples: (usize, usize),
    audio_discontinuities: usize,
    audio_samples_ratio: f64,
    av_tc_ms: Vec<f64>,
    av_arrival_ms: Vec<f64>,
    alpha_fourcc: String,
    alpha_max_err: i32,
    key_colour_err_opaque: i32,
    key_colour_err_half: i32,
    key_colour_err_quarter: i32,
    /// IDN receiver counters (metadata messages, decode errors, drops, transport).
    idn_rx: String,
}

fn percentile(v: &[f64], q: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s.get(((s.len().max(1) - 1) as f64 * q).round() as usize)
        .copied()
        .unwrap_or(0.0)
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

fn secs() -> f64 {
    std::env::var("IDN_LOOPBACK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10.0)
}

fn run() -> Row {
    let label = "IDN -> IDN";
    let name = unique("idn");
    let mut row = Row::default();

    // 1-2: discovery and colour.
    let sender = Sender::start(&name, false);
    let (mut r, disc) = connect(&name, sender.created);
    row.discovery_s = disc;
    let mut last = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut got = 0;
    while got < 15 && Instant::now() < deadline {
        if let Some(f) = r.capture_video(Duration::from_millis(500)).unwrap() {
            if got == 0 {
                row.first_frame_s = sender.created.elapsed().as_secs_f64();
            }
            got += 1;
            last = Some(f);
        }
    }
    let f = last.unwrap_or_else(|| panic!("{label}: no video"));
    row.size = (f.width, f.height);
    row.rate = f.frame_rate;
    row.fourcc = f.fourcc_str();
    for (i, c) in BARS.iter().enumerate() {
        let x = (i as u32 * 2 + 1) * f.width / 16;
        let p = f.pixel_rgba(x, f.height / 2);
        for k in 0..3 {
            row.bars_max_err = row.bars_max_err.max((p[k] as i32 - c[k] as i32).abs());
        }
    }

    // 3: pacing and audio.
    sender.set(Mode::Counter);
    let settle = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < settle {
        let _ = r.capture(Duration::from_millis(100)).unwrap();
    }
    let start = Instant::now();
    let end = start + Duration::from_secs_f64(secs());
    let mut video: Vec<(f64, i64, Option<u32>, bool)> = Vec::new();
    let mut audio: Vec<AudioRec> = Vec::new();
    while Instant::now() < end {
        match r.capture(Duration::from_millis(200)).unwrap() {
            Some(Received::Video(f)) => {
                let t = start.elapsed().as_secs_f64() * 1000.0;
                let flash = f.pixel_rgba(f.width / 2, f.height * 3 / 4)[0] > 128;
                video.push((t, f.timecode, counter(&f), flash));
            }
            Some(Received::Audio(a)) => {
                let t = start.elapsed().as_secs_f64() * 1000.0;
                let ch1 = if a.channels > 1 {
                    a.channel(1).to_vec()
                } else {
                    Vec::new()
                };
                audio.push((t, a.timecode, a.sample_rate, a.channel(0).to_vec(), ch1));
            }
            None => {}
        }
    }
    assert!(
        video.len() > 10,
        "{label}: only {} video frames",
        video.len()
    );
    let iv: Vec<f64> = video.windows(2).map(|w| w[1].0 - w[0].0).collect();
    let m = mean(&iv);
    row.frames = video.len();
    row.fps = (video.len() - 1) as f64 / ((video.last().unwrap().0 - video[0].0) / 1000.0);
    row.jitter_sd_ms = (iv.iter().map(|d| (d - m).powi(2)).sum::<f64>() / iv.len() as f64).sqrt();
    row.jitter_p99_ms = percentile(&iv, 0.99);
    row.jitter_max_ms = percentile(&iv, 1.0);
    let counters: Vec<u32> = video.iter().filter_map(|v| v.2).collect();
    row.unreadable = video.len() - counters.len();
    for w in counters.windows(2) {
        if w[1] == w[0] {
            row.repeats += 1;
        } else if w[1] != w[0] + 1 {
            row.skips += 1;
        }
    }
    // Audio: continuity by timecode, tone by concatenation.
    row.audio_blocks = audio.len();
    if !audio.is_empty() {
        let sizes: Vec<usize> = audio.iter().map(|a| a.3.len()).collect();
        row.audio_block_samples = (*sizes.iter().min().unwrap(), *sizes.iter().max().unwrap());
        for w in audio.windows(2) {
            let expect = w[0].1 + (w[0].3.len() as i64 * 10_000_000) / w[0].2.max(1) as i64;
            if (w[1].1 - expect).abs() > 10_000 {
                row.audio_discontinuities += 1;
            }
        }
        let ch0: Vec<f32> = audio.iter().flat_map(|a| a.3.iter().copied()).collect();
        let dur = ch0.len() as f64 / RATE as f64;
        let crossings = ch0
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        row.tone_hz = crossings as f64 / 2.0 / dur;
        let peak = ch0.iter().fold(0.0f32, |a, v| a.max(v.abs()));
        let rms = (ch0.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / ch0.len() as f64).sqrt();
        row.tone_peak_dbfs = 20.0 * (peak as f64).log10();
        row.tone_rms_dbfs = 20.0 * rms.log10();
        row.audio_samples_ratio = ch0.len() as f64 / (secs() * RATE as f64);
        // A/V: beep onsets (first loud sample of a burst on the right channel).
        let flashes: Vec<(f64, i64)> = video
            .windows(2)
            .filter(|w| w[1].3 && !w[0].3)
            .map(|w| (w[1].0, w[1].1))
            .collect();
        // A burst is one onset: loud samples within 100 ms of the last loud one belong to it.
        let mut last_loud: Option<i64> = None;
        for (t, tc, rate, _, ch1) in &audio {
            for (i, v) in ch1.iter().enumerate() {
                if v.abs() <= 0.05 {
                    continue;
                }
                let at = tc + i as i64 * 10_000_000 / (*rate).max(1) as i64;
                let onset = last_loud.is_none_or(|l| at - l > 1_000_000);
                last_loud = Some(at);
                if !onset {
                    continue;
                }
                if let Some(o) = flashes
                    .iter()
                    .map(|f| (at - f.1) as f64 / 10_000.0)
                    .min_by(|a, b| a.abs().total_cmp(&b.abs()))
                {
                    row.av_tc_ms.push(o);
                }
                let arrival = t + i as f64 * 1000.0 / *rate as f64;
                if let Some(o) = flashes
                    .iter()
                    .map(|f| arrival - f.0)
                    .min_by(|a, b| a.abs().total_cmp(&b.abs()))
                {
                    row.av_arrival_ms.push(o);
                }
            }
        }
    }
    {
        let st = r.stats();
        row.idn_rx = format!(
            "metadata {} | video errors {} | audio errors {} | dropped {} | reconnects {} | extensions {:?} | via {:?}",
            st.metadata,
            st.video_errors,
            st.audio_errors,
            st.dropped,
            st.reconnects,
            st.extensions,
            st.via
        );
    }
    drop(r);
    eprintln!("{label} sender: {}", sender.finish());

    // 4: alpha (a second source, with alpha).
    let aname = format!("{name}-key");
    let sender = Sender::start(&aname, true);
    let (mut r, _) = connect(&aname, sender.created);
    let mut last = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut got = 0;
    while got < 10 && Instant::now() < deadline {
        if let Some(f) = r.capture_video(Duration::from_millis(500)).unwrap() {
            got += 1;
            last = Some(f);
        }
    }
    let f = last.unwrap_or_else(|| panic!("{label}: no key video"));
    row.alpha_fourcc = f.fourcc_str();
    for j in 0..H / TILE {
        for i in 0..W / TILE {
            let (c, a) = tile(i, j);
            let p = f.pixel_rgba(
                (i * TILE + TILE / 2) * f.width / W,
                (j * TILE + TILE / 2) * f.height / H,
            );
            row.alpha_max_err = row.alpha_max_err.max((p[3] as i32 - a as i32).abs());
            let err = (0..3)
                .map(|k| (p[k] as i32 - c[k] as i32).abs())
                .max()
                .unwrap();
            match a {
                255 | 192 => row.key_colour_err_opaque = row.key_colour_err_opaque.max(err),
                128 => row.key_colour_err_half = row.key_colour_err_half.max(err),
                64 => row.key_colour_err_quarter = row.key_colour_err_quarter.max(err),
                _ => {}
            }
        }
    }
    drop(r);
    eprintln!("{label} key sender: {}", sender.finish());
    eprintln!("RESULT {label}: {row:#?}");
    eprintln!(
        "SUMMARY | {label} | disc {:.2} s | first frame {:.2} s | {}x{} {}/{} {} | bars max err {} | {:.2} fps, sd {:.2} ms, p99 {:.1} ms, max {:.1} ms | {} frames, {} repeats, {} skips, {} unreadable | tone {:.1} Hz, peak {:.2} dBFS, rms {:.2} dBFS | audio blocks {} ({}..{} samples), {} discontinuities, {:.3} of expected samples | A/V tc {:.1} ms (n={}), arrival {:.1} ms | key {} alpha err {}, colour err a>=192 {}, a=128 {}, a=64 {} |",
        row.discovery_s,
        row.first_frame_s,
        row.size.0,
        row.size.1,
        row.rate.0,
        row.rate.1,
        row.fourcc,
        row.bars_max_err,
        row.fps,
        row.jitter_sd_ms,
        row.jitter_p99_ms,
        row.jitter_max_ms,
        row.frames,
        row.repeats,
        row.skips,
        row.unreadable,
        row.tone_hz,
        row.tone_peak_dbfs,
        row.tone_rms_dbfs,
        row.audio_blocks,
        row.audio_block_samples.0,
        row.audio_block_samples.1,
        row.audio_discontinuities,
        row.audio_samples_ratio,
        mean(&row.av_tc_ms),
        row.av_tc_ms.len(),
        mean(&row.av_arrival_ms),
        row.alpha_fourcc,
        row.alpha_max_err,
        row.key_colour_err_opaque,
        row.key_colour_err_half,
        row.key_colour_err_quarter,
    );
    row
}

/// Loose bounds: the stream works and looks right. The printed numbers are the result.
fn check(row: &Row, label: &str) {
    assert_eq!(row.size, (W, H), "{label}: size");
    assert!(
        row.bars_max_err <= 12,
        "{label}: bars error {}",
        row.bars_max_err
    );
    assert!(
        (row.fps - FPS as f64).abs() < 3.0,
        "{label}: fps {}",
        row.fps
    );
    assert!(row.audio_blocks > 0, "{label}: no audio");
    assert!(
        (row.tone_hz - 1000.0).abs() < 20.0,
        "{label}: tone {} Hz",
        row.tone_hz
    );
    assert!(
        row.alpha_max_err <= 6,
        "{label}: alpha error {}",
        row.alpha_max_err
    );
}

#[test]
fn idn_loopback() {
    let row = run();
    check(&row, "IDN -> IDN");
}
