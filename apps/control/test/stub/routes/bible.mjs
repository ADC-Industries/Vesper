// The Bible page's stub routes (`/scripture/...`, docs/SCRIPTURE-PLAN.md) over a made-up mini
// translation (`fixtures/bible/scripture-text.json`: real book names, invented verses; a verse
// without its own text is built from the fixture's phrases). Two translations (`TST`, `SYN`)
// share the text. Parsing understands "Ps 23:1-4, 6", "John 3:16–18", "Genesis 2" and the like.
// While `bibleStub.on` (scripts/shots/bible.mjs), `GET /themes` also lists two made-up scripture
// themes; otherwise the shared themes fixture answers. Writes create nothing: `POST
// /scripture/presentations` answers like the server and remembers the last body (`bibleStub.last`).
// Parallel Bible (batch 16): a third, Spanish-tagged translation `PRU` (invented phrases) can be
// the `secondTranslation`; each slide then has its `second` text and the preview draws both
// (side by side, or stacked in pale gold).

export const bibleStub = {
  on: false,
  last: null,
  /** `POST /scripture/presentations/{id}/resplit`: set by the library stub (it holds the
   * documents); answers the result, or null for a presentation that isn't a passage. */
  resplit: null,
};

const THEMES = [
  { id: "5eed0000-0000-4000-8000-0000000b0001", name: "Scripture — Lower third", canvasSize: { width: 1920, height: 1080 }, slides: [{ id: "5eed0000-0000-4000-8000-0000000b0011", name: "Scripture" }] },
  { id: "5eed0000-0000-4000-8000-0000000b0002", name: "Scripture — Full screen", canvasSize: { width: 1920, height: 1080 }, slides: [{ id: "5eed0000-0000-4000-8000-0000000b0012", name: "Scripture" }] },
];

const ALIASES = {
  GEN: ["gen", "ge", "gn", "genesis"],
  PSA: ["ps", "psa", "pss", "psalm", "psalms"],
  JHN: ["jn", "jhn", "joh", "john"],
};

/** The Spanish-tagged made-up translation: invented phrases, never real scripture. */
const PRU = { id: "PRU", abbreviation: "PRU", name: "Prueba Sintética", language: "es", license: "Inventada para pruebas", origin: "local", books: 3 };
const ES_PHRASES = [
  "El farolero caminó por el camino tranquilo al alba",
  "y la puerta del huerto quedó abierta a cada viajero",
  "Las campanas callaron sobre los cerros de arcilla",
  "mientras el río despertaba despacio entre las piedras",
  "Los pájaros cantaron sus siete canciones de la mañana",
  "y cada piedra del camino estaba tibia bajo el sol",
  "Nadie contó las horas en aquel día de fiesta",
  "porque la mesa larga tenía pan para todos",
];

const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

