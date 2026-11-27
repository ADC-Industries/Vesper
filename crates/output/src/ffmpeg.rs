//! `ffmpeg` / `ffplay` output: raw frames piped to a child process's stdin, for a preview
//! window, a recording, or any other ffmpeg output.
//!
//! The process is spawned on the first frame (when the size and rate are known) with
//!
//! ```text
//! <program> <input_args...> -f rawvideo -pixel_format <rgb0|rgba> -video_size WxH
//!           -framerate N/D -i - <output_args...>
//! ```
//!
//! Opaque frames are sent as `rgb0` (the premultiplied bytes as-is, which is the frame
//! flattened over black, so no conversion); frames with alpha as straight `rgba`.
//!
//! `send` never waits on the pipe: it copies the frame into a pooled buffer and queues it for
//! a writer thread. If ffmpeg cannot keep up and the queue is full, the frame is dropped and
//! counted ([`FfmpegSink::dropped`]); ffmpeg sees a constant-rate stream with the frame
//! missing. A rawvideo stream cannot change size mid-stream, so a frame with a different
//! size, rate or alpha mode than the first is an error (create a new sink).

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use model::FrameRate;

use crate::convert::{self, PixelFormat};
use crate::worker::{Task, Worker};
use crate::{Frame, FrameSink};

const STDERR_TAIL_LINES: usize = 20;

/// How to run ffmpeg/ffplay.
#[derive(Clone, Debug)]
pub struct FfmpegConfig {
    /// Program to run (`ffmpeg`, `ffplay`, or a full path).
    pub program: PathBuf,
    /// Arguments before the rawvideo input (global options, `-re`, ...).
    pub input_args: Vec<OsString>,
    /// Arguments after `-i -` (codec options and the output file).
    pub output_args: Vec<OsString>,
    /// Frames that may wait for the writer thread before new frames are dropped (min 1).
    pub queue_depth: usize,
    /// Name used in logs and [`FrameSink::name`].
    pub label: String,
    /// Kill the process when the sink is finished instead of closing stdin and waiting for it
    /// to exit (right for a preview window, wrong for a recording that must be finalised).
    pub kill_on_finish: bool,
    /// How long to wait for the process to exit after stdin is closed before killing it.
    pub exit_timeout: Duration,
}

impl FfmpegConfig {
    /// `ffmpeg` with the given output arguments (e.g. `["-c:v", "ffv1", "out.mkv"]`).
    pub fn ffmpeg<I, S>(output_args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        FfmpegConfig {
            program: "ffmpeg".into(),
            input_args: args(["-hide_banner", "-nostats", "-loglevel", "warning", "-y"]),
            output_args: output_args.into_iter().map(Into::into).collect(),
            queue_depth: 3,
            label: "ffmpeg".into(),
            kill_on_finish: false,
            exit_timeout: Duration::from_secs(30),
        }
    }

    /// An `ffplay` preview window titled `title`.
    pub fn preview(title: &str) -> Self {
        let mut input_args = args([
            "-hide_banner",
            "-loglevel",
            "warning",
            "-autoexit",
            "-fflags",
            "nobuffer",
            "-window_title",
        ]);
        input_args.push(title.into());
        FfmpegConfig {
            program: "ffplay".into(),
            input_args,
            output_args: Vec::new(),
            queue_depth: 2,
            label: format!("ffplay: {title}"),
            kill_on_finish: true,
            exit_timeout: Duration::from_secs(2),
        }
    }

    /// Records H.264 (libx264, `yuv420p`, CRF 18) to `path`; the container follows the
    /// extension (`.mp4`, `.mkv`, `.mov`). Alpha is not kept; use [`FfmpegConfig::ffmpeg`]
    /// with e.g. `-c:v ffv1` or `-c:v qtrle` for that.
    pub fn record(path: impl Into<PathBuf>) -> Self {
        let path: PathBuf = path.into();
        let mut c = Self::ffmpeg(args([
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-pix_fmt", "yuv420p",
        ]));
        c.label = format!("ffmpeg record: {}", path.display());
        c.output_args.push(path.into_os_string());
        c.queue_depth = 4;
        c
    }
}

fn args<const N: usize>(a: [&str; N]) -> Vec<OsString> {
    a.iter().map(OsString::from).collect()
}

/// Stream parameters fixed when the process starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StreamFormat {
    width: u32,
    height: u32,
    alpha: bool,
    rate: FrameRate,
}

impl StreamFormat {
    fn of(frame: &Frame<'_>) -> Self {
        StreamFormat {
            width: frame.width,
            height: frame.height,
            alpha: frame.has_alpha(),
            rate: frame.frame_rate,
        }
    }
    fn ffmpeg_pix_fmt(self) -> &'static str {
        if self.alpha { "rgba" } else { "rgb0" }
    }
}

