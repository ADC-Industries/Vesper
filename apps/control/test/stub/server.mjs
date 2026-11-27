// A stub Vesper server for Oriel's screenshots and UI work: serves the built app (`dist/`) and
// answers /api/v1 from the fixtures in `fixtures/` (captured from a real server by
// `capture.mjs`, synthetic content only), with a scripted WebSocket (hello, state, results,
// pong, timer ticks, meters, preview frames) and generated pictures (SVG) for thumbnails.
//
//   node test/stub/server.mjs [--port 5599] [--mode local|unpaired|down] [--dist dist]
//
// or from a script: `const stub = await startStub({ port, dist, previewJpeg, mode })`, then
// `stub.setMode("down")`, `stub.push({type: "notice", …})`, `await stub.close()`.
//
// `stub.revoke()` closes the open sockets with 4003 (the client's key was revoked) and switches to
// `unpaired`.
//
// Modes: `local` (a request from the server machine: no pairing, Oriel opens its main view),
// `unpaired` (Oriel shows the pairing screen), `down` (every API request and socket is refused,
// open sockets are dropped: Oriel's offline banner). Writes (POST / PUT / DELETE) other than
// commands, thumbnail prefetches and edits answer `501 notImplemented`; nothing is stored.
//
// No dependencies: the WebSocket server is the small subset of RFC 6455 Oriel needs.
//
// Per-area additions (so parallel lanes don't edit the same file):
// - Fixtures: `fixtures/*.json` are the captured, shared ones (`capture.mjs`). A page's own
//   fixtures go in `fixtures/<area>/*.json` (e.g. `fixtures/library/`); they are served the same
//   way (file name = the GET path with `/` as `__`). A name used twice is an error.
// - Routes: every `routes/<area>.mjs` default-exports `(ctx) => handler`; `handler(req, res,
//   { path, parts, query })` answers a request (and returns true) before the stub's own routes.
//   `ctx` holds the fixtures map `fx`, the show state `state`, `bump()` (revision + 1, sent to
//   every socket), `push(msg)` (a server message to every socket), `json(res, status, body)`, `svg(res, body)` and `apiError(res, status, code,
//   message)`. `startStub({ routes: ["library"] })` loads only those areas' routes (a second stub
//   in a shot script, so the other areas' route state isn't set up again).

