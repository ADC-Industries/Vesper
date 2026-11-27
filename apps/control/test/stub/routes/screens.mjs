// The Screens & Looks page's stub routes: while `screensStub.on`, the box has three looks (one
// live), a test pattern on its first screen, and three attached displays (`GET /displays`,
// `state.displays`): one connected and driven by the stage screen, one unplugged, one free.
// The looks and masks can be created, edited and deleted (in memory; `start` puts them back).
// Off by default, so nothing else changes; `scripts/shots/screens.mjs` turns it on (and sends
// `documentChanged` for the looks). Screens can be saved (PUT /screens/{id}, checked like the
// server's stage mask rules). Names are made up. With `screensStub.source = "window"`
// (set before `start`) the displays are instead the monitors of a one-PC setup, shown as windows:
// the main monitor (Oriel's) and a projector to its right, driven by the stage screen.
// With `screensStub.wall` the audience screen is an Edge Blend wall of three projectors (its
// IDN output on the left tile, a placeholder on the middle one, the right one empty), and
// `POST /displays/{id}/identify` answers 204.

// Every stub started in this process (a shot may start a second one, e.g. the Library's Codex
// box): start / stop act on all of them.
const instances = new Set();
export const screensStub = {
  on: false,
  /** "kms" (a display box) or "window" (the operator's PC). */
  source: "kms",
  /** A KMS box with a keyer's two inputs on DP-2 and DP-3 too (Fill/Key shots; set before `start`). */
  keyer: false,
  /** The audience screen as a three-projector Edge Blend wall (set before `start`). */
  wall: false,
  start: () => instances.forEach((i) => i.start()),
  stop: () => instances.forEach((i) => i.stop()),
  /** Another operator deletes the mask of this name: a later save of it answers 409, as the
   *  server's "was deleted by another operator". */
  deleteMaskElsewhere: (name) => instances.forEach((i) => i.deleteMaskElsewhere?.(name)),
};

const id = (n) => `5eed0000-0000-4000-8000-0000000c${String(n).padStart(4, "0")}`;

const hd = { width: 1920, height: 1080, refreshMhz: 60000 };
const DISPLAY_OUT = "5eed0000-0000-4000-8000-0000000d0001";
const PLACEHOLDER_OUT = "5eed0000-0000-4000-8000-0000000d0002";
const WALL = { kind: "edgeBlend", columns: 3, overlap: { x: 240, y: 0 } };

