// Chords (batch 16): `POST /chordpro/parse` reads a ChordPro text into a synthetic presentation
// (sections from `{start_of_…}` / `{soc}` and blank lines, chords from `[…]`, `{title}` and
// `{key}`; a small subset of the server's reader, enough for the dialog's preview; a
// two-language file read with `split`: alternating lines, a `---` line, a separator inside each
// line or `{ci: …}` comments, `{meta: second_language xx}`, as `Slide.translation`), and
// `GET /presentations/{id}/chordpro` answers a short made-up song. Always on; nothing is stored.

const uuid = (n) => `c0000000-0000-4000-8000-${String(n).padStart(12, "0")}`;

function parse(text) {
  const out = { title: null, key: null, second: null, sections: [] };
  let cur = null;
  let stanza = [];
  let notes = [];
  const counts = {};
  const open = (kind, label) => {
    end();
    counts[kind] = (counts[kind] ?? 0) + 1;
    const base = { verse: "Verse", chorus: "Chorus", bridge: "Bridge" }[kind] ?? "Part";
    cur = { name: label || (base === "Verse" || counts[kind] > 1 ? `${base} ${counts[kind]}` : base), stanzas: [], notes: [] };
    out.sections.push(cur);
  };
  const end = () => {
    if (!stanza.length) return;
    if (!cur) open("verse");
    cur.stanzas.push(stanza);
    cur.notes.push(notes);
    stanza = [];
    notes = [];
  };
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trimEnd();
    const d = /^\{\s*([a-z_]+)\s*(?::\s*(.*))?\}$/i.exec(line.trim());
    if (d) {
      const [, name, value] = d;
      const n = name.toLowerCase();
      const meta = n === "meta" ? /^(\S+)\s+(.*)$/.exec(value?.trim() ?? "") : null;
      if (meta?.[1] === "second_language") out.second = meta[2].trim();
      else if ((n === "ci" || n === "comment_italic") && stanza.length && value?.trim()) notes.push(value.trim());
      else if (n === "title" || n === "t") out.title = value?.trim() || null;
      else if (n === "key") out.key = value?.trim() || null;
      else if (n === "soc" || n === "start_of_chorus") open("chorus", value);
      else if (n === "sov" || n === "start_of_verse") open("verse", value);
      else if (n === "sob" || n === "start_of_bridge") open("bridge", value);
      else if (n.startsWith("start_of_")) open("part", value);
      else if (n.startsWith("end_of_") || /^eo[cvb]$/.test(n)) {
        end();
        cur = null;
      }
      continue;
    }
    if (!line.trim()) {
      end();
      continue;
    }
    let plain = "";
    const chords = [];
    for (const part of line.split(/(\[[^\]]*\])/)) {
      if (part.startsWith("[") && part.endsWith("]")) chords.push({ at: [...plain].length, chord: part.slice(1, -1) });
      else plain += part;
    }
    stanza.push({ runs: plain ? [{ text: plain }] : undefined, ...(chords.length ? { chords } : {}) });
  }
  end();
  return out;
}

const textOf = (p) => (p.runs ?? []).map((r) => r.text).join("");

/** One stanza → [first-language paragraphs, second-language lines] (as the server's split). */
function splitStanza(paras, notes, split) {
  const t = split?.type ?? "none";
  if (t === "comments") return [paras, notes];
  if (t === "alternating") {
    const a = [];
    const b = [];
    let n = 0;
    for (const p of paras) {
      if (!textOf(p).trim()) {
        if (p.chords) a.push(p);
        continue;
      }
      if (n++ % 2 === 0) a.push(p);
      else b.push(textOf(p).trim());
    }
    return [a, b];
  }
  const sep = t === "line" ? "---" : t === "inline" ? (split.separator ?? "").trim() : "";
  if (!sep) return [paras, []];
  const a = [];
  const b = [];
  let below = false;
  for (const p of paras) {
    const text = textOf(p);
    const at = text.indexOf(sep);
    if (text.trim() === sep) below = true;
    else if (at >= 0) {
      const first = text.slice(0, at).trimEnd();
      const chords = (p.chords ?? []).filter((c) => c.at <= [...first].length);
      a.push({ runs: [{ text: first }], ...(chords.length ? { chords } : {}) });
      if (text.slice(at + sep.length).trim()) b.push(text.slice(at + sep.length).trim());
    } else if (below) {
      if (text.trim()) b.push(text.trim());
    } else a.push(p);
  }
  return [a, b];
}

