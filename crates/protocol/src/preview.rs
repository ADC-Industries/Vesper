//! Live preview frames: binary WebSocket messages sent after a `subscribePreview` command.
//!
//! ```text
//! offset  size  field
//!      0    16  screen ID: the UUID's 16 bytes in RFC 4122 order (as in its text form)
//!     16     8  frame counter, u64 little-endian (the screen's render sequence: it grows
//!               with every new picture; equal numbers = same picture)
//!     24     4  width, u32 little-endian (px)
//!     28     4  height, u32 little-endian (px)
//!     32     …  JPEG (starts with FF D8)
//! ```

use model::ScreenId;

/// Size of the header in front of the JPEG.
pub const PREVIEW_HEADER_LEN: usize = 32;

/// The header of a binary preview frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewFrameHeader {
    /// The screen.
    pub screen: ScreenId,
    /// Frame counter (render sequence of the screen).
    pub frame: u64,
    /// JPEG width, px.
    pub width: u32,
    /// JPEG height, px.
    pub height: u32,
}

impl PreviewFrameHeader {
    /// The header bytes.
    pub fn to_bytes(&self) -> [u8; PREVIEW_HEADER_LEN] {
        let mut b = [0u8; PREVIEW_HEADER_LEN];
        b[..16].copy_from_slice(self.screen.0.as_bytes());
        b[16..24].copy_from_slice(&self.frame.to_le_bytes());
        b[24..28].copy_from_slice(&self.width.to_le_bytes());
        b[28..32].copy_from_slice(&self.height.to_le_bytes());
        b
    }

    /// A whole binary message: header + JPEG.
    pub fn encode(&self, jpeg: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(PREVIEW_HEADER_LEN + jpeg.len());
        out.extend_from_slice(&self.to_bytes());
        out.extend_from_slice(jpeg);
        out
    }

    /// Splits a binary message into its header and JPEG (`None` when too short).
    pub fn decode(message: &[u8]) -> Option<(PreviewFrameHeader, &[u8])> {
        if message.len() < PREVIEW_HEADER_LEN {
            return None;
        }
        let u32_at = |i: usize| u32::from_le_bytes(message[i..i + 4].try_into().unwrap_or([0; 4]));
        let header = PreviewFrameHeader {
            screen: ScreenId(uuid_from(&message[..16])),
            frame: u64::from_le_bytes(message[16..24].try_into().unwrap_or([0; 8])),
            width: u32_at(24),
            height: u32_at(28),
        };
        Some((header, &message[PREVIEW_HEADER_LEN..]))
    }
}

fn uuid_from(b: &[u8]) -> uuid::Uuid {
    let mut a = [0u8; 16];
    a.copy_from_slice(&b[..16]);
    uuid::Uuid::from_bytes(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_round_trips() {
        let h = PreviewFrameHeader {
            screen: ScreenId::new(),
            frame: 0x0102_0304_0506_0708,
            width: 480,
            height: 270,
        };
        let msg = h.encode(&[0xFF, 0xD8, 0xFF]);
        assert_eq!(msg.len(), PREVIEW_HEADER_LEN + 3);
        assert_eq!(&msg[16..18], &[0x08, 0x07], "little-endian frame counter");
        assert_eq!(msg[..16], *h.screen.0.as_bytes());
        let (back, jpeg) = PreviewFrameHeader::decode(&msg).unwrap();
        assert_eq!(back, h);
        assert_eq!(jpeg, &[0xFF, 0xD8, 0xFF]);
        assert!(PreviewFrameHeader::decode(&msg[..10]).is_none());
    }
}
