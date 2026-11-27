// Recording and streaming (docs/RECORD-STREAM-PLAN.md): `GET /streaming` (the box's settings,
// two made-up targets: an RTMPS platform with a saved key and an SRT overflow room on a
// documentation-range address; keys never sent), `GET /streaming/status`, and the writes and
// actions in memory. Idle by default; `streamingStub.live()` starts a recording of the first
// screen (42 min in) and the RTMPS stream, as the shots want; `streamingStub.reset()` stops all.

const T0 = "5ee70000-0000-4000-8000-00000000a";
const TARGETS = [
  { id: `${T0}001`, name: "Main stream", kind: "rtmp", url: "rtmps://live.example.com/app", hasKey: true, screen: null, codec: "h264", videoMbps: 6, audioKbps: 160 },
  { id: `${T0}002`, name: "Overflow room", kind: "srt", url: "srt://192.0.2.60:9000?passphrase=***", hasKey: false, screen: null, codec: "h264", videoMbps: 8, audioKbps: 192 },
];
const RECORDING = { screen: null, container: "mp4", codec: "h264", videoMbps: 12, audioKbps: 192, splitMinutes: 0, folder: null, minFreeGb: 2 };

const state = { recording: null, streams: new Map(), targets: [], settings: null };

export const streamingStub = {
  reset() {
    state.recording = null;
    state.streams = new Map();
    state.targets = structuredClone(TARGETS);
    state.settings = structuredClone(RECORDING);
  },
  /** A recording 42 minutes in and the first target live. */
  live() {
    const now = Date.now();
    state.recording = { started: now - 2537_000, part: 1, file: "Main Projector 2026-11-29 10-28-03.mp4" };
    state.streams.set(TARGETS[0].id, { started: now - 1985_000 });
  },
};
streamingStub.reset();

export default function streamingRoutes({ json, apiError, fx }) {
  const screens = () => fx.get("screens") ?? [];
  const firstScreen = () => screens()[0] ?? { id: "", name: "Screen" };
  const readBody = (req, fn) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      let v = {};
      try {
        v = body ? JSON.parse(body) : {};
      } catch {
        /* empty */
      }
      fn(v);
    });
  };
  const status = () => {
    const now = Date.now();
    const sc = firstScreen();
    const r = state.recording;
    return {
      recording: r
        ? {
            screen: sc.id,
            screenName: sc.name,
            file: r.file,
            state: "live",
            startedAt: r.started,
            seconds: (now - r.started) / 1000,
            bytes: Math.round(((now - r.started) / 1000) * 1_540_000),
            part: r.part,
            videoKbps: 12000,
            skippedPictures: 0,
            dropped: 0,
            freeBytes: 412_000_000_000,
          }
        : null,
      streams: [...state.streams].map(([id, s]) => {
        const t = state.targets.find((x) => x.id === id) ?? { name: "?" };
        const secs = (now - s.started) / 1000;
        return {
          id,
          name: t.name,
          screen: sc.id,
          state: "live",
          startedAt: s.started,
          seconds: secs,
          bytes: Math.round(secs * 770_000),
          sentKbps: 6140,
          videoKbps: 6000,
          reconnects: 0,
          dropped: 0,
          skippedPictures: 0,
          lagMs: 40,
        };
      }),
    };
  };
  const settings = () => ({
    recording: state.settings,
    targets: state.targets,
    recordingsFolder: state.settings.folder ?? "/var/lib/vesper/recordings",
    encoders: [
      { codec: "h264", label: "VA-API" },
      { codec: "hevc", label: "VA-API" },
    ],
  });
  return (req, res, { path }) => {
    const m = req.method;
    if (path === "/streaming" && m === "GET") return json(res, 200, settings()), true;
    if (path === "/streaming/status" && m === "GET") return json(res, 200, status()), true;
    if (path === "/streaming/recording" && m === "PUT") {
      readBody(req, (b) => {
        state.settings = { ...RECORDING, ...b };
        json(res, 200, settings());
      });
      return true;
    }
    if (path === "/streaming/targets" && m === "POST") {
      readBody(req, (b) => {
        const t = { ...TARGETS[0], ...b, id: `${T0}${String(100 + state.targets.length)}`, hasKey: !!b.key };
        delete t.key;
        state.targets.push(t);
        json(res, 200, t);
      });
      return true;
    }
    const tm = path.match(/^\/streaming\/targets\/([^/]+)(\/start|\/stop)?$/);
    if (tm) {
      const i = state.targets.findIndex((t) => t.id === tm[1]);
      if (i < 0) return apiError(res, 404, "notFound", "no such target"), true;
      if (tm[2] === "/start" && m === "POST") {
        if (state.streams.has(tm[1])) return apiError(res, 409, "conflict", "that target is already streaming"), true;
        state.streams.set(tm[1], { started: Date.now() });
        return json(res, 200, status()), true;
      }
      if (tm[2] === "/stop" && m === "POST") {
        state.streams.delete(tm[1]);
        return json(res, 200, status()), true;
      }
      if (!tm[2] && m === "PUT") {
        readBody(req, (b) => {
          const old = state.targets[i];
          // A saved key is kept only for the same server (scheme, host, port); for another one
          // it is typed again (or cleared with "").
          const server = (u) => (u ?? "").toLowerCase().replace(/^([a-z]*:\/\/)?([^/?#]*).*$/, (_, s, a) => `${s ?? ""}${a.slice(a.lastIndexOf("@") + 1)}`);
          const same = (b.kind ?? old.kind) === old.kind && server(b.url ?? old.url) === server(old.url);
          if (b.key === undefined && !same && old.hasKey && (b.kind ?? old.kind) === "rtmp") {
            return apiError(res, 400, "badRequest", "enter the stream key again for the new server (or remove it): it is kept only for the same server");
          }
          const t = { ...old, ...b, id: old.id, hasKey: b.key === undefined ? same && old.hasKey : !!b.key };
          delete t.key;
          state.targets[i] = t;
          json(res, 200, t);
        });
        return true;
      }
      if (!tm[2] && m === "DELETE") {
        state.targets.splice(i, 1);
        state.streams.delete(tm[1]);
        res.writeHead(204).end();
        return true;
      }
    }
    if (path === "/recording/start" && m === "POST") {
      if (state.recording) return apiError(res, 409, "conflict", "already recording"), true;
      state.recording = { started: Date.now(), part: 1, file: `${firstScreen().name} 2026-11-29 10-28-03.mp4` };
      return json(res, 200, status()), true;
    }
    if (path === "/recording/stop" && m === "POST") {
      if (!state.recording) return apiError(res, 409, "conflict", "not recording"), true;
      state.recording = null;
      return json(res, 200, status()), true;
    }
    if (path === "/recording/split" && m === "POST") {
      if (!state.recording) return apiError(res, 409, "conflict", "not recording"), true;
      state.recording.part += 1;
      return json(res, 200, status()), true;
    }
    return false;
  };
}
