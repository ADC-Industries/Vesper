//! PNG output: a single snapshot or every Nth frame, encoded on a worker thread.
//!
//! Opaque screens are written as 8-bit RGB (flattened over black); screens with alpha as
//! 8-bit straight RGBA. Files carry an sRGB chunk.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use crate::convert::{self, PixelFormat};
use crate::worker::{Task, Worker};
use crate::{Frame, FrameSink};

/// Encodes `frame` as PNG into `w`.
pub fn encode_png(frame: &Frame<'_>, w: impl Write) -> Result<()> {
    frame.validate()?;
    let mut scratch = Vec::new();
    encode_raw(
        frame.pixels,
        frame.width,
        frame.height,
        frame.has_alpha(),
        &mut scratch,
        w,
    )
}

/// Writes `frame` to `path` as PNG (synchronously).
pub fn write_png(path: impl AsRef<Path>, frame: &Frame<'_>) -> Result<()> {
    let path = path.as_ref();
    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut w = BufWriter::new(file);
    encode_png(frame, &mut w)?;
    w.flush()
        .with_context(|| format!("writing {}", path.display()))
}

fn encode_raw(
    premul: &[u8],
    width: u32,
    height: u32,
    alpha: bool,
    scratch: &mut Vec<u8>,
    w: impl Write,
) -> Result<()> {
    let (format, color) = if alpha {
        (PixelFormat::Rgba, png::ColorType::Rgba)
    } else {
        (PixelFormat::Rgb, png::ColorType::Rgb)
    };
    convert::convert(premul, width, height, format, scratch)?;
    let mut enc = png::Encoder::new(w, width, height);
    enc.set_color(color);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = enc.write_header()?;
    writer.write_image_data(scratch)?;
    writer.finish()?;
    Ok(())
}

/// What a [`PngSink`] writes.
#[derive(Clone, Debug)]
enum Mode {
    /// The first frame received, to this path; later frames are ignored.
    Snapshot(PathBuf),
    /// Frames 0, n, 2n, ... into `dir` as `{prefix}{index:06}.png` (index = frame count).
    EveryNth {
        dir: PathBuf,
        prefix: String,
        n: u64,
    },
}

struct Meta {
    path: PathBuf,
    width: u32,
    height: u32,
    alpha: bool,
}

struct PngTask {
    scratch: Vec<u8>,
}

impl Task<Meta> for PngTask {
    fn process(&mut self, data: &[u8], m: Meta) -> Result<()> {
        let file =
            File::create(&m.path).with_context(|| format!("creating {}", m.path.display()))?;
        let mut w = BufWriter::new(file);
        encode_raw(data, m.width, m.height, m.alpha, &mut self.scratch, &mut w)?;
        w.flush()
            .with_context(|| format!("writing {}", m.path.display()))?;
        log::debug!("wrote {}", m.path.display());
        Ok(())
    }
}

/// Writes frames as PNG files: a single snapshot or every Nth frame.
///
/// `send` only copies the frame; encoding happens on a worker thread. If the worker is still
/// busy with earlier frames, the frame is dropped (counted in [`PngSink::dropped`]). Dropping
/// the sink (or calling [`PngSink::finish`]) waits for queued files to be written.
pub struct PngSink {
    name: String,
    mode: Mode,
    frames_seen: u64,
    written: u64,
    done: bool,
    worker: Worker<Meta>,
}

impl PngSink {
    /// Writes the next frame received to `path`, then ignores further frames.
    pub fn snapshot(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        Self::new(
            format!("PNG snapshot: {}", path.display()),
            Mode::Snapshot(path),
        )
    }

    /// Writes frame 0 and then every `n`th frame into `dir` as `frame_000000.png`, ...
    /// (the number is the index of the frame received by this sink).
    pub fn every_nth(dir: impl Into<PathBuf>, n: u64) -> Result<Self> {
        Self::every_nth_with_prefix(dir, "frame_", n)
    }

    /// Like [`PngSink::every_nth`] with a custom file name prefix.
    pub fn every_nth_with_prefix(
        dir: impl Into<PathBuf>,
        prefix: impl Into<String>,
        n: u64,
    ) -> Result<Self> {
        ensure!(n > 0, "n must be at least 1");
        let dir = dir.into();
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Self::new(
            format!("PNG every {n}: {}", dir.display()),
            Mode::EveryNth {
                dir,
                prefix: prefix.into(),
                n,
            },
        )
    }

