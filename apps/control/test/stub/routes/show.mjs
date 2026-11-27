// The Show page's stub data: a long synthetic playlist ("Harvest Gathering", about 40 items and
// 400 slides: made-up songs with arrangements, readings, headers with colours, disabled slides,
// media cues and a media item) in a folder "Seasonal", generated at start-up into the fixtures
// map, so the stub's own routes serve the documents, thumbnails and triggers. Edits of these
// documents (disable a slide, media cues, arrangements) and playlist moves are applied in memory.
// Everything here is made up: titles, lyrics and names.

const FOLDER = "5eed0000-0000-4000-8000-00000000f001";
const PLAYLIST = "5eed0000-0000-4000-8000-00000000f002";
const BACKGROUNDS = { name: "Blue Dawn", asset: "39b7d1a4-11f4-45b5-9d59-e92a937e4d0d" };
const LOOP = { name: "Slow Clouds", asset: "164aa983-68dc-452a-8ede-009c44a2bdd6" };

const COLOURS = {
  Intro: { r: 0.23, g: 0.24, b: 0.27 },
  Verse: { r: 0.184, g: 0.373, b: 0.722 },
  Chorus: { r: 0.122, g: 0.62, b: 0.557 },
  Bridge: { r: 0.722, g: 0.271, b: 0.42 },
  Ending: { r: 0.478, g: 0.357, b: 0.839 },
  Reading: { r: 0.18, g: 0.49, b: 0.357 },
  Slide: { r: 0.23, g: 0.24, b: 0.27 },
};

const TITLES = [
  "Lanterns on the Water", "Golden Field Song", "Open Gate", "River of Morning", "The Long Table",
  "Seedtime and Harvest", "Bright Orchard", "Hands Full of Bread", "Under the Sycamore", "Low Valley Hymn",
  "Shepherd of the Hills", "Wide Sky Anthem", "Gather the Grain", "Evening Ember", "Morning Bell",
  "Quiet Harbor", "Steady Lamp", "Song of the Sower", "Harvest Moon Rising", "Home Before Dark",
  "Little Candle", "Hillside Chorus", "Faithful Rain", "Bread for the Road", "Copper Leaves", "Northwind Song",
  "Gleaners at Dusk",
];
const NAMES = ["R. Ellison", "M. Okafor", "J. Lindqvist", "T. Varga", "A. Moreau", "S. Halloran", "D. Kimura", "L. Petrov"];
const WORDS = [
  ["golden", "quiet", "open", "steady", "bright", "gentle", "wide", "early", "silver", "humble"],
  ["fields", "rivers", "lanterns", "hills", "orchards", "valleys", "harbors", "tables", "doors", "roads"],
  ["sing", "wait", "shine", "rise", "rest", "gather", "turn", "carry", "welcome", "remember"],
  ["the morning", "the evening", "the harvest", "the long day", "the new light", "the open sky", "the gathered grain", "the last field"],
];

let seq = 0;
const id = (n) => `5eed0000-0000-4000-8000-${n.toString(16).padStart(12, "0")}`;
const next = () => id(0x100 + seq++);

function line(n) {
  const w = (k) => WORDS[k][(n * (k + 3) + k) % WORDS[k].length];
  const cap = (s) => s[0].toUpperCase() + s.slice(1);
  return n % 2 ? `${cap(w(0))} ${w(1)} ${w(2)}` : `${cap(w(2))} with ${w(3)}`;
}

function textSlide(lines, extra = {}) {
  return {
    id: next(),
    ...extra,
    elements: [
      {
        id: next(),
        name: "Text",
        bounds: { x: 96, y: 54, width: 1728, height: 972 },
        kind: {
          type: "text",
          text: {
            paragraphs: lines.map((t) => ({ runs: [{ text: t }] })),
            defaultStyle: { fontFamily: "Arial", fontWeight: 400, size: 80, color: { r: 1, g: 1, b: 1 } },
            defaultParagraph: { alignment: "center" },
          },
        },
      },
    ],
  };
}

function section(name, colour, slides) {
  return { id: next(), name, color: COLOURS[colour], slides };
}

