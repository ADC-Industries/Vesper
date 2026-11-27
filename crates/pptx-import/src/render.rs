//! The picture fallback: slides drawn by LibreOffice (headless), an optional program on the
//! server. Each slide that needs a picture is cut into its own one-slide package (the slide,
//! its layout, master, theme and media: the parts it reaches through its relationships) and
//! all of them are converted to PNG in one LibreOffice run, with a profile of its own (so a
//! LibreOffice the operator has open doesn't interfere) and a time limit. Nothing here blocks
//! an import: every failure is a message, and the import goes on editable only.

use crate::opc::{Rels, child, children, is};
use crate::zip::{Archive, Writer};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How slide pictures are made.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Renderer {
    /// No pictures (only the deck's own thumbnail, for its first slide).
    Off,
    /// LibreOffice when found: `VESPER_SOFFICE`, else `soffice` / `libreoffice` on the `PATH`,
    /// else its usual install folder on Windows.
    #[default]
    Auto,
    /// This program, called like LibreOffice's `soffice`.
    Program(PathBuf),
}

impl Renderer {
    /// The program to run, if any.
    pub fn program(&self) -> Option<PathBuf> {
        match self {
            Renderer::Off => None,
            Renderer::Program(p) => Some(p.clone()),
            Renderer::Auto => find_soffice(),
        }
    }
}

