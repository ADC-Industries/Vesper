// Captures the stub server's fixtures from a real Vesper server, so their shapes are exactly
// what the server sends (not guessed from the types).
//
//   1. Start a server on an EMPTY scratch data folder (never the church's data):
//        mkdir -p /tmp/stubdata/portable /tmp/stubdata/machine
//        ./target/debug/vesper --data /tmp/stubdata --listen 127.0.0.1:5599 --no-mdns \
//          --software --no-site-manager          (needs the `render` group: sg render -c …)
//   2. node apps/control/test/stub/capture.mjs http://127.0.0.1:5599
//   3. Stop the server (kill its PID) and delete the scratch folder.
//
// The script creates synthetic content through the REST API (made-up song titles and lyrics,
// screens, a playlist with a header, timers, messages, a prop, a stage layout), triggers a
// slide so LIVE / NEXT show, and writes every GET answer Oriel needs at start and on its main
// screens to `fixtures/*.json`. Machine-specific values (host name, audio devices, fonts) are
// replaced with made-up ones. Media bins cannot be created over the API yet (G17), so
// `fixtures/media-bins*.json` are written by hand from the protocol types: keep them when
// re-capturing (delete the other fixtures first, as old documents keep their own IDs).

import { writeFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const BASE = (process.argv[2] ?? "http://127.0.0.1:5599").replace(/\/+$/, "");
const API = `${BASE}/api/v1`;
const OUT = join(dirname(fileURLToPath(import.meta.url)), "fixtures");
mkdirSync(OUT, { recursive: true });

async function call(method, path, body) {
  const res = await fetch(API + path, {
    method,
    headers: body === undefined ? {} : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${text}`);
  return text ? JSON.parse(text) : null;
}
const get = (p) => call("GET", p);
const post = (p, b) => call("POST", p, b);
const id = () => crypto.randomUUID();
const rgb = (hex) => ({ r: parseInt(hex.slice(1, 3), 16) / 255, g: parseInt(hex.slice(3, 5), 16) / 255, b: parseInt(hex.slice(5, 7), 16) / 255 });

// Group colours from the design canvas.
const GROUP = { Verse: "#2F5FB8", Chorus: "#1F9E8E", Bridge: "#B8456B", Ending: "#7A5BD6", Title: "#3A3D46", Intro: "#3A3D46" };

// Made-up songs. Every title and line here was written for these fixtures.
const SONGS = [
  {
    name: "Morning Lantern",
    sections: [
      ["Verse 1", ["Wake the sleepy valley now", "Carry light across the plain"], ["Every window, every door", "Opens wide to greet the day"]],
      ["Chorus", ["Hold the lantern high", "Let it fill the sky"], ["We will sing until the dawn", "Lifts the shadows from the field"]],
      ["Verse 2", ["Through the orchard, past the mill", "Footsteps gather on the hill"]],
      ["Bridge", ["Brighter, brighter", "Climbing ever higher"]],
      ["Ending", ["Hold the lantern high"]],
    ],
  },
  {
    name: "River of Kindness",
    sections: [
      ["Verse 1", ["Down by the river of kindness", "Stones turn smooth in the stream"], ["Water that never runs empty", "Carries the weary along"]],
      ["Chorus", ["Flow on, flow on", "Over the meadow and home"]],
      ["Verse 2", ["Bring every bucket and pitcher", "There is enough for us all"]],
    ],
  },
  {
    name: "Wide Open Fields",
    sections: [
      ["Verse 1", ["Wide open fields under silver rain", "Seeds in the furrow wait again"]],
      ["Chorus", ["Grow, little seed, grow tall", "Sun on the garden wall"]],
      ["Bridge", ["Season by season", "Rooted and reaching"]],
    ],
  },
  {
    name: "Harbor Light",
    sections: [
      ["Verse 1", ["Out where the grey waves wander", "A small light keeps the shore"]],
      ["Chorus", ["Harbor light, harbor light", "Guide the boats in tonight"]],
    ],
  },
  { name: "Evening Ember", sections: [["Verse 1", ["Gather near the ember glow", "Stories old and stories slow"]], ["Chorus", ["Rest now, rest now", "The stars keep watch above"]]] },
  { name: "Welcome", sections: [["Title", ["Welcome to Maple Street Gathering"], ["Coffee and tea in the hall after the service"]]] },
];

function blankSlide(template) {
  const s = structuredClone(template);
  s.id = id();
  for (const e of s.elements ?? []) e.id = id();
  return s;
}

async function makePresentation(library, song) {
  const created = await post("/presentations", { name: song.name, library });
  const doc = await get(`/presentations/${created.id}`);
  const template = doc.sections[0].slides[0];
  const ops = [];
  let after;
  for (const [name, ...slides] of song.sections) {
    const section = { id: id(), name, color: rgb(GROUP[name.replace(/ \d+$/, "")] ?? "#3A3D46"), slides: [] };
    ops.push({ type: "addSection", ...(after ? { after } : {}), section });
    after = section.id;
    let prev;
    for (const lines of slides) {
      const slide = blankSlide(template);
      ops.push({ type: "addSlide", section: section.id, ...(prev ? { after: prev } : {}), slide });
      ops.push({ type: "setSlideText", slide: slide.id, text: lines.join("\n") });
      prev = slide.id;
    }
  }
  ops.push({ type: "deleteSection", section: doc.sections[0].id });
  await post(`/presentations/${created.id}/edits`, { clientId: "capture", baseRevision: created.revision, ops });
  return created.id;
}

// ---------------------------------------------------------------- content

const songsLib = await post("/libraries", { name: "Songs" });
const otherLib = await post("/libraries", { name: "Announcements" });
const ids = {};
for (const s of SONGS) ids[s.name] = await makePresentation(s.name === "Welcome" ? otherLib.id : songsLib.id, s);

const folder = await post("/playlists", { name: "Sundays", kind: "folder" });
const service = await post("/playlists", { name: "Sunday Morning", kind: "playlist", purpose: "service", parent: folder.id });
await post("/playlists", { name: "Midweek Evening", kind: "playlist", purpose: "service" });
{
  const items = [
    { id: id(), kind: { type: "header", name: "Pre-service", color: rgb("#3A3D46") } },
    { id: id(), kind: { type: "presentation", presentation: { id: ids.Welcome, name: "Welcome" } } },
    { id: id(), kind: { type: "header", name: "Worship", color: rgb("#2F5FB8") } },
    { id: id(), kind: { type: "presentation", presentation: { id: ids["Morning Lantern"], name: "Morning Lantern" } } },
    { id: id(), kind: { type: "presentation", presentation: { id: ids["River of Kindness"], name: "River of Kindness" } } },
    { id: id(), kind: { type: "presentation", presentation: { id: ids["Harbor Light"], name: "Harbor Light" } } },
  ];
  const ops = [];
  let after;
  for (const item of items) {
    ops.push({ type: "addItem", ...(after ? { after } : {}), item });
    after = item.id;
  }
  await post(`/playlists/${service.id}/edits`, { clientId: "capture", ops });
}

const audience = await post("/screens", { name: "Main Projector", kind: { type: "audience" }, size: { width: 1920, height: 1080 } });
const stage = await post("/screens", { name: "Stage Monitor", kind: { type: "stage" }, size: { width: 1920, height: 1080 } });
await post("/outputs", { name: "Main Projector", screen: audience.id, transport: { type: "idn", sourceName: "Main Projector" } }).catch((e) =>
  console.warn("output not created:", e.message),
);

await post("/timers", { name: "Service Countdown", kind: { type: "countdown", seconds: 600 } });
await post("/timers", { name: "Sermon", kind: { type: "elapsed" } });
await post("/timers", { name: "Doors Open", kind: { type: "countdownToTime", secondsAfterMidnight: 10 * 3600 + 15 * 60 } });
await post("/messages", { name: "Parking", text: "The car with plate ${plate} is blocking the gate" }).catch((e) => console.warn("message:", e.message));
await post("/messages", { name: "Nursery", text: "Nursery: parent of ${number}, please come to the nursery" }).catch((e) => console.warn("message:", e.message));
{
  const welcome = await get(`/presentations/${ids.Welcome}`);
  await post("/props", { name: "Lower Third", fromSlide: { presentation: ids.Welcome, slide: welcome.sections[0].slides[0].id } }).catch((e) =>
    console.warn("prop:", e.message),
  );
}
await post("/stage-layouts", { name: "Band", template: "lyricsClockTimer" }).catch((e) => console.warn("stage layout:", e.message));
await post("/stage-layouts", { name: "Speaker", template: "notes" }).catch((e) => console.warn("stage layout:", e.message));

// Live: the second slide of "Morning Lantern" from the playlist, so the cursor sits on the third.
{
  const pl = await get(`/playlists/${service.id}`);
  const item = pl.items.find((i) => i.kind?.type === "presentation" && i.kind.presentation?.id === ids["Morning Lantern"]) ?? pl.items[3];
  const doc = await get(`/presentations/${ids["Morning Lantern"]}`);
  const slide = doc.sections[0].slides[1].id;
  const r = await post("/commands", { type: "triggerSlide", presentation: ids["Morning Lantern"], slide, playlistItem: { playlist: service.id, item: item.id } });
  if (!r.ok) console.warn("trigger:", r);
}
await new Promise((r) => setTimeout(r, 1500));

// ---------------------------------------------------------------- capture

const HOST_NAME = (await get("/server")).name;
const scrub = (v) => {
  let s = JSON.stringify(v);
  s = s.split(HOST_NAME).join("Booth A");
  return JSON.parse(s);
};
const save = (name, v) => writeFileSync(join(OUT, `${name}.json`), JSON.stringify(scrub(v), null, 2) + "\n");

const server = await get("/server");
delete server.pairingCode;
save("server", server);
// Start the countdown so a timer runs.
{
  const timers = await get("/timers");
  await post("/commands", { type: "timerStart", timer: timers[0].id }).catch((e) => console.warn("timerStart:", e.message));
}
for (const r of [
  "state",
  "libraries",
  "presentations",
  "playlists",
  "looks",
  "screens",
  "outputs",
  "output-transports",
  "clear-groups",
  "groups",
  "themes",
  "timers",
  "messages",
  "props",
  "stage-layouts",
  "stage-layout-templates",
  "diagnostics",
  "codex",
]) {
  const v = await get(`/${r}`);
  if (r === "diagnostics") Object.assign(v, { adapter: "Example GPU (Vulkan)", uptimeSeconds: 3600 });
  save(r, v);
}
for (const l of await get("/libraries")) save(`libraries/${l.id}/presentations`.replaceAll("/", "__"), await get(`/libraries/${l.id}/presentations`));
for (const p of await get("/presentations")) save(`presentations__${p.id}`, await get(`/presentations/${p.id}`));
const walk = (nodes) => nodes.flatMap((n) => [n, ...walk(n.children ?? [])]);
for (const n of walk(await get("/playlists"))) if (n.kind === "playlist" || n.type === "playlist") save(`playlists__${n.id}`, await get(`/playlists/${n.id}`));
for (const p of await get("/props")) save(`props__${p.id}__document`, await get(`/props/${p.id}/document`));
for (const s of await get("/stage-layouts")) save(`stage-layouts__${s.id}__document`, await get(`/stage-layouts/${s.id}/document`));
save("codex__search", await get("/codex/search?q=light"));

// Machine-specific answers: made up.
save("fonts", {
  families: ["Inter", "Noto Sans", "Noto Serif", "DejaVu Sans"].map((family) => ({
    family,
    faces: [
      { weight: 400, italic: false, postscriptName: `${family.replace(/ /g, "")}-Regular` },
      { weight: 700, italic: false, postscriptName: `${family.replace(/ /g, "")}-Bold` },
    ],
  })),
  substitutions: [],
  fallbackFamily: "DejaVu Sans",
});
save("audio-devices", [
  { name: "default", description: "Default output", available: true, channels: 2 },
  { name: "hw:1,0", description: "USB Audio Interface", available: true, channels: 2 },
]);
save("audio-device", { schemaVersion: 2, device: "default" });

// The WebSocket's first messages (hello, state).
await new Promise((resolve, reject) => {
  const ws = new WebSocket(`${API.replace(/^http/, "ws")}/ws?protocolVersion=1`);
  const got = {};
  ws.onmessage = (ev) => {
    if (typeof ev.data !== "string") return;
    const m = JSON.parse(ev.data);
    if (m.type === "hello" || m.type === "state") got[m.type] = m;
    if (got.hello && got.state) {
      save("ws__hello", got.hello);
      save("ws__state", got.state);
      ws.close();
      resolve();
    }
  };
  ws.onerror = reject;
});
console.log(`fixtures written to ${OUT}`);
