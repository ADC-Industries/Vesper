// The Library page's stub routes (see server.mjs's header for the interface): a synthetic Codex
// with three collections. "Booth A" (active at start) is the captured fixtures' content; "Main
// Hall" (~28 made-up songs on three shelves, images and videos with generated thumbnails,
// themes, props) and "Youth Room" are generated here. Every title, lyric line and name is made
// up. Also: the item trash, search, the collection switch (`409 liveShow` while something is
// live, unless confirmed) and two roles for the side connection's screenshots. Round 4: the
// collections' writes and a collection in the collection trash, resumable uploads (chunks are
// counted, not kept; `uploadDelay` slows each chunk down for the screenshots), missing media
// with relink, "Find missing files" and folder matching, and the storage figures. Batch 4: the
// media library's items (`GET /codex/library`) and a synthetic "Used by" for media and props.
// Batch 8 (G14): three made-up show boxes in `GET /codex` (`boxes`) and each collection's
// `boxes {showing, synced}`.
//
// Stub control (not part of the API): `POST /api/v1/__stub/library` with
//   {"reset": true}                       the Codex's content as at start (Booth A active)
//   {"role": "showBox", "codexBox": {serverId, name, addresses}, "reachable": true|false}
//                                          this stub is a show box: `GET /codex` says `client`,
//                                          and Codex management answers `409 codexElsewhere`
//   {"role": "codexBox", "name": "…"}      this stub stands in for the box holding the Codex
//                                          (its own server ID and name in `GET /server`)
//   {"role": "normal"}
//   {"uploadDelay": 1500}                  each upload chunk answers after that many ms
//   {"dormant": true|false}                (show box) this box's own collection is dormant (G15):
//                                          `GET /codex` says `dormantCollection`; `POST
//                                          /codex/dormant/upload` starts an upload that stays
//                                          half way until {"dormantFinish": true}
// The content is shared by every stub in the process (one Codex); the role is per stub.

import { createHash } from "node:crypto";
import { bibleStub } from "./bible.mjs";

const BOOTH_A = "6c931ace-df87-4f16-960c-43669b655534";
export const CODEX_BOX_ID = "5d0c7e3b-8a51-4c2e-9f7a-0b4e6d2c1a90";

