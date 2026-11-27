//! A small zip reader and writer: enough for OOXML packages (stored and deflated entries, the
//! central directory; no zip64, no encryption, no multi-disk). Reading is bounded (zip bombs):
//! an entry larger than its declared size, or than [`MAX_ENTRY`], is refused; so is an archive
//! with more than [`MAX_ENTRIES`] entries or entries sharing their bytes; and all the entries
//! one archive inflates add up to at most [`MAX_TOTAL`] (each entry counted once, however often
//! it is read).

use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::sync::Mutex;

/// Largest entry read, uncompressed (a picture or a video in a deck; still bounded).
pub const MAX_ENTRY: u64 = 512 << 20;
/// Largest XML part read as text.
pub const MAX_TEXT: u64 = 64 << 20;
/// Most bytes one archive inflates in all (distinct entries; a deck at the 1 GiB upload limit
/// is mostly pictures and videos, which barely compress).
pub const MAX_TOTAL: u64 = 2 << 30;
/// Most entries an archive may have (a 1 000-slide deck has about 5 000).
pub const MAX_ENTRIES: usize = 20_000;

/// One file in the archive.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Its name (`ppt/slides/slide1.xml`), `/`-separated.
    pub name: String,
    method: u16,
    compressed: u64,
    /// Uncompressed size, as the central directory says.
    pub size: u64,
    local_offset: u64,
}

/// A zip archive in memory.
pub struct Archive<'a> {
    data: &'a [u8],
    entries: Vec<Entry>,
    /// Entries inflated so far (by local offset) and their bytes in all.
    inflated: Mutex<(HashSet<u64>, u64)>,
    budget: u64,
}

