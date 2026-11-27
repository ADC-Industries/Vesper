// Live network sources (G7): `GET /sources` (synthetic cameras on documentation-range
// addresses) and `GET /sources/thumbnail` (a generated "camera" picture per source, 204 for a
// source that is lost). The source on the video-input layer (`triggerLiveSource`, applied by
// server.mjs) is reported `live` with its size; a change of it is pushed as `sourcesChanged`.
// OMT sources are named `omt:HOST (name)` (docs/OMT-PLAN.md).
// Logical video inputs (stage 4): `/video-inputs` CRUD in memory (made-up names), kept in
// `state.videoInputs` for server.mjs's `triggerVideoInput`; a change pushes `documentChanged`.

const SOURCES = [
  { name: "BOOTH-PC (Cam 1 Wide)", address: "192.0.2.21:5961", idn: true, listed: true, size: [1920, 1080], hue: 205 },
  { name: "BOOTH-PC (Cam 2 Pulpit)", address: "192.0.2.21:5962", idn: true, listed: true, size: [1920, 1080], hue: 28 },
  { name: "LOBBY-PC (Lobby cam)", address: "192.0.2.34:5961", idn: false, listed: true, size: [1280, 720], hue: 140, lost: true },
  { name: "STREAM-PC (Program)", address: "192.0.2.40:5961", idn: true, listed: true, size: [1920, 1080], hue: 280 },
  // An OMT (Open Media Transport) source: named `omt:` + its OMT name, `omt: true`.
  { name: "omt:GFX-PC (Lower thirds)", omt: true, idn: false, listed: true, size: [1920, 1080], hue: 95 },
];

function cameraSvg(src, w) {
  const h = Math.round((w * 9) / 16);
  const label = src.name.slice(src.name.indexOf("(") + 1, -1);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 320 180">
<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="hsl(${src.hue},35%,38%)"/><stop offset="1" stop-color="hsl(${src.hue},30%,14%)"/></linearGradient></defs>
<rect width="320" height="180" fill="url(#g)"/>
<rect x="0" y="128" width="320" height="52" fill="hsl(${src.hue},18%,10%)"/>
<rect x="120" y="70" width="80" height="62" rx="6" fill="hsl(${src.hue},12%,22%)"/>
<circle cx="160" cy="56" r="20" fill="hsl(${src.hue},15%,30%)"/>
<text x="10" y="170" font-family="sans-serif" font-size="12" fill="#fff" opacity="0.75">${label}</text>
</svg>`;
}

const INPUTS = [
  { id: "1a9e0000-0000-4000-8000-0000000000c1", name: "Wide shot", sortKey: "a", source: { type: "idn", sourceName: "BOOTH-PC (Cam 1 Wide)" } },
  { id: "1a9e0000-0000-4000-8000-0000000000c2", name: "Pulpit camera", sortKey: "b", source: { type: "idn", sourceName: "BOOTH-PC (Cam 2 Pulpit)" } },
  { id: "1a9e0000-0000-4000-8000-0000000000c3", name: "Baptistry", sortKey: "c" },
];

export default function sourcesRoutes({ json, svg, state, push, apiError }) {
  let rev = 1;
  state.videoInputs = INPUTS.map((v) => ({ schemaVersion: 2, revision: `stub-in-${rev++}`, ...structuredClone(v) }));
  const changed = (id, change) => push({ type: "documentChanged", kind: "videoInput", id, change });
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
  const inputRoutes = (req, res, path) => {
    const list = state.videoInputs;
    if (path === "/video-inputs" && req.method === "GET") {
      json(res, 200, list);
      return true;
    }
    if (path === "/video-inputs" && req.method === "POST") {
      readBody(req, (b) => {
        let n = 1;
        while (list.some((v) => v.name === `Camera ${n}`)) n++;
        const v = { schemaVersion: 2, revision: `stub-in-${rev++}`, id: crypto.randomUUID(), sortKey: `z${rev}`, ...b, name: (b.name ?? "").trim() || `Camera ${n}` };
        if (!v.source) delete v.source;
        list.push(v);
        changed(v.id, "added");
        json(res, 201, v);
      });
      return true;
    }
    const m = /^\/video-inputs\/([^/]+)$/.exec(path);
    if (!m) return false;
    const i = list.findIndex((v) => v.id === m[1]);
    if (i < 0) {
      apiError(res, 404, "notFound", `video input ${m[1]} not found`);
      return true;
    }
    if (req.method === "GET") json(res, 200, list[i]);
    else if (req.method === "DELETE") {
      list.splice(i, 1);
      if (state.state.layers?.videoInput?.input === m[1]) state.state.layers.videoInput = null;
      changed(m[1], "removed");
      res.writeHead(204, { "Access-Control-Allow-Origin": "*" });
      res.end();
    } else if (req.method === "PUT") {
      readBody(req, (b) => {
        const v = { ...b, schemaVersion: 2, id: m[1], revision: `stub-in-${rev++}` };
        if (!v.source) delete v.source;
        list[i] = v;
        changed(v.id, "changed");
        json(res, 200, v);
      });
    } else return false;
    return true;
  };
  const list = () => {
    const live = state.state.layers?.videoInput?.source ?? null;
    return SOURCES.map((s) => {
      const out = { name: s.name, ...(s.address ? { address: s.address } : {}), idn: s.idn, ...(s.omt ? { omt: true } : {}), listed: s.listed, status: "idle" };
      if (s.name === live) Object.assign(out, s.lost ? { status: "lost", message: "connection closed" } : { status: "live", width: s.size[0], height: s.size[1], frameRate: 29.97 });
      return out;
    });
  };
  let last = null;
  const t = setInterval(() => {
    const live = state.state.layers?.videoInput?.source ?? null;
    if (live !== last) {
      last = live;
      push({ type: "sourcesChanged", sources: list() });
    }
  }, 200);
  t.unref?.();

  return (req, res, { path, query }) => {
    if (path.startsWith("/video-inputs")) return inputRoutes(req, res, path);
    if (req.method !== "GET") return false;
    if (path === "/sources") {
      json(res, 200, { sources: list() });
      return true;
    }
    if (path === "/sources/thumbnail") {
      const src = SOURCES.find((s) => s.name === query.get("source"));
      if (!src || src.lost) {
        res.writeHead(204, { "Access-Control-Allow-Origin": "*" });
        res.end();
        return true;
      }
      svg(res, cameraSvg(src, Math.min(640, Math.max(16, Number(query.get("w")) || 320))));
      return true;
    }
    return false;
  };
}
