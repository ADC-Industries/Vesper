// Site ▸ Displays' stub routes (see server.mjs's header for the interface): Vesper Mosaic on the
// synthetic site of routes/site.mjs. "Lobby Display" (fp-lobby) is a display box with three
// screens (two follow the "Lobby" channel, one shows a camera), "Side Screen Node" (fp-side) is
// offline with its last-known screen, and "Chapel Box" (fp-chapel, a Vesper box) has its Node's
// display service on too: a conflict. "Stage Wall Node" (fp-stagewall) drives four screens
// that make the 2 × 2 "Stage wall" following the "Main" channel, in step. Two channels. Names and
// sources are made up.
//
// Stub control (not part of the API): `POST /api/v1/__stub/mosaic {"reset": true}`;
// `{"boxGone": "fp-stagewall"}` takes a display box out of the list (removed from the site): its
// wall keeps its cells, as the server does, and the designer shows them "(not listed now)".

const settings = () => ({ fit: "fit", rotation: 0, overscan: 0, fallback: "logo", fallbackAfterSeconds: 5 });
const HUE = { "BOOTH-PC (Cam 1 Wide)": 205, "STREAM-PC (Program)": 280, "BOOTH-PC (Cam 2 Pulpit)": 28 };

function initial() {
  const scr = (number, name, feed, showing, status, extra = {}) => ({ number, name, width: 1920, height: 1080, feed, showing, settings: settings(), status, ...extra });
  return {
    managed: true,
    logo: { hash: "5e1f0a2b", width: 1200, height: 400 },
    machines: [
      { machine: "fp-hall", name: "Hall Mini PC", health: "online", headlessOk: true },
      { machine: "fp-office", name: "Office PC", health: "online", headlessOk: false },
    ],
    channels: [
      { id: "ch-lobby", name: "Lobby", source: "STREAM-PC (Program)", screens: 2, router: "SITE-SERVER (Channel Lobby)" },
      { id: "ch-main", name: "Main", source: "BOOTH-PC (Cam 1 Wide)", screens: 4, router: "SITE-SERVER (Channel Main)" },
    ],
    walls: [
      {
        id: "wall-stage",
        name: "Stage wall",
        rows: 2,
        columns: 2,
        bezelMm: [3.5, 3.5],
        screenMm: [1209, 680],
        feed: { kind: "channel", channel: "ch-main" },
        cells: [0, 1, 2, 3].map((i) => ({ row: Math.floor(i / 2), column: i % 2, machine: "fp-stagewall", screen: i + 1 })),
        status: "showing",
        synced: true,
        delayMs: 42,
        lateRecently: 0,
      },
    ],
    boxes: [
      {
        machine: "fp-chapel",
        name: "Chapel Box",
        health: "online",
        service: "running",
        added: false,
        vesper: true,
        conflict: "This machine runs a Vesper server and its IDN Node's display service: two programs would drive the same screens. Switch the display service off there (its screens are Vesper's outputs).",
        reachable: false,
        screens: [],
      },
      {
        machine: "fp-lobby",
        name: "Lobby Display",
        health: "needsAttention",
        service: "running",
        address: "192.0.2.61",
        addressFrom: "site",
        serviceMode: "headless",
        serviceModeReported: true,
        headlessOk: true,
        updateManagedBy: "Vesper",
        logoState: "onBox",
        added: false,
        vesper: false,
        reachable: true,
        screens: [
          scr(1, "HDMI-1", { kind: "channel", channel: "ch-lobby" }, "STREAM-PC (Program)", "showing"),
          scr(2, "HDMI-2", { kind: "channel", channel: "ch-lobby" }, "STREAM-PC (Program)", "showing", { settings: { ...settings(), rotation: 90 } }),
          scr(3, "DP-1", { kind: "source", source: "BOOTH-PC (Cam 2 Pulpit)" }, "BOOTH-PC (Cam 2 Pulpit)", "lost", { problem: "No pictures for 4 s" }),
        ],
      },
      {
        machine: "fp-stagewall",
        name: "Stage Wall Node",
        health: "online",
        service: "running",
        address: "192.0.2.63",
        addressFrom: "site",
        headlessOk: true,
        logoState: "sending",
        added: false,
        vesper: false,
        reachable: true,
        screens: ["HDMI-1", "HDMI-2", "HDMI-3", "HDMI-4"].map((name, i) => {
          const tile = { rows: 2, columns: 2, row: Math.floor(i / 2), column: i % 2, bezelMm: [3.5, 3.5], screenMm: [1209, 680], group: "wall:Stage wall" };
          return scr(i + 1, name, { kind: "channel", channel: "ch-main" }, "BOOTH-PC (Cam 1 Wide)", "showing", {
            settings: { ...settings(), tile },
            wall: "wall-stage",
            sync: { group: "wall:Stage wall", mode: "sync/1", delayMs: 40 + (i % 2) * 2, late: 0, lateRecently: 0 },
          });
        }),
      },
      {
        machine: "fp-side",
        name: "Side Screen Node",
        health: "offline",
        service: "running",
        address: "192.0.2.62",
        addressFrom: "admin",
        added: false,
        vesper: false,
        reachable: false,
        problem: "offline in the site",
        screens: [scr(1, "", { kind: "source", source: "BOOTH-PC (Cam 1 Wide)" }, "", "offline", { problem: "offline in the site", width: undefined, height: undefined })],
      },
    ],
  };
}