export default function screensRoutes({ fx, state, bump, json, svg }) {
  const main = state.state.screens.find((s) => s.role === "audience")?.id;
  const stage = state.state.screens.find((s) => s.role === "stage");
  const kmsDisplays = [
    {
      id: "HDMI-A-1",
      index: 0,
      name: "Example Monitor",
      manufacturer: "XYZ",
      connected: true,
      modes: [hd, { width: 1920, height: 1080, refreshMhz: 59940 }, { width: 1920, height: 1080, refreshMhz: 50000 }, { width: 1280, height: 720, refreshMhz: 60000 }],
      preferred: hd,
      boundTo: stage ? [DISPLAY_OUT] : [],
    },
    { id: "HDMI-A-2", index: 1, name: "HDMI-A-2", connected: false, modes: [], boundTo: [] },
    {
      id: "DP-1",
      index: 2,
      name: "Side Panel",
      connected: true,
      modes: [{ width: 3840, height: 2160, refreshMhz: 30000 }, hd],
      preferred: { width: 3840, height: 2160, refreshMhz: 30000 },
      boundTo: [],
    },
  ];
  const winDisplays = [
    { id: "\\\\.\\DISPLAY1", index: 0, name: "Display 1", connected: true, modes: [hd], preferred: hd, source: "window", position: { x: 0, y: 0 }, primary: true, boundTo: [] },
    { id: "\\\\.\\DISPLAY2", index: 1, name: "Display 2", connected: true, modes: [hd], preferred: hd, source: "window", position: { x: 1920, y: 0 }, boundTo: stage ? [DISPLAY_OUT] : [] },
  ];
  const win = () => screensStub.source === "window";
  const keyerDisplays = [
    { id: "DP-2", index: 3, name: "Keyer Fill", connected: true, modes: [hd], preferred: hd, boundTo: [] },
    { id: "DP-3", index: 4, name: "Keyer Key", connected: true, modes: [hd], preferred: hd, boundTo: [] },
  ];
  const displays = () => (win() ? winDisplays : screensStub.keyer ? [...kmsDisplays, ...keyerDisplays] : kmsDisplays);
  const displayOutput = () =>
    stage && {
      revision: "d15p1a70",
      schemaVersion: 2,
      id: DISPLAY_OUT,
      name: win() ? "Display 2" : "Example Monitor",
      screen: stage.id,
      transport: win()
        ? { type: "display", selector: { connector: winDisplays[1].id, name: "Display 2" } }
        : { type: "display", selector: { connector: "HDMI-A-1", name: "Example Monitor" }, mode: hd },
    };
  // The wall: the fixture's audience output on the left tile, a placeholder on the middle one.
  const wallOutput = (o) => (screensStub.wall && o.screen === main ? { ...o, tile: 0 } : o);
  const placeholderOutput = () => ({
    revision: "p1ac3h01",
    schemaVersion: 2,
    id: PLACEHOLDER_OUT,
    name: "Centre Projector",
    screen: main,
    transport: { type: "placeholder" },
    tile: 1,
  });
  // Edited by POST / PUT / DELETE /looks (while the stub is on; `start` puts them back).
  const initialLooks = () => [
    { schemaVersion: 2, id: id(1), name: "Full Service", sortKey: "a", screens: [] },
    { schemaVersion: 2, id: id(2), name: "Lyrics Only", sortKey: "b", screens: main ? [{ screen: main, layers: { media: false, videoInput: false } }] : [] },
    {
      schemaVersion: 2,
      id: id(3),
      name: "Announcements",
      sortKey: "c",
      screens: main ? [{ screen: main, layers: { slide: false, messages: false }, mask: id(22) }] : [],
      transition: { kind: { type: "dissolve" }, duration: 1.5 },
    },
  ];
  // Masks (made up): a 4:3 keyhole window and an overlay corner bug.
  const shape = (n, kind, x, y, w, h, color) => ({
    id: `5eed0000-0000-4000-8000-0000000f${String(n).padStart(4, "0")}`,
    name: `${kind === "ellipse" ? "Ellipse" : "Rectangle"} 1`,
    bounds: { x, y, width: w, height: h },
    kind: { type: "shape", shape: { type: kind } },
    fill: { enabled: true, kind: { type: "color", color } },
  });
  const initialMasks = () => [
    { schemaVersion: 2, id: id(21), name: "Center 4:3", sortKey: "a", size: { width: 1920, height: 1080 }, mode: "keyhole", feather: 12, slide: { id: id(31), elements: [shape(1, "rectangle", 240, 0, 1440, 1080, { r: 0, g: 0, b: 0 })] } },
    { schemaVersion: 2, id: id(22), name: "Corner bug", sortKey: "b", size: { width: 1920, height: 1080 }, slide: { id: id(32), elements: [shape(2, "ellipse", 1660, 60, 200, 200, { r: 0.43, g: 0.36, b: 0.9 })] } },
  ];
  let masks = initialMasks();
  /** Masks deleted by "another operator" (`deleteMaskElsewhere`). */
  const goneMasks = new Set();
  let looks = initialLooks();
  let lookSeq = 0;
  const rev = (l) => `${l.id.slice(-4)}${JSON.stringify(l).length.toString(16)}`;
  const entry = (l) => ({ ...l, revision: rev(l) });
  const sorted = () => looks.sort((a, b) => (a.sortKey < b.sortKey ? -1 : a.sortKey > b.sortKey ? 1 : 0));
  // Oriel reads the list again after its own edits (the stub sends no events for them).
  const lookChanged = () => sorted();

  const inst = {};
  instances.add(inst);
  inst.deleteMaskElsewhere = (name) => {
    const m = masks.find((x) => x.name === name);
    if (!m) return;
    goneMasks.add(m.id);
    masks = masks.filter((x) => x !== m);
  };
  inst.start = () => {
    screensStub.on = true;
    looks = initialLooks();
    masks = initialMasks();
    saved.clear();
    state.state.activeLook = looks[1].id;
    const s = state.state.screens.find((x) => x.id === main);
    if (s) s.testPattern = { type: "colorBars" };
    state.state.displays = displays();
    const ms = state.state.screens.find((x) => x.id === main);
    if (screensStub.wall && ms) {
      const base = fx.get("screens").find((e) => e.id === main);
      saved.set(main, { doc: { ...base, size: { width: 5280, height: 1080 }, arrangement: WALL }, n: 1 });
      ms.width = 5280;
      for (const o of ms.outputs) if (o.binding) o.width = 1920;
      ms.outputs.push({
        name: "Placeholder: Centre Projector",
        kind: "placeholder",
        origin: "binding",
        binding: PLACEHOLDER_OUT,
        width: 1920,
        height: ms.height,
        frameRate: ms.frameRate,
        delayFrames: 0,
        status: "unavailable",
        detail: "No hardware yet",
        framesSent: 0,
        framesDropped: 0,
        errors: 0,
        embedAudio: false,
        audioDb: 0,
        audioMuted: false,
      });
    }
    const st = state.state.screens.find((x) => x.id === stage?.id);
    if (st && !st.outputs.some((o) => o.binding === DISPLAY_OUT)) {
      st.outputs.push({
        name: `Display: ${win() ? winDisplays[1].id : "HDMI-A-1"}`,
        kind: "display",
        origin: "binding",
        binding: DISPLAY_OUT,
        width: st.width,
        height: st.height,
        frameRate: st.frameRate,
        delayFrames: 0,
        status: "ok",
        detail: null,
        framesSent: 5400,
        framesDropped: 0,
        errors: 0,
        embedAudio: false,
        audioDb: 0,
        audioMuted: false,
      });
    }
    bump();
  };
  inst.stop = () => {
    screensStub.on = false;
    screensStub.source = "kms";
    screensStub.keyer = false;
    if (screensStub.wall) {
      const ms = state.state.screens.find((x) => x.id === main);
      if (ms) {
        ms.width = 1920;
        for (const o of ms.outputs) if (o.binding) o.width = 1920;
      }
    }
    screensStub.wall = false;
    saved.clear();
    state.state.activeLook = null;
    for (const s of state.state.screens) {
      s.testPattern = null;
      s.outputs = s.outputs.filter((o) => o.binding !== DISPLAY_OUT && o.binding !== PLACEHOLDER_OUT);
    }
    delete state.state.displays;
    bump();
  };

  // Colour correction and corner pin per screen (PUT /screens/{id}/color, /corner-pin),
  // served back in GET /screens. Not stored on disk.
  const adjust = new Map();
  // Screens saved with PUT /screens/{id} while the stub is on (e.g. a stage screen's mask),
  // as the server checks them: a newly set mask must exist and the screen be a stage screen; a
  // mask kept as it was that no longer fits is dropped. `start` puts them back.
  const saved = new Map();
  const withSaved = (e) => (saved.has(e.id) ? { ...saved.get(e.id).doc, revision: `${e.revision}-s${saved.get(e.id).n}` } : e);
  const withAdjust = (e0) => {
    const e = withSaved(e0);
    return { ...e, ...(adjust.get(e.id) ?? {}), revision: adjust.has(e.id) ? `${e.revision}-${adjust.get(e.id).n}` : e.revision };
  };
  const stageMaskUsers = (maskId) => fx.get("screens").map(withSaved).filter((e) => e.mask === maskId);
  const isDefault = (v) => Object.keys(v).length === 0;

  // POST /looks, GET / PUT / DELETE /looks/{id}, POST /looks/{id}/duplicate (If-Match checked).
  function looksRoute(req, res, parts) {
    const cur = parts[1] && looks.find((l) => l.id === parts[1]);
    const ifMatch = (req.headers["if-match"] ?? "").replaceAll('"', "");
    if (parts[1] && !cur) return json(res, 404, { error: { code: "notFound", message: `look ${parts[1]} not found` } });
    if (cur && ifMatch && ifMatch !== rev(cur) && (req.method === "PUT" || req.method === "DELETE"))
      return json(res, 409, { error: { code: "conflict", message: "this look was changed by another operator; reload it or overwrite" }, current: entry(cur) });
    if (req.method === "GET") return json(res, 200, entry(cur));
    if (req.method === "DELETE") {
      if (state.state.activeLook === cur.id) return json(res, 409, { error: { code: "conflict", message: "this look is live; make another look live first" } });
      looks = looks.filter((l) => l !== cur);
      lookChanged();
      res.writeHead(204);
      return res.end();
    }
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      const v = JSON.parse(body || "{}");
      const after = (l) => looks.filter((x) => x.sortKey > l.sortKey).map((x) => x.sortKey).sort()[0];
      if (req.method === "PUT") {
        const { revision: _r, ...doc } = v;
        Object.keys(cur).forEach((k) => delete cur[k]);
        Object.assign(cur, doc, { id: parts[1], name: (doc.name ?? "").trim() });
        lookChanged();
        return json(res, 200, entry(cur));
      }
      const n = ++lookSeq;
      const newId = `5eed0000-0000-4000-8000-0000000e${String(n).padStart(4, "0")}`;
      let doc;
      if (parts[2] === "duplicate") {
        const next = after(cur);
        // Right after the original ("b" → "bV" sorts before "c").
        doc = { ...structuredClone(cur), id: newId, name: v.name || `${cur.name} copy`, sortKey: next && next <= `${cur.sortKey}V` ? `${cur.sortKey}0V` : `${cur.sortKey}V` };
      } else {
        const last = looks.map((l) => l.sortKey).sort().pop() ?? "";
        let k = 1;
        while (looks.some((l) => l.name.toLowerCase() === `look ${k}`)) k++;
        doc = { schemaVersion: 2, screens: [], ...v, id: newId, name: v.name || `Look ${k}`, sortKey: `${last}V` };
      }
      looks.push(doc);
      lookChanged();
      json(res, 201, entry(doc));
    });
  }

  // GET / POST /masks, GET / PUT / DELETE /masks/{id} (`?detach=true` for one that looks use),
  // POST /masks/{id}/duplicate.
  function masksRoute(req, res, parts, query) {
    const cur = parts[1] && masks.find((m) => m.id === parts[1]);
    if (parts[1] && !cur && req.method === "PUT" && goneMasks.has(parts[1]))
      return json(res, 409, { error: { code: "conflict", message: "mask was deleted by another operator" }, current: null });
    if (parts[1] && !cur) return json(res, 404, { error: { code: "notFound", message: `mask ${parts[1]} not found` } });
    if (req.method === "GET") return json(res, 200, cur ? entry(cur) : masks.map(entry));
    if (req.method === "DELETE") {
      const users = looks.filter((l) => (l.screens ?? []).some((x) => x.mask === cur.id)).map((l) => l.id);
      const stages = stageMaskUsers(cur.id).map((e) => e.id);
      if ((users.length || stages.length) && query?.get("detach") !== "true")
        return json(res, 409, {
          error: { code: "conflict", message: `${users.length} look(s) and ${stages.length} stage screen(s) use this mask; delete with detach to remove it from them` },
          current: { looks: users, screens: stages },
        });
      for (const l of looks) for (const x of l.screens ?? []) if (x.mask === cur.id) delete x.mask;
      for (const id of stages) delete saved.get(id).doc.mask;
      masks = masks.filter((m) => m !== cur);
      return json(res, 200, { looks: users, screens: stages });
    }
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      const v = JSON.parse(body || "{}");
      if (req.method === "PUT") {
        const { revision: _r, ...doc } = v;
        Object.keys(cur).forEach((k) => delete cur[k]);
        Object.assign(cur, doc, { id: parts[1] });
        return json(res, 200, entry(cur));
      }
      const n = ++lookSeq;
      const newId = `5eed0000-0000-4000-8000-0000000e${String(n).padStart(4, "0")}`;
      const last = masks.map((m) => m.sortKey).sort().pop() ?? "";
      let k = 1;
      while (masks.some((m) => m.name.toLowerCase() === `mask ${k}`)) k++;
      const doc =
        parts[2] === "duplicate"
          ? { ...structuredClone(cur), id: newId, name: v.name || `${cur.name} copy`, sortKey: `${cur.sortKey}V` }
          : { schemaVersion: 2, size: { width: 1920, height: 1080 }, slide: { id: `${newId.slice(0, -4)}5${newId.slice(-3)}`, elements: [] }, ...v, id: newId, name: v.name || `Mask ${k}`, sortKey: `${last}V` };
      masks.push(doc);
      masks.sort((a, b) => (a.sortKey < b.sortKey ? -1 : 1));
      json(res, 201, entry(doc));
    });
  }

  return (req, res, { path, parts, query }) => {
    if (screensStub.on && req.method === "POST" && parts[0] === "displays" && parts[2] === "identify") {
      res.writeHead(204);
      res.end();
      return true;
    }
    if (screensStub.on && parts[0] === "masks") {
      masksRoute(req, res, parts, query);
      return true;
    }
    if (req.method === "PUT" && parts[0] === "screens" && (parts[2] === "color" || parts[2] === "corner-pin")) {
      const entry = fx.get("screens").find((e) => e.id === parts[1]);
      if (!entry) {
        json(res, 404, { error: { code: "notFound", message: `screen ${parts[1]} not found` } });
        return true;
      }
      let body = "";
      req.on("data", (d) => (body += d));
      req.on("end", () => {
        const v = JSON.parse(body || "{}");
        const cur = adjust.get(entry.id) ?? { n: 0 };
        const field = parts[2] === "color" ? "color" : "cornerPin";
        const next = { ...cur, n: cur.n + 1 };
        if (isDefault(v)) delete next[field];
        else next[field] = v;
        adjust.set(entry.id, next);
        const { n: _n, ...shown } = withAdjust(entry);
        json(res, 200, shown);
      });
      return true;
    }
    if (screensStub.on && req.method === "PUT" && parts[0] === "screens" && parts.length === 2) {
      const base = fx.get("screens").find((e) => e.id === parts[1]);
      if (!base) {
        json(res, 404, { error: { code: "notFound", message: `screen ${parts[1]} not found` } });
        return true;
      }
      let body = "";
      req.on("data", (d) => (body += d));
      req.on("end", () => {
        const { revision: _r, ...doc } = JSON.parse(body || "{}");
        const old = withSaved(base);
        const stage = doc.kind?.type === "stage";
        if (doc.mask && doc.mask === old.mask && (!stage || !masks.some((m) => m.id === doc.mask))) delete doc.mask;
        if (doc.mask && !stage) return json(res, 400, { error: { code: "badRequest", field: "mask", message: "only a stage screen has its own mask (an audience screen's comes from the look)" } });
        if (doc.mask && !masks.some((m) => m.id === doc.mask)) return json(res, 400, { error: { code: "badRequest", field: "mask", message: "no such mask" } });
        saved.set(base.id, { doc: { ...doc, id: base.id }, n: (saved.get(base.id)?.n ?? 0) + 1 });
        const { n: _n, ...shown } = withAdjust(base);
        json(res, 200, shown);
      });
      return true;
    }
    if (req.method === "GET" && path === "/screens" && (adjust.size || saved.size)) {
      json(res, 200, fx.get("screens").map((e) => {
        const { n: _n, ...shown } = withAdjust(e);
        return shown;
      }));
      return true;
    }
    // Template slide pictures for the look's theme picker: a made-up lower third.
    if (screensStub.on && req.method === "GET" && parts[0] === "themes" && parts[4] === "thumbnail") {
      const w = 320;
      const h = 180;
      const full = parts[1].endsWith("2");
      svg(
        res,
        `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}"><rect width="${w}" height="${h}" fill="${full ? "#1F3A4D" : "#000000"}"/>${full ? "" : `<rect y="${h * 0.7}" width="${w}" height="${h * 0.3}" fill="#000000" stroke="#3A3D46"/>`}<text x="${w / 2}" y="${full ? h / 2 : h * 0.88}" text-anchor="middle" fill="#FFFFFF" font-family="Inter, sans-serif" font-size="20" font-weight="600">Sample text</text></svg>`,
      );
      return true;
    }
    if (screensStub.on && parts[0] === "looks" && (req.method !== "GET" || parts[1])) {
      looksRoute(req, res, parts);
      return true;
    }
    if (!screensStub.on || req.method !== "GET") return false;
    if (path === "/looks") json(res, 200, looks);
    else if (path === "/displays") json(res, 200, displays());
    else if (path === "/outputs") json(res, 200, [...fx.get("outputs").map(wallOutput), ...(stage ? [displayOutput()] : []), ...(screensStub.wall ? [placeholderOutput()] : [])]);
    else if (path === "/output-transports")
      json(
        res,
        200,
        fx.get("output-transports").map((t) =>
          t.kind === "display"
            ? win()
              ? { ...t, supported: true, available: true, status: "windows on this desktop", detail: "A borderless window covering the chosen monitor (or a plain window, for rehearsal) on this computer's desktop." }
              : { ...t, supported: true, available: true, status: "direct (KMS)", detail: "Full screen on the monitors and projectors attached to this box, driven directly (no desktop needed)." }
            : t,
        ),
      );
    else return false;
    return true;
  };
}
