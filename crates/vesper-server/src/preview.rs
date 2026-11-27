//! Live screen previews over the WebSocket.
//!
//! A client sends `subscribePreview {screen, fps, width}`; the server then pushes binary frames
//! ([`protocol::PreviewFrameHeader`] + JPEG) of that screen at most `fps` times per second
//! (capped at [`protocol::MAX_PREVIEW_FPS`]) and only when the picture changed (plus once right
//! away). Nothing is encoded while nobody is subscribed. Encoding runs on the blocking pool,
//! never on the render thread, and one encode per (screen, width, frame) is shared by every
//! subscriber ([`PreviewHub`]).
//!
//! A frame is known by its identity (the `Arc` the screen published), not by its sequence
//! number alone: a screen whose render target is re-created (size, alpha, background) counts
//! from 0 again, and its new frames must not be taken for cached old ones.

use crate::{Shared, lock};
use axum::body::Bytes;
use image::ImageEncoder;
use model::ScreenId;
use protocol::PreviewFrameHeader;
use render::RenderedFrame;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::mpsc;

/// JPEG quality of previews.
pub const QUALITY: u8 = 75;

/// Encodes a frame as JPEG, scaled to `width` (keeping the aspect ratio) when given. Frames
/// are premultiplied, so their colour channels are already the picture over black (which is
/// how JPEG, having no alpha, shows screens with transparency).
pub fn encode_jpeg(
    frame: &RenderedFrame,
    width: Option<u32>,
    quality: u8,
) -> anyhow::Result<Vec<u8>> {
    let mut rgb = Vec::with_capacity((frame.width * frame.height * 3) as usize);
    for p in frame.data.as_chunks::<4>().0 {
        rgb.extend_from_slice(&p[..3]);
    }
    let mut img = image::RgbImage::from_raw(frame.width, frame.height, rgb)
        .ok_or_else(|| anyhow::anyhow!("frame size mismatch"))?;
    if let Some(w) = width.filter(|w| *w < frame.width) {
        let h = ((frame.height as u64 * w as u64) / frame.width.max(1) as u64).max(1) as u32;
        img = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    }
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).write_image(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(out)
}

type Key = (ScreenId, u32);

/// Shares encoded preview messages between subscribers.
#[derive(Default)]
pub struct PreviewHub {
    /// Latest encoded message per (screen, width): (the frame, header + JPEG).
    latest: Mutex<HashMap<Key, (Weak<RenderedFrame>, Bytes)>>,
}

/// `seen` is `frame` itself (a re-created target's frame with the same sequence is not).
fn same_frame(seen: &Weak<RenderedFrame>, frame: &Arc<RenderedFrame>) -> bool {
    std::ptr::eq(seen.as_ptr(), Arc::as_ptr(frame))
}

impl PreviewHub {
    /// The binary message for `frame` of `screen` at `width`, encoding it if needed.
    pub async fn message(
        &self,
        screen: ScreenId,
        width: u32,
        frame: Arc<RenderedFrame>,
    ) -> anyhow::Result<Bytes> {
        let key = (screen, width);
        if let Some((seen, m)) = lock(&self.latest).get(&key)
            && same_frame(seen, &frame)
        {
            return Ok(m.clone());
        }
        let seen = Arc::downgrade(&frame);
        let msg = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
            let jpeg = encode_jpeg(&frame, Some(width), QUALITY)?;
            let (w, h) = jpeg_size(&jpeg).unwrap_or((width, 0));
            let header = PreviewFrameHeader {
                screen,
                frame: frame.sequence,
                width: w,
                height: h,
            };
            Ok(header.encode(&jpeg))
        })
        .await??;
        let msg = Bytes::from(msg);
        lock(&self.latest).insert(key, (seen, msg.clone()));
        Ok(msg)
    }
}

/// Width and height from a baseline JPEG's SOF0 marker.
fn jpeg_size(j: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < j.len() {
        if j[i] != 0xFF {
            return None;
        }
        let marker = j[i + 1];
        let len = u16::from_be_bytes([j[i + 2], j[i + 3]]) as usize;
        if matches!(marker, 0xC0..=0xC2) {
            let h = u16::from_be_bytes([j[i + 5], j[i + 6]]) as u32;
            let w = u16::from_be_bytes([j[i + 7], j[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

/// Runs one subscription until the receiver of `out` goes away: polls the screen's latest
/// frame at `fps` and sends a message whenever the frame changed. `out` should be a small
/// channel: when the client is slow, frames are dropped rather than queued.
pub async fn run_subscription(
    shared: Arc<Shared>,
    hub: Arc<PreviewHub>,
    screen: ScreenId,
    fps: f64,
    width: u32,
    out: mpsc::Sender<Bytes>,
) {
    let fps = fps.clamp(0.2, protocol::MAX_PREVIEW_FPS);
    let mut tick = tokio::time::interval(Duration::from_secs_f64(1.0 / fps));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last: Weak<RenderedFrame> = Weak::new();
    loop {
        tick.tick().await;
        if out.is_closed() {
            break;
        }
        let Some(rt) = shared.screen(screen) else {
            break;
        };
        let Some(frame) = lock(&rt.latest).clone() else {
            continue;
        };
        if same_frame(&last, &frame) {
            continue;
        }
        match hub.message(screen, width, frame.clone()).await {
            // A full channel (slow client) drops this frame; it is retried on the next tick,
            // so the client always ends up with the newest picture.
            Ok(m) => match out.try_send(m) {
                Ok(()) => last = Arc::downgrade(&frame),
                Err(mpsc::error::TrySendError::Full(_)) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => break,
            },
            Err(e) => {
                tracing::warn!(%screen, "preview encode failed: {e:#}");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(value: u8, sequence: u64) -> Arc<RenderedFrame> {
        Arc::new(RenderedFrame {
            width: 64,
            height: 36,
            data: vec![value; 64 * 36 * 4],
            alpha: model::AlphaMode::Opaque,
            sequence,
        })
    }

    #[tokio::test]
    async fn a_recreated_target_restarting_its_count_is_not_served_from_the_cache() {
        // X1-2 (server side): a screen's render target re-created (size, alpha, background)
        // counts its frames from 0 again; the cache used to match on the number alone.
        let hub = PreviewHub::default();
        let screen = ScreenId::new();
        let black = frame(0, 3);
        let a = hub.message(screen, 32, black.clone()).await.unwrap();
        // The same frame again: shared.
        assert_eq!(hub.message(screen, 32, black).await.unwrap(), a);
        // The new target's frame 3: a new picture.
        let b = hub.message(screen, 32, frame(255, 3)).await.unwrap();
        assert_ne!(a, b);
    }
}
