//! Conversions in a child process of the server's own (`docs/MEDIA-PREP-PLAN.md`, "The
//! converter"): `vesper media-convert --config FILE` (a hidden subcommand of the `vesper`
//! binary) runs IDN's converter on one file and writes its progress as JSON lines on stdout.
//! The server starts it with the job's settings at the job's priority, reads the lines, and
//! cancels it by killing it (the partial output removed). A crash inside FFmpeg ends only the
//! child: the job fails with a note, the server and the show go on.
//!
//! Priorities ([`Level`]): the child sets its own at start on Linux (nice 19 and the idle I/O
//! class, before any thread of its own starts, so every thread IDN starts has them too);
//! Windows starts it in its priority class (below normal, or idle while something is live).
//! Once something goes live, a child at the normal priority is lowered to the lowest
//! ([`Running::lower`]: every thread of it on Linux, its class on Windows); it stays there
//! until it ends (Linux doesn't let a process raise a priority again without privileges).
//!
//! The child also stops when its stdin closes: the server holds it open for the child's life,
//! so a server that went away (killed, crashed) leaves no conversion running. It ignores the
//! stop signals (SIGTERM, SIGINT, SIGHUP) a service stop or a Ctrl-C sends the whole group:
//! the server stops it, and keeps its job for the next start.

use media::convert::{ConvertError, ConvertOptions, Converted};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// The subcommand of the `vesper` binary that runs one conversion.
pub const SUBCOMMAND: &str = "media-convert";

/// How conversions run (the server's [`crate::CodexOptions::converter`]).
#[derive(Clone, Debug, Default)]
pub struct Converter {
    /// The program run as `PROGRAM media-convert --config FILE` (the `vesper` binary). `None`:
    /// in the server's own process, on a thread (tests and embedders without the binary).
    pub program: Option<PathBuf>,
    /// Test hooks handed to the child.
    pub test: TestHook,
}

impl Converter {
    /// The running program itself (the `vesper` server): on Linux `/proc/self/exe`, which
    /// still starts the same program after a package update replaced its file.
    pub fn this_program() -> Converter {
        #[cfg(target_os = "linux")]
        let program = Some(PathBuf::from("/proc/self/exe"));
        #[cfg(not(target_os = "linux"))]
        let program = std::env::current_exe().ok();
        Converter {
            program,
            test: TestHook::default(),
        }
    }
}

/// The priority a conversion runs at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    /// As the server (the setting `priority: normal`, nothing live): IDN leaves its threads
    /// as they are.
    Normal,
    /// Low (the default): Linux nice 19 and idle I/O; Windows below normal; IDN's low
    /// priority.
    #[default]
    Low,
    /// The lowest (something is live): Linux as [`Level::Low`]; Windows idle.
    Lowest,
}

/// Test hooks: how a child misbehaves on purpose.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TestHook {
    /// The child aborts (an abnormal end, as a crash inside FFmpeg would be) at its first
    /// progress, or before it reports when none came.
    pub crash: bool,
    /// The child stops at its first progress (after reporting it) until it is killed.
    pub stall: bool,
}

/// One conversion, as the server hands it to the child (`--config FILE`, JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    /// The file converted.
    pub input: PathBuf,
    /// The MP4 written (IDN writes `OUTPUT.part` and renames it once complete).
    pub output: PathBuf,
    /// HEVC instead of H.264.
    pub hevc: bool,
    /// The constant frame rate (numerator, denominator).
    #[serde(default)]
    pub frame_rate: Option<(u32, u32)>,
    /// Fitted inside this size.
    #[serde(default)]
    pub max_size: Option<(u32, u32)>,
    /// Every picture deinterlaced.
    #[serde(default)]
    pub deinterlace_all: bool,
    /// The video bit rate, kbit/s.
    #[serde(default)]
    pub video_kbps: Option<u32>,
    /// The GPU's encoder where there is one.
    pub gpu: bool,
    /// The priority it starts at.
    #[serde(default)]
    pub level: Level,
    /// Test hooks.
    #[serde(default)]
    pub test: TestHook,
}

impl Job {
    /// A job converting `input` into `output` with `o`, at `level`.
    pub fn new(input: &Path, output: &Path, o: &ConvertOptions, level: Level) -> Job {
        Job {
            input: input.to_path_buf(),
            output: output.to_path_buf(),
            hevc: o.hevc,
            frame_rate: o.frame_rate,
            max_size: o.max_size,
            deinterlace_all: o.deinterlace_all,
            video_kbps: o.video_kbps,
            gpu: o.gpu,
            level,
            test: TestHook::default(),
        }
    }