    fn new(name: String, mode: Mode) -> Result<Self> {
        let worker = Worker::spawn(
            "png-sink",
            2,
            PngTask {
                scratch: Vec::new(),
            },
        )?;
        Ok(PngSink {
            name,
            mode,
            frames_seen: 0,
            written: 0,
            done: false,
            worker,
        })
    }

    /// Frames dropped because the encoder was busy.
    pub fn dropped(&self) -> u64 {
        self.worker.dropped
    }

    /// Frames queued for writing so far.
    pub fn queued(&self) -> u64 {
        self.written
    }

    /// True once a snapshot sink has taken its frame.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Waits for all queued files to be written and reports the first error, if any.
    pub fn finish(mut self) -> Result<()> {
        self.worker.finish()
    }
}

impl FrameSink for PngSink {
    fn name(&self) -> &str {
        &self.name
    }

    fn send(&mut self, frame: &Frame<'_>) -> Result<()> {
        frame.validate()?;
        self.worker.check()?;
        let index = self.frames_seen;
        self.frames_seen += 1;
        if self.done {
            return Ok(());
        }
        let path = match &self.mode {
            Mode::Snapshot(p) => p.clone(),
            Mode::EveryNth { dir, prefix, n } => {
                if !index.is_multiple_of(*n) {
                    return Ok(());
                }
                dir.join(format!("{prefix}{index:06}.png"))
            }
        };
        let Some(mut buf) = self.worker.try_buffer() else {
            self.worker.note_dropped();
            return Ok(());
        };
        buf.clear();
        buf.extend_from_slice(frame.pixels);
        let meta = Meta {
            path,
            width: frame.width,
            height: frame.height,
            alpha: frame.has_alpha(),
        };
        if self.worker.submit(buf, meta)? {
            self.written += 1;
            if matches!(self.mode, Mode::Snapshot(_)) {
                self.done = true;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{frame, pattern};
    use model::AlphaMode;
    use std::io::BufReader;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("output-png-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn decode(path: &Path) -> (png::ColorType, u32, u32, Vec<u8>) {
        let dec = png::Decoder::new(BufReader::new(File::open(path).unwrap()));
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        buf.truncate(info.buffer_size());
        (info.color_type, info.width, info.height, buf)
    }

    #[test]
    fn snapshot_writes_first_frame_only() {
        let dir = tmpdir("snap");
        let path = dir.join("sub/shot.png");
        let mut sink = PngSink::snapshot(&path).unwrap();
        let a = pattern(16, 8, 1);
        let b = pattern(16, 8, 2);
        sink.send(&frame(16, 8, &a, AlphaMode::Straight)).unwrap();
        sink.send(&frame(16, 8, &b, AlphaMode::Straight)).unwrap();
        assert!(sink.is_done());
        sink.finish().unwrap();
        let (ct, w, h, data) = decode(&path);
        assert_eq!((ct, w, h), (png::ColorType::Rgba, 16, 8));
        let mut want = Vec::new();
        convert::convert(&a, 16, 8, PixelFormat::Rgba, &mut want).unwrap();
        assert_eq!(data, want);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn every_nth_names_and_opaque_rgb() {
        let dir = tmpdir("nth");
        let mut sink = PngSink::every_nth(&dir, 3).unwrap();
        let px = pattern(8, 4, 3);
        for _ in 0..7 {
            sink.send(&frame(8, 4, &px, AlphaMode::Opaque)).unwrap();
            // Give the worker time so nothing is dropped in this test.
            sink.worker.wait_idle();
        }
        let queued = sink.queued();
        sink.finish().unwrap();
        assert_eq!(queued, 3);
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["frame_000000.png", "frame_000003.png", "frame_000006.png"]
        );
        let (ct, _, _, data) = decode(&dir.join("frame_000003.png"));
        assert_eq!(ct, png::ColorType::Rgb);
        assert_eq!(&data[0..3], &px[0..3]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn write_png_direct() {
        let dir = tmpdir("direct");
        let px = pattern(5, 3, 9);
        let path = dir.join("x.png");
        write_png(&path, &frame(5, 3, &px, AlphaMode::Opaque)).unwrap();
        let (ct, w, h, _) = decode(&path);
        assert_eq!((ct, w, h), (png::ColorType::Rgb, 5, 3));
        assert!(write_png(dir.join("bad.png"), &frame(5, 4, &px, AlphaMode::Opaque)).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