/** A made-up song: verses, chorus, maybe a bridge, an ending; two arrangements. */
function song(n) {
  const title = TITLES[n % TITLES.length];
  let k = n * 17;
  const lyric = () => [line(k++), line(k++)];
  const verses = 2 + (n % 3);
  const secs = [];
  for (let v = 1; v <= verses; v++) secs.push(section(`Verse ${v}`, "Verse", [textSlide(lyric()), textSlide(lyric()), textSlide(lyric())]));
  const chorus = section("Chorus", "Chorus", [textSlide(lyric()), textSlide(lyric()), textSlide(lyric())]);
  secs.splice(1, 0, chorus);
  if (n % 2 === 0) secs.push(section("Bridge", "Bridge", [textSlide(lyric())]));
  secs.push(section("Ending", "Ending", [textSlide(lyric())]));
  const verseIds = secs.filter((s) => s.name.startsWith("Verse")).map((s) => s.id);
  const full = { id: next(), name: "Full", sections: verseIds.flatMap((v) => [v, chorus.id]) };
  const short = { id: next(), name: "Short", sections: [verseIds[0], chorus.id, verseIds[verseIds.length - 1], chorus.id] };
  return {
    schemaVersion: 2,
    id: next(),
    name: title,
    songInfo: { authors: NAMES[n % NAMES.length], copyrightYear: 1990 + ((n * 7) % 34), ccliNumber: String(7000100 + n * 37) },
    sections: secs,
    arrangements: [full, short],
    selectedArrangement: full.id,
  };
}

function reading(n) {
  let k = 500 + n * 11;
  const slides = Array.from({ length: 4 + (n % 3) }, () => textSlide([line(k++), line(k++), line(k++)]));
  return { schemaVersion: 2, id: next(), name: `Reading ${n + 1}: ${["The Field", "The Sower", "The Long Road", "The Table"][n % 4]}`, sections: [section("Reading", "Reading", slides)] };
}

function textDoc(name, lines) {
  return { schemaVersion: 2, id: next(), name, sections: [section("Slide", "Slide", lines.map((l) => textSlide(l)))] };
}

function build() {
  const docs = [];
  const items = [];
  const header = (name, color) => items.push({ id: next(), displayName: name, enabled: true, kind: { type: "header", name, color }, presentation: null });
  const add = (doc) => {
    docs.push(doc);
    const slideCount = doc.sections.reduce((c, s) => c + s.slides.length, 0);
    items.push({
      id: next(),
      displayName: doc.name,
      enabled: true,
      kind: { type: "presentation", presentation: { id: doc.id, name: doc.name } },
      presentation: {
        id: doc.id,
        name: doc.name,
        library: null,
        slideCount,
        destination: "slide",
        kind: doc.songInfo || doc.arrangements ? "song" : doc.name.startsWith("Reading") ? "scripture" : "text",
        ...(doc.songInfo ?? {}),
      },
    });
  };

  header("Pre-service", { r: 0.36, g: 0.55, b: 0.94 });
  items.push({ id: next(), displayName: LOOP.name, enabled: true, kind: { type: "media", asset: LOOP.asset }, presentation: null });
  const welcome = textDoc("Welcome & Notices", [["Welcome to the Harvest Gathering"], ["Soup supper after the service", "Fellowship hall"], ["Volunteers meet at the side door"], ["Quiet please, we begin shortly"]]);
  // A media cue on the welcome slide, and a disabled notice.
  welcome.sections[0].slides[0].actions = [{ id: next(), name: BACKGROUNDS.name, kind: { type: "media", asset: BACKGROUNDS.asset } }];
  welcome.sections[0].slides[3].enabled = false;
  add(welcome);

  let s = 0;
  header("Gathering", { r: 0.31, g: 0.71, b: 0.53 });
  for (let i = 0; i < 6; i++) add(song(s++));
  header("Word", { r: 0.25, g: 0.65, b: 0.63 });
  add(reading(0));
  add(reading(1));
  add(textDoc("Message: Seasons", [["Seasons"], ["Part one", "Planting"], ["Part two", "Waiting"], ["Part three", "Gathering"], ["Questions to take home"]]));
  header("Response", { r: 0.48, g: 0.36, b: 0.84 });
  for (let i = 0; i < 9; i++) add(song(s++));
  header("Communion", { r: 0.72, g: 0.27, b: 0.42 });
  add(reading(2));
  for (let i = 0; i < 7; i++) add(song(s++));
  header("Sending", { r: 0.95, g: 0.55, b: 0.1 });
  for (let i = 0; i < 5; i++) add(song(s++));
  add(textDoc("Blessing", [["Go in peace"], ["Thank you for gathering with us"]]));

  // A few disabled slides across the songs.
  for (const d of docs.slice(1, 12)) {
    const sl = d.sections[d.sections.length - 1].slides;
    if (sl.length) sl[sl.length - 1].enabled = false;
  }
  // A second media cue, mid-playlist.
  docs[8].sections[0].slides[0].actions = [{ id: next(), name: LOOP.name, kind: { type: "media", asset: LOOP.asset } }];
  return { docs, items };
}

