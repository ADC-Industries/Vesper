// SongSelect: `POST /songselect/parse` reads a lyrics text (base64 `data`, UTF-8 or UTF-16 LE
// with a mark, or `text`) into a synthetic presentation: the title on the first line, headings
// ("Verse 1", "Chorus", …) starting sections, the footer's writers, `CCLI Song #`, `©` and
// `CCLI License #` lines as the song's metadata. A small subset of the server's reader
// (model::songselect), enough for the dialog's preview. CCLI number 1234567 is "already in the
// library" (the fixture song "Morning Lantern"), so the duplicate flag shows. Always on;
// nothing is stored. Every song here is made up.

const uuid = (n) => `55000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const HEADING = /^(verse|chorus|pre-chorus|bridge|tag|intro|outro|ending|interlude|coro|estrofa|puente)(\s*\d+[a-z]?)?:?$/i;
const DUPLICATE = { number: "1234567", id: "ac7a4ae2-b205-4187-a614-8f7dbd67faec", name: "Morning Lantern" };

function decode(data) {
  const b = Buffer.from(data, "base64");
  if (b[0] === 0xff && b[1] === 0xfe) return b.subarray(2).toString("utf16le");
  if (b[0] === 0xef && b[1] === 0xbb && b[2] === 0xbf) return b.subarray(3).toString("utf8");
  return b.toString("utf8");
}

function parse(text) {
  const lines = text.replace(/\r\n?/g, "\n").split("\n").map((l) => l.trim());
  const song = { title: null, authors: null, ccli: null, year: null, owners: null, licence: null, sections: [] };
  let end = lines.length;
  while (end > 0) {
    const l = lines[end - 1];
    let m;
    if (!l) end--;
    else if ((m = /^CCLI Song\s*#?\s*(\d+)/i.exec(l))) (song.ccli = m[1]), end--;
    else if ((m = /^CCLI Licen[cs]e\s*#?\s*(\d+)/i.exec(l))) (song.licence = m[1]), end--;
    else if ((m = /^©\s*(\d{4})?\s*(.*)$/.exec(l))) (song.year = m[1] ? Number(m[1]) : null), (song.owners = m[2].split(/[|;]/).map((s) => s.trim()).filter(Boolean).join("; ")), end--;
    else if (/^For use solely with/i.test(l)) end--;
    else if (!song.authors && l.includes("|")) (song.authors = l.split("|").map((s) => s.trim()).join(", ")), end--;
    else break;
  }
  let i = lines.findIndex((l) => l);
  if (i >= 0 && i < end && !HEADING.test(lines[i])) song.title = lines[i++];
  let cur = null;
  for (const l of lines.slice(Math.max(i, 0), end)) {
    if (!l) {
      if (cur?.stanza.length) cur.stanzas.push(cur.stanza), (cur.stanza = []);
      continue;
    }
    if (HEADING.test(l)) {
      if (cur?.stanza.length) cur.stanzas.push(cur.stanza);
      cur = { name: l.replace(/:$/, ""), stanzas: [], stanza: [] };
      song.sections.push(cur);
      continue;
    }
    if (!cur) song.sections.push((cur = { name: `Verse ${song.sections.length + 1}`, stanzas: [], stanza: [] }));
    cur.stanza.push(l);
  }
  if (cur?.stanza.length) cur.stanzas.push(cur.stanza);
  song.sections = song.sections.filter((s) => s.stanzas.length);
  return song;
}

export default function songSelectRoutes({ json, apiError }) {
  return (req, res, { parts }) => {
    if (!(req.method === "POST" && parts[0] === "songselect" && parts[1] === "parse")) return false;
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      const { data, text, linesPerSlide = 0 } = JSON.parse(body || "{}");
      if (!data && !text) return apiError(res, 400, "badRequest", "give data or text");
      const song = parse(data ? decode(data) : text);
      if (!song.sections.length) return apiError(res, 400, "badRequest", "no lyrics found in the song");
      let n = 0;
      const sections = song.sections.map((s) => ({
        id: uuid((n += 1)),
        name: s.name,
        slides: s.stanzas
          .flatMap((st) => (linesPerSlide ? Array.from({ length: Math.ceil(st.length / linesPerSlide) }, (_, k) => st.slice(k * linesPerSlide, (k + 1) * linesPerSlide)) : [st]))
          .map((st) => ({
            id: uuid((n += 2)),
            elements: [{ id: uuid(n + 1), name: "Lyrics", bounds: { x: 160, y: 240, width: 1600, height: 600 }, kind: { type: "text", text: { paragraphs: st.map((l) => ({ runs: [{ text: l }] })) } } }],
          })),
      }));
      const songInfo = Object.fromEntries(
        Object.entries({ title: song.title, authors: song.authors, ccliNumber: song.ccli, publisher: song.owners, copyrightYear: song.year, display: !!song.ccli || undefined }).filter(([, v]) => v !== null && v !== undefined),
      );
      json(res, 200, {
        format: "lyrics",
        presentation: { schemaVersion: 2, id: uuid(9000 + (song.ccli ? Number(song.ccli) % 1000 : 0)), name: song.title ?? "Untitled", songInfo, canvasSize: { width: 1920, height: 1080 }, sections },
        sections: sections.length,
        slides: sections.reduce((k, s) => k + s.slides.length, 0),
        chordLines: 0,
        ...(song.licence ? { licenseNumber: song.licence } : {}),
        ...(song.ccli === DUPLICATE.number ? { duplicates: [{ id: DUPLICATE.id, name: DUPLICATE.name }] } : {}),
      });
    });
    return true;
  };
}