function thumb(source, w) {
  const hue = HUE[source] ?? 200;
  const h = Math.round((w * 9) / 16);
  const label = source.slice(source.indexOf("(") + 1, -1);
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 320 180">
<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue},40%,40%)"/><stop offset="1" stop-color="hsl(${hue},30%,12%)"/></linearGradient></defs>
<rect width="320" height="180" fill="url(#g)"/>
<rect x="24" y="40" width="272" height="70" rx="8" fill="hsl(${hue},20%,18%)" opacity="0.8"/>
<text x="160" y="84" text-anchor="middle" font-family="sans-serif" font-size="20" fill="#fff" opacity="0.85">${label}</text>
</svg>`;
}

export default function mosaicRoutes({ json, svg, apiError, push }) {
  let st = initial();
  const readBody = (req, fn) => {
    let b = "";
    req.on("data", (c) => (b += c));
    req.on("end", () => fn(b ? JSON.parse(b) : {}));
  };
  const counts = () => {
    for (const c of st.channels) c.screens = st.boxes.flatMap((b) => b.screens).filter((s) => s.feed.kind === "channel" && s.feed.channel === c.id).length;
    st.walls.sort((a, b) => a.name.localeCompare(b.name));
  };
  const resolve = (feed) => (feed.kind === "source" ? feed.source : feed.kind === "channel" ? (st.channels.find((c) => c.id === feed.channel)?.source ?? "") : "");
  const changed = () => {
    counts();
    push({ type: "siteChanged", parts: ["mosaic"] });
  };
  // A wall applied to its screens (the server's rules, simply): the same tile geometry and group,
  // the wall's feed; screens that left lose their tile.
  const applyWall = (w, before = []) => {
    const all = st.boxes.flatMap((b) => b.screens.map((s) => ({ b, s })));
    for (const c of before) {
      const x = all.find(({ b, s }) => b.machine === c.machine && s.number === c.screen);
      if (x && !w.cells.some((k) => k.machine === c.machine && k.screen === c.screen)) {
        delete x.s.wall;
        delete x.s.sync;
        delete x.s.settings.tile;
      }
    }
    const source = resolve(w.feed);
    let worst = w.cells.length ? "showing" : "idle";
    for (const c of w.cells) {
      const x = all.find(({ b, s }) => b.machine === c.machine && s.number === c.screen);
      if (!x) continue;
      const group = `wall:${w.name}`;
      x.s.wall = w.id;
      x.s.feed = w.feed;
      x.s.settings = { ...x.s.settings, tile: { rows: w.rows, columns: w.columns, row: c.row, column: c.column, bezelMm: w.bezelMm, ...(w.screenMm ? { screenMm: w.screenMm } : {}), group } };
      if (x.b.reachable) {
        x.s.showing = source;
        x.s.status = source ? "showing" : "idle";
        x.s.sync = source ? { group, mode: "sync/1", delayMs: 38, late: 0, lateRecently: 0 } : undefined;
      } else worst = "offline";
      if (x.s.status !== "showing" && worst === "showing") worst = x.s.status;
    }
    Object.assign(w, { status: worst, synced: worst === "showing", delayMs: worst === "showing" ? 38 : undefined, lateRecently: 0 });
    if (!w.cells.length) w.problem = "No screens in the wall yet";
    else delete w.problem;
  };
  const wallBody = (b) => ({
    name: (b.name ?? "").trim(),
    rows: b.rows,
    columns: b.columns,
    bezelMm: b.bezelMm ?? [0, 0],
    ...(b.screenMm ? { screenMm: b.screenMm } : {}),
    feed: b.feed ?? { kind: "none" },
    cells: (b.cells ?? []).slice().sort((x, y) => x.row - y.row || x.column - y.column),
  });
  const wallRefused = (b, id) => {
    if (!b.name) return [400, "badRequest", "name: 1 to 48 characters"];
    if (st.walls.some((w) => w.id !== id && w.name.toLowerCase() === b.name.toLowerCase())) return [409, "conflict", "a wall has that name"];
    const other = st.walls.find((w) => w.id !== id && w.cells.some((c) => b.cells.some((k) => k.machine === c.machine && k.screen === c.screen)));
    if (other) return [409, "conflict", `a screen is in the wall "${other.name}" already`];
    return null;
  };
  const findScreen = (machine, n) => st.boxes.find((b) => b.machine === machine && !b.vesper)?.screens.find((s) => s.number === Number(n));

  return (req, res, { path, parts, query }) => {
    const m = req.method;
    if (path === "/__stub/mosaic" && m === "POST") {
      readBody(req, (b) => {
        if (b.reset) st = initial();
        if (b.boxGone) {
          st.boxes = st.boxes.filter((x) => x.machine !== b.boxGone);
          for (const w of st.walls) if (w.cells.some((c) => c.machine === b.boxGone)) Object.assign(w, { status: "offline", synced: false, delayMs: undefined, problem: "A screen's display box isn't in the site now" });
          changed();
        }
        json(res, 200, {});
      });
      return true;
    }
    if (parts[0] !== "mosaic") return false;
    if (path === "/mosaic/logo") {
      if (m === "GET") {
        if (!st.logo) return apiError(res, 404, "notFound", "no logo"), true;
        return svg(res, `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="400" viewBox="0 0 300 100"><rect width="300" height="100" fill="#1d2433"/><circle cx="52" cy="50" r="30" fill="#e8b04a"/><text x="96" y="62" font-family="serif" font-size="34" fill="#f4efe6">Site Logo</text></svg>`), true;
      }
      if (m === "DELETE") {
        st.logo = undefined;
        for (const b of st.boxes) delete b.logoState;
        changed();
        res.writeHead(204).end();
        return true;
      }
      req.on("data", () => {});
      req.on("end", () => {
        st.logo = { hash: `h${Date.now()}`, width: 800, height: 400 };
        for (const b of st.boxes) if (b.reachable && !b.vesper) b.logoState = "sending";
        changed();
        json(res, 200, st);
      });
      return true;
    }
    if (parts[1] === "boxes" && parts[3] === "service" && m === "PUT") {
      readBody(req, (b) => {
        const bx = st.boxes.find((x) => x.machine === parts[2]);
        const mc = st.machines.find((x) => x.machine === parts[2]);
        if (bx) {
          bx.serviceMode = b.mode;
          bx.serviceModeReported = false;
        }
        else if (mc) {
          st.machines = st.machines.filter((x) => x !== mc);
          st.boxes.push({ machine: mc.machine, name: mc.name, health: mc.health, service: "starting", serviceMode: b.mode, headlessOk: mc.headlessOk, added: true, vesper: false, reachable: false, problem: "Not read yet", screens: [] });
        }
        changed();
        json(res, 200, st);
      });
      return true;
    }
    if (path === "/mosaic" && m === "GET") return json(res, 200, st), true;
    if (path === "/mosaic/channels" && m === "GET") return json(res, 200, st.channels), true;
    if ((path === "/mosaic/identify" || path === "/mosaic/test-pattern") && m === "POST") {
      readBody(req, (b) => {
        if (path.endsWith("test-pattern")) for (const bx of st.boxes) if (bx.reachable) for (const s of bx.screens) s.testPattern = !!b.on;
        changed();
        res.writeHead(204).end();
      });
      return true;
    }
    if (path === "/mosaic/channels" && m === "POST") {
      readBody(req, (b) => {
        if (st.channels.some((c) => c.name.toLowerCase() === (b.name ?? "").trim().toLowerCase())) return apiError(res, 409, "conflict", "a channel has that name");
        const name = b.name.trim();
        const c = { id: `ch-${Date.now()}`, name, source: b.source ?? "", screens: 0, router: `SITE-SERVER (Channel ${name})` };
        st.channels.push(c);
        changed();
        json(res, 201, c);
      });
      return true;
    }
    if (path === "/mosaic/walls" && m === "POST") {
      readBody(req, (b) => {
        const w = { id: `wall-${Date.now()}`, ...wallBody(b) };
        const no = wallRefused(w, w.id);
        if (no) return apiError(res, ...no);
        st.walls.push(w);
        applyWall(w);
        changed();
        json(res, 201, w);
      });
      return true;
    }
    if (parts[1] === "walls" && parts[2]) {
      const w = st.walls.find((x) => x.id === parts[2]);
      if (!w) return apiError(res, 404, "notFound", "no such wall"), true;
      if (m === "DELETE") {
        applyWall({ ...w, cells: [] }, w.cells);
        st.walls = st.walls.filter((x) => x !== w);
        changed();
        res.writeHead(204).end();
        return true;
      }
      readBody(req, (b) => {
        if (parts[3] === "identify") return res.writeHead(204).end();
        const before = w.cells;
        if (parts[3] === "feed") w.feed = b;
        else {
          const nw = wallBody(b);
          const no = wallRefused(nw, w.id);
          if (no) return apiError(res, ...no);
          Object.assign(w, nw);
        }
        applyWall(w, before);
        changed();
        json(res, 200, st);
      });
      return true;
    }
    if (parts[1] === "channels" && parts[2]) {
      const c = st.channels.find((x) => x.id === parts[2]);
      if (!c) return apiError(res, 404, "notFound", "no such channel"), true;
      if (m === "DELETE") {
        if (c.screens > 0) return apiError(res, 409, "conflict", `${c.screens} screen(s) follow this channel: give them another feed first`), true;
        st.channels = st.channels.filter((x) => x !== c);
        changed();
        res.writeHead(204).end();
        return true;
      }
      readBody(req, (b) => {
        if (b.name) {
          c.name = b.name.trim();
          c.router = `SITE-SERVER (Channel ${c.name})`;
        }
        if (b.source !== undefined) {
          c.source = b.source;
          for (const bx of st.boxes) for (const s of bx.screens) if (s.feed.kind === "channel" && s.feed.channel === c.id && bx.reachable) s.showing = c.source;
        }
        changed();
        json(res, 200, st);
      });
      return true;
    }
    if (parts[1] === "boxes" && parts[3] === "screens") {
      const s = findScreen(parts[2], parts[4]);
      if (!s) return apiError(res, 404, "notFound", "no such display box or screen"), true;
      const box = st.boxes.find((b) => b.machine === parts[2]);
      const what = parts[5];
      if (what === "thumbnail") {
        if (!s.showing || !box.reachable) return apiError(res, 404, "notFound", "the screen shows nothing"), true;
        return svg(res, thumb(s.showing, Math.min(640, Number(query.get("w")) || 320))), true;
      }
      readBody(req, (b) => {
        if (what === "feed") {
          if (s.wall) return apiError(res, 409, "conflict", "this screen is a tile of a wall: change the wall's feed");
          s.feed = b;
          if (!box.reachable) {
            changed();
            return apiError(res, 503, "unavailable", "Kept; the box didn't take it now: offline in the site");
          }
          s.showing = resolve(b);
          s.status = s.showing ? "showing" : "idle";
          delete s.problem;
        } else if (what === "settings") s.settings = { ...settings(), ...b };
        else if (what === "test-pattern") s.testPattern = !!b.on;
        changed();
        if (what === "identify" || what === "test-pattern") res.writeHead(204).end();
        else json(res, 200, st);
      });
      return true;
    }
    if (parts[1] === "boxes" && parts[2]) {
      readBody(req, (b) => {
        const bx = st.boxes.find((x) => x.machine === parts[2]);
        if (bx && b.address !== undefined) {
          bx.address = b.address || "192.0.2.64";
          bx.addressFrom = b.address ? "admin" : "site";
        }
        changed();
        json(res, 200, st);
      });
      return true;
    }
    return false;
  };
}