/** The Show page during playback (round 6 shots): a video on the Media layer with its transport,
 * a track on the Audio layer, a prop and a message with its token filled in. `POST
 * /stub/show-playback` puts it up, `POST /stub/show-reset` takes it down again. */
const VIDEO = { name: "Ocean Shimmer", asset: "dd6f949a-9b0d-4a3a-b565-de85e13d6526", duration: 31.5 };
const TRACK = { name: "Gentle Piano Loop", asset: "106ee9f3-d80a-4fab-b9ad-dc71bfc8be08", duration: 184 };
const PROP = "55a5fa28-6180-40d0-af62-d4a768903dad";
const MESSAGE = "f8741105-b173-4963-b5e2-ed4b75a3c4b7";

function playback(s, fx) {
  const playing = (asset, kind, position, duration, loop) => ({
    asset: asset.asset,
    name: asset.name,
    kind,
    online: true,
    playback: { state: "playing", position, duration, loop, rate: 1, hw: true },
  });
  s.layers.media = playing(VIDEO, "video", 12.4, VIDEO.duration, true);
  s.layers.audio = playing(TRACK, "audio", 41, TRACK.duration, false);
  s.transport = { layer: "media", asset: VIDEO.asset, name: VIDEO.name, state: "playing", position: 12.4, duration: VIDEO.duration, loop: true, rate: 1, hw: true };
  const prop = (fx.get("props") ?? []).find((p) => p.id === PROP);
  s.layers.props = prop ? [{ prop: PROP, name: prop.name }] : [];
  const msg = (fx.get("messages") ?? []).find((m) => m.id === MESSAGE);
  s.layers.messages = msg
    ? [{ message: MESSAGE, name: msg.name, text: msg.text.replace("${number}", "47"), tokens: [{ name: "number", value: "47" }], visible: true }]
    : [];
  s.audio.layers.audio.db = -6;
}

function reset(s) {
  s.layers.media = null;
  s.layers.audio = null;
  s.transport = null;
  s.layers.props = [];
  s.layers.messages = [];
  s.audio.layers.audio.db = 0;
  // G7: the camera the Video Inputs shot put live.
  s.layers.videoInput = null;
  s.restorableLayers = (s.restorableLayers ?? []).filter((l) => l !== "videoInput");
}