    /// The converter's options (IDN's low priority unless [`Level::Normal`]).
    pub fn options(&self) -> ConvertOptions {
        ConvertOptions {
            hevc: self.hevc,
            frame_rate: self.frame_rate,
            max_size: self.max_size,
            deinterlace_all: self.deinterlace_all,
            video_kbps: self.video_kbps,
            gpu: self.gpu,
            low_priority: self.level != Level::Normal,
        }
    }
}

/// What a finished conversion made, as the child reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Made {
    encoder: String,
    width: u32,
    height: u32,
    frame_rate: (u32, u32),
    duration: f64,
    frames: u64,
    repeated: u64,
    dropped: u64,
    deinterlaced: u64,
    audio: bool,
    notes: Vec<String>,
    took_ms: u64,
}

impl From<&Converted> for Made {
    fn from(c: &Converted) -> Made {
        Made {
            encoder: c.encoder.clone(),
            width: c.width,
            height: c.height,
            frame_rate: c.frame_rate,
            duration: c.duration,
            frames: c.frames,
            repeated: c.repeated,
            dropped: c.dropped,
            deinterlaced: c.deinterlaced,
            audio: c.audio,
            notes: c.notes.clone(),
            took_ms: c.took.as_millis() as u64,
        }
    }
}

impl From<Made> for Converted {
    fn from(m: Made) -> Converted {
        Converted {
            encoder: m.encoder,
            width: m.width,
            height: m.height,
            frame_rate: m.frame_rate,
            duration: m.duration,
            frames: m.frames,
            repeated: m.repeated,
            dropped: m.dropped,
            deinterlaced: m.deinterlaced,
            audio: m.audio,
            notes: m.notes,
            took: Duration::from_millis(m.took_ms),
        }
    }
}

/// One line the child writes on its stdout.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Line {
    /// How far it is (about four times a second).
    Progress {
        fraction: Option<f64>,
        done: f64,
        speed: f64,
    },
    /// Finished: the output is complete.
    Done(Made),
    /// It failed (nothing left behind).
    Failed { error: String },
    /// Stopped (its stdin closed).
    Cancelled,
}

fn emit(line: &Line) {
    let Ok(text) = serde_json::to_string(line) else {
        return;
    };
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{text}");
    let _ = out.flush();
}

/// The child: `vesper media-convert --config FILE` (`args`: what follows the subcommand).
/// Returns the process's exit code.
pub fn main(args: Vec<OsString>) -> i32 {
    let config = match args.as_slice() {
        [flag, file] if flag == "--config" => PathBuf::from(file),
        _ => {
            eprintln!("usage: vesper {SUBCOMMAND} --config FILE");
            return 2;
        }
    };
    let job: Job = match std::fs::read(&config)
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()))
    {
        Ok(j) => j,
        Err(e) => {
            eprintln!("{SUBCOMMAND}: {}: {e}", config.display());
            return 2;
        }
    };
    // A stop of the server (systemd's stop or restart signals every process of the service;
    // Ctrl-C in a terminal, the whole foreground group) isn't ours to act on: the server
    // stops us (its stop kills us, or our stdin closes as it exits) and keeps the job for its
    // next start. Dying of the signal first would look like a crash (R1-1).
    #[cfg(unix)]
    ignore_stop_signals();
    // Before any thread starts: threads inherit it.
    set_own_priority(job.level);
    #[cfg(unix)]
    if job.test.crash {
        // A crash on purpose leaves no core file behind.
        let none = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: a plain syscall with a valid struct.
        unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) };
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        // The server holds our stdin open while it waits for us: its end stops us.
        let cancel = cancel.clone();
        let _ = std::thread::Builder::new()
            .name("stdin".into())
            .spawn(move || {
                let mut sink = [0u8; 256];
                let mut stdin = std::io::stdin();
                while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
                cancel.store(true, Ordering::Relaxed);
            });
    }
    let mut first = true;
    let r = media::convert::convert(
        &job.input,
        &job.output,
        &job.options(),
        |p| {
            if first {
                first = false;
                if job.test.crash {
                    crash();
                }
            }
            emit(&Line::Progress {
                fraction: p.fraction,
                done: p.done,
                speed: p.speed,
            });
            if job.test.stall {
                loop {
                    std::thread::sleep(Duration::from_secs(3600));
                }
            }
        },
        &cancel,
    );
    if job.test.crash {
        crash();
    }
    emit(&match r {
        Ok(c) => Line::Done(Made::from(&c)),
        Err(ConvertError::Cancelled) => Line::Cancelled,
        Err(ConvertError::Failed(error)) => Line::Failed { error },
    });
    0
}

