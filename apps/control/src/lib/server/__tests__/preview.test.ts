import { describe, expect, it } from "vitest";
import { PreviewDecoder, parsePreviewFrame, uuidFromBytes } from "../preview";
import { previewMessage } from "./fixtures";

const SCREEN = "5c2f3e1a-7b4d-4e86-9a1f-2d6c8e0b4f71";

describe("preview frames", () => {
  it("parses the 32-byte header and keeps the JPEG", () => {
    const f = parsePreviewFrame(previewMessage(SCREEN, 2 ** 40 + 7, 480, 270));
    expect(f).not.toBeNull();
    expect(f!.screen).toBe(SCREEN);
    expect(f!.frame).toBe(2 ** 40 + 7);
    expect(f!.width).toBe(480);
    expect(f!.height).toBe(270);
    expect([...f!.jpeg]).toEqual([0xff, 0xd8, 0xff, 0xd9]);
  });

  it("rejects short messages and non-JPEG payloads", () => {
    expect(parsePreviewFrame(new ArrayBuffer(10))).toBeNull();
    expect(parsePreviewFrame(new ArrayBuffer(33))).toBeNull();
    expect(parsePreviewFrame(previewMessage(SCREEN, 1, 4, 4, [0x89, 0x50, 0x4e, 0x47]))).toBeNull();
  });

  it("formats UUID bytes in text order", () => {
    const bytes = new Uint8Array(16).map((_, i) => i);
    expect(uuidFromBytes(bytes)).toBe("00010203-0405-0607-0809-0a0b0c0d0e0f");
  });

  it("keeps one blob URL per screen, drops stale frames and revokes replaced URLs", () => {
    let n = 0;
    const revoked: string[] = [];
    const dec = new PreviewDecoder({ create: () => `blob:${++n}`, revoke: (u) => revoked.push(u) });
    const seen: number[] = [];
    dec.onFrame((img) => seen.push(img.frame));

    expect(dec.push(previewMessage(SCREEN, 5, 480, 270))?.url).toBe("blob:1");
    expect(dec.push(previewMessage(SCREEN, 5, 480, 270))).toBeNull(); // repeat
    expect(dec.push(previewMessage(SCREEN, 6, 480, 270))?.url).toBe("blob:2");
    expect(revoked).toEqual(["blob:1"]);
    expect(seen).toEqual([5, 6]);

    // A restarted server counts from zero again: after a reconnect, low counters are accepted.
    dec.resetCounters();
    expect(dec.push(previewMessage(SCREEN, 1, 480, 270))?.url).toBe("blob:3");
    dec.clear();
    expect(revoked).toEqual(["blob:1", "blob:2", "blob:3"]);
    expect(dec.get(SCREEN)).toBeUndefined();
  });

  it("a screen re-created on the server (size, alpha, background) counts from 0 again: its frames show (X1-2)", () => {
    let n = 0;
    const dec = new PreviewDecoder({ create: () => `blob:${++n}`, revoke: () => {} });
    for (let f = 1; f <= 40; f++) dec.push(previewMessage(SCREEN, f, 480, 270));
    // Reconfigured (same socket, no reconnect): the new output's first frames.
    expect(dec.push(previewMessage(SCREEN, 0, 480, 270))?.frame).toBe(0);
    expect(dec.push(previewMessage(SCREEN, 1, 480, 270))?.frame).toBe(1);
    expect(dec.push(previewMessage(SCREEN, 1, 480, 270))).toBeNull();
    // A new size with the counter where it was: a new picture too.
    expect(dec.push(previewMessage(SCREEN, 1, 480, 360))?.height).toBe(360);
  });
});
