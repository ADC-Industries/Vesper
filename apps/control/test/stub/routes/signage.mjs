// Site ▸ Signage's stub routes (see server.mjs's header for the interface): the site's signage
// on the synthetic site of routes/site.mjs. Three channels ("Lobby" on a schedule with a
// weekend slot past midnight, "Hallway" with a feed in its playlist, "Kids Wing" with nothing
// scheduled), three playlists, an events feed (a made-up published sheet address) and a news
// feed whose last read failed (its last rows kept), the weather for a made-up place, and two
// service times. Each channel's source (`SITE-SERVER (Lobby)`) gets a generated picture at
// `GET /sources/thumbnail` (its layout's zones). Names and addresses are made up.
//
// Stub control (not part of the API): `POST /api/v1/__stub/signage {"reset": true}`.

const HUE = { Lobby: 215, Hallway: 160, "Kids Wing": 30 };

function initial() {
  const now = Date.now();
  const item = (id, content, duration = 10, transition) => ({ id, content, duration, ...(transition ? { transition } : {}) });
  return {
    managed: true,
    channels: [
      {
        id: "sg-lobby",
        name: "Lobby",
        layout: "mainSidebarTicker",
        defaultPlaylist: "pl-welcome",
        slots: [
          { id: "sl-sun", playlist: "pl-sunday", days: [6], start: 480, end: 780, priority: 2 },
          { id: "sl-wed", playlist: "pl-youth", days: [2], start: 1080, end: 1290, priority: 1 },
          { id: "sl-fri", playlist: "pl-youth", days: [4, 5], start: 1260, end: 60, priority: 1, from: "2026-11-01", until: "2026-12-31" },
        ],
        ticker: "Welcome home! Coffee and tea in the fellowship hall after every service.",
        tickerFeed: "fd-events",
        screen: "c0ffee00-0000-4000-8000-000000000001",
        source: "SITE-SERVER (Lobby)",
        now: { playlist: "pl-welcome", item: "Welcome", next: "Events · Feed", health: "ok", overridden: false },
      },
      {
        id: "sg-hall",
        name: "Hallway",
        layout: "mainTicker",
        defaultPlaylist: "pl-sunday",
        slots: [],
        ticker: "Small groups meet Tuesday evenings.",
        screen: "c0ffee00-0000-4000-8000-000000000002",
        source: "SITE-SERVER (Hallway)",
        now: { playlist: "pl-sunday", item: "Feed: News", next: "Sunrise loop (loop)", health: "problem", problem: "the feed News has no rows (timed out)", overridden: false },
      },
      {
        id: "sg-kids",
        name: "Kids Wing",
        layout: "full",
        slots: [],
        ticker: "",
        screen: "c0ffee00-0000-4000-8000-000000000003",
        source: "SITE-SERVER (Kids Wing)",
        now: { health: "idle", overridden: false },
      },
    ],
    playlists: [
      {
        id: "pl-welcome",
        name: "Welcome loop",
        items: [
          item("it-1", { kind: "text", title: "Welcome", body: "We're glad you're here. Ask us anything at the welcome desk." }, 12),
          item("it-2", { kind: "feed", feed: "fd-events", rows: 4 }, 15),
          item("it-3", { kind: "image", asset: "a1b2c3d4-0000-4000-8000-00000000000a" }, 8, { kind: { type: "cut" } }),
        ],
      },
      {
        id: "pl-sunday",
        name: "Sunday morning",
        items: [
          item("it-4", { kind: "feed", feed: "fd-news", rows: 3 }, 15),
          item("it-5", { kind: "video", asset: "a1b2c3d4-0000-4000-8000-00000000000b" }, 30, { kind: { type: "dissolve" }, duration: 2 }),
          item("it-6", { kind: "text", title: "Nursery open", body: "For ages 0–3, down the hall to the left." }, 10),
        ],
      },
      { id: "pl-youth", name: "Youth night", items: [item("it-7", { kind: "text", title: "Youth night", body: "Fridays at 7 in the gym" }, 10)] },
    ],
    feeds: [
      {
        id: "fd-events",
        name: "Events",
        url: "https://docs.google.com/spreadsheets/d/e/EXAMPLE-PUBLISHED-SHEET/pub?output=csv",
        format: "auto",
        refresh: 900,
        mapping: { title: "Event", body: "Where", when: "When" },
        columns: ["When", "Event", "Where"],
        rows: [
          { When: "Sun 9:00", Event: "Pancake breakfast", Where: "Fellowship hall" },
          { When: "Tue 19:00", Event: "Small groups", Where: "Homes around town" },
          { When: "Fri 19:00", Event: "Youth night", Where: "Gym" },
          { When: "Sat 10:00", Event: "Food drive", Where: "Parking lot" },
        ],
        readAt: now - 4 * 60000,
      },
      {
        id: "fd-news",
        name: "News",
        url: "https://news.example.org/feed.rss",
        format: "xml",
        refresh: 3600,
        mapping: { title: "title", body: "", when: "" },
        columns: [],
        rows: [],
        problem: "timed out",
      },
    ],
    settings: {
      place: { name: "Springfield, Example State", latitude: 39.8, longitude: -89.64 },
      fahrenheit: true,
      services: [
        { day: 6, minute: 600, name: "Sunday service" },
        { day: 2, minute: 1140, name: "Midweek" },
      ],
    },
    weather: { temperature: 47, high: 52, low: 38, summary: "Partly cloudy", readAt: now - 10 * 60000, credit: "Weather data by Open-Meteo.com" },
  };
}

