// Binary live-preview frames (crates/protocol/README.md, "Live preview").
//
//   offset  size  field
//        0    16  screen ID (UUID bytes, text order)
//       16     8  frame counter, u64 LE
//       24     4  width,  u32 LE
//       28     4  height, u32 LE
//       32     …  JPEG

import { PREVIEW_HEADER_LEN } from "$protocol";

export interface PreviewFrame {
  /** Screen ID as a lowercase UUID string. */
  screen: string;
  /** Frame counter (grows with every new picture; starts again from 0 when the screen's
   *  output is re-created: a size, alpha or background change). */
  frame: number;
  width: number;
  height: number;
  /** The JPEG bytes (a view into the original buffer, no copy). */
  jpeg: Uint8Array;
}

const HEX: string[] = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, "0"));

/** Formats 16 bytes as a UUID string (8-4-4-4-12). */
export function uuidFromBytes(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < 16; i++) {
    if (i === 4 || i === 6 || i === 8 || i === 10) s += "-";
    s += HEX[bytes[i] ?? 0];
  }
  return s;
}

/**
 * Parses one binary preview message. Returns `null` for anything that is not a valid frame
 * (too short, or not a JPEG), so a malformed message can never break the socket handler.
 */
export function parsePreviewFrame(data: ArrayBuffer): PreviewFrame | null {
  if (data.byteLength < PREVIEW_HEADER_LEN + 2) return null;
  const view = new DataView(data);
  const jpeg = new Uint8Array(data, PREVIEW_HEADER_LEN);
  if (jpeg[0] !== 0xff || jpeg[1] !== 0xd8) return null;
  const frame = view.getBigUint64(16, true);
  return {
    screen: uuidFromBytes(new Uint8Array(data, 0, 16)),
    frame: Number(frame),
    width: view.getUint32(24, true),
    height: view.getUint32(28, true),
    jpeg,
  };
}

/** The newest picture of one screen, as a blob URL an `<img>` can show. */
export interface PreviewImage {
  screen: string;
  frame: number;
  width: number;
  height: number;
  url: string;
}

export interface UrlFactory {
  create(blob: Blob): string;
  revoke(url: string): void;
}

const browserUrls: UrlFactory = {
  create: (blob) => URL.createObjectURL(blob),
  revoke: (url) => URL.revokeObjectURL(url),
};

/**
 * Turns frames into blob URLs, one current URL per screen. Repeated frames are dropped, and the
 * URL a new frame replaces is revoked so memory stays flat.
 *
 * One socket carries one subscription per screen, in order, so a counter lower than the last
 * one is not an old frame: the server re-created the screen's output (its size, alpha or
 * background changed) and counts from 0 again. It is taken as the newest picture.
 */
export class PreviewDecoder {
  private current = new Map<string, PreviewImage>();
  private listeners = new Set<(img: PreviewImage) => void>();

  constructor(private urls: UrlFactory = browserUrls) {}

  /** Handles one binary message. Returns the new image, or `null` if it was dropped. */
  push(data: ArrayBuffer): PreviewImage | null {
    const f = parsePreviewFrame(data);
    if (!f) return null;
    const prev = this.current.get(f.screen);
    if (prev && f.frame === prev.frame && f.width === prev.width && f.height === prev.height) return null;
    const blob = new Blob([f.jpeg as Uint8Array<ArrayBuffer>], { type: "image/jpeg" });
    const img: PreviewImage = {
      screen: f.screen,
      frame: f.frame,
      width: f.width,
      height: f.height,
      url: this.urls.create(blob),
    };
    this.current.set(f.screen, img);
    if (prev) this.urls.revoke(prev.url);
    for (const l of this.listeners) l(img);
    return img;
  }

  get(screen: string): PreviewImage | undefined {
    return this.current.get(screen);
  }

  /**
   * Forgets the frame counter of every screen (keeping the picture), so frames are accepted
   * again after a reconnect: a restarted server starts counting from zero.
   */
  resetCounters(): void {
    for (const [k, v] of this.current) this.current.set(k, { ...v, frame: -1 });
  }

  /** Drops one screen's picture (or all) and revokes the URL. */
  clear(screen?: string): void {
    for (const [k, v] of this.current) {
      if (screen === undefined || k === screen) {
        this.urls.revoke(v.url);
        this.current.delete(k);
      }
    }
  }

  onFrame(fn: (img: PreviewImage) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
}