const uuid = (seed) => {
  const h = createHash("md5").update(String(seed)).digest("hex");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-4${h.slice(13, 16)}-a${h.slice(17, 20)}-${h.slice(20, 32)}`;
};
const sha = (seed) => createHash("sha256").update(String(seed)).digest("hex");
const rgb = (hex) => ({ r: parseInt(hex.slice(1, 3), 16) / 255, g: parseInt(hex.slice(3, 5), 16) / 255, b: parseInt(hex.slice(5, 7), 16) / 255 });
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

// ---------------------------------------------------------------- synthetic songs

const SHELVES = {
  main: [
    {
      name: "Songs",
      songs: [
        "Amber Fields", "Copper Sky", "Glass Harbor", "Hollow Pine Road", "Kettle Song", "Lantern Road",
        "Marigold Morning", "Northbound Light", "Open Gate", "Orchard Rain", "Salt and Cedar", "Seven Rivers",
        "The Quiet Orchard", "Twelve Small Stones", "Willow Steps", "Wooden Bridge",
      ],
    },
    { name: "Christmas", songs: ["Candle in the Window", "Juniper Street Snow", "Starlit Stable Lane", "Winter Lanterns", "Woolen Mittens Carol", "Frost on the Bell Tower"] },
    { name: "Kids", songs: ["Busy Bee Clap", "Hilltop Hello", "Jumping Puddles", "Pinecone Parade", "Rainbow Kites", "Tiny Boat Big Sea"] },
  ],
  youth: [{ name: "Songs", songs: ["Midnight Skatepark", "Neon Lighthouse", "Paper Planes Again", "Static and Sunrise"] }],
};

const WORDS = {
  a: ["morning", "river", "lantern", "harbor", "meadow", "cedar", "willow", "ember", "valley", "orchard", "candle", "window"],
  b: ["bright", "quiet", "golden", "steady", "gentle", "open", "early", "silver", "patient", "faithful"],
  v: ["carry", "gather", "follow", "lift", "hold", "light", "welcome", "greet", "remember", "sing to"],
};
const pick = (list, n) => list[n % list.length];

function lyricLine(seed, i) {
  const n = parseInt(sha(`${seed}:${i}`).slice(0, 6), 16);
  const forms = [
    () => `We ${pick(WORDS.v, n)} the ${pick(WORDS.b, n >> 3)} ${pick(WORDS.a, n >> 6)}`,
    () => `Over the ${pick(WORDS.a, n >> 2)}, ${pick(WORDS.b, n >> 5)} and slow`,
    () => `${pick(WORDS.b, n >> 4).replace(/^./, (c) => c.toUpperCase())} as the ${pick(WORDS.a, n)} at dawn`,
    () => `Every ${pick(WORDS.a, n >> 7)} will ${pick(WORDS.v, n >> 1)} us home`,
  ];
  return forms[n % forms.length]();
}

const GROUPS = [
  ["Verse 1", "#2F5FB8"],
  ["Chorus", "#1F9E8E"],
  ["Verse 2", "#2F5FB8"],
  ["Bridge", "#B8456B"],
  ["Ending", "#7A5BD6"],
];

function textElement(seed, lines) {
  return {
    id: uuid(`${seed}:el`),
    name: "Text",
    bounds: { x: 96, y: 54, width: 1728, height: 972 },
    kind: {
      type: "text",
      text: {
        paragraphs: lines.map((t) => ({ runs: [{ text: t }] })),
        defaultStyle: { fontFamily: "Inter", fontWeight: 600, size: 80, color: { r: 1, g: 1, b: 1 } },
        defaultParagraph: { alignment: "center" },
      },
    },
  };
}

function makeSong(collection, library, title, k) {
  const id = uuid(`song:${collection}:${title}`);
  const nGroups = 3 + (k % 3);
  const sections = GROUPS.slice(0, nGroups).map(([name, colour], g) => ({
    id: uuid(`${id}:sec:${g}`),
    name,
    color: rgb(colour),
    slides: Array.from({ length: g === 1 ? 2 : 1 + ((k + g) % 2) }, (_, s) => ({
      id: uuid(`${id}:slide:${g}:${s}`),
      elements: [textElement(`${id}:${g}:${s}`, [lyricLine(id, g * 10 + s * 2), lyricLine(id, g * 10 + s * 2 + 1)])],
    })),
  }));
  const day = String(1 + (k % 27)).padStart(2, "0");
  // Made-up credits; every fifth song has none (the columns stay empty).
  const WRITERS = ["P. Ashdown", "K. Moreland and E. Thistle", "N. Okafor-Hale", "B. Vance"];
  const songInfo = k % 5 === 4 ? undefined : { authors: WRITERS[k % WRITERS.length], copyrightYear: 1984 + ((k * 5) % 38), ccliNumber: String(8100200 + k * 41) };
  const doc = {
    schemaVersion: 2,
    id,
    name: title,
    library,
    ...(songInfo ? { songInfo } : {}),
    sections,
    destination: "slide",
    modifiedAt: `2026-10-${day}T10:00:00Z`,
  };
  const summary = {
    id,
    name: title,
    library,
    slideCount: sections.reduce((n, s) => n + s.slides.length, 0),
    destination: "slide",
    modifiedAt: doc.modifiedAt,
    kind: songInfo ? "song" : "text",
    ...(songInfo ?? {}),
  };
  return { doc, summary };
}

function generated(key, collection) {
  const libraries = [];
  const songs = [];
  let k = 0;
  SHELVES[key].forEach((shelf, i) => {
    const lib = uuid(`shelf:${collection}:${shelf.name}`);
    libraries.push({ id: lib, name: shelf.name, sortKey: String.fromCharCode(77 + i), presentationCount: shelf.songs.length });
    for (const t of shelf.songs) songs.push(makeSong(collection, lib, t, k++));
  });
  return { libraries, songs };
}

// ---------------------------------------------------------------- media, themes, props

const MEDIA_MAIN = [
  ["Dusk over the Ridge", "image", "#3B2A6E", "#E08A4F"],
  ["Morning Fog", "image", "#5A6B7A", "#C9D3DB"],
  ["Warm Bokeh Lights", "image", "#4A2A1A", "#F5C26B"],
  ["Pine Silhouettes", "image", "#122A22", "#5FA37E"],
  ["Calm Lake Blue", "image", "#10284A", "#6FA8DC"],
  ["Paper Texture", "image", "#6B5A45", "#D8C8AA", 0, "missing"],
  ["Harvest Window", "image", "#5A3A1A", "#E8B26B", 0, "missing"],
  ["Slow Waves", "video", "#0E3A4A", "#4FB4C8", 30],
  ["Light Particles", "video", "#1A1440", "#A99BFF", 24],
  ["Drifting Clouds", "video", "#2A3E5C", "#B9CCE6", 40],
  ["Kids Confetti Loop", "video", "#4A1A3A", "#FFB86B", 18, "missing"],
];

function makeMedia(collection, list) {
  return list.map(([name, kind, c1, c2, duration, availability], i) => {
    const id = uuid(`asset:${collection}:${name}`);
    const ext = kind === "video" ? "mp4" : "jpg";
    const hash = sha(`blob:${collection}:${name}`);
    return {
      asset: {
        schemaVersion: 2,
        id,
        // The server leaves `kind` out for images (the default), as here.
        ...(kind === "image" ? {} : { kind }),
        name: `${name}.${ext}`,
        location: { type: "library", id: uuid(`item:${hash}`), sha256: hash },
        status: availability ?? "online",
        probe: { width: 1920, height: 1080, ...(duration ? { duration, frameRate: { num: 30, den: 1 }, codec: "h264", hasAudio: false } : { codec: "mjpeg" }) },
        ...(availability === "missing" ? { origin: { platform: "windows", absoluteHint: `D:\\Media\\Backgrounds\\${name}.${ext}`, relativePath: `Backgrounds/${name}.${ext}` } } : {}),
      },
      revision: hash.slice(0, 16),
      availability: availability ?? "online",
      ...(availability === "missing" ? { problem: "The file isn't in the media library any more" } : {}),
      colours: [c1, c2, i],
    };
  });
}

function boothAMedia(fx) {
  const out = [];
  for (const [key, bin] of fx) {
    if (!key.startsWith("media-bins__") || bin?.browser !== "media") continue;
    for (const it of bin.items ?? []) {
      if (out.some((m) => m.asset.id === it.asset)) continue;
      const hash = sha(`blob:booth:${it.asset}`);
      const ext = it.kind === "video" ? "mp4" : "jpg";
      out.push({
        asset: {
          schemaVersion: 2,
          id: it.asset,
          ...(it.kind === "image" ? {} : { kind: it.kind }),
          name: `${it.name}.${ext}`,
          location: { type: "library", id: uuid(`item:${hash}`), sha256: hash },
          status: it.availability,
          probe: { width: 1920, height: 1080, ...(it.duration ? { duration: it.duration, codec: "h264" } : {}) },
        },
        revision: hash.slice(0, 16),
        availability: it.availability,
        colours: ["#1E2A55", "#5B8DEF", out.length],
      });
    }
  }
  return out;
}

/** A theme's template slide: a background colour and one text slot (lyrics, or a passage and
 * a reference), plus a thin accent bar. */
function templateSlide(themeId, name, bg, accent) {
  const id = uuid(`${themeId}:${name}`);
  const slot = (key, slotName, bounds, size, align) => ({
    id: uuid(`${id}:${key}`),
    name: slotName,
    bounds,
    placeholder: { slot: key },
    themed: true,
    kind: {
      type: "text",
      text: {
        defaultStyle: { fontFamily: "Inter", fontWeight: 600, size, color: { r: 1, g: 1, b: 1 } },
        defaultParagraph: { alignment: align },
        verticalAlign: "middle",
        fit: "shrinkToFit",
      },
    },
  });
  const bar = { id: uuid(`${id}:bar`), name: "Accent bar", bounds: { x: 0, y: 1040, width: 1920, height: 40 }, themed: true, fill: { enabled: true, kind: { type: "color", color: accent } } };
  const scripture = /passage|scripture/i.test(name);
  const elements = scripture
    ? [bar, slot("scripture", "Passage", { x: 120, y: 90, width: 1680, height: 760 }, 64, "left"), slot("reference", "Reference", { x: 120, y: 880, width: 1680, height: 120 }, 44, "right")]
    : [bar, slot("lyrics", "Lyrics", { x: 96, y: 54, width: 1728, height: 972 }, /lower|two/i.test(name) ? 54 : 80, "center")];
  return { id: uuid(`tpl:${id}`), name, slide: { id, background: bg, elements } };
}

const THEME_COLOURS = [
  [{ r: 0.08, g: 0.07, b: 0.18 }, { r: 0.55, g: 0.42, b: 1 }],
  [{ r: 0.02, g: 0.12, b: 0.14 }, { r: 0.18, g: 0.72, b: 0.62 }],
  [{ r: 0.12, g: 0.08, b: 0.04 }, { r: 0.96, g: 0.65, b: 0.14 }],
  [{ r: 0.14, g: 0.03, b: 0.08 }, { r: 0.95, g: 0.36, b: 0.5 }],
];

/** The collection's themes as documents (`GET /themes/{id}`, the editor's document). */
function themeDocs(collection, names) {
  return names.map(([name, slides], i) => {
    const id = uuid(`theme:${collection}:${name}`);
    const [bg, accent] = THEME_COLOURS[i % THEME_COLOURS.length];
    return { schemaVersion: 2, id, name, canvasSize: { width: 1920, height: 1080 }, slides: slides.map((s) => templateSlide(id, s, bg, accent)) };
  });
}

/** `GET /themes`: the summaries, with how many songs follow each. */
const themeSummaries = (c) =>
  (c.themeDocs ?? []).map((t) => ({
    id: t.id,
    name: t.name,
    canvasSize: t.canvasSize,
    slides: t.slides.map((s) => ({ id: s.id, name: s.name, slide: s.slide.id })),
    usedBy: c.songs.filter((x) => x.doc.sections.some((sec) => sec.slides.some((sl) => sl.themeLink && sl.templateOrigin?.theme.id === t.id))).length,
  }));

/** The song follows the theme's first template slide on every slide (its text in the slot);
 * `moved` slides keep a lyrics box of their own (an override). */
function followTheme(song, theme, moved = []) {
  const tpl = theme.slides[0];
  const slotEl = tpl.slide.elements.find((e) => e.placeholder);
  const barEl = tpl.slide.elements.find((e) => !e.placeholder);
  let k = 0;
  for (const sec of song.doc.sections)
    for (const sl of sec.slides) {
      const text = sl.elements.find((e) => e.kind?.type === "text");
      const own = moved.includes(k++);
      const lyrics = {
        ...structuredClone(slotEl),
        id: text?.id ?? uuid(`${sl.id}:lyrics`),
        fromTheme: { element: slotEl.id, ...(own ? { overrides: ["bounds"] } : {}) },
        ...(own ? { bounds: { x: 96, y: 600, width: 1728, height: 420 } } : {}),
      };
      lyrics.kind.text.paragraphs = text?.kind.text.paragraphs ?? [];
      sl.elements = [{ ...structuredClone(barEl), id: uuid(`${sl.id}:bar`), fromTheme: { element: barEl.id } }, lyrics];
      sl.background = tpl.slide.background;
      sl.templateOrigin = { theme: { id: theme.id, name: theme.name }, slide: tpl.id };
      sl.themeLink = {};
    }
  song.summary.theme = theme.id;
  song.summary.themeName = theme.name;
}

function propList(collection, names) {
  return names.map(([name, line], i) => {
    const id = uuid(`prop:${collection}:${name}`);
    return {
      revision: sha(id).slice(0, 16),
      schemaVersion: 2,
      id,
      name,
      sortKey: String.fromCharCode(77 + i),
      slide: { id: uuid(`${id}:slide`), elements: [textElement(`${id}:t`, [line])] },
    };
  });
}

// ---------------------------------------------------------------- the Codex

function initialCodex(fx) {
  const main = uuid("collection:main-hall");
  const youth = uuid("collection:youth-room");
  const mainGen = generated("main", main);
  const youthGen = generated("youth", youth);
  const collections = [
    { id: BOOTH_A, name: "Booth A", color: rgb("#6E5CE6"), sortKey: "M", purpose: "content", createdAt: "2026-09-01T09:00:00Z" },
    { id: main, name: "Main Hall", color: rgb("#4FB487"), sortKey: "N", purpose: "content", createdAt: "2026-09-02T09:00:00Z" },
    { id: youth, name: "Youth Room", color: rgb("#F5A524"), sortKey: "O", purpose: "content", createdAt: "2026-09-03T09:00:00Z" },
  ].map((c) => ({ schemaVersion: 1, ...c }));
  const content = {
    [BOOTH_A]: {
      fixtures: true,
      media: boothAMedia(fx),
      themes: [],
      props: null,
      libraries: fx.get("libraries") ?? [],
      songs: [],
    },
    [main]: {
      libraries: mainGen.libraries,
      songs: mainGen.songs,
      media: makeMedia(main, MEDIA_MAIN),
      themeDocs: themeDocs(main, [
        ["Evening Lyrics", ["Lyrics", "Chorus", "Long verse"]],
        ["Livestream Lower Third", ["Lyrics", "Two lines"]],
        ["Scripture Plain", ["Passage", "Scripture"]],
      ]),
      props: propList(main, [
        ["Welcome Banner", "Welcome! We're glad you're here"],
        ["Speaker Lower Third", "[Speaker name] · [Title]"],
        ["Be Right Back", "We'll be right back"],
      ]),
    },
    [youth]: {
      libraries: youthGen.libraries,
      songs: youthGen.songs,
      media: makeMedia(youth, MEDIA_MAIN.slice(0, 3)),
      themeDocs: themeDocs(youth, [["Youth Bold", ["Lyrics"]]]),
      props: [],
    },
  };
  // Songs that follow a theme (linked themes): the first four of Main Hall follow "Evening
  // Lyrics" (the first one's second slide keeps its own lyrics box), the next two the lower third.
  const mc = content[main];
  mc.songs.slice(0, 4).forEach((x, i) => followTheme(x, mc.themeDocs[0], i === 0 ? [1] : []));
  mc.songs.slice(4, 6).forEach((x) => followTheme(x, mc.themeDocs[1]));
  for (const c of Object.values(content)) c.themes = c.themeDocs ? themeSummaries(c) : c.themes;
  const by = (client) => ({ client, machine: "ef405d09-d621-4bc9-8846-beefcf530b86" });
  const trash = [
    { id: uuid("trash:1"), collection: main, kind: "presentation", docId: uuid("gone:1"), name: "Harvest Table (old arrangement)", deletedAt: "2026-11-22T18:40:00Z", expiresAt: "2026-12-22T18:40:00Z", by: by("Booth PC"), subtree: 0 },
    { id: uuid("trash:2"), collection: main, kind: "playlistNode", docId: uuid("gone:2"), name: "Rehearsal Thursday", deletedAt: "2026-11-20T21:05:00Z", expiresAt: "2026-12-20T21:05:00Z", by: by("Lobby laptop"), subtree: 2 },
    { id: uuid("trash:3"), collection: BOOTH_A, kind: "prop", docId: uuid("gone:3"), name: "Old Logo Bug", deletedAt: "2026-11-02T08:15:00Z", expiresAt: "2026-12-02T08:15:00Z", by: by("Booth PC"), subtree: 0 },
    { id: uuid("trash:4"), collection: main, kind: "timer", docId: uuid("gone:4"), name: "Countdown 3 min", deletedAt: "2026-10-28T10:00:00Z", expiresAt: "2026-11-27T10:00:00Z", by: by("server machine"), subtree: 0 },
  ];
  const camp = uuid("collection:summer-camp");
  const deleted = [
    {
      collection: { schemaVersion: 1, id: camp, name: "Summer Camp 2025", color: rgb("#2FB8C6"), sortKey: "P", purpose: "content", createdAt: "2025-05-10T09:00:00Z", description: "Songs and slides for the camp week" },
      revision: sha(`rev:${camp}`).slice(0, 16),
      counts: { presentations: 14, libraries: 2, playlists: 5, themes: 1, mediaAssets: 9, mediaBins: 1, globalGroups: 4, timers: 1, messages: 1, props: 2, macros: 0, clearGroups: 1, stageLayouts: 1 },
      documentBytes: 260_000,
      mediaBytes: 640_000_000,
      trashItems: 3,
      deletedAt: "2026-11-18T16:20:00Z",
      by: by("Lobby laptop"),
    },
  ];
  return { active: BOOTH_A, collections, content, trash, deleted, uploads: new Map() };
}

let codex = null;

/** The library stub's Codex state (null until its routes are set up), for other areas' routes
 * that add to its documents (`codex-prep.mjs`: media preparation on the media). */
export function stubCodex() {
  return codex;
}

export default function libraryRoutes(ctx) {
  const { fx, state, json, svg, apiError } = ctx;
  if (!codex) codex = initialCodex(fx);
  /** This stub's part: normal, a show box, or the box holding the Codex. */
  let role = { role: "normal" };
  let uploadDelay = 0;
  /** G15: this show box's own, dormant collection and its upload (null: none). */
  let dormant = null;
  const DORMANT_ID = uuid("collection:dormant:own");
  const dormantIdle = () => ({
    state: "idle",
    source: DORMANT_ID,
    name: "Chapel Songs",
    songs: 46,
    documents: { done: 0, total: 212 },
    media: { done: 0, total: 0 },
    bytes: { done: 0, total: 0 },
  });

  const cur = () => codex.content[codex.active];
  const songsOf = (c) => (c.fixtures ? fx.get("presentations") ?? [] : c.songs.map((s) => s.summary));
  const docOf = (id) => {
    for (const c of Object.values(codex.content)) for (const s of c.songs) if (s.doc.id === id) return s.doc;
    return null;
  };
  const propsOf = (c) => (c.props ?? fx.get("props") ?? []);
  const nameOf = (id) => codex.collections.find((c) => c.id === id)?.name ?? "";

  function status() {
    const base = fx.get("codex");
    const docs = Object.values(codex.content).reduce((n, c) => n + songsOf(c).length + c.libraries.length + c.themes.length + propsOf(c).length, 0);
    const s = {
      ...base,
      collection: codex.active,
      collectionName: nameOf(codex.active),
      collections: codex.collections.length,
      documents: docs,
      trash: codex.trash.length,
      deletedCollections: codex.deleted.length,
      mediaLibrary: { available: true, served: true },
    };
    if (role.role !== "showBox") s.boxes = SHOW_BOXES.map((b) => ({ ...b, collection: b.collection === "active" ? codex.active : b.collection }));
    if (role.role === "showBox") {
      const reachable = role.reachable !== false;
      return {
        ...s,
        role: "client",
        ...(dormant ? { dormantCollection: DORMANT_ID } : {}),
        sync: {
          reachable,
          serverId: role.codexBox?.serverId,
          ...(role.codexBox?.name ? { name: role.codexBox.name } : {}),
          lastSync: "2026-11-23T13:58:00Z",
          seq: 412,
          codexSeq: reachable ? 412 : 415,
          behind: reachable ? 0 : 3,
          ...(reachable ? {} : { problem: "The site's Codex can't be reached" }),
          media: { pinned: 14, cached: 14, copying: 0, missing: 0, failed: 0, bytes: 2_400_000_000, capBytes: 34_359_738_368 },
        },
      };
    }
    return s;
  }

  // G14: three made-up show boxes (two on the active collection, one of them behind; one not
  // heard from for an hour, on Youth Room).
  const SHOW_BOXES = [
    { fingerprint: "SHA256:aa11", name: "Chapel Booth", collection: "active", seq: 412, behind: 0, lastSeen: "2026-11-24T09:58:30Z", online: true },
    { fingerprint: "SHA256:bb22", name: "Lobby Display", collection: "active", seq: 409, behind: 3, lastSeen: "2026-11-24T09:58:10Z", online: true },
    { fingerprint: "SHA256:cc33", name: "Youth Room PC", collection: uuid("collection:youth-room"), seq: 120, behind: 0, lastSeen: "2026-11-24T08:51:00Z", online: false },
  ];
  function boxesOf(id) {
    const on = SHOW_BOXES.map((b) => ({ ...b, collection: b.collection === "active" ? codex.active : b.collection })).filter((b) => b.online && b.collection === id);
    const self = id === codex.active ? 1 : 0;
    return { showing: on.length + self, synced: on.filter((b) => b.behind === 0).length + self };
  }

  function summary(c) {
    const content = codex.content[c.id];
    const songs = songsOf(content);
    return {
      collection: c,
      revision: sha(JSON.stringify(c)).slice(0, 16),
      active: c.id === codex.active,
      readOnly: false,
      counts: {
        presentations: songs.length,
        libraries: content.libraries.length,
        playlists: content.fixtures ? 3 : 2,
        themes: content.themes.length,
        mediaAssets: content.media.length,
        mediaBins: 3,
        globalGroups: 4,
        timers: 2,
        messages: 2,
        props: propsOf(content).length,
        macros: 0,
        clearGroups: 1,
        stageLayouts: 2,
      },
      documentBytes: 180_000 + songs.length * 6_000,
      mediaBytes: content.media.length * 48_000_000,
      // G14: the Vespers showing it (this box when active, and the made-up show boxes below).
      boxes: role.role === "showBox" ? { showing: 0, synced: 0 } : boxesOf(c.id),
    };
  }

  function search(query) {
    const q = (query.get("q") ?? "").trim();
    if (!q) return { status: 400, body: { error: { code: "badRequest", message: "q is missing", field: "q" } } };
    const scope = query.get("collection");
    const kinds = (query.get("kinds") ?? "").split(",").filter(Boolean);
    const words = q.toLowerCase().replace(/"/g, "").split(/\s+/).filter(Boolean);
    const starts = (text) => {
      const hay = text.toLowerCase().split(/[^\p{L}\p{N}']+/u);
      return words.every((w) => hay.some((h) => h.startsWith(w)));
    };
    const mark = (text) => {
      const parts = [];
      for (const piece of text.split(/(\s+)/)) {
        const hit = words.some((w) => piece.toLowerCase().replace(/^[^\p{L}\p{N}]+/u, "").startsWith(w));
        parts.push(hit ? { text: piece, mark: true } : { text: piece });
      }
      return parts;
    };
    const ids = scope === "all" ? codex.collections.map((c) => c.id) : [scope && scope !== "" ? scope : codex.active];
    const hits = [];
    for (const cid of ids) {
      const c = codex.content[cid];
      if (!c) return { status: 404, body: { error: { code: "notFound", message: "no such collection" } } };
      const base = { collection: cid, collectionName: nameOf(cid) };
      if (!kinds.length || kinds.includes("presentation")) {
        for (const p of songsOf(c)) {
          if (starts(p.name)) {
            hits.push({ ...base, kind: "presentation", id: p.id, title: p.name, field: "name", snippet: mark(p.name), rank: 0 });
            continue;
          }
          const doc = c.fixtures ? fx.get(`presentations__${p.id}`) : docOf(p.id);
          let index = 0;
          let found = null;
          for (const sec of doc?.sections ?? []) {
            for (const sl of sec.slides ?? []) {
              const text = (sl.elements ?? []).flatMap((e) => e.kind?.text?.paragraphs ?? []).map((pp) => pp.runs.map((r) => r.text).join("")).join(" / ");
              if (!found && starts(text)) found = { text, index, slide: sl.id, section: sec.id };
              index++;
            }
          }
          if (found) hits.push({ ...base, kind: "presentation", id: p.id, title: p.name, field: "text", snippet: mark(found.text), slideIndex: found.index, slide: found.slide, section: found.section, rank: 1 });
        }
      }
      const named = [
        ["theme", c.themes.map((t) => [t.id, t.name])],
        ["prop", propsOf(c).map((p) => [p.id, p.name ?? ""])],
        ["mediaAsset", c.media.map((m) => [m.asset.id, m.asset.name ?? ""])],
      ];
      for (const [kind, list] of named) {
        if (kinds.length && !kinds.includes(kind)) continue;
        for (const [id, title] of list) if (starts(title)) hits.push({ ...base, kind, id, title, field: "name", snippet: mark(title), rank: 0 });
      }
    }
    hits.sort((a, b) => a.rank - b.rank || a.title.localeCompare(b.title));
    const limit = Number(query.get("limit")) || 50;
    return { status: 200, body: { hits: hits.slice(0, limit).map(({ rank: _r, ...h }) => h), total: hits.length } };
  }

  function mediaThumb(hash, w) {
    for (const c of Object.values(codex.content)) {
      const m = c.media.find((x) => x.asset.location?.sha256 === hash);
      if (!m) continue;
      const [c1, c2, i] = m.colours;
      const h = Math.round((w * 9) / 16);
      const name = (m.asset.name ?? "").replace(/\.[^.]+$/, "");
      const shapes =
        m.asset.kind === "video"
          ? `<path d="M0 ${h * 0.7} Q ${w * 0.25} ${h * (0.55 + (i % 3) * 0.05)} ${w * 0.5} ${h * 0.7} T ${w} ${h * 0.68} V ${h} H 0 Z" fill="${c2}" opacity="0.35"/>`
          : `<circle cx="${w * (0.3 + (i % 4) * 0.12)}" cy="${h * 0.38}" r="${h * 0.16}" fill="${c2}" opacity="0.55"/>`;
      return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${c1}"/><stop offset="1" stop-color="${c2}"/></linearGradient></defs><rect width="${w}" height="${h}" fill="url(#g)"/>${shapes}<text x="${w * 0.05}" y="${h * 0.9}" fill="#FFFFFF" fill-opacity="0.75" font-family="Inter, 'DejaVu Sans', sans-serif" font-size="${Math.max(9, Math.round(w / 26))}" font-weight="600">${esc(name)}</text></svg>`;
    }
    return null;
  }

  function slideSvgFor(lines, colour, w, transparent) {
    const h = Math.round((w * 9) / 16);
    const size = Math.max(10, Math.round(w / 22));
    const gap = size * 1.3;
    const top = h / 2 - ((lines.length - 1) * gap) / 2 + size * 0.35;
    const text = lines.map((l, i) => `<text x="${w / 2}" y="${(top + i * gap).toFixed(1)}" text-anchor="middle">${esc(l)}</text>`).join("");
    const bg = transparent
      ? ""
      : `<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${colour}" stop-opacity="0.55"/><stop offset="0.6" stop-color="#121317"/><stop offset="1" stop-color="#0B0C0F"/></linearGradient></defs><rect width="${w}" height="${h}" fill="url(#g)"/>`;
    return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">${bg}<g fill="#FFFFFF" font-family="Inter, 'DejaVu Sans', sans-serif" font-size="${size}" font-weight="600">${text}</g></svg>`;
  }

  const lines = (slide) => (slide?.elements ?? []).flatMap((e) => e.kind?.text?.paragraphs ?? []).map((p) => p.runs.map((r) => r.text).join(""));
  const hexOf = (c) => (c ? "#" + [c.r, c.g, c.b].map((v) => Math.round(v * 255).toString(16).padStart(2, "0")).join("") : "#3A3D46");

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

  const isLive = () => {
    const l = state.state.layers;
    return !!(l.slide || l.announcements || l.media || l.foreground || l.audio || l.props.length || l.messages.length);
  };

  const elsewhere = (res) =>
    json(res, 409, {
      error: { code: "codexElsewhere", message: `Codex management goes to the box that holds the site's Codex (${role.codexBox?.name ?? "unknown"})` },
      codex: role.codexBox,
    });

  bibleStub.resplit = (id) => {
    const song = cur().songs.find((x) => x.doc.id === id);
    const g = song?.doc.scripture?.generated;
    if (!g) return null;
    delete g.stale;
    return { presentation: id, slideCount: song.doc.sections.reduce((n, s) => n + s.slides.length, 0), changed: true };
  };
  return function handle(req, res, { path, parts, query }) {
    const m = req.method;
    const w = Math.min(1920, Math.max(16, Number(query.get("w")) || 320));

    if (path === "/__stub/library" && m === "POST") {
      readBody(req, (b) => {
        if (b.reset) codex = initialCodex(fx);
        if (b.role) role = { role: b.role, codexBox: b.codexBox ?? null, reachable: b.reachable, name: b.name };
        if (typeof b.uploadDelay === "number") uploadDelay = b.uploadDelay;
        if (b.reset || b.role === "normal") dormant = null;
        if (b.dormant === true) dormant = dormantIdle();
        if (b.dormant === false) dormant = null;
        if (b.dormantFinish && dormant)
          dormant = { ...dormant, state: "done", documents: { ...dormant.documents, done: dormant.documents.total }, media: { done: 9, total: 9 }, bytes: { done: 1_840_000_000, total: 1_840_000_000 }, finishedAt: "2026-11-24T10:04:00Z" };
        if (b.stalePassage) {
          // A song stands in for a passage made on the Bible page whose theme changed after it
          // was edited (batch 9): `scripture.generated.stale`.
          const song = cur().songs.find((x) => x.doc.name === b.stalePassage);
          if (song) song.doc.scripture = { book: { name: "John", key: "JHN" }, chapters: { start: 1, end: 1 }, label: "John 1:1–5 (TST)", generated: { translation: "TST", reference: "John 1:1-5", slides: "edited", stale: true } };
        }
        json(res, 200, { ok: true, role: role.role, active: codex.active });
      });
      return true;
    }
    if (role.role === "codexBox" && path === "/server" && m === "GET") {
      json(res, 200, { ...fx.get("server"), id: CODEX_BOX_ID, name: role.name ?? "Codex Hub" });
      return true;
    }

    // ---- the Codex
    if (parts[0] === "codex") {
      if (path === "/codex" && m === "GET") return json(res, 200, status()), true;
      if (path === "/codex/collections" && m === "GET") return json(res, 200, codex.collections.map(summary)), true;
      if (path === "/codex/dormant/upload") {
        if (!dormant || role.role !== "showBox") return apiError(res, 404, "notFound", "this box has no dormant collection of its own"), true;
        if (m === "GET") return json(res, 200, dormant), true;
        if (m === "DELETE") {
          if (dormant.state === "copying") dormant = { ...dormant, state: "cancelled", finishedAt: "2026-11-24T10:02:00Z" };
          res.writeHead(204);
          res.end();
          return true;
        }
        if (m === "POST") {
          // Half way: the documents are in, the third of nine files is on its way.
          dormant = {
            ...dormant,
            state: "copying",
            collection: uuid("collection:uploaded:chapel"),
            collectionName: "Chapel Songs (from Chapel Booth)",
            documents: { ...dormant.documents, done: dormant.documents.total },
            media: { done: 2, total: 9 },
            bytes: { done: 612_000_000, total: 1_840_000_000 },
            startedAt: "2026-11-24T10:00:00Z",
          };
          return json(res, 202, dormant), true;
        }
      }
      if (path === "/codex/collection-trash" && m === "GET") return json(res, 200, codex.deleted), true;
      if (["collections", "collection-trash"].includes(parts[1]) && m !== "GET" && role.role === "showBox") return elsewhere(res), true;
      if (path === "/codex/collections" && m === "POST") {
        readBody(req, (b) => {
          const name = String(b.name ?? "").trim();
          if (!name || name.length > 200) return json(res, 400, { error: { code: "badRequest", message: "A collection needs a name (1–200 characters)", field: "name" } });
          const c = { schemaVersion: 1, id: uuid(`collection:new:${name}:${Date.now()}`), name, sortKey: "Z", purpose: "content", createdAt: new Date().toISOString(), ...(b.color ? { color: b.color } : {}), ...(b.description ? { description: b.description } : {}) };
          codex.collections.push(c);
          codex.content[c.id] = { libraries: [], songs: [], media: [], themes: [], props: [] };
          json(res, 201, summary(c));
        });
        return true;
      }
      if (parts[1] === "collections" && parts.length === 3 && (m === "PUT" || m === "DELETE")) {
        const c = codex.collections.find((x) => x.id === parts[2]);
        if (!c) return apiError(res, 404, "notFound", "There is no such collection"), true;
        if (m === "DELETE") {
          if (c.id === codex.active) return apiError(res, 409, "conflict", "This box shows that collection: assign another one first"), true;
          codex.collections = codex.collections.filter((x) => x !== c);
          const sm = summary({ ...c });
          codex.deleted.unshift({ collection: c, revision: sm.revision, counts: sm.counts, documentBytes: sm.documentBytes, mediaBytes: sm.mediaBytes, trashItems: codex.trash.filter((t) => t.collection === c.id).length, deletedAt: new Date().toISOString(), by: { client: "Oriel (stub)", machine: "stub" } });
          res.writeHead(204);
          res.end();
          return true;
        }
        readBody(req, (b) => {
          if (b.name !== undefined) c.name = String(b.name);
          if (b.description !== undefined) b.description === null ? delete c.description : (c.description = b.description);
          if (b.color !== undefined) b.color === null ? delete c.color : (c.color = b.color);
          json(res, 200, summary(c));
        });
        return true;
      }
      if (parts[1] === "collection-trash" && parts.length >= 3 && m !== "GET") {
        const d = codex.deleted.find((x) => x.collection.id === parts[2]);
        if (!d) return apiError(res, 404, "notFound", "That collection is not in the collection trash"), true;
        codex.deleted = codex.deleted.filter((x) => x !== d);
        if (parts[3] === "restore" && m === "POST") {
          codex.collections.push(d.collection);
          codex.content[d.collection.id] ??= { libraries: [], songs: [], media: [], themes: [], props: [] };
          return json(res, 200, summary(d.collection)), true;
        }
        res.writeHead(204);
        res.end();
        return true;
      }
      // Uploads: the bytes are counted, not kept.
      if (path === "/codex/uploads" && m === "POST") {
        if (role.role === "showBox") return elsewhere(res), true;
        readBody(req, (b) => {
          if (!/\.(jpe?g|png|webp|gif|mp4|mov|webm|mp3|wav)$/i.test(b.name ?? "")) return json(res, 400, { error: { code: "badRequest", message: "This kind of file can't go into the media library", field: "name" } });
          const now = new Date();
          const u = { id: uuid(`upload:${b.name}:${now.getTime()}:${Math.random()}`), collection: b.collection, name: b.name, size: b.size, offset: 0, bin: b.bin, replaces: b.replaces, relativePath: b.relativePath, createdAt: now.toISOString(), updatedAt: now.toISOString(), expiresAt: new Date(now.getTime() + 86_400_000).toISOString() };
          codex.uploads.set(u.id, u);
          const { replaces: _r, relativePath: _p, ...info } = u;
          json(res, 201, info);
        });
        return true;
      }
      if (path === "/codex/uploads" && m === "GET") return json(res, 200, [...codex.uploads.values()]), true;
      if (parts[1] === "uploads" && parts[2]) {
        const u = codex.uploads.get(parts[2]);
        if (!u) return apiError(res, 404, "notFound", "There is no such upload"), true;
        const info = () => {
          const { replaces: _r, relativePath: _p, ...i } = u;
          return i;
        };
        if (parts.length === 3 && m === "GET") return json(res, 200, info()), true;
        if (parts.length === 3 && m === "DELETE") {
          codex.uploads.delete(u.id);
          res.writeHead(204);
          res.end();
          return true;
        }
        if (parts.length === 3 && m === "PUT") {
          const range = /^bytes (\d+)-(\d+)\/(\d+)$/.exec(req.headers["content-range"] ?? "");
          let got = 0;
          req.on("data", (d) => (got += d.length));
          req.on("end", () =>
            setTimeout(() => {
              if (!range) return json(res, 400, { error: { code: "badRequest", message: "Content-Range is missing" } });
              if (Number(range[1]) !== u.offset) return json(res, 409, { error: { code: "conflict", message: "The chunk doesn't start at the upload's offset" }, offset: u.offset });
              u.offset += got;
              u.updatedAt = new Date().toISOString();
              json(res, 200, info());
            }, uploadDelay),
          );
          return true;
        }
        if (parts[3] === "finish" && m === "POST") {
          readBody(req, (b) => {
            if (u.offset < u.size) return json(res, 409, { error: { code: "conflict", message: "Bytes are missing" }, offset: u.offset });
            codex.uploads.delete(u.id);
            const c = codex.content[u.collection];
            const hash = b.sha256 ?? sha(u.id);
            if (u.replaces) {
              const a = c?.media.find((x) => x.asset.id === u.replaces);
              if (!a) return apiError(res, 404, "notFound", "There is no such media asset");
              a.availability = "online";
              a.asset.status = "online";
              delete a.problem;
              return json(res, 201, { collection: u.collection, asset: a.asset, revision: sha(`${hash}:r`).slice(0, 16), deduplicated: false, relinked: true });
            }
            // The same file again: the collection's asset for it, no second one (`existing`).
            const had = c?.media.find((x) => x.asset.location?.sha256 === hash);
            if (had) return json(res, 201, { collection: u.collection, asset: had.asset, revision: had.revision, deduplicated: true, existing: true, ...(u.bin ? { binItem: uuid(`bin-item:${u.id}`) } : {}) });
            const video = /\.(mp4|mov|webm)$/i.test(u.name);
            const [entry] = makeMedia(u.collection, [[u.name.replace(/\.[^.]+$/, ""), video ? "video" : "image", "#22324A", "#7AA7F0", video ? 20 : 0]]);
            if (b.sha256) entry.asset.location.sha256 = b.sha256;
            c?.media.push(entry);
            json(res, 201, { collection: u.collection, asset: entry.asset, revision: entry.revision, deduplicated: false, ...(u.bin ? { binItem: uuid(`bin-item:${u.id}`) } : {}) });
          });
          return true;
        }
      }
      // Missing media: relink, search, folder match.
      if (parts[1] === "collections" && parts[3] === "media" && m === "POST") {
        const c = codex.content[parts[2]];
        if (!c) return apiError(res, 404, "notFound", "no such collection"), true;
        readBody(req, (b) => {
          const missing = c.media.filter((x) => x.availability === "missing" && (!b.assets || b.assets.includes(x.asset.id)));
          if (parts[5] === "relink") {
            const a = c.media.find((x) => x.asset.id === parts[4]);
            if (!a) return apiError(res, 404, "notFound", "There is no such media asset");
            const src = c.media.find((x) => x.asset.location?.type === "library" && x.asset.location.id === b.id);
            a.availability = "online";
            a.asset.status = "online";
            if (src) {
              a.asset.location = { ...src.asset.location };
              a.colours = src.colours;
            }
            delete a.problem;
            return json(res, 200, { asset: a.asset, revision: sha(`${a.asset.id}:relinked`).slice(0, 16) });
          }
          if (parts[4] === "search") return json(res, 200, { checked: missing.length, relinked: [], ambiguous: [], notFound: missing.map((x) => x.asset.id), statusChanged: 0 });
          if (parts[4] === "match") {
            const files = b.files ?? [];
            const matches = [];
            const unmatched = [];
            for (const a of missing) {
              const f = files.find((x) => x.name === a.asset.name);
              if (f) matches.push({ asset: a.asset.id, name: a.asset.name, file: f, by: f.relativePath ? "relativePath" : "fileName" });
              else unmatched.push(a.asset.id);
            }
            return json(res, 200, { matches, ambiguous: [], unmatched });
          }
          apiError(res, 404, "notFound", "no such route");
        });
        return true;
      }
      if (path === "/codex/assignment" && m === "PUT") {
        readBody(req, (b) => {
          if (!codex.content[b.collection]) return json(res, 400, { error: { code: "badRequest", message: "There is no such collection", field: "collection" } });
          if (isLive() && !b.confirm && b.collection !== codex.active) {
            const s = state.state.layers.slide;
            return apiError(res, 409, "liveShow", s ? `A slide of “${s.presentationName}” is live` : "Something is live");
          }
          codex.active = b.collection;
          json(res, 200, status());
        });
        return true;
      }
      if (path === "/codex/trash") {
        if (m === "GET") return json(res, 200, codex.trash.filter((t) => codex.collections.some((c) => c.id === t.collection))), true;
        if (m === "DELETE") {
          if (role.role === "showBox") return elsewhere(res), true;
          const purged = codex.trash.length;
          codex.trash = [];
          return json(res, 200, { purged }), true;
        }
      }
      if (parts[1] === "trash" && parts[3] === "restore" && m === "POST") {
        const t = codex.trash.find((x) => x.id === parts[2]);
        if (!t) return apiError(res, 404, "notFound", "That item is no longer in the trash"), true;
        codex.trash = codex.trash.filter((x) => x !== t);
        return json(res, 200, { collection: t.collection, kind: t.kind, documents: [t.docId] }), true;
      }
      if (parts[1] === "collections" && parts[3] === "media" && parts.length === 4 && m === "GET") {
        const c = codex.content[parts[2]];
        if (!c) return apiError(res, 404, "notFound", "no such collection"), true;
        const want = (query.get("status") ?? "all").split(",");
        const list = c.media
          .filter((x) => want.includes("all") || want.includes(x.availability))
          .map(({ colours: _c, ...x }) => x)
          .sort((a, b) => (a.asset.name ?? "").localeCompare(b.asset.name ?? ""));
        return json(res, 200, list), true;
      }
      if (path === "/codex/search" && m === "GET") {
        const r = search(query);
        return json(res, r.status, r.body), true;
      }
      if (path === "/codex/storage" && m === "GET") {
        return json(res, 200, {
          disk: { total: 2_000_000_000_000, available: 1_400_000_000_000, keepFree: 100_000_000_000 },
          collections: codex.collections.map((c) => {
            const media = codex.content[c.id].media;
            return { collection: c.id, name: c.name, documents: 40, documentBytes: 300_000, mediaBytes: media.length * 48_000_000, uniqueMediaBytes: media.length * 30_000_000, assets: media.length, missing: media.filter((x) => x.availability === "missing").length };
          }),
          library: { bytes: 1_200_000_000, items: 22, trashItems: 1, trashDays: 30 },
          media: { files: 22, bytes: 1_200_000_000 },
          shared: { files: 3, bytes: 120_000_000 },
          unreferenced: { files: 1, bytes: 40_000_000 },
          managed: { files: 0, bytes: 0 },
          trash: { items: codex.trash.length, itemBytes: 80_000, collections: codex.deleted.length, collectionBytes: codex.deleted.reduce((n, d) => n + d.documentBytes, 0), mediaBytes: 0 },
          thumbs: { files: 22, bytes: 2_000_000 },
          uploads: { files: 0, bytes: 0 },
        }), true;
      }
      // The media library's items (every collection's library files, once each).
      if (path === "/codex/library" && m === "GET") {
        if (role.role === "showBox") return elsewhere(res), true;
        const kind = query.get("kind");
        const q = (query.get("q") ?? "").toLowerCase().split(/\s+/).filter(Boolean);
        const seen = new Map();
        for (const c of Object.values(codex.content))
          for (const x of c.media) {
            const loc = x.asset.location;
            if (loc?.type !== "library" || x.availability === "missing" || seen.has(loc.id)) continue;
            const k = x.asset.kind === "video" ? "video" : x.asset.kind === "audio" ? "sound" : "picture";
            const folder = k === "video" ? "Motion" : "Backgrounds";
            const name = x.asset.name ?? "";
            seen.set(loc.id, { id: loc.id, path: `${folder}/${name}`, title: name.replace(/\.[^.]+$/, ""), kind: k, size: k === "video" ? 48_000_000 : 2_400_000, sha256: loc.sha256 });
          }
        const items = [...seen.values()]
          .filter((i) => (!kind || i.kind === kind) && q.every((w) => `${i.path} ${i.title}`.toLowerCase().includes(w)))
          .sort((a, b) => a.path.localeCompare(b.path));
        const limit = Number(query.get("limit") ?? 500);
        return json(res, 200, { items: items.slice(0, limit), total: items.length }), true;
      }
      if (parts[1] === "blobs" && parts[3] === "thumbnail") {
        const body = mediaThumb((parts[2] ?? "").replace(/\.[^.]+$/, ""), w);
        if (body) return svg(res, body), true;
      }
      return false;
    }

    // ---- "Used by" (synthetic: the collection's first songs and a playlist, by the ID)
    if ((parts[0] === "media" || parts[0] === "props") && parts[2] === "used-by" && m === "GET") {
      const songs = songsOf(cur());
      const n = parseInt(createHash("md5").update(parts[1]).digest("hex").slice(0, 2), 16) % 3;
      const uses = songs.slice(0, n + 1).map((x) => ({ kind: "presentation", id: x.id, name: x.name }));
      if (parts[0] === "media" && n > 0) uses.push({ kind: "playlistNode", id: uuid(`pl:${parts[1]}`), name: "Sunday Morning" });
      if (parts[0] === "media" && n === 2) uses.push({ kind: "prop", id: uuid(`pr:${parts[1]}`), name: "Corner logo" });
      return json(res, 200, uses), true;
    }

    // ---- themes (linked themes): documents, renders, used-by, and the writes in memory
    if (parts[0] === "themes" && !cur().fixtures) {
      const c = cur();
      const docs = (c.themeDocs ??= []);
      const refresh = () => (c.themes = themeSummaries(c));
      const t = parts[1] ? docs.find((x) => x.id === parts[1]) : null;
      if (parts.length === 1 && m === "POST") {
        readBody(req, (b) => {
          const taken = (n) => docs.some((x) => x.name.toLowerCase() === n.toLowerCase());
          let name = String(b.name ?? "").trim() || "Theme";
          for (let i = 2; taken(name); i++) name = `${String(b.name ?? "").trim() || "Theme"} ${i}`;
          const [doc] = themeDocs(`${codex.active}:${Date.now()}`, [[name, ["Lyrics", "Title", "Scripture"]]]);
          docs.push(doc);
          refresh();
          json(res, 201, { kind: "theme", id: doc.id, revision: sha(doc.id).slice(0, 16) });
        });
        return true;
      }
      if (parts[1] && !t) return apiError(res, 404, "notFound", `theme ${parts[1]} not found`), true;
      if (t && parts.length === 2 && m === "GET") return json(res, 200, { theme: t, revision: sha(JSON.stringify(t)).slice(0, 16) }), true;
      if (t && parts.length === 2 && m === "PUT") {
        readBody(req, (b) => {
          t.name = String(b.name ?? t.name).trim() || t.name;
          if (b.secondLanguage) t.secondLanguage = b.secondLanguage;
          refresh();
          json(res, 200, { kind: "theme", id: t.id, revision: sha(JSON.stringify(t)).slice(0, 16) });
        });
        return true;
      }
      if (t && parts[2] === "duplicate" && m === "POST") {
        const copy = structuredClone(t);
        copy.id = uuid(`theme:copy:${t.id}:${Date.now()}`);
        copy.name = `${t.name} copy`;
        docs.push(copy);
        refresh();
        return json(res, 201, { kind: "theme", id: copy.id, revision: sha(copy.id).slice(0, 16) }), true;
      }
      if (t && parts.length === 2 && m === "DELETE") {
        const users = c.songs.filter((x) => x.summary.theme === t.id);
        const to = query.get("reassign") ? docs.find((x) => x.id === query.get("reassign")) : null;
        if (users.length && !to && query.get("unlink") !== "true") {
          return json(res, 409, { error: { code: "conflict", message: `the theme “${t.name}” is used by ${users.length} presentations` }, current: { usedBy: users.map((x) => ({ kind: "presentation", id: x.doc.id, name: x.doc.name })) } }), true;
        }
        for (const x of users) {
          if (to) followTheme(x, to);
          else {
            delete x.summary.theme;
            delete x.summary.themeName;
            for (const sec of x.doc.sections) for (const sl of sec.slides) delete sl.themeLink;
          }
        }
        c.themeDocs = docs.filter((x) => x !== t);
        refresh();
        return json(res, 200, { kind: "theme", documents: [t] }), true;
      }
      if (t && parts[2] === "used-by" && m === "GET" && !query.get("template")) {
        return json(res, 200, c.songs.filter((x) => x.summary.theme === t.id).map((x) => ({ kind: "presentation", id: x.doc.id, name: x.doc.name }))), true;
      }
      if (t && parts[2] === "document" && m === "GET") {
        const doc = { schemaVersion: 2, id: t.id, name: t.name, canvasSize: t.canvasSize, sections: [{ id: t.id, name: t.name, slides: t.slides.map((s) => ({ ...s.slide, label: { text: s.name }, templateOrigin: { theme: { id: t.id, name: t.name }, slide: s.id } })) }] };
        return json(res, 200, doc), true;
      }
      if (t && parts[2] === "used-by" && m === "GET" && query.get("template")) {
        // Songs following that template slide (the stub's songs follow a theme's first one).
        const first = t.slides[0]?.id === query.get("template");
        return json(res, 200, first ? c.songs.filter((x) => x.summary.theme === t.id).map((x) => ({ kind: "presentation", id: x.doc.id, name: x.doc.name })) : []), true;
      }
      if (t && parts[2] === "edits" && m === "POST") {
        // The template-slide ops the editor sends (batch 9): add, duplicate, move, delete,
        // rename (label) and an object's role; others are answered without a change.
        readBody(req, (b) => {
          const at = (sid) => t.slides.findIndex((x) => x.slide.id === sid);
          for (const op of b.ops ?? []) {
            if (op.type === "addSlide" && op.slide) {
              const { label, templateOrigin: _o, ...slide } = op.slide;
              const k = op.after ? at(op.after) + 1 : 0;
              t.slides.splice(k, 0, { id: uuid(`tpl:${slide.id}`), name: label?.text ?? "Slide", slide });
            } else if (op.type === "duplicateSlide" && at(op.slide) >= 0) {
              const k = at(op.slide);
              const copy = structuredClone(t.slides[k]);
              copy.slide.id = uuid(`dup:${op.slide}:${Date.now()}`);
              copy.id = uuid(`tpl:${copy.slide.id}`);
              for (const e of copy.slide.elements ?? []) e.id = uuid(`el:${e.id}:${copy.slide.id}`);
              t.slides.splice(k + 1, 0, copy);
            } else if (op.type === "deleteSlide" && at(op.slide) >= 0 && t.slides.length > 1) {
              t.slides.splice(at(op.slide), 1);
            } else if (op.type === "moveSlide" && at(op.slide) >= 0) {
              const [x] = t.slides.splice(at(op.slide), 1);
              t.slides.splice(op.after ? at(op.after) + 1 : 0, 0, x);
            } else if (op.type === "setSlide" && at(op.slide) >= 0 && op.label?.text) {
              t.slides[at(op.slide)].name = op.label.text;
            } else if (op.type === "setElement" && at(op.slide) >= 0 && op.patch && "placeholder" in op.patch) {
              const e = (t.slides[at(op.slide)].slide.elements ?? []).find((x) => x.id === op.element);
              if (e && op.patch.placeholder) e.placeholder = op.patch.placeholder;
              else if (e) delete e.placeholder;
            }
          }
          refresh();
          json(res, 200, { revision: sha(JSON.stringify(t)).slice(0, 16), applied: b.ops ?? [], inverse: [] });
        });
        return true;
      }
      if (t && parts[2] === "slides" && (parts[4] === "thumbnail" || parts[4] === "render")) {
        const s = t.slides.find((x) => x.slide.id === parts[3]);
        if (!s) return apiError(res, 404, "notFound", "slide not in theme"), true;
        const sample = /passage|scripture/i.test(s.name) ? ["The lantern keeper guards the quiet road", "Psalm 23:1"] : /title/i.test(s.name) ? ["Song title"] : ["Sample lyric line", "and the line after it"];
        return svg(res, slideSvgFor(sample, hexOf(s.slide.background), w, parts[4] === "render")), true;
      }
    }
    if (parts[0] === "presentations" && parts[2] === "edits" && m === "POST" && !cur().fixtures) {
      const c = cur();
      const song = c.songs.find((x) => x.doc.id === parts[1]);
      if (song) {
        readBody(req, (b) => {
          for (const op of b.ops ?? []) {
            if (op.type === "applyTheme") {
              const t = (c.themeDocs ?? []).find((x) => x.id === op.theme);
              if (t) followTheme(song, t);
            }
            if (op.type === "linkTheme") {
              // "Link, keeping the look" (batch 9): the stub links every slide to the first
              // theme and keeps the slides as they are otherwise.
              const t = (c.themeDocs ?? [])[0];
              if (t) {
                let applied = 0;
                for (const sec of song.doc.sections)
                  for (const sl of sec.slides)
                    if (!sl.themeLink) {
                      sl.templateOrigin = { theme: { id: t.id, name: t.name }, slide: t.slides[0].id };
                      sl.themeLink = {};
                      applied++;
                    }
                song.summary.theme = t.id;
                song.summary.themeName = t.name;
                c.themes = themeSummaries(c);
                json(res, 200, { revision: sha(`${song.doc.id}:${Date.now()}`).slice(0, 16), applied: Array.from({ length: applied }, () => ({ type: "replaceSlide" })), inverse: [] });
                return;
              }
            }
            if (op.type === "resetToTheme") {
              for (const sec of song.doc.sections)
                for (const sl of sec.slides)
                  if (sl.id === op.slide)
                    for (const e of sl.elements)
                      if (e.fromTheme?.overrides) {
                        const tpl = (c.themeDocs ?? []).flatMap((t) => t.slides).flatMap((s) => s.slide.elements).find((x) => x.id === e.fromTheme.element);
                        if (tpl) e.bounds = tpl.bounds;
                        delete e.fromTheme.overrides;
                      }
            }
          }
          c.themes = themeSummaries(c);
          json(res, 200, { revision: sha(`${song.doc.id}:${Date.now()}`).slice(0, 16), applied: [], inverse: [] });
        });
        return true;
      }
    }

    // ---- the active collection's documents (Booth A: the captured fixtures answer)
    const c = cur();
    if (parts[0] === "props" && parts[2] === "thumbnail") {
      const p = propsOf(c).find((x) => x.id === parts[1]);
      if (p && !c.fixtures) return svg(res, slideSvgFor(lines(p.slide), "#3A3D46", w, true)), true;
      return false;
    }
    if (c.fixtures) {
      if (path === "/themes" && m === "GET") return json(res, 200, c.themes), true;
      return false;
    }
    if (m !== "GET") return false;
    if (path === "/libraries") return json(res, 200, c.libraries), true;
    if (parts[0] === "libraries" && parts[2] === "presentations") return json(res, 200, c.songs.filter((s) => s.summary.library === parts[1]).map((s) => s.summary)), true;
    if (path === "/presentations") return json(res, 200, c.songs.map((s) => s.summary)), true;
    if (path === "/themes") return json(res, 200, c.themes), true;
    if (path === "/props") return json(res, 200, c.props), true;
    if (parts[0] === "presentations") {
      const doc = docOf(parts[1]);
      if (!doc) return false;
      if (parts.length === 2) return json(res, 200, doc), true;
      if (parts[2] === "slides" && (parts[4] === "thumbnail" || parts[4] === "render")) {
        for (const sec of doc.sections) for (const sl of sec.slides) if (sl.id === parts[3]) return svg(res, slideSvgFor(lines(sl), hexOf(sec.color), w, parts[4] === "render")), true;
      }
    }
    return false;
  };
}