/// LibreOffice's `soffice`, where an install puts it.
pub fn find_soffice() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("VESPER_SOFFICE").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let names: &[&str] = if cfg!(windows) {
        &["soffice.exe", "soffice.com"]
    } else {
        &["soffice", "libreoffice"]
    };
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            for n in names {
                let p = dir.join(n);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(base) = std::env::var_os(var) {
                let p = Path::new(&base).join("LibreOffice/program/soffice.exe");
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// One LibreOffice run at a time on a server (it's heavy).
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// The slides of the deck, as `p:sldIdLst` lists them.
pub struct SlideList<'x> {
    /// `presentation.xml`'s text.
    pub xml: &'x str,
    /// Byte range of `p:sldIdLst`, and of each `p:sldId` with its relationship ID.
    pub list: std::ops::Range<usize>,
    pub items: Vec<(std::ops::Range<usize>, String)>,
}

impl<'x> SlideList<'x> {
    /// Reads the list from `presentation.xml`: the slides whose relationship `is_slide` says
    /// leads to a slide part (the same slides, in the same order, as the import reads, so an
    /// index means the same slide in both).
    pub fn read(xml: &'x str, is_slide: impl Fn(&str) -> bool) -> Result<Self> {
        let doc = crate::opc::xml(xml)?;
        let lst = child(doc.root_element(), "sldIdLst").context("no slide list")?;
        // The relationship ID (`r:id`), not the numeric `id`.
        let items = children(lst, "sldId")
            .filter_map(|s| {
                let rid = s
                    .attributes()
                    .find(|a| a.name() == "id" && a.namespace().is_some())?
                    .value()
                    .to_string();
                is_slide(&rid).then(|| (s.range(), rid))
            })
            .collect();
        Ok(SlideList {
            xml,
            list: lst.range(),
            items,
        })
    }

    /// `presentation.xml` listing only slide `keep` (an index into `items`).
    fn only(&self, keep: usize) -> String {
        let (r, _) = &self.items[keep];
        // The list's start tag, then the kept slide, then the list's end tag.
        let tag = &self.xml[self.list.start + 1..];
        let open_len = tag.find('>').map(|i| i + 2).unwrap_or(0);
        let name: String = tag
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/')
            .collect();
        format!(
            "{}{}</{name}>{}",
            &self.xml[..self.list.start + open_len],
            &self.xml[r.clone()],
            &self.xml[self.list.end..]
        )
    }
}

/// `presentation.xml.rels` without the slide relationships other than `keep_rid`.
fn rels_without_other_slides(xml: &str, keep_rid: &str) -> Result<(String, BTreeSet<String>)> {
    let doc = crate::opc::xml(xml)?;
    let mut cut = Vec::new();
    let mut dropped = BTreeSet::new();
    for r in doc
        .root_element()
        .children()
        .filter(|n| is(n, "Relationship"))
    {
        let slide = r.attribute("Type").is_some_and(|t| t.ends_with("/slide"));
        if slide && r.attribute("Id") != Some(keep_rid) {
            cut.push(r.range());
            if let Some(id) = r.attribute("Id") {
                dropped.insert(id.to_string());
            }
        }
    }
    let mut out = String::with_capacity(xml.len());
    let mut at = 0;
    for c in cut {
        out.push_str(&xml[at..c.start]);
        at = c.end;
    }
    out.push_str(&xml[at..]);
    Ok((out, dropped))
}

/// `[Content_Types].xml` without the overrides of parts `kept` says are left out.
fn without_missing_overrides(xml: &str, kept: impl Fn(&str) -> bool) -> Result<String> {
    let doc = crate::opc::xml(xml)?;
    let mut out = String::with_capacity(xml.len());
    let mut at = 0;
    for o in doc.root_element().children().filter(|n| is(n, "Override")) {
        let part = o.attribute("PartName").unwrap_or_default();
        if !kept(part.trim_start_matches('/')) {
            out.push_str(&xml[at..o.range().start]);
            at = o.range().end;
        }
    }
    out.push_str(&xml[at..]);
    Ok(out)
}

/// Relationship kinds a slide's picture doesn't need: embedded videos and sounds (LibreOffice
/// draws a video's poster picture, which is its own `image` relationship). They can be larger
/// than the import reads, and are left out of the one-slide packages.
const NOT_DRAWN: &[&str] = &["video", "audio", "media"];

/// True when relationship `kind` (`TargetMode` `external`) stays in a one-slide package.
/// External targets (web addresses, files on the server) never reach LibreOffice, which
/// would fetch or read them while drawing; only hyperlinks stay, which it doesn't follow.
fn kept_rel(kind: &str, external: bool) -> bool {
    if external {
        kind == "hyperlink"
    } else {
        !NOT_DRAWN.contains(&kind)
    }
}

/// A `.rels` file without the relationships [`kept_rel`] leaves out.
fn rels_for_package(xml: &str) -> Result<String> {
    let doc = crate::opc::xml(xml)?;
    let mut out = String::with_capacity(xml.len());
    let mut at = 0;
    for r in doc
        .root_element()
        .children()
        .filter(|n| is(n, "Relationship"))
    {
        let kind = r
            .attribute("Type")
            .and_then(|t| t.rsplit('/').next())
            .unwrap_or_default();
        let external = r
            .attribute("TargetMode")
            .is_some_and(|m| m.eq_ignore_ascii_case("External"));
        if !kept_rel(kind, external) {
            out.push_str(&xml[at..r.range().start]);
            at = r.range().end;
        }
    }
    out.push_str(&xml[at..]);
    Ok(out)
}

fn rels_name(part: &str) -> String {
    match part.rfind('/') {
        Some(i) => format!("{}/_rels/{}.rels", &part[..i], &part[i + 1..]),
        None => format!("_rels/{part}.rels"),
    }
}

/// A package holding only slide `keep` of the deck.
pub fn one_slide_package(
    zip: &Archive,
    pres_part: &str,
    slides: &SlideList,
    keep: usize,
) -> Result<Vec<u8>> {
    let (_, rid) = slides.items.get(keep).context("no such slide")?;
    let pres_rels_name = rels_name(pres_part);
    let pres_rels_xml = zip.text(&pres_rels_name)?;
    let (pres_rels, dropped) = rels_without_other_slides(&pres_rels_xml, rid)?;
    let pres_xml = slides.only(keep);
    // The parts the kept slide (and the rest of the package) reach.
    let mut keep_parts: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for r in Rels::of(zip, "").0.values() {
        if !r.external {
            queue.push_back(r.target.clone());
        }
    }
    while let Some(p) = queue.pop_front() {
        if !keep_parts.insert(p.clone()) || !zip.has(&p) {
            continue;
        }
        for (id, r) in Rels::of(zip, &p).0 {
            if r.external || !kept_rel(&r.kind, false) || (p == pres_part && dropped.contains(&id))
            {
                continue;
            }
            queue.push_back(r.target);
        }
    }
    let mut w = Writer::new();
    let types = zip.text("[Content_Types].xml")?;
    w.add(
        "[Content_Types].xml",
        without_missing_overrides(&types, |p| {
            p.eq_ignore_ascii_case(pres_part) || keep_parts.contains(p)
        })?
        .as_bytes(),
    )?;
    w.add(
        "_rels/.rels",
        rels_for_package(&zip.text("_rels/.rels")?)?.as_bytes(),
    )?;
    for e in zip.entries() {
        let n = e.name.as_str();
        if n == "[Content_Types].xml" || n == "_rels/.rels" || n.ends_with('/') {
            continue;
        }
        if n.eq_ignore_ascii_case(pres_part) {
            w.add(n, pres_xml.as_bytes())?;
        } else if n.eq_ignore_ascii_case(&pres_rels_name) {
            w.add(n, rels_for_package(&pres_rels)?.as_bytes())?;
        } else if n
            .strip_suffix(".rels")
            .and_then(|s| s.split_once("/_rels/").map(|(d, f)| format!("{d}/{f}")))
            .is_some_and(|p| keep_parts.contains(&p))
        {
            // A kept part's relationships, without the external and media ones.
            if let Ok(x) = zip.text(n).and_then(|x| rels_for_package(&x)) {
                w.add(n, x.as_bytes())?;
            }
        } else if keep_parts.contains(n) {
            // A part that can't be read (too large, broken) is left out: LibreOffice draws
            // the slide without it.
            if let Ok(b) = zip.read_entry(e) {
                w.add(n, &b)?;
            }
        }
    }
    w.finish()
}

/// Draws slides `which` (indices into `slides.items`) as PNGs about `width`×`height` with
/// `program` (LibreOffice's `soffice`). Returns the pictures it made; slides it couldn't
/// draw are missing from the map.
pub fn render(
    program: &Path,
    zip: &Archive,
    pres_part: &str,
    slides: &SlideList,
    which: &[usize],
    size: (u32, u32),
    work: &Path,
) -> Result<HashMap<usize, Vec<u8>>> {
    if which.is_empty() {
        return Ok(HashMap::new());
    }
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let input = work.join("in");
    let output = work.join("out");
    let profile = work.join("profile");
    for d in [&input, &output, &profile] {
        std::fs::create_dir_all(d)?;
    }
    let mut files = Vec::new();
    let mut failed = None;
    for &i in which {
        // A slide whose package can't be made goes without its picture; the others still
        // get theirs.
        let pkg = match one_slide_package(zip, pres_part, slides, i) {
            Ok(p) => p,
            Err(e) => {
                failed.get_or_insert(e);
                continue;
            }
        };
        let f = input.join(format!("slide-{}.pptx", i + 1));
        std::fs::write(&f, pkg)?;
        files.push((i, f));
    }
    if files.is_empty() {
        return Err(failed
            .unwrap_or_else(|| anyhow::anyhow!("no slide could be prepared"))
            .context("the slides couldn't be prepared for LibreOffice"));
    }
    let filter = format!(
        "png:impress_png_Export:{{\"PixelWidth\":{{\"type\":\"long\",\"value\":\"{}\"}},\"PixelHeight\":{{\"type\":\"long\",\"value\":\"{}\"}}}}",
        size.0, size.1
    );
    let profile_url = file_url(&profile);
    let mut cmd = Command::new(program);
    cmd.arg("--headless")
        .arg("--invisible")
        .arg("--norestore")
        .arg("--nolockcheck")
        .arg("--nodefault")
        .arg(format!("-env:UserInstallation={profile_url}"))
        .arg("--convert-to")
        .arg(&filter)
        .arg("--outdir")
        .arg(&output)
        .args(files.iter().map(|(_, f)| f))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let limit = Duration::from_secs(60 + 10 * which.len() as u64);
    let (status, err) = run_limited(&mut cmd, limit)
        .with_context(|| format!("couldn't start {}", program.display()))?
        .map_err(|()| {
            anyhow::anyhow!(
                "LibreOffice took longer than {} s: stopped",
                limit.as_secs()
            )
        })?;
    let mut out = HashMap::new();
    for (i, f) in &files {
        let stem = f.file_stem().unwrap_or_default();
        let png = output.join(stem).with_extension("png");
        if let Ok(b) = std::fs::read(&png)
            && image::ImageReader::new(std::io::Cursor::new(&b))
                .with_guessed_format()
                .ok()
                .and_then(|r| r.into_dimensions().ok())
                .is_some()
        {
            out.insert(*i, b);
        }
    }
    if out.is_empty() {
        bail!(
            "LibreOffice made no pictures (exit {status}){}",
            err.lines()
                .next()
                .map(|l| format!(": {l}"))
                .unwrap_or_default()
        );
    }
    Ok(out)
}

/// Runs `cmd` (its standard error piped) for at most `limit`: its exit status and standard
/// error, or `Err(())` when it took longer. LibreOffice's `soffice` is a launcher that starts
/// the real program as its own child, so the command runs in a process group of its own
/// (Unix) or a job object (Windows), and a run that takes too long is stopped with
/// everything it started, not just the launcher.
pub fn run_limited(
    cmd: &mut Command,
    limit: Duration,
) -> std::io::Result<Result<(ExitStatus, String), ()>> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn()?;
    let group = Group::of(&child);
    // Standard error is read as it comes (a full pipe would stall the program).
    let reader = child.stderr.take().map(|mut e| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut s = String::new();
            let _ = e.read_to_string(&mut s);
            s
        })
    });
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break Some(s);
        }
        if start.elapsed() > limit {
            group.kill();
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    // Whatever the launcher left behind goes too (its work folder is removed next).
    group.kill();
    let err = match status {
        // A program still holding the pipe after the kill doesn't hold this thread up.
        Some(_) => reader.and_then(|r| r.join().ok()).unwrap_or_default(),
        None => String::new(),
    };
    Ok(status.map(|s| (s, err)).ok_or(()))
}

