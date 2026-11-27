//! The synthetic camera: an IDN Open Protocol source (compatible with NDI®) sent by the harness,
//! 640×360 at 30 fps with a moving bar and a quiet tone. The scenario takes it off the network
//! and brings it back (a camera unplugged and plugged in again).

use output::{AlphaMode, AudioSink, Frame, FrameRate, FrameSink, IdnSenderConfig, IdnSink};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const W: u32 = 640;
const H: u32 = 360;
const FPS: u64 = 30;

/// The camera, running on its own thread until dropped.
pub struct Camera {
    /// On the network (else withdrawn).
    pub on: Arc<AtomicBool>,
    /// Its full name (`HOST (name)`) once it was first on the network.
    pub full_name: Arc<Mutex<Option<String>>>,
    /// Times it failed to start.
    pub errors: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Camera {
    /// Starts a camera named `name`.
    pub fn start(name: String) -> Camera {
        let on = Arc::new(AtomicBool::new(true));
        let stop = Arc::new(AtomicBool::new(false));
        let full_name = Arc::new(Mutex::new(None));
        let errors = Arc::new(AtomicU64::new(0));
        let thread = {
            let (on, stop, full, errors) =
                (on.clone(), stop.clone(), full_name.clone(), errors.clone());
            std::thread::Builder::new()
                .name("camera".into())
                .spawn(move || run(name, on, stop, full, errors))
                .expect("camera thread")
        };
        Camera {
            on,
            full_name,
            errors,
            stop,
            thread: Some(thread),
        }
    }

    /// Waits until it has a full name (at most `wait`).
    pub fn name(&self, wait: Duration) -> Option<String> {
        let end = Instant::now() + wait;
        loop {
            if let Some(n) = self.full_name.lock().unwrap().clone() {
                return Some(n);
            }
            if Instant::now() > end {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(
    name: String,
    on: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    full: Arc<Mutex<Option<String>>>,
    errors: Arc<AtomicU64>,
) {
    let mut buf = vec![0u8; (W * H * 4) as usize];
    let mut n: u64 = 0;
    let start = Instant::now();
    let mut sink: Option<IdnSink> = None;
    let mut audio = None;
    let samples = (48_000 / FPS) as usize;
    while !stop.load(Ordering::Relaxed) {
        let want = on.load(Ordering::Relaxed);
        if want && sink.is_none() {
            match IdnSink::new(IdnSenderConfig::new(name.clone())) {
                Ok(s) => {
                    *full.lock().unwrap() = Some(s.full_name().to_string());
                    audio = Some(s.audio_sender());
                    sink = Some(s);
                }
                Err(e) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                    eprintln!("camera: {e:#}");
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        } else if !want && sink.is_some() {
            audio = None;
            sink = None;
        }
        if let (Some(s), Some(a)) = (sink.as_mut(), audio.as_mut()) {
            // Dark blue with a bright bar sweeping across.
            let bar = (n * 8 % W as u64) as u32;
            for y in 0..H {
                for x in 0..W {
                    let i = ((y * W + x) * 4) as usize;
                    let lit = x >= bar && x < bar + 40;
                    let v = if lit {
                        [230, 230, 200, 255]
                    } else {
                        [20, 30, 70, 255]
                    };
                    buf[i..i + 4].copy_from_slice(&v);
                }
            }
            let tc = (n * 10_000_000 / FPS) as i64;
            // A quiet 440 Hz tone (−40 dBFS), well under the sync clip's beeps.
            let tone: Vec<f32> = (0..samples * 2)
                .map(|i| {
                    let k = n as usize * samples + i / 2;
                    (k as f32 * 2.0 * std::f32::consts::PI * 440.0 / 48_000.0).sin() * 0.01
                })
                .collect();
            let _ = a.send(&tone, 2, 48_000, tc);
            let _ = s.send(&Frame {
                width: W,
                height: H,
                pixels: &buf,
                alpha: AlphaMode::Opaque,
                frame_rate: FrameRate {
                    num: FPS as u32,
                    den: 1,
                },
                timecode: tc,
            });
        }
        n += 1;
        let due = start + Duration::from_nanos(n * 1_000_000_000 / FPS);
        if let Some(d) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        }
    }
}
