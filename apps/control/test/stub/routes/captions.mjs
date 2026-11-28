// Live captions (docs/CAPTIONS-PLAN.md): `GET/PUT /captions/config`, `GET /captions/models`,
// `POST /captions/lines`, and the state's `captions` (running on the program audio with a
// synthetic line, so the Show page's captions strip has something to show). A saved
// configuration switches the state on or off.
//
// Stub control (not part of the API): `POST /api/v1/__stub/captions` with
//   {"downloading": {"model": "small.en", "downloaded": 233000000, "size": 466000000}}
//                        the engine downloading a model (`size` null: size unknown)
//   {"gpuFailed": true}  the GPU failed: running on the CPU, with the note saying so
//   {"onGpu": true}      running on a synthetic GPU
//   {"reset": true}      running again, as at start (on the CPU)

const MODELS = [
  { name: "tiny.en", sizeMb: 75, note: "fastest, least accurate", downloaded: false },
  { name: "base.en", sizeMb: 142, note: "a small machine", downloaded: true },
  { name: "small.en", sizeMb: 466, note: "a good CPU (8 threads or more)", downloaded: false },
  { name: "medium.en", sizeMb: 1500, note: "a GPU, or a very fast CPU", downloaded: false },
  { name: "large-v3-turbo", sizeMb: 1600, note: "a GPU (RTX 3060 or newer): the most accurate", downloaded: false },
];

const RUNNING = {
  enabled: true,
  status: "running",
  detail: null,
  progress: null,
  downloaded: null,
  downloadSize: null,
  model: "base.en",
  device: "CPU (8 threads)",
  gpu: false,
  note: null,
  load: 0.34,
  lines: [
    { line: 3, text: "Welcome to the evening service.", final: true },
    { line: 4, text: "Please turn with me to the reading", final: false },
  ],
};

export default function captionsRoutes({ state, bump, json, apiError }) {
  let config = { schemaVersion: 1, enabled: true, model: "base.en", processor: "cpu", words: "Pastor Example, Ephesians", maxLines: 2 };
  state.state.captions = structuredClone(RUNNING);
  const read = (req) =>
    new Promise((ok) => {
      let body = "";
      req.on("data", (d) => (body += d));
      req.on("end", () => ok(body ? JSON.parse(body) : {}));
    });

  return (req, res, { path }) => {
    if (path === "/captions/config" && req.method === "GET") {
      json(res, 200, config);
      return true;
    }
    if (path === "/captions/config" && req.method === "PUT") {
      void read(req).then((cfg) => {
        config = { ...cfg, schemaVersion: 1 };
        state.state.captions = config.enabled
          ? { ...structuredClone(RUNNING), model: config.source?.type === "external" ? null : (config.model ?? "auto") }
          : { enabled: false, status: "off", detail: null, progress: null, downloaded: null, downloadSize: null, model: null, device: null, gpu: null, note: null, load: null, lines: [] };
        json(res, 200, config);
        bump();
      });
      return true;
    }
    if (path === "/__stub/captions" && req.method === "POST") {
      void read(req).then((b) => {
        if (b.reset) state.state.captions = structuredClone(RUNNING);
        if (b.onGpu) state.state.captions = { ...structuredClone(RUNNING), device: "Synthetic GPU 1", gpu: true, load: 0.08 };
        if (b.gpuFailed)
          state.state.captions = {
            ...structuredClone(RUNNING),
            device: "CPU (8 threads)",
            gpu: false,
            note: "The GPU failed (out of GPU memory): captions run on the CPU until they are switched off and on again or the settings change.",
            load: 0.34,
          };
        const d = b.downloading;
        if (d) {
          const size = d.size ?? null;
          state.state.captions = {
            ...structuredClone(RUNNING),
            status: "downloading",
            detail: `downloading the ${d.model} model`,
            progress: size ? Math.min(100, Math.floor((d.downloaded * 100) / size)) : null,
            downloaded: d.downloaded,
            downloadSize: size,
            model: null,
            load: null,
            lines: [],
          };
        }
        res.writeHead(204);
        res.end();
        bump();
      });
      return true;
    }
    if (path === "/captions/models" && req.method === "GET") {
      json(res, 200, MODELS);
      return true;
    }
    if (path === "/captions/lines" && req.method === "POST") {
      if (!config.enabled) apiError(res, 409, "conflict", "captions are off");
      else {
        res.writeHead(204);
        res.end();
      }
      return true;
    }
    return false;
  };
}