fn u16_at(d: &[u8], at: usize) -> Result<u16> {
    let b = d.get(at..at + 2).context("zip: truncated")?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], at: usize) -> Result<u32> {
    let b = d.get(at..at + 4).context("zip: truncated")?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

impl<'a> Archive<'a> {
    /// Reads the central directory.
    pub fn open(data: &'a [u8]) -> Result<Self> {
        // End of central directory: the last `PK\5\6` within the last 64 KiB + 22 bytes.
        let min = data.len().saturating_sub(22 + 65_535);
        let eocd = (min..data.len().saturating_sub(21))
            .rev()
            .find(|&i| data[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
            .context("not a zip file (no end of central directory): is it a .pptx?")?;
        let count = u16_at(data, eocd + 10)? as usize;
        let cd_offset = u32_at(data, eocd + 16)? as usize;
        if cd_offset == 0xffff_ffff || count == 0xffff {
            bail!("zip64 archives are not supported");
        }
        if count > MAX_ENTRIES {
            bail!("the file has too many parts ({count})");
        }
        let mut entries = Vec::with_capacity(count);
        let mut p = cd_offset;
        for _ in 0..count {
            if u32_at(data, p)? != 0x0201_4b50 {
                bail!("zip: broken central directory");
            }
            let method = u16_at(data, p + 10)?;
            let compressed = u32_at(data, p + 20)? as u64;
            let size = u32_at(data, p + 24)? as u64;
            let name_len = u16_at(data, p + 28)? as usize;
            let extra_len = u16_at(data, p + 30)? as usize;
            let comment_len = u16_at(data, p + 32)? as usize;
            let local_offset = u32_at(data, p + 42)? as u64;
            let name = data
                .get(p + 46..p + 46 + name_len)
                .context("zip: truncated name")?;
            let name = String::from_utf8_lossy(name).replace('\\', "/");
            entries.push(Entry {
                name,
                method,
                compressed,
                size,
                local_offset,
            });
            p += 46 + name_len + extra_len + comment_len;
        }
        // Each entry's bytes are its own: entries sharing (overlapping) data would inflate the
        // same stream many times over.
        let mut ranges = Vec::with_capacity(entries.len());
        for e in &entries {
            if e.method == 0 && e.compressed != e.size {
                bail!("zip: {} is stored with two different sizes", e.name);
            }
            let lo = e.local_offset as usize;
            ranges.push((lo, Archive::data_start(data, e)? + e.compressed as usize));
        }
        ranges.sort_unstable();
        if ranges.windows(2).any(|w| w[1].0 < w[0].1) {
            bail!("zip: entries overlap");
        }
        Ok(Archive {
            data,
            entries,
            inflated: Mutex::new((HashSet::new(), 0)),
            budget: MAX_TOTAL,
        })
    }

    /// Where `e`'s compressed bytes start (after its local header).
    fn data_start(data: &[u8], e: &Entry) -> Result<usize> {
        let lo = e.local_offset as usize;
        if u32_at(data, lo)? != 0x0403_4b50 {
            bail!("zip: broken entry {}", e.name);
        }
        let name_len = u16_at(data, lo + 26)? as usize;
        let extra_len = u16_at(data, lo + 28)? as usize;
        Ok(lo + 30 + name_len + extra_len)
    }

    /// Lowers the most bytes this archive inflates in all (tests).
    pub fn with_budget(mut self, budget: u64) -> Self {
        self.budget = budget;
        self
    }

    /// Every entry, in archive order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The entry named `name` (OOXML part names compare without case).
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        let name = name.trim_start_matches('/');
        self.entries.iter().find(|e| e.name == name).or_else(|| {
            self.entries
                .iter()
                .find(|e| e.name.eq_ignore_ascii_case(name))
        })
    }

    /// True when the archive has `name`.
    pub fn has(&self, name: &str) -> bool {
        self.entry(name).is_some()
    }

    /// The uncompressed bytes of `name`.
    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let e = self
            .entry(name)
            .with_context(|| format!("the file has no part {name}"))?;
        self.read_entry(e)
    }

    /// `name` as UTF-8 text (a byte-order mark dropped).
    pub fn text(&self, name: &str) -> Result<String> {
        let e = self
            .entry(name)
            .with_context(|| format!("the file has no part {name}"))?;
        if e.size > MAX_TEXT {
            bail!("{name} is too large ({} bytes)", e.size);
        }
        let b = self.read_entry(e)?;
        let s = String::from_utf8(b).with_context(|| format!("{name} is not UTF-8"))?;
        Ok(s.trim_start_matches('\u{feff}').to_string())
    }

    /// The uncompressed bytes of one entry.
    pub fn read_entry(&self, e: &Entry) -> Result<Vec<u8>> {
        if e.size > MAX_ENTRY {
            bail!("{} is too large ({} bytes)", e.name, e.size);
        }
        {
            let mut seen = self.inflated.lock().unwrap_or_else(|e| e.into_inner());
            if !seen.0.contains(&e.local_offset) {
                if seen.1 + e.size > self.budget {
                    bail!(
                        "the file unpacks to more than {} MiB: too large to import",
                        self.budget >> 20
                    );
                }
                seen.0.insert(e.local_offset);
                seen.1 += e.size;
            }
        }
        let start = Archive::data_start(self.data, e)?;
        let raw = self
            .data
            .get(start..start + e.compressed as usize)
            .with_context(|| format!("zip: {} is truncated", e.name))?;
        match e.method {
            0 => Ok(raw.to_vec()),
            8 => {
                let mut out = Vec::with_capacity(e.size.min(64 << 20) as usize);
                // One byte more than declared tells a lying header from a true one.
                flate2::read::DeflateDecoder::new(raw)
                    .take(e.size + 1)
                    .read_to_end(&mut out)
                    .with_context(|| format!("zip: {} doesn't inflate", e.name))?;
                if out.len() as u64 != e.size {
                    bail!("zip: {} is not the size its header says", e.name);
                }
                Ok(out)
            }
            m => bail!("zip: {} uses compression method {m}, not supported", e.name),
        }
    }
}

/// Writes a zip archive (deflated entries) into memory.
#[derive(Default)]
pub struct Writer {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl Writer {
    /// An empty archive.
    pub fn new() -> Self {
        Writer::default()
    }

    /// Adds `name` with `data`, deflated.
    pub fn add(&mut self, name: &str, data: &[u8]) -> Result<()> {
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(data)?;
        let compressed = enc.finish()?;
        let crc = crc32fast::hash(data);
        let offset = u32::try_from(self.out.len()).context("zip: archive over 4 GiB")?;
        let csize = u32::try_from(compressed.len()).context("zip: entry over 4 GiB")?;
        let size = u32::try_from(data.len()).context("zip: entry over 4 GiB")?;
        let n = name.as_bytes();
        let name_len = u16::try_from(n.len()).context("zip: name too long")?;
        // Local header.
        self.out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        self.out.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        self.out.extend_from_slice(&8u16.to_le_bytes()); // deflate
        self.out.extend_from_slice(&0u32.to_le_bytes()); // time, date
        self.out.extend_from_slice(&crc.to_le_bytes());
        self.out.extend_from_slice(&csize.to_le_bytes());
        self.out.extend_from_slice(&size.to_le_bytes());
        self.out.extend_from_slice(&name_len.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(n);
        self.out.extend_from_slice(&compressed);
        // Central directory record.
        let c = &mut self.central;
        c.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        c.extend_from_slice(&20u16.to_le_bytes()); // made by
        c.extend_from_slice(&20u16.to_le_bytes()); // needed
        c.extend_from_slice(&0x0800u16.to_le_bytes());
        c.extend_from_slice(&8u16.to_le_bytes());
        c.extend_from_slice(&0u32.to_le_bytes());
        c.extend_from_slice(&crc.to_le_bytes());
        c.extend_from_slice(&csize.to_le_bytes());
        c.extend_from_slice(&size.to_le_bytes());
        c.extend_from_slice(&name_len.to_le_bytes());
        c.extend_from_slice(&[0; 8]); // extra, comment, disk, internal attributes
        c.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(n);
        self.count = self.count.checked_add(1).context("zip: too many entries")?;
        Ok(())
    }

    /// The finished archive.
    pub fn finish(mut self) -> Result<Vec<u8>> {
        let cd_offset = u32::try_from(self.out.len()).context("zip: archive over 4 GiB")?;
        let cd_len = u32::try_from(self.central.len()).context("zip: directory too large")?;
        self.out.extend_from_slice(&self.central);
        self.out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        self.out.extend_from_slice(&[0; 4]); // disk numbers
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&cd_len.to_le_bytes());
        self.out.extend_from_slice(&cd_offset.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut w = Writer::new();
        w.add("a.xml", b"<a/>").unwrap();
        let big = vec![7u8; 100_000];
        w.add("ppt/media/image1.bin", &big).unwrap();
        let z = w.finish().unwrap();
        let a = Archive::open(&z).unwrap();
        assert_eq!(a.entries().len(), 2);
        assert_eq!(a.text("a.xml").unwrap(), "<a/>");
        assert_eq!(a.read("/PPT/media/image1.bin").unwrap(), big);
        assert!(a.read("nope").is_err());
    }

    #[test]
    fn refuses_garbage() {
        assert!(Archive::open(b"hello, this is not a zip file at all").is_err());
        assert!(Archive::open(&[]).is_err());
    }

    #[test]
    fn refuses_an_entry_bigger_than_declared() {
        let mut w = Writer::new();
        w.add("x", &[1u8; 1000]).unwrap();
        let mut z = w.finish().unwrap();
        // Make the central directory claim 10 bytes.
        let a = Archive::open(&z).unwrap();
        let cd = z.len() - 22 - (46 + 1);
        drop(a);
        z[cd + 24..cd + 28].copy_from_slice(&10u32.to_le_bytes());
        let a = Archive::open(&z).unwrap();
        assert!(a.read("x").is_err());
    }

    /// Many small entries that each inflate a lot: the total is bounded, each entry counted
    /// once however often it is read.
    #[test]
    fn bounds_what_one_archive_inflates_in_all() {
        let mut w = Writer::new();
        for i in 0..5 {
            w.add(&format!("ppt/media/image{i}.png"), &vec![0u8; 1 << 20])
                .unwrap();
        }
        let z = w.finish().unwrap();
        assert!(z.len() < 50_000, "a small file: {}", z.len());
        let a = Archive::open(&z).unwrap().with_budget(3 << 20);
        for _ in 0..3 {
            a.read("ppt/media/image0.png").unwrap();
        }
        a.read("ppt/media/image1.png").unwrap();
        a.read("ppt/media/image2.png").unwrap();
        let e = a.read("ppt/media/image3.png").unwrap_err();
        assert!(e.to_string().contains("too large to import"), "{e}");
        // Re-reading what was already inflated still works.
        a.read("ppt/media/image1.png").unwrap();
    }

    /// Central-directory entries pointing at the same bytes (one stream counted many times),
    /// and a stored entry whose sizes disagree, are refused.
    #[test]
    fn refuses_shared_entries_and_lying_stored_sizes() {
        let mut w = Writer::new();
        w.add("a", &[1u8; 1000]).unwrap();
        w.add("b", &[2u8; 1000]).unwrap();
        let z = w.finish().unwrap();
        // Point b's directory record at a's local header.
        let cd = u32::from_le_bytes(z[z.len() - 6..z.len() - 2].try_into().unwrap()) as usize;
        let second = cd + 46 + 1;
        let mut shared = z.clone();
        shared[second + 42..second + 46].copy_from_slice(&0u32.to_le_bytes());
        // Same name length, so the local header still reads; the ranges overlap.
        assert!(
            Archive::open(&shared)
                .err()
                .is_some_and(|e| e.to_string().contains("overlap"))
        );
        let mut stored = z.clone();
        stored[cd + 10..cd + 12].copy_from_slice(&0u16.to_le_bytes()); // method: stored
        stored[cd + 24..cd + 28].copy_from_slice(&0u32.to_le_bytes()); // size 0
        assert!(
            Archive::open(&stored)
                .err()
                .is_some_and(|e| e.to_string().contains("two different sizes"))
        );
    }

    #[test]
    fn refuses_too_many_entries() {
        let mut w = Writer::new();
        for i in 0..=MAX_ENTRIES {
            w.add(&format!("{i}"), b"").unwrap();
        }
        let z = w.finish().unwrap();
        assert!(
            Archive::open(&z)
                .err()
                .is_some_and(|e| e.to_string().contains("too many parts"))
        );
    }
}