type Tail = Arc<Mutex<VecDeque<String>>>;

/// The child process, shared by the writer task and the sink (which kills it from outside
/// the writer thread when that is blocked in a write, see [`FfmpegSink`]'s `Drop`).
type SharedChild = Arc<Mutex<Child>>;

fn child(c: &SharedChild) -> std::sync::MutexGuard<'_, Child> {
    c.lock().unwrap_or_else(|e| e.into_inner())
}

struct PipeTask {
    program: String,
    child: SharedChild,
    stdin: Option<ChildStdin>,
    tail: Tail,
    convert_to: Option<PixelFormat>,
    format: StreamFormat,
    scratch: Vec<u8>,
    kill_on_finish: bool,
    exit_timeout: Duration,
}

impl PipeTask {
    fn tail_text(&self) -> String {
        let t = self
            .tail
            .lock()
            .map(|t| t.iter().cloned().collect::<Vec<_>>());
        match t {
            Ok(lines) if !lines.is_empty() => format!(":\n{}", lines.join("\n")),
            _ => String::new(),
        }
    }

    /// Closes stdin and waits up to `timeout` for the child, killing it after that.
    fn close_and_wait(&mut self, timeout: Duration) -> Result<std::process::ExitStatus> {
        drop(self.stdin.take());
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = child(&self.child).try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                let mut c = child(&self.child);
                let _ = c.kill();
                return Ok(c.wait()?);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Task<()> for PipeTask {
    fn process(&mut self, data: &[u8], _: ()) -> Result<()> {
        let bytes = match self.convert_to {
            Some(f) => {
                convert::convert(
                    data,
                    self.format.width,
                    self.format.height,
                    f,
                    &mut self.scratch,
                )?;
                &self.scratch[..]
            }
            None => data,
        };
        let stdin = self.stdin.as_mut().ok_or_else(|| anyhow!("stdin closed"))?;
        if let Err(e) = stdin.write_all(bytes) {
            let status = self.close_and_wait(Duration::from_secs(2))?;
            bail!(
                "{} stopped accepting frames ({e}; {status}){}",
                self.program,
                self.tail_text()
            );
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        if self.kill_on_finish {
            drop(self.stdin.take());
            let mut c = child(&self.child);
            let _ = c.kill();
            let _ = c.wait();
            return Ok(());
        }
        let status = self.close_and_wait(self.exit_timeout)?;
        if !status.success() {
            bail!("{} exited with {status}{}", self.program, self.tail_text());
        }
        Ok(())
    }
}

/// Pipes frames into `ffmpeg` or `ffplay` (see the [module docs](self)).
pub struct FfmpegSink {
    config: FfmpegConfig,
    running: Option<(StreamFormat, Worker<()>)>,
    /// The running process (also held by the writer task).
    child: Option<SharedChild>,
    frames_queued: u64,
}

impl FfmpegSink {
    /// A dropped `kill_on_finish` sink kills its process before the writer thread is joined:
    /// a write blocked on a process that stopped reading (a paused preview window) then fails
    /// at once instead of holding up the drop. ([`finish`](Self::finish) still writes what is
    /// queued first.)
    fn kill_first(&mut self) {
        if self.config.kill_on_finish
            && let Some(c) = &self.child
        {
            let _ = child(c).kill();
        }
    }
}

impl Drop for FfmpegSink {
    fn drop(&mut self) {
        self.kill_first();
    }
}

impl FfmpegSink {
    /// Creates the sink; the process starts on the first frame.
    pub fn new(config: FfmpegConfig) -> Self {
        FfmpegSink {
            config,
            running: None,
            child: None,
            frames_queued: 0,
        }
    }

    /// An `ffplay` preview window ([`FfmpegConfig::preview`]).
    pub fn preview(title: &str) -> Self {
        Self::new(FfmpegConfig::preview(title))
    }

    /// An H.264 recording ([`FfmpegConfig::record`]).
    pub fn record(path: impl Into<PathBuf>) -> Self {
        Self::new(FfmpegConfig::record(path))
    }

    /// Frames dropped because the pipe was backed up.
    pub fn dropped(&self) -> u64 {
        self.running.as_ref().map_or(0, |(_, w)| w.dropped)
    }

    /// Frames queued for the process so far.
    pub fn frames_queued(&self) -> u64 {
        self.frames_queued
    }

    /// Writes the queued frames, closes stdin and waits for the process to exit (or kills it
    /// if [`FfmpegConfig::kill_on_finish`]). Reports a non-zero exit status as an error.
    pub fn finish(mut self) -> Result<()> {
        match self.running.take() {
            Some((_, mut w)) => w.finish(),
            None => Ok(()),
        }
    }

    fn spawn(&mut self, fmt: StreamFormat) -> Result<Worker<()>> {
        let c = &self.config;
        let program = c.program.display().to_string();
        let mut cmd = Command::new(&c.program);
        cmd.args(&c.input_args)
            .args(["-f", "rawvideo", "-pixel_format", fmt.ffmpeg_pix_fmt()])
            .arg("-video_size")
            .arg(format!("{}x{}", fmt.width, fmt.height))
            .arg("-framerate")
            .arg(format!("{}/{}", fmt.rate.num, fmt.rate.den.max(1)))
            .args(["-i", "-"])
            .args(&c.output_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        log::info!("{}: starting {:?}", c.label, cmd);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("could not start {program} (is it installed and on PATH?)"))?;
        let stdin = child.stdin.take();
        let tail: Tail = Arc::default();
        if let Some(stderr) = child.stderr.take() {
            let tail = tail.clone();
            let label = c.label.clone();
            std::thread::Builder::new()
                .name("ffmpeg-stderr".into())
                .spawn(move || {
                    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                        log::debug!("[{label}] {line}");
                        if let Ok(mut t) = tail.lock() {
                            if t.len() == STDERR_TAIL_LINES {
                                t.pop_front();
                            }
                            t.push_back(line);
                        }
                    }
                })?;
        }
        let child = Arc::new(Mutex::new(child));
        self.child = Some(child.clone());
        let task = PipeTask {
            program,
            child,
            stdin,
            tail,
            convert_to: fmt.alpha.then_some(PixelFormat::Rgba),
            format: fmt,
            scratch: Vec::new(),
            kill_on_finish: c.kill_on_finish,
            exit_timeout: c.exit_timeout,
        };
        Worker::spawn("ffmpeg-writer", c.queue_depth, task)
    }
}

impl FrameSink for FfmpegSink {
    fn name(&self) -> &str {
        &self.config.label
    }

    fn send(&mut self, frame: &Frame<'_>) -> Result<()> {
        frame.validate()?;
        let fmt = StreamFormat::of(frame);
        let worker = match &mut self.running {
            Some((running_fmt, w)) => {
                if *running_fmt != fmt {
                    bail!(
                        "{}: frame format changed from {running_fmt:?} to {fmt:?}; a raw video \
                         pipe cannot change format, create a new sink",
                        self.config.label
                    );
                }
                w
            }
            None => {
                let w = self.spawn(fmt)?;
                &mut self.running.insert((fmt, w)).1
            }
        };
        worker.check()?;
        let Some(mut buf) = worker.try_buffer() else {
            worker.note_dropped();
            return Ok(());
        };
        buf.clear();
        buf.extend_from_slice(frame.pixels);
        if worker.submit(buf, ())? {
            self.frames_queued += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{frame, pattern};
    use model::AlphaMode;

    fn have(program: &str) -> bool {
        Command::new(program)
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    fn tmpfile(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("output-ffmpeg-test-{}-{name}", std::process::id()))
    }

    fn raw_capture(alpha: AlphaMode, name: &str) -> (Vec<u8>, Vec<u8>) {
        let out = tmpfile(name);
        let mut cfg = FfmpegConfig::ffmpeg(["-f", "rawvideo", "-pix_fmt", "rgba"]);
        cfg.output_args.push(out.clone().into());
        cfg.queue_depth = 32;
        let mut sink = FfmpegSink::new(cfg);
        let px = pattern(32, 16, 5);
        for _ in 0..10 {
            sink.send(&frame(32, 16, &px, alpha)).unwrap();
        }
        assert_eq!(sink.dropped(), 0);
        sink.finish().unwrap();
        let data = std::fs::read(&out).unwrap();
        std::fs::remove_file(&out).unwrap();
        (px, data)
    }

    #[test]
    fn pipes_straight_rgba_through_ffmpeg() {
        if !have("ffmpeg") {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let (px, data) = raw_capture(AlphaMode::Premultiplied, "alpha.raw");
        assert_eq!(data.len(), 10 * 32 * 16 * 4);
        let mut want = Vec::new();
        convert::convert(&px, 32, 16, PixelFormat::Rgba, &mut want).unwrap();
        assert_eq!(&data[..want.len()], &want[..]);
        assert_eq!(&data[9 * want.len()..], &want[..]);
    }

    #[test]
    fn opaque_frames_are_flattened() {
        if !have("ffmpeg") {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let (px, data) = raw_capture(AlphaMode::Opaque, "opaque.raw");
        let mut want = Vec::new();
        convert::convert(&px, 32, 16, PixelFormat::Rgbx, &mut want).unwrap();
        assert_eq!(&data[..want.len()], &want[..]);
    }

    /// ffmpeg reading with `-re` at 10 fps cannot keep up with frames sent in a tight loop:
    /// `send` must stay fast and drop frames instead of blocking.
    #[test]
    fn never_blocks_when_the_pipe_is_backed_up() {
        if !have("ffmpeg") {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let mut cfg = FfmpegConfig::ffmpeg(["-f", "null", "-"]);
        cfg.input_args.push("-re".into());
        cfg.queue_depth = 2;
        cfg.kill_on_finish = true;
        let mut sink = FfmpegSink::new(cfg);
        let (w, h) = (1920, 1080);
        let px = pattern(w, h, 1);
        let mut f = frame(w, h, &px, AlphaMode::Opaque);
        f.frame_rate = FrameRate::fps(10);
        let start = Instant::now();
        let n = 100;
        for _ in 0..n {
            sink.send(&f).unwrap();
        }
        let elapsed = start.elapsed();
        let dropped = sink.dropped();
        eprintln!("{n} sends in {elapsed:?}, {dropped} dropped");
        assert!(
            dropped >= n - 10,
            "expected most frames dropped, got {dropped}"
        );
        assert!(
            elapsed < Duration::from_secs(3),
            "send blocked: {elapsed:?}"
        );
        sink.finish().unwrap();
    }

    /// A03-4: a preview whose process stopped reading (a paused ffplay window) is dropped at
    /// once: the process is killed before the blocked writer is joined.
    #[test]
    fn dropping_a_stalled_preview_does_not_hang() {
        // `sh -c 'exec sleep 30'` never reads its stdin (the rawvideo arguments become $0,
        // $1, …); `exec` keeps it one process, as ffplay is.
        let mut cfg = FfmpegConfig::ffmpeg(Vec::<OsString>::new());
        cfg.program = "sh".into();
        cfg.input_args = vec!["-c".into(), "exec sleep 30".into()];
        cfg.kill_on_finish = true;
        let mut sink = FfmpegSink::new(cfg);
        let (w, h) = (640, 360);
        let px = pattern(w, h, 0);
        let f = frame(w, h, &px, AlphaMode::Opaque);
        // Far more than a pipe holds: the writer thread blocks in a write.
        for _ in 0..8 {
            let _ = sink.send(&f);
            std::thread::sleep(Duration::from_millis(20));
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            drop(sink);
            let _ = tx.send(());
        });
        assert!(
            rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "dropping the sink hangs"
        );
    }

    #[test]
    fn missing_program_is_a_clear_error() {
        let mut cfg = FfmpegConfig::ffmpeg(["-f", "null", "-"]);
        cfg.program = "definitely-not-an-installed-ffmpeg".into();
        let mut sink = FfmpegSink::new(cfg);
        let px = pattern(4, 2, 0);
        let err = sink.send(&frame(4, 2, &px, AlphaMode::Opaque)).unwrap_err();
        assert!(format!("{err:#}").contains("could not start"), "{err:#}");
    }

    #[test]
    fn dead_process_surfaces_as_error() {
        // `true` ignores its arguments and exits without reading stdin.
        let mut cfg = FfmpegConfig::ffmpeg(Vec::<OsString>::new());
        cfg.program = "true".into();
        cfg.input_args.clear();
        let mut sink = FfmpegSink::new(cfg);
        let px = pattern(256, 256, 0);
        let f = frame(256, 256, &px, AlphaMode::Opaque);
        let deadline = Instant::now() + Duration::from_secs(10);
        let err = loop {
            if let Err(e) = sink.send(&f) {
                break e;
            }
            assert!(Instant::now() < deadline, "no error after the child exited");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(
            format!("{err:#}").contains("stopped accepting frames"),
            "{err:#}"
        );
    }

    #[test]
    fn format_change_is_rejected() {
        if !have("ffmpeg") {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let mut cfg = FfmpegConfig::ffmpeg(["-f", "null", "-"]);
        cfg.kill_on_finish = true;
        let mut sink = FfmpegSink::new(cfg);
        let a = pattern(8, 8, 0);
        let b = pattern(16, 8, 0);
        sink.send(&frame(8, 8, &a, AlphaMode::Opaque)).unwrap();
        let err = sink.send(&frame(16, 8, &b, AlphaMode::Opaque)).unwrap_err();
        assert!(err.to_string().contains("format changed"));
        sink.finish().unwrap();
    }
}
