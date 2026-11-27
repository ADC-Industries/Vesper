//! Decoded frames in system memory (IDN's pictures, with Vesper's timeline facts).

use idn_ffmpeg::media as idn;
use std::sync::Arc;

/// Plane layout of a [`VideoFrame`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaneFormat {
    /// 8-bit: a Y plane (1 byte per pixel) and an interleaved CbCr plane at half width and
    /// half height (2 bytes per chroma sample pair).
    Nv12,
    /// 16-bit little-endian words, MSB-aligned (10 significant bits for a 10-bit source, 12
    /// for a 12-bit one: [`VideoFrame::bit_depth`]): a Y plane (2 bytes per pixel) and an
    /// interleaved CbCr plane at half resolution (4 bytes per pair).
    P010,
}

impl PlaneFormat {
    /// Significant bits per sample the renderer reads (P010 is read at 10-bit scale; deeper
    /// sources keep their extra bits as fractions).
    pub fn bits(self) -> u32 {
        match self {
            PlaneFormat::Nv12 => 8,
            PlaneFormat::P010 => 10,
        }
    }

    pub(crate) fn of(f: idn::PixelFormat) -> Option<PlaneFormat> {
        match f {
            idn::PixelFormat::Nv12 => Some(PlaneFormat::Nv12),
            idn::PixelFormat::P010 => Some(PlaneFormat::P010),
            _ => None,
        }
    }
}

/// YCbCr → RGB matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Matrix {
    /// ITU-R BT.601 (SD).
    Bt601,
    /// ITU-R BT.709 (HD).
    Bt709,
    /// ITU-R BT.2020 non-constant luminance (UHD).
    Bt2020,
}

impl Matrix {
    /// The matrix a stream declares (FFmpeg's colour-space name); unspecified / unknown =
    /// [`Matrix::default_for`] its size.
    pub(crate) fn of(space: Option<&str>, width: u32, height: u32) -> Matrix {
        match space {
            Some("bt709") => Matrix::Bt709,
            Some("bt470bg" | "smpte170m" | "fcc") => Matrix::Bt601,
            Some("bt2020nc" | "bt2020c" | "bt2020_ncl" | "bt2020_cl") => Matrix::Bt2020,
            _ => Matrix::default_for(width, height),
        }
    }

    /// The matrix of a stream that does not say: BT.709 for HD (≥ 720 lines or ≥ 1280
    /// wide), BT.601 for SD.
    pub fn default_for(width: u32, height: u32) -> Matrix {
        if height >= 720 || width >= 1280 {
            Matrix::Bt709
        } else {
            Matrix::Bt601
        }
    }

    /// Luma coefficients (Kr, Kb).
    pub fn kr_kb(self) -> (f64, f64) {
        match self {
            Matrix::Bt601 => (0.299, 0.114),
            Matrix::Bt709 => (0.2126, 0.0722),
            Matrix::Bt2020 => (0.2627, 0.0593),
        }
    }

    /// Short name (`bt709`).
    pub fn name(self) -> &'static str {
        match self {
            Matrix::Bt601 => "bt601",
            Matrix::Bt709 => "bt709",
            Matrix::Bt2020 => "bt2020",
        }
    }
}

/// One decoded frame (see the crate docs for `t` vs `pts`). The pixels are IDN's picture,
/// shared, never copied.
pub struct VideoFrame {
    /// Timeline time, seconds (continuous across loops).
    pub t: f64,
    /// Position in the file, seconds from its start.
    pub pts: f64,
    /// Nominal duration, seconds.
    pub duration: f64,
    /// Unique per player, increasing in the order pictures are handed out (a new number =
    /// new pixels).
    pub seq: u64,
    /// Seeks the player had taken when the frame was handed out.
    pub generation: u64,
    /// Which pass of a looping playback (0 = first).
    pub loop_index: u32,
    /// Picture width, px (as stored: before [`VideoFrame::rotation`]).
    pub width: u32,
    /// Picture height, px (as stored).
    pub height: u32,
    /// Plane layout.
    pub format: PlaneFormat,
    /// The source's significant bits per sample (8, 10, 12).
    pub bit_depth: u8,
    /// Colour matrix.
    pub matrix: Matrix,
    /// Full range (0–255) instead of limited (16–235 / 16–240).
    pub full_range: bool,
    /// Transfer the file states (`smpte2084` PQ, `arib-std-b67` HLG, `bt709`, …).
    pub transfer: Option<String>,
    /// Primaries the file states (`bt709`, `bt2020`, …).
    pub primaries: Option<String>,
    /// Counter-clockwise turn (0, 90, 180, 270) that shows the picture upright (a phone's
    /// portrait video); not applied to the planes.
    pub rotation: u16,
    /// GPU decoding was expected for the stream (see [`crate::VideoInfo::hw`]).
    pub hw: bool,
    pub(crate) pic: Arc<idn::VideoFrame>,
}

impl VideoFrame {
    /// Plane `i` (0 = Y, 1 = CbCr) and its row stride in bytes.
    pub fn plane(&self, i: usize) -> (&[u8], usize) {
        let p = &self.pic.planes[i];
        (&p.data, p.stride)
    }

    /// Width of the chroma plane in samples (pairs).
    pub fn chroma_width(&self) -> u32 {
        self.width.div_ceil(2)
    }

    /// Height of the chroma plane in rows.
    pub fn chroma_height(&self) -> u32 {
        self.height.div_ceil(2)
    }

    /// The luma code value at (x, y) (8-bit scale for NV12, 10-bit for P010); for tests and
    /// tools.
    pub fn luma(&self, x: u32, y: u32) -> u32 {
        let (p, stride) = self.plane(0);
        match self.format {
            PlaneFormat::Nv12 => p[y as usize * stride + x as usize] as u32,
            PlaneFormat::P010 => {
                let i = y as usize * stride + x as usize * 2;
                (u16::from_le_bytes([p[i], p[i + 1]]) >> 6) as u32
            }
        }
    }
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoFrame")
            .field("t", &self.t)
            .field("pts", &self.pts)
            .field("seq", &self.seq)
            .field("loop", &self.loop_index)
            .field("size", &(self.width, self.height))
            .field("format", &self.format)
            .field("bit_depth", &self.bit_depth)
            .field("matrix", &self.matrix)
            .field("full_range", &self.full_range)
            .field("rotation", &self.rotation)
            .field("hw", &self.hw)
            .finish()
    }
}
