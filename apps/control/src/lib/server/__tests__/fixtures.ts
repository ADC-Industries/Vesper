import type { ShowState } from "$protocol";

export function showState(over: Partial<ShowState> = {}): ShowState {
  return {
    layers: { videoInput: null, media: null, foreground: null, slide: null, announcements: null, props: [], messages: [], audio: null },
    restorableLayers: [],
    activeLook: null,
    transitions: [],
    transport: null,
    timers: [],
    stageLayouts: [],
    stageScreens: [],
    stageMessage: null,
    screens: [],
    screensEnabled: { audience: true, stage: true },
    identify: false,
    options: { continuousPlaylist: true },
    cursor: null,
    diagnostics: { fontSubstitutions: 0, fontMisses: 0, missingMedia: 0, unsupported: 0, notes: 0, loadIssues: 0, skippedActions: 0 },
    audio: {
      device: "",
      outputState: "null",
      detail: null,
      masterDb: 0,
      muted: false,
      levels: [-60, -60],
      xruns: 0,
      layers: { audio: { db: 0, muted: false }, media: { db: 0, muted: false }, slide: { db: 0, muted: false }, videoInput: { db: 0, muted: false } },
      shuffle: false,
      repeat: false,
      playlist: null,
    },
    clockFormat: { use24Hour: false, seconds: false },
    displays: [],
    build: null,
    captions: { enabled: false, status: "off", detail: null, progress: null, downloaded: null, downloadSize: null, model: null, device: null, load: null, lines: [] },
    ...over,
  };
}

/** Builds a binary preview message as the server sends it. */
export function previewMessage(screen: string, frame: number, width: number, height: number, jpeg = [0xff, 0xd8, 0xff, 0xd9]): ArrayBuffer {
  const buf = new ArrayBuffer(32 + jpeg.length);
  const v = new DataView(buf);
  const hex = screen.replace(/-/g, "");
  for (let i = 0; i < 16; i++) v.setUint8(i, parseInt(hex.slice(i * 2, i * 2 + 2), 16));
  v.setBigUint64(16, BigInt(frame), true);
  v.setUint32(24, width, true);
  v.setUint32(28, height, true);
  new Uint8Array(buf, 32).set(jpeg);
  return buf;
}