import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { createServer } from "node:http";
import { dirname, extname, join, normalize, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { presenceStub } from "./routes/presence.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const FIXTURES = join(HERE, "fixtures");
const ROUTES = join(HERE, "routes");

// ---------------------------------------------------------------- fixtures

/** The shared fixtures (`fixtures/*.json`) and every area's (`fixtures/<area>/*.json`). */
function loadFixtures() {
  const out = new Map();
  const load = (dir) => {
    for (const f of readdirSync(dir, { withFileTypes: true })) {
      if (f.isDirectory() && dir === FIXTURES) load(join(dir, f.name));
      else if (f.isFile() && f.name.endsWith(".json")) {
        const key = f.name.slice(0, -5);
        if (out.has(key)) throw new Error(`stub: fixture ${key} is defined twice (${join(dir, f.name)})`);
        out.set(key, JSON.parse(readFileSync(join(dir, f.name), "utf8")));
      }
    }
  };
  load(FIXTURES);
  return out;
}

/** Every area's extra routes (`routes/*.mjs`), in file name order. */
async function loadRoutes(ctx, only = null) {
  if (!existsSync(ROUTES)) return [];
  const files = readdirSync(ROUTES)
    .filter((f) => f.endsWith(".mjs") && (!only || only.includes(f.slice(0, -4))))
    .sort();
  return Promise.all(files.map(async (f) => (await import(pathToFileURL(join(ROUTES, f)).href)).default(ctx)));
}

const GROUP_COLOURS = { verse: "#2F5FB8", chorus: "#1F9E8E", bridge: "#B8456B", ending: "#7A5BD6" };
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const hex = (c) => (c ? "#" + [c.r, c.g, c.b].map((v) => Math.round(v * 255).toString(16).padStart(2, "0")).join("") : null);

function groupColour(name, colour) {
  const key = String(name ?? "").toLowerCase().replace(/\s*\d+$/, "");
  return GROUP_COLOURS[key] ?? hex(colour) ?? "#3A3D46";
}

/** Plain text of a slide's text elements (paragraphs joined with newlines). */
function slideText(slide) {
  const lines = [];
  for (const e of slide?.elements ?? []) {
    const t = e.kind?.type === "text" ? e.kind.text : null;
    for (const p of t?.paragraphs ?? []) lines.push((p.runs ?? []).map((r) => r.text ?? "").join(""));
  }
  return lines;
}

/** A slide picture: its text in white on a dark gradient tinted by its group's colour. */
export function slideSvg(lines, colour, w = 320, h = Math.round((w * 9) / 16), transparent = false) {
  const size = Math.max(10, Math.round(w / 22));
  const gap = size * 1.3;
  const top = h / 2 - ((lines.length - 1) * gap) / 2 + size * 0.35;
  const text = lines
    .map((l, i) => `<text x="${w / 2}" y="${(top + i * gap).toFixed(1)}" text-anchor="middle">${esc(l)}</text>`)
    .join("");
  const bg = transparent
    ? ""
    : `<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${colour}" stop-opacity="0.55"/><stop offset="0.6" stop-color="#121317"/><stop offset="1" stop-color="#0B0C0F"/></linearGradient></defs><rect width="${w}" height="${h}" fill="url(#g)"/>`;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">${bg}<g fill="#FFFFFF" font-family="Inter, 'DejaVu Sans', sans-serif" font-size="${size}" font-weight="600">${text}</g></svg>`;
}

/** Colour bars for media thumbnails, shifted by a hash of the asset ID. */
export function barsSvg(seed, w = 320, h = Math.round((w * 9) / 16)) {
  const bars = ["#C0C0C0", "#C0C000", "#00C0C0", "#00C000", "#C000C0", "#C00000", "#0000C0"];
  let n = 0;
  for (const ch of String(seed)) n = (n * 31 + ch.charCodeAt(0)) >>> 0;
  const shift = n % bars.length;
  const bw = w / bars.length;
  const rects = bars.map((_, i) => `<rect x="${(i * bw).toFixed(1)}" width="${(bw + 0.5).toFixed(1)}" height="${h}" fill="${bars[(i + shift) % bars.length]}"/>`).join("");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">${rects}<rect y="${h * 0.75}" width="${w}" height="${h * 0.25}" fill="#1A1C21"/></svg>`;
}

// ---------------------------------------------------------------- WebSocket (RFC 6455 subset)

function wsAccept(key) {
  return createHash("sha1").update(key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest("base64");
}

function wsFrame(opcode, payload) {
  const len = payload.length;
  let head;
  if (len < 126) head = Buffer.from([0x80 | opcode, len]);
  else if (len < 65536) {
    head = Buffer.alloc(4);
    head[0] = 0x80 | opcode;
    head[1] = 126;
    head.writeUInt16BE(len, 2);
  } else {
    head = Buffer.alloc(10);
    head[0] = 0x80 | opcode;
    head[1] = 127;
    head.writeBigUInt64BE(BigInt(len), 2);
  }
  return Buffer.concat([head, payload]);
}

class WsPeer {
  constructor(socket, onText, onClose) {
    this.socket = socket;
    this.buf = Buffer.alloc(0);
    this.open = true;
    this.subs = new Map();
    socket.on("data", (d) => {
      this.buf = Buffer.concat([this.buf, d]);
      this.parse(onText);
    });
    const end = () => {
      if (!this.open) return;
      this.open = false;
      onClose(this);
    };
    socket.on("close", end);
    socket.on("error", end);
  }
  parse(onText) {
    for (;;) {
      if (this.buf.length < 2) return;
      const op = this.buf[0] & 0x0f;
      const masked = (this.buf[1] & 0x80) !== 0;
      let len = this.buf[1] & 0x7f;
      let off = 2;
      if (len === 126) {
        if (this.buf.length < 4) return;
        len = this.buf.readUInt16BE(2);
        off = 4;
      } else if (len === 127) {
        if (this.buf.length < 10) return;
        len = Number(this.buf.readBigUInt64BE(2));
        off = 10;
      }
      const maskOff = off;
      if (masked) off += 4;
      if (this.buf.length < off + len) return;
      const payload = Buffer.from(this.buf.subarray(off, off + len));
      if (masked) for (let i = 0; i < len; i++) payload[i] ^= this.buf[maskOff + (i % 4)];
      this.buf = this.buf.subarray(off + len);
      if (op === 0x1) onText(this, payload.toString("utf8"));
      else if (op === 0x8) {
        this.raw(wsFrame(0x8, payload.subarray(0, 2)));
        this.socket.end();
      } else if (op === 0x9) this.raw(wsFrame(0xa, payload));
    }
  }
  raw(b) {
    if (this.open && !this.socket.destroyed) this.socket.write(b);
  }
  send(obj) {
    this.raw(wsFrame(0x1, Buffer.from(JSON.stringify(obj))));
  }
  sendBinary(b) {
    this.raw(wsFrame(0x2, b));
  }
  drop() {
    this.open = false;
    this.socket.destroy();
  }
  /** Closes the socket with a close code (e.g. 4003: the key was revoked). */
  closeWith(code, reason = "") {
    const body = Buffer.concat([Buffer.from([code >> 8, code & 0xff]), Buffer.from(reason)]);
    this.raw(wsFrame(0x8, body));
    this.open = false;
    this.socket.end();
  }
}

// ---------------------------------------------------------------- server

const MIME = { ".html": "text/html; charset=utf-8", ".js": "text/javascript", ".css": "text/css", ".svg": "image/svg+xml", ".woff2": "font/woff2", ".png": "image/png", ".json": "application/json", ".ico": "image/x-icon" };

/**
 * Starts the stub. `previewJpeg` (a Buffer) is sent as every screen's live preview; without it
 * no preview frames are sent.
 */
export async function startStub({ port = 0, host = "127.0.0.1", dist = resolve(HERE, "../../dist"), previewJpeg = null, mode = "local", routes = null } = {}) {
  const fx = loadFixtures();
  const state = structuredClone(fx.get("ws__state"));
  const hello = fx.get("ws__hello");
  const peers = new Set();
  let current = mode;

  const presentation = (id) => fx.get(`presentations__${id}`) ?? null;
  const findSlide = (pid, sid) => {
    const doc = presentation(pid);
    for (const sec of doc?.sections ?? []) for (const s of sec.slides ?? []) if (s.id === sid) return { doc, sec, slide: s };
    return null;
  };
  const allSlides = (pid) => (presentation(pid)?.sections ?? []).flatMap((sec) => (sec.slides ?? []).map((slide) => ({ sec, slide })));
  /** Document indices in play order, as the server's `show::play_order`. */
  const playOrder = (pid, ref) => {
    const doc = presentation(pid);
    const item = ref ? fx.get(`playlists__${ref.playlist}`)?.items?.find((it) => it.id === ref.item) : null;
    const find = (id) => (id ? doc?.arrangements?.find((a) => a.id === id) : undefined);
    const arr = find(item?.kind?.arrangement) ?? find(doc?.selectedArrangement);
    const starts = new Map();
    let at = 0;
    for (const sec of doc?.sections ?? []) {
      starts.set(sec.id, [at, (sec.slides ?? []).length]);
      at += (sec.slides ?? []).length;
    }
    const order = (arr?.sections ?? []).flatMap((id) => {
      const r = starts.get(id);
      return r ? Array.from({ length: r[1] }, (_, k) => r[0] + k) : [];
    });
    return order.length ? order : Array.from({ length: at }, (_, k) => k);
  };

  const bump = () => {
    state.revision += 1;
    for (const p of peers) p.send(state);
  };

  // What each clear took off a layer (`restoreLayer` brings it back while the layer is empty).
  const cleared = {};
  const empty = (s, l) => (Array.isArray(s.layers[l]) ? s.layers[l].length === 0 : !s.layers[l]);
  const restorable = (s) => {
    s.restorableLayers = ["videoInput", "media", "slide", "announcements", "props", "messages", "audio"].filter((l) => cleared[l] !== undefined && empty(s, l));
  };
  function take(s, l) {
    if (!(l in s.layers) || empty(s, l)) return;
    cleared[l] = s.layers[l];
    s.layers[l] = Array.isArray(s.layers[l]) ? [] : null;
    if (l === "media" && s.transport?.layer === "media") s.transport = null;
    restorable(s);
  }

  function applyCommand(c) {
    const s = state.state;
    switch (c.type) {
      case "triggerSlide": {
        const list = allSlides(c.presentation);
        const i = c.slide ? list.findIndex((x) => x.slide.id === c.slide) : (c.index ?? 0);
        const hit = list[i];
        if (!hit) return;
        // The play order (the item's arrangement, else the song's own, else document order) and
        // the occurrence: `step` when it holds the slide, else the first at or after the live one.
        const order = playOrder(c.presentation, c.playlistItem);
        const prev = s.layers.slide;
        const same = prev?.presentation === c.presentation && (prev.playlist?.item ?? null) === (c.playlistItem?.item ?? null);
        const from = same && prev.step != null ? prev.step : 0;
        const step =
          c.step != null && order[c.step] === i ? c.step : order.map((_, k) => (from + k) % order.length).find((k) => order[k] === i);
        const pos = (j, k) => ({
          presentation: c.presentation,
          presentationName: presentation(c.presentation)?.name,
          slide: list[j].slide.id,
          slideIndex: j,
          section: list[j].sec.id,
          sectionName: list[j].sec.name,
          playlist: c.playlistItem ?? null,
          ...(k != null ? { step: k } : {}),
        });
        s.layers.slide = pos(i, step);
        s.build = null;
        const after = step != null ? order.findIndex((d, k) => k > step && list[d].slide.enabled !== false) : -1;
        s.cursor = after >= 0 ? pos(order[after], after) : step == null && i + 1 < list.length ? pos(i + 1, null) : null;
        delete cleared.slide;
        restorable(s);
        return bump();
      }
      case "clearAll":
        for (const l of ["videoInput", "slide", "media", "props", "messages", "audio"]) take(s, l);
        return bump();
      case "clearLayer":
        // Captions (routes/captions.mjs): the lines come off; captions keep running.
        if (c.layer === "captions" && s.captions) s.captions.lines = [];
        take(s, c.layer);
        return bump();
      case "triggerLiveSource": {
        // G7 (routes/sources.mjs lists the sources and reports this one live).
        const i = c.source.indexOf(" (");
        s.layers.videoInput = { name: i > 0 && c.source.endsWith(")") ? c.source.slice(i + 2, -1) : c.source, source: c.source };
        delete cleared.videoInput;
        restorable(s);
        return bump();
      }
      case "triggerVideoInput": {
        // Stage 4 (routes/sources.mjs keeps the logical inputs in `state.videoInputs`).
        const v = (state.videoInputs ?? []).find((x) => x.id === c.input);
        if (!v?.source?.sourceName) return;
        s.layers.videoInput = { input: v.id, name: v.name, source: v.source.sourceName };
        delete cleared.videoInput;
        restorable(s);
        return bump();
      }
      case "restoreLayer":
        if (cleared[c.layer] !== undefined && empty(s, c.layer)) s.layers[c.layer] = cleared[c.layer];
        delete cleared[c.layer];
        restorable(s);
        return bump();
      case "setClockFormat":
        s.clockFormat = { ...(s.clockFormat ?? { use24Hour: false, seconds: false }), ...("use24Hour" in c ? { use24Hour: !!c.use24Hour } : {}), ...("seconds" in c ? { seconds: !!c.seconds } : {}) };
        return bump();
      case "mediaSetLoop": {
        const l = c.layer ?? s.transport?.layer;
        const m = l ? s.layers[l === "slide" ? "foreground" : l] : null;
        if (m?.playback) m.playback.loop = !!c.loop;
        if (s.transport && s.transport.layer === l) s.transport.loop = !!c.loop;
        return bump();
      }
      case "setScreensEnabled":
        if (c.role) s.screensEnabled[c.role] = c.enabled;
        return bump();
      case "setMute":
        if (c.target === "layer" && s.audio?.layers?.[c.layer]) s.audio.layers[c.layer].muted = !!c.muted;
        return bump();
    }
  }

  function onText(peer, text) {
    let m;
    try {
      m = JSON.parse(text);
    } catch {
      return;
    }
    if (m.type === "ping") return peer.send({ type: "pong", id: m.id });
    // (What Oriel has open: the presence shots copy it into other people's lists.)
    if (m.type === "presence") return void (presenceStub.reported = m.open ?? []);
    if (m.type !== "command") return;
    const c = m.command ?? {};
    if (c.type === "subscribePreview") peer.subs.set(c.screen, { fps: c.fps ?? 10, width: c.width ?? 480 });
    else if (c.type === "unsubscribePreview") peer.subs.delete(c.screen);
    else applyCommand(c);
    peer.send({ type: "result", id: m.id, ok: true, revision: state.revision });
  }

  // Preview frames (2 a second), timer ticks (1 a second), meters (5 a second).
  let frameNo = 0;
  const previewHeader = (screen, w, h) => {
    const b = Buffer.alloc(32);
    Buffer.from(screen.replace(/-/g, ""), "hex").copy(b, 0);
    b.writeBigUInt64LE(BigInt(++frameNo), 16);
    b.writeUInt32LE(w, 24);
    b.writeUInt32LE(h, 28);
    return b;
  };
  const timers = [
    setInterval(() => {
      if (!previewJpeg) return;
      for (const p of peers) for (const [screen] of p.subs) p.sendBinary(Buffer.concat([previewHeader(screen, 640, 360), previewJpeg]));
    }, 500),
    setInterval(() => {
      const ticks = [];
      for (const t of state.state.timers) {
        if (t.state !== "running") continue;
        t.currentSeconds = Math.max(0, t.currentSeconds - 1);
        const s = Math.round(t.currentSeconds);
        t.display = `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
        ticks.push({ id: t.id, state: t.state, currentSeconds: t.currentSeconds, display: t.display });
      }
      if (ticks.length) for (const p of peers) p.send({ type: "timerTick", timers: ticks });
    }, 1000),
    setInterval(() => {
      const t = Date.now() / 300;
      const levels = [-18 + 6 * Math.sin(t), -20 + 6 * Math.cos(t * 1.3)];
      for (const p of peers) p.send({ type: "audioLevels", levels, xruns: 0 });
    }, 200),
  ];

  const json = (res, status, body) => {
    res.writeHead(status, { "Content-Type": "application/json", "Access-Control-Allow-Origin": "*" });
    res.end(body === undefined ? "" : JSON.stringify(body));
  };
  const svg = (res, body) => {
    res.writeHead(200, { "Content-Type": "image/svg+xml", "Cache-Control": "no-cache", "Access-Control-Allow-Origin": "*" });
    res.end(body);
  };
  const apiError = (res, status, code, message) => json(res, status, { error: { code, message } });
  // `push(msg)`: a server message to every connected client (e.g. `presenceChanged`).
  const push = (msg) => {
    for (const p of peers) p.send(msg);
  };
  const extraRoutes = await loadRoutes({ fx, state, bump, json, svg, apiError, push }, routes);

  function api(req, res, path, query) {
    const w = Math.min(1920, Math.max(16, Number(query.get("w")) || 320));
    const parts = path.split("/").filter(Boolean);
    const server = fx.get("server");
    if (current === "unpaired") {
      if (path === "/server") return json(res, 200, { ...server, auth: "unpaired" });
      if (path === "/pair" && req.method === "POST") return apiError(res, 401, "unauthorized", "That code is not right. Check the code on the server and try again.");
      return apiError(res, 401, "unauthorized", "pair first");
    }
    if (req.method === "OPTIONS") {
      res.writeHead(204, { "Access-Control-Allow-Origin": "*", "Access-Control-Allow-Headers": "Authorization, Content-Type, If-Match, X-Client-Id", "Access-Control-Allow-Methods": "GET, POST, PUT, DELETE" });
      return res.end();
    }
    // The areas' own routes (routes/*.mjs).
    for (const handle of extraRoutes) if (handle(req, res, { path, parts, query })) return;
    // Pictures.
    if (parts[0] === "presentations" && parts[2] === "slides" && (parts[4] === "thumbnail" || parts[4] === "render")) {
      const hit = findSlide(parts[1], parts[3]);
      return svg(res, slideSvg(slideText(hit?.slide), groupColour(hit?.sec.name, hit?.sec.color), w, undefined, parts[4] === "render"));
    }
    if ((parts[0] === "props" || parts[0] === "stage-layouts") && parts[2] === "slides" && parts[4] === "render") {
      const doc = fx.get(`${parts[0]}__${parts[1]}__document`);
      return svg(res, slideSvg(slideText(doc?.sections?.[0]?.slides?.[0]), "#3A3D46", w, undefined, true));
    }
    if (parts[0] === "props" && parts[2] === "thumbnail") {
      const doc = fx.get(`props__${parts[1]}__document`);
      return svg(res, slideSvg(slideText(doc?.sections?.[0]?.slides?.[0]), "#3A3D46", w, undefined, true));
    }
    if ((parts[0] === "stage-layouts" || parts[0] === "stage-layout-templates") && parts[2] === "thumbnail") {
      return svg(res, slideSvg(["CURRENT", "Next", "10:00"], "#15161A", w));
    }
    if (parts[0] === "media" && parts[2] === "thumbnail") return svg(res, barsSvg(parts[1], w));
    if (parts[0] === "media" && parts.length === 2) {
      return json(res, 200, { asset: { schemaVersion: 2, id: parts[1], kind: "image", name: "Generated", status: "online" }, online: true });
    }
    if (parts[0] === "codex" && parts[1] === "blobs") return svg(res, barsSvg(parts[2] ?? "", w));
    // Writes.
    if (req.method !== "GET") {
      if (path === "/commands") {
        let body = "";
        req.on("data", (d) => (body += d));
        req.on("end", () => {
          try {
            applyCommand(JSON.parse(body));
          } catch {
            /* ignore */
          }
          json(res, 200, { ok: true, revision: state.revision });
        });
        return;
      }
      if (parts[0] === "presentations" && parts[2] === "thumbnails") {
        return json(res, 202, { slides: allSlides(parts[1]).length, cached: allSlides(parts[1]).length, queued: 0 });
      }
      return apiError(res, 501, "notImplemented", "the stub server does not store changes");
    }
    // Reads.
    if (path === "/state") return json(res, 200, { revision: state.revision, state: state.state });
    if (path === "/server") return json(res, 200, server);
    const key = parts.join("__");
    if (fx.has(key)) return json(res, 200, fx.get(key));
    return apiError(res, 404, "notFound", `no route GET ${path} in the stub`);
  }

  function serveStatic(res, urlPath) {
    const rel = normalize(decodeURIComponent(urlPath)).replace(/^([/\\])+/, "");
    let file = join(dist, rel);
    if (!file.startsWith(dist) || !existsSync(file) || statSync(file).isDirectory()) file = join(dist, "index.html");
    if (!existsSync(file)) {
      res.writeHead(404, { "Content-Type": "text/plain" });
      return res.end("dist/ not built: run npm run build");
    }
    res.writeHead(200, { "Content-Type": MIME[extname(file)] ?? "application/octet-stream", "Cache-Control": "no-cache" });
    res.end(readFileSync(file));
  }

  const httpServer = createServer((req, res) => {
    const u = new URL(req.url ?? "/", "http://stub");
    if (u.pathname.startsWith("/api/v1/") || u.pathname === "/api/v1") {
      if (current === "down") return req.socket.destroy();
      return api(req, res, u.pathname.slice("/api/v1".length) || "/", u.searchParams);
    }
    serveStatic(res, u.pathname);
  });

  httpServer.on("upgrade", (req, socket) => {
    const u = new URL(req.url ?? "/", "http://stub");
    const key = req.headers["sec-websocket-key"];
    if (current !== "local" || u.pathname !== "/api/v1/ws" || !key) return socket.destroy();
    socket.write(
      "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" + `Sec-WebSocket-Accept: ${wsAccept(key)}\r\n\r\n`,
    );
    const peer = new WsPeer(socket, onText, (p) => peers.delete(p));
    peers.add(peer);
    peer.send(hello);
    peer.send(state);
  });

  await new Promise((r) => httpServer.listen(port, host, r));
  const addr = httpServer.address();
  const url = `http://${host}:${addr.port}`;

  return {
    url,
    port: addr.port,
    /** `local`, `unpaired` or `down` (drops every open socket). */
    setMode(m) {
      current = m;
      if (m !== "local") for (const p of peers) p.drop();
    },
    /** The site removed this client: open sockets close with 4003 and the API asks to pair. */
    revoke() {
      current = "unpaired";
      for (const p of peers) p.closeWith(4003, "key revoked");
    },
    /** Sends a server message (e.g. a `notice`) to every connected client. */
    push(msg) {
      for (const p of peers) p.send(msg);
    },
    setPreviewJpeg(b) {
      previewJpeg = b;
    },
    /** Sets `state.build` (the live slide's object build progress, plan G18: `{step, steps}` for the live slide; null clears) and sends the state. */
    setBuild(b) {
      const l = state.state.layers.slide;
      state.state.build = b && l ? { layer: "slide", presentation: l.presentation, slide: l.slide, ...b } : null;
      bump();
    },
    async close() {
      for (const t of timers) clearInterval(t);
      for (const p of peers) p.drop();
      httpServer.closeAllConnections?.();
      await new Promise((r) => httpServer.close(r));
    },
  };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const arg = (name, def) => {
    const i = process.argv.indexOf(`--${name}`);
    return i > 0 ? process.argv[i + 1] : def;
  };
  const stub = await startStub({
    port: Number(arg("port", 5599)),
    mode: arg("mode", "local"),
    dist: resolve(arg("dist", resolve(HERE, "../../dist"))),
  });
  console.log(`stub Vesper server at ${stub.url}/ (mode ${arg("mode", "local")})`);
}