/// The test hook's crash: its last words on stderr (as FFmpeg or a driver says something
/// before an abort), then an abnormal end.
fn crash() -> ! {
    eprintln!("{SUBCOMMAND}: crashing on purpose (test hook)");
    std::process::abort();
}

/// SIGTERM, SIGINT and SIGHUP ignored (before any thread starts: the whole process).
#[cfg(unix)]
fn ignore_stop_signals() {
    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        // SAFETY: setting a signal's disposition to "ignore"; no handler runs.
        unsafe { libc::signal(sig, libc::SIG_IGN) };
    }
}

/// The calling thread's priority (the child's first thread, before others start).
fn set_own_priority(level: Level) {
    #[cfg(target_os = "linux")]
    linux::lower(0, level);
    #[cfg(windows)]
    let _ = level; // Started in its class (`creation_flags`).
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = level;
}

#[cfg(target_os = "linux")]
mod linux {
    use super::Level;

    const IOPRIO_WHO_PROCESS: libc::c_long = 1;
    const IOPRIO_CLASS_IDLE: libc::c_long = 3;
    const IOPRIO_CLASS_SHIFT: libc::c_long = 13;

    /// One thread (`tid` 0: the calling one) at `level`: nice 19 and the idle I/O class below
    /// normal; nothing at normal.
    pub fn lower(tid: libc::pid_t, level: Level) {
        if level == Level::Normal {
            return;
        }
        // SAFETY: plain syscalls on a thread id.
        unsafe {
            if libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, 19) != 0 {
                tracing::debug!(
                    "lowering a conversion's priority: {}",
                    std::io::Error::last_os_error()
                );
            }
            libc::syscall(
                libc::SYS_ioprio_set,
                IOPRIO_WHO_PROCESS,
                tid as libc::c_long,
                IOPRIO_CLASS_IDLE << IOPRIO_CLASS_SHIFT,
            );
        }
    }

    /// Every thread of process `pid` at `level`.
    pub fn lower_process(pid: u32, level: Level) {
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
            return;
        };
        for t in tasks.flatten() {
            if let Some(tid) = t.file_name().to_str().and_then(|n| n.parse().ok()) {
                lower(tid, level);
            }
        }
    }
}

#[cfg(windows)]
fn priority_class(level: Level) -> u32 {
    use windows_sys::Win32::System::Threading::{
        BELOW_NORMAL_PRIORITY_CLASS, IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS,
    };
    match level {
        Level::Normal => NORMAL_PRIORITY_CLASS,
        Level::Low => BELOW_NORMAL_PRIORITY_CLASS,
        Level::Lowest => IDLE_PRIORITY_CLASS,
    }
}

/// A conversion running in a child process.
pub struct Running {
    child: Child,
    lines: mpsc::Receiver<Line>,
    /// The end of what the child wrote on stderr.
    stderr: Arc<Mutex<String>>,
    /// The thread reading it (done once the child's stderr closes).
    stderr_reader: Option<std::thread::JoinHandle<()>>,
    level: Level,
    output: PathBuf,
    config: PathBuf,
    ended: bool,
}

/// How much of a child's stderr is kept.
const STDERR_KEPT: usize = 2000;