export default function bibleRoutes({ fx, json, svg, apiError }) {
  const data = fx.get("scripture-text");
  const translations = [...fx.get("scripture__translations"), PRU];

  const bookOf = (key) => data.books.find((b) => b.key === key);
  const refName = (b) => (b.key === "PSA" ? "Psalm" : b.name);

  function verseText(book, c, v, tr) {
    if (tr?.language === "es") {
      const p = ES_PHRASES;
      const i = (book.charCodeAt(0) * 7 + c * 5 + v * 3) % p.length;
      return `${p[i]}, ${p[(i + 3) % p.length].toLowerCase()}.`;
    }
    const own = data.verses[`${book} ${c}`];
    if (Array.isArray(own) && own[v - 1]) return own[v - 1];
    if (own && !Array.isArray(own) && own[String(v)]) return own[String(v)];
    const p = data.phrases;
    const i = (book.charCodeAt(0) * 7 + c * 5 + v * 3) % p.length;
    return `${p[i]}, ${p[(i + 5) % p.length].toLowerCase()}.`;
  }

  /** `{ok, book, bookName, ranges, label, error}` like the server's parse. */
  function parse(q) {
    const m = /^\s*((?:[1-3]\s*)?[a-z]+)\.?\s*(.*)$/i.exec(q ?? "");
    if (!m) return { ok: false, ranges: [], error: "Type a book and a chapter" };
    const word = m[1].replace(/\s+/g, "").toLowerCase();
    const key = Object.keys(ALIASES).find((k) => ALIASES[k].includes(word));
    const b = key && bookOf(key);
    if (!b) return { ok: false, ranges: [] };
    const base = { book: b.key, bookName: b.name };
    const rest = m[2].trim().replace(/[–—]/g, "-");
    if (!rest) return { ...base, ok: false, ranges: [], error: `Add a chapter (${b.name} has ${b.chapters.length})` };
    const rm = /^(\d+)(?::(\d+)(?:-(\d+))?((?:\s*,\s*\d+(?:-\d+)?)*))?$/.exec(rest);
    if (!rm) return { ...base, ok: false, ranges: [], error: "Write it as chapter:verse, e.g. 23:1-6" };
    const c = Number(rm[1]);
    const count = b.chapters[c - 1];
    if (!count) return { ...base, ok: false, ranges: [], error: `${b.name} has ${b.chapters.length} chapters` };
    const ranges = [];
    if (!rm[2]) ranges.push({ chapter: c, verse: 1, endChapter: c, endVerse: count });
    else {
      ranges.push({ chapter: c, verse: Number(rm[2]), endChapter: c, endVerse: Number(rm[3] ?? rm[2]) });
      for (const part of (rm[4] ?? "").split(",").map((s) => s.trim()).filter(Boolean)) {
        const [a, z] = part.split("-").map(Number);
        ranges.push({ chapter: c, verse: a, endChapter: c, endVerse: z ?? a });
      }
    }
    const bad = ranges.find((r) => r.verse < 1 || r.endVerse > count || r.endVerse < r.verse);
    if (bad) return { ...base, ok: false, ranges: [], error: `${refName(b)} ${c} has ${count} verses` };
    const spans = ranges.map((r) => (r.verse === r.endVerse ? `${r.verse}` : `${r.verse}–${r.endVerse}`));
    const whole = !rm[2];
    const label = whole ? `${refName(b)} ${c}` : `${refName(b)} ${c}:${spans.join(", ")}`;
    return { ...base, ok: true, ranges, label };
  }

  function passage(tr, ref) {
    const p = parse(ref);
    if (!p.ok) return { error: p.error ?? `"${ref}" is not a reference` };
    const verses = [];
    for (const r of p.ranges) for (let v = r.verse; v <= r.endVerse; v++) verses.push({ book: p.book, chapter: r.chapter, verse: v, text: verseText(p.book, r.chapter, v, tr) });
    return { translation: tr.id, book: p.book, label: p.label, ranges: p.ranges, ...(tr.attribution ? { attribution: tr.attribution } : {}), verses };
  }

  function slides(body) {
    const tr = translations.find((t) => t.id === body.translation);
    if (!tr) return { status: 404, error: `no translation ${body.translation}` };
    const p = passage(tr, body.reference);
    if (p.error) return { status: 400, error: p.error };
    // The server lays each slide out in the theme's text box; a character budget stands in for
    // that here (a themed lower third holds less than the plain layout). As many whole verses
    // as fit, at most `versesPerSlide`; a verse too long for a slide breaks between words and
    // continues on the next (" …" / "… ").
    const per = Number(body.versesPerSlide) || Infinity;
    const cap = Math.min(Number(body.maxChars) || Infinity, body.theme ? 100 : 220);
    const piece = (v, words, first, rest) => ({ v, text: `${first ? "" : "… "}${words.join(" ")}${rest ? " …" : ""}`, first });
    const textOf = (g) => g.map((x) => (body.verseNumbers && x.first ? `${x.v.verse} ${x.text}` : x.text)).join(" ");
    const fits = (g) => textOf(g).length <= cap;
    const groups = [];
    let i = 0;
    let done = 0;
    while (i < p.verses.length) {
      const g = [];
      while (i < p.verses.length && g.length < per) {
        const left = p.verses[i].text.split(/\s+/).slice(done);
        const whole = piece(p.verses[i], left, done === 0, false);
        if (fits([...g, whole])) {
          g.push(whole);
          i++;
          done = 0;
          continue;
        }
        if (g.length) break;
        let take = 1;
        while (take + 1 < left.length && fits([piece(p.verses[i], left.slice(0, take + 1), done === 0, true)])) take++;
        if (left.length <= 1) {
          g.push(whole);
          i++;
          done = 0;
        } else {
          g.push(piece(p.verses[i], left.slice(0, take), done === 0, true));
          done += take;
        }
        break;
      }
      groups.push(g);
    }
    const name = refName(bookOf(p.book));
    const tr2 = body.secondTranslation && body.secondTranslation !== tr.id ? translations.find((t) => t.id === body.secondTranslation) : null;
    if (body.secondTranslation && body.secondTranslation !== tr.id && !tr2) return { status: 404, error: `no translation ${body.secondTranslation}` };
    const abbr = tr2 ? `${tr.abbreviation} · ${tr2.abbreviation}` : tr.abbreviation;
    const out = groups.map((g, k) => {
      const a = g[0].v.verse;
      const z = g[g.length - 1].v.verse;
      const ref = `${name} ${g[0].v.chapter}:${a === z ? a : `${a}–${z}`} ${abbr}`;
      const last = k === groups.length - 1;
      const showRef = body.referenceOn === "each" || (body.referenceOn === "last" && last);
      const second = tr2 ? g.filter((x) => x.first).map((x) => (body.verseNumbers ? `${x.v.verse} ` : "") + verseText(p.book, x.v.chapter, x.v.verse, tr2)).join(" ") : null;
      return {
        text: textOf(g),
        ...(second ? { second } : {}),
        ...(showRef ? { reference: ref } : {}),
        ...(last && tr.attribution ? { attribution: tr.attribution } : {}),
      };
    });
    return { name: `${p.label} (${abbr})`, slides: out };
  }

  /** Words wrapped to lines of at most `perLine` characters. */
  function wrap(text, perLine) {
    const lines = [];
    for (const word of text.split(/\s+/)) {
      const l = lines[lines.length - 1];
      if (l !== undefined && (l + " " + word).length <= perLine) lines[lines.length - 1] = `${l} ${word}`;
      else lines.push(word);
    }
    return lines;
  }

  /** A parallel slide's picture: both texts side by side, or the second stacked under the first. */
  function parallelSvg(s, w, layout) {
    const h = Math.round((w * 9) / 16);
    const size = Math.max(7, Math.round(w / 34));
    const lh = size * 1.25;
    const col = (text, x, cw, top, extra) =>
      wrap(text, Math.floor(cw / (size * 0.6)))
        .slice(0, 9)
        .map((l, i) => `<text x="${x}" y="${(top + i * lh).toFixed(1)}" text-anchor="middle"${extra}>${esc(l)}</text>`)
        .join("");
    const gold = ` fill="#FFDE8F" font-style="italic"`;
    let body;
    if (layout === "stacked") {
      const a = wrap(s.text, Math.floor((w * 0.86) / (size * 0.52))).length;
      const top = h * 0.18;
      body = col(s.text, w / 2, w * 0.86, top, "") + col(s.second, w / 2, w * 0.86, top + (Math.min(a, 9) + 0.8) * lh, ` font-size="${Math.round(size * 0.85)}"${gold}`);
    } else {
      const top = h * 0.22;
      body = col(s.text, w * 0.27, w * 0.44, top, "") + col(s.second, w * 0.73, w * 0.44, top, gold);
    }
    const ref = s.reference ? `<text x="${w * 0.95}" y="${(h - size * 1.1).toFixed(1)}" text-anchor="end" fill="#D1D1DB" font-size="${Math.round(size * 0.8)}">${esc(s.reference)}</text>` : "";
    return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}"><rect width="${w}" height="${h}" fill="#000000"/><g fill="#FFFFFF" font-family="Inter, 'DejaVu Sans', sans-serif" font-size="${size}" font-weight="600">${body}${ref}</g></svg>`;
  }

  /** A generated slide's picture: the text wrapped on black, the reference under it. */
  function previewSvg(s, w, themed) {
    const h = Math.round((w * 9) / 16);
    const size = Math.max(8, Math.round(w / 26));
    const perLine = Math.floor((w * 0.86) / (size * 0.52));
    const lines = wrap(s.text, perLine);
    const shown = lines.slice(0, 7);
    const lower = themed === "lower";
    const top = lower ? h - (shown.length + 1.6) * size * 1.25 : h / 2 - ((shown.length - 1) * size * 1.25) / 2;
    const text = shown.map((l, i) => `<text x="${w / 2}" y="${(top + i * size * 1.25).toFixed(1)}" text-anchor="middle">${esc(l)}</text>`).join("");
    const ref = s.reference ? `<text x="${w / 2}" y="${(lower ? h - size * 0.6 : h - size * 1.2).toFixed(1)}" text-anchor="middle" fill="#F5C46B" font-size="${Math.round(size * 0.8)}">${esc(s.reference)}</text>` : "";
    const band = lower ? `<rect y="${(top - size * 1.2).toFixed(1)}" width="${w}" height="${(h - top + size * 1.2).toFixed(1)}" fill="#000000" fill-opacity="0.78"/>` : "";
    const bg = themed ? `<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#1F3A4D"/><stop offset="1" stop-color="#0B0C0F"/></linearGradient></defs><rect width="${w}" height="${h}" fill="url(#g)"/>` : `<rect width="${w}" height="${h}" fill="#000000"/>`;
    return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">${bg}${band}<g fill="#FFFFFF" font-family="Inter, 'DejaVu Sans', sans-serif" font-size="${size}" font-weight="600">${text}${ref}</g></svg>`;
  }

  return (req, res, { parts, query }) => {
    if (bibleStub.on && req.method === "GET" && parts[0] === "themes" && parts.length === 1) {
      json(res, 200, THEMES);
      return true;
    }
    if (parts[0] !== "scripture") return false;
    if (req.method === "POST" && parts[1] === "presentations" && parts[3] === "resplit") {
      // A passage split again (batch 9): the library stub keeps the documents.
      const r = bibleStub.resplit?.(parts[2]);
      if (r) json(res, 200, r);
      else apiError(res, 400, "badRequest", "not a passage made on the Bible page (it can't be split again)");
      return true;
    }
    const tr = translations.find((t) => t.id === (query.get("translation") ?? parts[2]));
    if (req.method === "GET") {
      if (parts[1] === "translations" && parts.length === 2) json(res, 200, translations);
      else if (parts[1] === "translations" && parts[3] === "books") {
        if (!tr) apiError(res, 404, "notFound", `no translation ${parts[2]}`);
        else json(res, 200, data.books);
      } else if (!tr) apiError(res, 404, "notFound", `no translation ${query.get("translation")}`);
      else if (parts[1] === "parse") json(res, 200, parse(query.get("q")));
      else if (parts[1] === "passage") {
        const p = passage(tr, query.get("ref"));
        if (p.error) apiError(res, 400, "badRequest", p.error);
        else json(res, 200, p);
      } else if (parts[1] === "search") {
        const words = (query.get("q") ?? "").toLowerCase().split(/\s+/).filter(Boolean);
        const limit = Math.min(500, Number(query.get("limit")) || 50);
        const hits = [];
        for (const b of data.books)
          b.chapters.forEach((n, ci) => {
            for (let v = 1; v <= n; v++) {
              const text = verseText(b.key, ci + 1, v);
              if (words.length && words.every((w) => text.toLowerCase().includes(w))) hits.push({ book: b.key, chapter: ci + 1, verse: v, text });
            }
          });
        json(res, 200, { translation: tr.id, hits: hits.slice(0, limit), total: hits.length });
      } else if (parts[1] === "preview") {
        const body = Object.fromEntries(query.entries());
        body.verseNumbers = body.verseNumbers === "true";
        const s = slides(body);
        const one = s.slides?.[Number(query.get("slide")) || 0];
        if (!one) apiError(res, 404, "notFound", s.error ?? "no such slide");
        else {
          const theme = THEMES.findIndex((t) => t.id === body.theme);
          const w = Math.min(1920, Math.max(16, Number(query.get("w")) || 320));
          if (one.second) svg(res, parallelSvg(one, w, body.secondLayout === "stacked" ? "stacked" : "sideBySide"));
          else svg(res, previewSvg(one, w, theme === 0 ? "lower" : theme === 1 ? "full" : null));
        }
      } else apiError(res, 404, "notFound", "no such scripture route");
      return true;
    }
    if (req.method !== "POST") return false;
    let raw = "";
    req.on("data", (d) => (raw += d));
    req.on("end", () => {
      const body = raw ? JSON.parse(raw) : {};
      const s = slides(body);
      if (s.error) return apiError(res, s.status, s.status === 404 ? "notFound" : "badRequest", s.error);
      if (parts[1] === "slides") return json(res, 200, s);
      if (parts[1] === "presentations") {
        bibleStub.last = body;
        return json(res, 201, {
          presentation: `5eed0000-0000-4000-8000-${String(Date.now()).slice(-12)}`,
          name: body.name ?? s.name,
          slideCount: s.slides.length,
          ...(body.then === "playlist" ? { playlistItem: `5eed0000-0000-4000-8001-${String(Date.now()).slice(-12)}` } : {}),
        });
      }
      return apiError(res, 404, "notFound", "no such scripture route");
    });
    return true;
  };
}