/// The processes a run started.
struct Group {
    #[cfg(unix)]
    pgid: i32,
    #[cfg(windows)]
    job: Option<windows_sys::Win32::Foundation::HANDLE>,
}

impl Group {
    #[cfg(unix)]
    fn of(child: &std::process::Child) -> Group {
        Group {
            pgid: child.id() as i32,
        }
    }

    /// Stops every process of the group, and waits (briefly) until they are gone.
    #[cfg(unix)]
    fn kill(&self) {
        // SAFETY: kill(2) with a negative pid signals the process group we created.
        if unsafe { libc::kill(-self.pgid, libc::SIGKILL) } != 0 {
            return;
        }
        let start = Instant::now();
        // Signal 0 tells whether any member is left (they are reaped by init once dead).
        while unsafe { libc::kill(-self.pgid, 0) } == 0 && start.elapsed() < Duration::from_secs(5)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(windows)]
    fn of(child: &std::process::Child) -> Group {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        // SAFETY: plain Win32 calls on a job we create and the child's own process handle.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Group { job: None };
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            AssignProcessToJobObject(job, child.as_raw_handle() as _);
            Group { job: Some(job) }
        }
    }

    /// Stops every process of the job.
    #[cfg(windows)]
    fn kill(&self) {
        if let Some(job) = self.job {
            // SAFETY: the job handle is ours until drop.
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(job, 1);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for Group {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            // SAFETY: closing our own handle (which also ends what is left in the job).
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(job);
            }
        }
    }
}

fn file_url(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.replace(' ', "%20");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// A run that takes too long is stopped with what it started: LibreOffice's launcher
    /// starts the real program as its own child, which a plain kill left running.
    #[test]
    fn a_run_over_its_limit_is_stopped_with_its_children() {
        let dir = std::env::temp_dir().join(format!("pptx-run-limited-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("pid");
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(format!(
                "sleep 300 & echo $! > '{}'; wait",
                pidfile.display()
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let r = run_limited(&mut cmd, Duration::from_secs(1)).unwrap();
        assert!(r.is_err(), "over its limit");
        let pid: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // Gone (or a zombie waiting for init to reap it).
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        let state = stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .unwrap_or("");
        assert!(
            stat.is_empty() || state == "Z",
            "the child still runs: {stat}"
        );

        // A run within its limit gives its status and standard error.
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("echo oops >&2; exit 3")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let (status, err) = run_limited(&mut cmd, Duration::from_secs(30))
            .unwrap()
            .unwrap();
        assert_eq!(status.code(), Some(3));
        assert_eq!(err.trim(), "oops");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