export default function chordsRoutes({ json, apiError }) {
  return (req, res, { parts }) => {
    if (req.method === "POST" && parts[0] === "chordpro" && parts[1] === "parse") {
      let body = "";
      req.on("data", (d) => (body += d));
      req.on("end", () => {
        const { text = "", linesPerSlide = 0, split } = JSON.parse(body || "{}");
        const song = parse(text);
        for (const s of song.sections) {
          const pairs = s.stanzas.map((st, i) => splitStanza(st, s.notes[i] ?? [], split));
          s.stanzas = pairs.map(([a]) => a);
          s.seconds = pairs.map(([, b]) => b);
        }
        if (!song.sections.length) return apiError(res, 400, "badRequest", "the ChordPro text has no lyric lines");
        let n = 0;
        const sections = song.sections.map((s) => ({
          id: uuid((n += 1)),
          name: s.name,
          // (Lines per slide: the second language stays with the stanza's first slide here.)
          slides: s.stanzas
            .flatMap((st, k) => (linesPerSlide ? Array.from({ length: Math.ceil(st.length / linesPerSlide) }, (_, i) => [st.slice(i * linesPerSlide, (i + 1) * linesPerSlide), i ? [] : s.seconds[k]]) : [[st, s.seconds[k]]]))
            .map(([paras, second]) => {
              const el = uuid((n += 2));
              return {
                id: uuid(n - 1),
                elements: [{ id: el, name: "Lyrics", bounds: { x: 160, y: 240, width: 1600, height: 600 }, kind: { type: "text", text: { paragraphs: paras } } }],
                ...(second.length ? { translation: { ...(song.second ? { language: song.second } : {}), texts: [{ element: el, paragraphs: second.map((l) => ({ runs: [{ text: l }] })) }] } } : {}),
              };
            }),
        }));
        const slides = sections.reduce((k, s) => k + s.slides.length, 0);
        const chordLines = song.sections.flatMap((s) => s.stanzas.flat()).filter((p) => p.chords).length;
        json(res, 200, {
          presentation: {
            schemaVersion: 2,
            id: uuid(999),
            name: song.title ?? "Untitled",
            ...(song.key ? { songInfo: { title: song.title ?? undefined, originalKey: song.key } } : {}),
            canvasSize: { width: 1920, height: 1080 },
            sections,
          },
          sections: sections.length,
          slides,
          chordLines,
          secondLanguageSlides: sections.flatMap((x) => x.slides).filter((x) => x.translation).length,
          unpairedStanzas: linesPerSlide ? song.sections.flatMap((s) => s.stanzas.filter((st, k) => st.length > linesPerSlide && s.seconds[k]?.length && s.seconds[k].length !== st.length)).length : 0,
        });
      });
      return true;
    }
    if (req.method === "GET" && parts[0] === "presentations" && parts[2] === "chordpro") {
      res.writeHead(200, { "Content-Type": "text/plain; charset=utf-8", "Content-Disposition": 'attachment; filename="song.cho"' });
      res.end("{title: Morning Light}\n{key: G}\n\n{start_of_verse: Verse 1}\n[G]The hills are [C]waking [Em7]slowly\nA [D/F#]lantern on the [G]shore\n{end_of_verse}\n");
      return true;
    }
    return false;
  };
}