impl Running {
    /// Starts `program media-convert` on `job`, its configuration written to `config`. On
    /// Linux the child starts at the calling thread's priority (then lowers itself to the
    /// job's): call it from a thread at the server's own.
    pub fn spawn(program: &Path, job: &Job, config: &Path) -> std::io::Result<Running> {
        let json = serde_json::to_vec(job).map_err(std::io::Error::other)?;
        std::fs::write(config, json)?;
        let mut cmd = Command::new(program);
        cmd.arg(SUBCOMMAND)
            .arg("--config")
            .arg(config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
            cmd.creation_flags(CREATE_NO_WINDOW | priority_class(job.level));
        }
        // The stop signals ignored from the very start, not only once the child's `main` runs:
        // an "ignore" disposition survives exec, so a service stop that lands while the child
        // is still starting can't kill it either (R1-1).
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: only `signal(2)` runs between fork and exec; it is async-signal-safe.
            unsafe {
                cmd.pre_exec(|| {
                    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
                        libc::signal(sig, libc::SIG_IGN);
                    }
                    Ok(())
                });
            }
        }
        let mut child = cmd.spawn()?;
        let (tx, lines) = mpsc::channel();
        if let Some(out) = child.stdout.take() {
            std::thread::Builder::new()
                .name("media-convert-out".into())
                .spawn(move || {
                    for l in std::io::BufReader::new(out).lines() {
                        let Ok(l) = l else { break };
                        match serde_json::from_str::<Line>(&l) {
                            Ok(line) => {
                                if tx.send(line).is_err() {
                                    break;
                                }
                            }
                            Err(_) => tracing::debug!("media conversion: {l}"),
                        }
                    }
                })?;
        }
        let stderr = Arc::new(Mutex::new(String::new()));
        let mut stderr_reader = None;
        if let Some(mut err) = child.stderr.take() {
            let kept = stderr.clone();
            let reader = std::thread::Builder::new()
                .name("media-convert-err".into())
                .spawn(move || {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = err.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        let mut k = crate::lock(&kept);
                        k.push_str(&String::from_utf8_lossy(&buf[..n]));
                        if k.len() > STDERR_KEPT * 2 {
                            let cut = k.len() - STDERR_KEPT;
                            let cut = (cut..k.len()).find(|i| k.is_char_boundary(*i)).unwrap_or(0);
                            k.replace_range(..cut, "");
                        }
                    }
                })?;
            stderr_reader = Some(reader);
        }
        Ok(Running {
            child,
            lines,
            stderr,
            stderr_reader,
            level: job.level,
            output: job.output.clone(),
            config: config.to_path_buf(),
            ended: false,
        })
    }

    /// The child's process id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The priority it runs at now.
    pub fn level(&self) -> Level {
        self.level
    }

    /// Lowers it to the lowest priority (something went live).
    pub fn lower(&mut self) {
        if self.level == Level::Lowest || self.ended {
            return;
        }
        #[cfg(target_os = "linux")]
        linux::lower_process(self.child.id(), Level::Lowest);
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::Threading::SetPriorityClass;
            // SAFETY: the child's process handle, open while `self.child` lives.
            unsafe {
                SetPriorityClass(
                    self.child.as_raw_handle() as _,
                    priority_class(Level::Lowest),
                );
            }
        }
        self.level = Level::Lowest;
    }

    /// Waits for the conversion: `progress` (0 to 1) as it goes; killed once `stop` says so
    /// (asked every quarter second; its partial output removed: [`ConvertError::Cancelled`]);
    /// lowered to the lowest priority once `lower` says so. A child that ended without
    /// saying how (a crash) is a failure with a note.
    pub fn wait(
        mut self,
        stop: &dyn Fn() -> bool,
        lower: &dyn Fn() -> bool,
        mut progress: impl FnMut(f64),
    ) -> Result<Converted, ConvertError> {
        let mut last = None;
        loop {
            if stop() {
                self.kill();
                return Err(ConvertError::Cancelled);
            }
            if lower() {
                self.lower();
            }
            match self.lines.recv_timeout(Duration::from_millis(250)) {
                Ok(Line::Progress { fraction, .. }) => {
                    if let Some(x) = fraction {
                        progress(x);
                    }
                }
                Ok(line) => {
                    last = Some(line);
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                // Its stdout closed: it ended.
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let status = self.child.wait();
        self.ended = true;
        match last {
            Some(Line::Done(m)) => Ok(m.into()),
            Some(Line::Failed { error }) => Err(ConvertError::Failed(error)),
            Some(Line::Cancelled) => {
                remove_partial(&self.output);
                Err(ConvertError::Cancelled)
            }
            Some(Line::Progress { .. }) | None => {
                remove_partial(&self.output);
                let how = match &status {
                    Ok(s) => ended_how(s),
                    Err(e) => format!("it couldn't be waited for: {e}"),
                };
                // Its last words may still be in the pipe: read to the end first (the pipe
                // closes with the child; a grandchild holding it open isn't waited for long).
                if let Some(reader) = self.stderr_reader.take() {
                    let until = std::time::Instant::now() + Duration::from_secs(2);
                    while !reader.is_finished() && std::time::Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    if reader.is_finished() {
                        let _ = reader.join();
                    }
                }
                let tail = crate::lock(&self.stderr)
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .map(|l| format!(" Its last words: {}", l.trim()))
                    .unwrap_or_default();
                Err(ConvertError::Failed(format!(
                    "the converter crashed ({how}); the server and the show weren't affected.{tail}"
                )))
            }
        }
    }

    /// Kills the child and removes its partial output.
    fn kill(&mut self) {
        if !self.ended {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.ended = true;
        }
        remove_partial(&self.output);
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.ended {
            self.kill();
        }
        let _ = std::fs::remove_file(&self.config);
    }
}

/// How a child ended, for people.
fn ended_how(s: &std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = s.signal() {
            let name = match sig {
                libc::SIGSEGV => " (SIGSEGV)",
                libc::SIGABRT => " (SIGABRT)",
                libc::SIGBUS => " (SIGBUS)",
                libc::SIGILL => " (SIGILL)",
                libc::SIGFPE => " (SIGFPE)",
                libc::SIGKILL => " (SIGKILL: killed, out of memory?)",
                _ => "",
            };
            return format!("signal {sig}{name}");
        }
    }
    match s.code() {
        Some(c) if !(0..=0xFFFF).contains(&c) => format!("exit code {:#010x}", c as u32),
        Some(c) => format!("exit code {c}"),
        None => "no exit code".into(),
    }
}

/// What a stopped conversion leaves: the output and IDN's `OUTPUT.part`.
pub fn remove_partial(output: &Path) {
    let mut part = output.as_os_str().to_owned();
    part.push(".part");
    for f in [output.to_path_buf(), PathBuf::from(part)] {
        let _ = std::fs::remove_file(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_round_trip() {
        let lines = [
            Line::Progress {
                fraction: Some(0.25),
                done: 1.5,
                speed: 4.0,
            },
            Line::Failed {
                error: "converting x.mov: Invalid data".into(),
            },
            Line::Cancelled,
            Line::Done(Made {
                encoder: "OpenH264".into(),
                width: 320,
                height: 180,
                frame_rate: (25, 1),
                duration: 2.0,
                frames: 50,
                repeated: 0,
                dropped: 0,
                deinterlaced: 0,
                audio: true,
                notes: vec![],
                took_ms: 400,
            }),
        ];
        for l in lines {
            let t = serde_json::to_string(&l).unwrap();
            assert!(!t.contains('\n'));
            assert_eq!(serde_json::from_str::<Line>(&t).unwrap(), l);
        }
        let c: Converted = Made {
            encoder: "VA-API".into(),
            width: 1920,
            height: 1080,
            frame_rate: (30000, 1001),
            duration: 1.0,
            frames: 30,
            repeated: 1,
            dropped: 2,
            deinterlaced: 3,
            audio: false,
            notes: vec!["n".into()],
            took_ms: 1500,
        }
        .into();
        assert_eq!(c.took, Duration::from_millis(1500));
        assert_eq!(Made::from(&c).took_ms, 1500);
    }

    #[test]
    fn a_job_keeps_the_options_and_the_level() {
        let o = ConvertOptions {
            hevc: true,
            frame_rate: Some((25, 1)),
            max_size: Some((1920, 1080)),
            deinterlace_all: false,
            video_kbps: Some(9_000),
            gpu: false,
            low_priority: true,
        };
        let j = Job::new(Path::new("/in.mov"), Path::new("/out.mp4"), &o, Level::Low);
        let back: Job = serde_json::from_str(&serde_json::to_string(&j).unwrap()).unwrap();
        assert_eq!(back, j);
        assert_eq!(back.options(), o);
        let normal = Job {
            level: Level::Normal,
            ..j
        };
        assert!(!normal.options().low_priority);
    }

    #[test]
    fn a_bad_command_line_is_refused() {
        assert_eq!(main(vec![]), 2);
        assert_eq!(
            main(vec!["--config".into(), "/nonexistent/x.json".into()]),
            2
        );
    }

    #[test]
    fn partial_outputs_are_removed() {
        let dir = std::env::temp_dir().join(format!("vesper-partial-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("Clip (prepared).mp4");
        std::fs::write(&out, b"x").unwrap();
        std::fs::write(dir.join("Clip (prepared).mp4.part"), b"x").unwrap();
        std::fs::write(dir.join("other.mp4"), b"x").unwrap();
        remove_partial(&out);
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("other.mp4")]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
