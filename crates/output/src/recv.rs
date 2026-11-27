//! What a verification receiver ([`crate::IdnReceiver`]) hands back: video frames and audio
//! blocks copied out of the library's buffers, in plain types tests and tools can measure.

/// The source part of a network source name (`"SOURCE"` in `"MACHINE (SOURCE)"`); the whole
/// name when it has no such suffix.
pub fn source_part(full_name: &str) -> &str {
    match (full_name.find(" ("), full_name.ends_with(')')) {
        (Some(i), true) => &full_name[i + 2..full_name.len() - 1],
        _ => full_name,
    }
}

/// FourCC of pictures with an alpha plane: straight RGBA.
pub const FOURCC_RGBA: u32 = u32::from_le_bytes(*b"RGBA");
/// FourCC of pictures without an alpha plane: RGB plus an unused byte (alpha reads as 255).
pub const FOURCC_RGBX: u32 = u32::from_le_bytes(*b"RGBX");

/// A received video frame (tightly packed).
#[derive(Clone, Debug)]
pub struct ReceivedFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// FourCC as delivered: [`FOURCC_RGBA`] or [`FOURCC_RGBX`].
    pub fourcc: u32,
    /// Frame rate numerator / denominator.
    pub frame_rate: (i32, i32),
    /// Timecode, 100 ns units.
    pub timecode: i64,
    /// `width * height * 4` bytes, R G B A/X order.
    pub data: Vec<u8>,
}

impl ReceivedFrame {
    /// The pixel at (x, y) as `[r, g, b, a]` (alpha 255 for `RGBX`).
    pub fn pixel_rgba(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        let p = &self.data[i..i + 4];
        let a = if self.fourcc == FOURCC_RGBX {
            255
        } else {
            p[3]
        };
        [p[0], p[1], p[2], a]
    }

    /// The FourCC as text (`"RGBA"`).
    pub fn fourcc_str(&self) -> String {
        String::from_utf8_lossy(&self.fourcc.to_le_bytes()).into_owned()
    }
}

/// A received audio block (planar float).
#[derive(Clone, Debug)]
pub struct ReceivedAudio {
    /// Sample rate, Hz.
    pub sample_rate: u32,
    /// Channels.
    pub channels: usize,
    /// Samples per channel.
    pub samples: usize,
    /// Timecode of the first sample, 100 ns units.
    pub timecode: i64,
    /// Planar: channel `c`'s samples are `data[c * samples .. (c + 1) * samples]`.
    pub data: Vec<f32>,
}

impl ReceivedAudio {
    /// One channel's samples.
    pub fn channel(&self, c: usize) -> &[f32] {
        &self.data[c * self.samples..(c + 1) * self.samples]
    }
}

/// What a receiver's `capture` returns.
#[derive(Clone, Debug)]
pub enum Received {
    /// A video frame.
    Video(ReceivedFrame),
    /// An audio block.
    Audio(ReceivedAudio),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_part_of_full_names() {
        assert_eq!(source_part("BOOTH (Key Feed)"), "Key Feed");
        assert_eq!(source_part("BOOTH (A (B))"), "A (B)");
        assert_eq!(source_part("plain"), "plain");
    }

    #[test]
    fn pixels_read_in_rgba_order() {
        let mut f = ReceivedFrame {
            width: 1,
            height: 1,
            fourcc: FOURCC_RGBA,
            frame_rate: (30, 1),
            timecode: 0,
            data: vec![1, 2, 3, 4],
        };
        assert_eq!(f.pixel_rgba(0, 0), [1, 2, 3, 4]);
        f.fourcc = FOURCC_RGBX;
        assert_eq!(f.pixel_rgba(0, 0), [1, 2, 3, 255]);
        assert_eq!(f.fourcc_str(), "RGBX");
    }
}