export default function ({ fx, json, apiError, state, bump }) {
  const { docs, items } = build();
  for (const d of docs) fx.set(`presentations__${d.id}`, d);
  const detail = { id: PLAYLIST, name: "Harvest Gathering", parent: FOLDER, purpose: "service", items };
  fx.set(`playlists__${PLAYLIST}`, detail);
  const tree = fx.get("playlists");
  if (tree && !tree.some((n) => n.id === FOLDER))
    tree.push({
      id: FOLDER,
      name: "Seasonal",
      kind: "folder",
      purpose: null,
      itemCount: 0,
      children: [{ id: PLAYLIST, name: "Harvest Gathering", kind: "playlist", purpose: "service", itemCount: items.length, children: [] }],
    });

  const body = (req) =>
    new Promise((resolve) => {
      let b = "";
      req.on("data", (d) => (b += d));
      req.on("end", () => {
        try {
          resolve(JSON.parse(b || "{}"));
        } catch {
          resolve({});
        }
      });
    });
  let rev = 1;

  const slideOf = (doc, sid) => doc.sections.flatMap((s) => s.slides).find((s) => s.id === sid);

  function editPresentation(doc, ops) {
    const inverse = [];
    for (const op of ops) {
      const slide = op.slide ? slideOf(doc, op.slide) : null;
      if (!slide) continue;
      if (op.type === "setSlide" && "enabled" in op) {
        inverse.unshift({ type: "setSlide", slide: op.slide, enabled: slide.enabled ?? true });
        slide.enabled = op.enabled;
      } else if (op.type === "addSlideAction") {
        slide.actions = [...(slide.actions ?? []), op.action];
        inverse.unshift({ type: "removeSlideAction", slide: op.slide, action: op.action.id });
      } else if (op.type === "removeSlideAction") {
        const a = (slide.actions ?? []).find((x) => x.id === op.action);
        slide.actions = (slide.actions ?? []).filter((x) => x.id !== op.action);
        if (a) inverse.unshift({ type: "addSlideAction", slide: op.slide, action: a });
      }
    }
    return inverse;
  }

  return (req, res, { parts }) => {
    // The sound card lost and being reconnected (`POST /stub/audio-lost`), and back
    // (`POST /stub/audio-back`): the status bar's "Sound card lost, reconnecting…".
    if (req.method === "POST" && parts[0] === "stub" && (parts[1] === "audio-lost" || parts[1] === "audio-back")) {
      const a = state.state.audio;
      if (parts[1] === "audio-lost") {
        a.device = "hw:CARD=USB,DEV=0";
        a.outputState = "error";
        a.detail = "hw:CARD=USB,DEV=0: no sound taken or given for a second (unplugged?)";
      } else {
        a.device = "";
        a.outputState = "null";
        a.detail = null;
      }
      bump();
      json(res, 200, { ok: true });
      return true;
    }
    if (req.method === "POST" && parts[0] === "stub" && (parts[1] === "show-playback" || parts[1] === "show-reset")) {
      if (parts[1] === "show-playback") playback(state.state, fx);
      else reset(state.state);
      bump();
      json(res, 200, { ok: true });
      return true;
    }
    if (req.method === "POST" && parts[0] === "presentations" && parts[2] === "edits") {
      const doc = fx.get(`presentations__${parts[1]}`);
      if (!doc || !docs.includes(doc)) return false;
      void body(req).then((b) => json(res, 200, { revision: String(++rev), applied: b.ops ?? [], inverse: editPresentation(doc, b.ops ?? []) }));
      return true;
    }
    if (req.method === "POST" && parts[0] === "playlists" && parts[1] === PLAYLIST && parts[2] === "edits") {
      void body(req).then((b) => {
        const inverse = [];
        for (const op of b.ops ?? []) {
          const it = items.find((i) => i.id === op.item);
          if (!it) continue;
          if (op.type === "setItem" && "arrangement" in op && it.kind.type === "presentation") {
            inverse.push({ type: "setItem", item: it.id, arrangement: it.kind.arrangement ?? null });
            if (op.arrangement) it.kind.arrangement = op.arrangement;
            else delete it.kind.arrangement;
          } else if (op.type === "moveItem") {
            const from = items.indexOf(it);
            inverse.push({ type: "moveItem", item: it.id, ...(from > 0 ? { after: items[from - 1].id } : {}) });
            items.splice(from, 1);
            items.splice(op.after ? items.findIndex((i) => i.id === op.after) + 1 : 0, 0, it);
          }
        }
        json(res, 200, { revision: String(++rev), applied: b.ops ?? [], inverse });
      });
      return true;
    }
    if (req.method === "PUT" && parts[0] === "playlists" && parts.length === 2) {
      // Moves (and renames) in the playlist tree.
      void body(req).then((b) => {
        const tree = fx.get("playlists");
        const take = (list) => {
          const i = list.findIndex((n) => n.id === parts[1]);
          if (i >= 0) return list.splice(i, 1)[0];
          for (const n of list) {
            const f = take(n.children);
            if (f) return f;
          }
          return null;
        };
        const find = (list, nid) => list.find((n) => n.id === nid) ?? list.map((n) => find(n.children, nid)).find(Boolean) ?? null;
        const node = "parent" in b || "after" in b ? take(tree) : find(tree, parts[1]);
        if (!node) return apiError(res, 404, "notFound", "no such playlist");
        if (b.name) node.name = b.name;
        if ("parent" in b || "after" in b) {
          const list = b.parent ? (find(tree, b.parent)?.children ?? tree) : tree;
          list.splice(b.after ? list.findIndex((n) => n.id === b.after) + 1 : 0, 0, node);
        }
        json(res, 200, { id: node.id, revision: String(++rev) });
      });
      return true;
    }
    return false;
  };
}