/** A channel's picture: its layout's zones in its colour. */
function channelSvg(c, w) {
  const h = Math.round((w * 9) / 16);
  const hue = HUE[c.name] ?? 260;
  const side = c.layout === "mainSidebar" || c.layout === "mainSidebarTicker";
  const tick = c.layout === "mainTicker" || c.layout === "mainSidebarTicker";
  const sw = side ? w * 0.26 : 0;
  const th = tick ? h * 0.09 : 0;
  const mw = w - sw;
  const mh = h - th;
  const title = c.now?.overridden ? "" : (c.now?.item ?? "");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">
<rect width="${w}" height="${h}" fill="hsl(${hue},45%,22%)"/>
<rect width="${mw}" height="${mh}" fill="hsl(${hue},55%,30%)"/>
<text x="${mw / 2}" y="${mh / 2}" fill="#fff" font-family="sans-serif" font-size="${Math.round(h / 9)}" font-weight="700" text-anchor="middle" dominant-baseline="middle">${title.replace(/[<&]/g, "")}</text>
${side ? `<rect x="${mw}" width="${sw}" height="${mh}" fill="#12141c"/><text x="${mw + sw / 2}" y="${h * 0.16}" fill="#fff" font-family="sans-serif" font-size="${Math.round(h / 9)}" font-weight="700" text-anchor="middle">10:42</text><text x="${mw + sw / 2}" y="${h * 0.45}" fill="#f2b84d" font-family="sans-serif" font-size="${Math.round(h / 22)}" text-anchor="middle">Sunday service</text><text x="${mw + sw / 2}" y="${h * 0.7}" fill="#fff" font-family="sans-serif" font-size="${Math.round(h / 10)}" text-anchor="middle">47°</text>` : ""}
${tick ? `<rect y="${mh}" width="${w}" height="${th}" fill="#12141c"/><rect y="${mh}" width="${w}" height="2" fill="#f2b84d"/><text x="8" y="${mh + th * 0.68}" fill="#fff" font-family="sans-serif" font-size="${Math.round(th * 0.5)}">${c.ticker.replace(/[<&]/g, "")}</text>` : ""}
${c.now?.overridden ? `<rect width="${w}" height="${h}" fill="#b31212"/><text x="${w / 2}" y="${h / 2}" fill="#fff" font-family="sans-serif" font-size="${Math.round(h / 12)}" font-weight="700" text-anchor="middle">OVERRIDE</text>` : ""}
</svg>`;
}

export default function signageRoutes({ json, svg, apiError, push }) {
  let st = initial();
  let n = 100;
  const id = (p) => `${p}-${++n}`;
  const readBody = (req, fn) => {
    let b = "";
    req.on("data", (c) => (b += c));
    req.on("end", () => fn(b ? JSON.parse(b) : {}));
  };
  const changed = () => {
    const o = st.override;
    for (const c of st.channels) c.now = { ...c.now, overridden: !!o && (o.channels.length === 0 || o.channels.includes(c.id)) };
    push({ type: "siteChanged", parts: ["signage"] });
  };
  const done = (res, status, body) => (body === undefined ? res.writeHead(status).end() : json(res, status, body));

  return (req, res, { path, parts, query }) => {
    if (path === "/__stub/signage" && req.method === "POST") {
      readBody(req, (b) => {
        if (b.reset) st = initial();
        res.writeHead(204).end();
      });
      return true;
    }
    if (path === "/sources/thumbnail" && req.method === "GET") {
      const c = st.channels.find((x) => x.source === query.get("source"));
      if (!c) return false;
      svg(res, channelSvg(c, Math.min(640, Math.max(16, Number(query.get("w")) || 320))));
      return true;
    }
    if (parts[0] !== "signage") return false;
    const m = req.method;
    if (parts.length === 1 && m === "GET") return json(res, 200, st), true;
    if (parts[1] === "places" && m === "GET") {
      const q = (query.get("q") ?? "").trim();
      json(res, 200, { places: q.length < 2 ? [] : [`${q}, Example State, United States`, `${q}, Other Province, Canada`].map((name, i) => ({ name, latitude: 40 + i, longitude: -90 - i })) });
      return true;
    }
    readBody(req, (b) => {
      if (parts[1] === "channels") {
        if (!parts[2] && m === "POST") {
          const name = (b.name ?? "").trim();
          if (!name) return apiError(res, 400, "badRequest", "a name needs 1–48 characters");
          if (st.channels.some((c) => c.name.toLowerCase() === name.toLowerCase())) return apiError(res, 409, "conflict", `a channel is already called “${name}”`);
          const c = { id: id("sg"), name, layout: "full", slots: [], ticker: "", screen: `c0ffee00-0000-4000-8000-${String(n).padStart(12, "0")}`, source: `SITE-SERVER (${name})`, now: { health: "idle", overridden: false } };
          st.channels.push(c);
          changed();
          return done(res, 201, c);
        }
        const c = st.channels.find((x) => x.id === parts[2]);
        if (!c) return apiError(res, 404, "notFound", "no such channel");
        if (m === "DELETE") {
          st.channels = st.channels.filter((x) => x !== c);
          changed();
          return done(res, 204);
        }
        if (b.name) {
          c.name = b.name.trim();
          c.source = `SITE-SERVER (${c.name})`;
        }
        for (const k of ["layout", "ticker"]) if (b[k] !== undefined) c[k] = b[k];
        if (b.slots) c.slots = b.slots.map((s) => ({ ...s, id: s.id || id("sl") }));
        if (b.defaultPlaylist !== undefined) b.defaultPlaylist ? (c.defaultPlaylist = b.defaultPlaylist) : delete c.defaultPlaylist;
        if (b.tickerFeed !== undefined) b.tickerFeed ? (c.tickerFeed = b.tickerFeed) : delete c.tickerFeed;
        changed();
        return done(res, 200, c);
      }
      if (parts[1] === "playlists") {
        if (!parts[2] && m === "POST") {
          const p = { ...b, id: id("pl"), items: (b.items ?? []).map((i) => ({ ...i, id: i.id || id("it") })) };
          st.playlists.push(p);
          changed();
          return done(res, 201, p);
        }
        const i = st.playlists.findIndex((x) => x.id === parts[2]);
        if (i < 0) return apiError(res, 404, "notFound", "no such playlist");
        if (m === "DELETE") {
          const user = st.channels.find((c) => c.defaultPlaylist === parts[2] || c.slots.some((s) => s.playlist === parts[2]));
          if (user) return apiError(res, 409, "conflict", `the channel ${user.name} plays it`);
          st.playlists.splice(i, 1);
          changed();
          return done(res, 204);
        }
        st.playlists[i] = { ...b, id: parts[2], items: (b.items ?? []).map((x) => ({ ...x, id: x.id || id("it") })) };
        changed();
        return done(res, 200, st.playlists[i]);
      }
      if (parts[1] === "feeds") {
        if (parts[2] === "preview") {
          if (!/^https?:\/\//.test(b.url ?? "")) return apiError(res, 400, "badRequest", "can't read it: not an address");
          return done(res, 200, { columns: ["Date", "Title", "Room"], rows: [{ Date: "Sun", Title: "Picnic", Room: "Park" }, { Date: "Wed", Title: "Choir", Room: "Chapel" }] });
        }
        if (!parts[2] && m === "POST") {
          const f = { ...b, id: id("fd"), columns: ["Date", "Title", "Room"], rows: [{ Date: "Sun", Title: "Picnic", Room: "Park" }], readAt: Date.now() };
          st.feeds.push(f);
          changed();
          return done(res, 201, f);
        }
        const i = st.feeds.findIndex((x) => x.id === parts[2]);
        if (i < 0) return apiError(res, 404, "notFound", "no such feed");
        if (m === "DELETE") {
          st.feeds.splice(i, 1);
          changed();
          return done(res, 204);
        }
        st.feeds[i] = { ...st.feeds[i], ...b, id: parts[2] };
        changed();
        return done(res, 200, st.feeds[i]);
      }
      if (parts[1] === "settings" && m === "PUT") {
        st.settings = b;
        changed();
        return done(res, 200, b);
      }
      if (parts[1] === "override") {
        if (m === "DELETE") {
          delete st.override;
          changed();
          return done(res, 204);
        }
        const since = Date.now();
        st.override = { ...b, since, ...(b.seconds ? { until: since + b.seconds * 1000 } : {}) };
        changed();
        return done(res, 200, st.override);
      }
      apiError(res, 404, "notFound", "no such route");
    });
    return true;
  };
}
