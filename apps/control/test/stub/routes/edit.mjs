// The slide editor's stub routes: a synthetic presentation whose slides have several objects
// (a backdrop, a title, a subtitle, shapes, a lower bar, an image), editable in memory, and
// renders drawn from the document (so a moved, restyled or hidden object shows in the shots).
//
// Off by default, so nothing else changes. `scripts/shots/edit.mjs` imports `editStub` and turns
// it on: while `on`, the presentation stands in for the one the Show page has open (the same ID
// as the Reflow stub's song), with its own renders, thumbnails and edits (applied to a copy;
// `inverse` is empty, so undo does nothing here). All names and text are made up.

export const editStub = {
  on: false,
  /** The presentation's ID (the one it stands in for). */
  id: "ac7a4ae2-b205-4187-a614-8f7dbd67faec",
  /** Back to the document as built below. */
  reset: () => {},
  /** Every edit received since the last reset, `{ops}` (the shots check what was sent). */
  edits: [],
};

const C = (r, g, b) => ({ r, g, b });
const WHITE = C(1, 1, 1);
const id = (n) => `e0000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const sid = (n) => `51000000-0000-4000-8000-${String(n).padStart(12, "0")}`;

function text(n, name, bounds, lines, style, extra = {}) {
  return {
    id: id(n),
    name,
    bounds,
    kind: {
      type: "text",
      shape: { type: "rectangle" },
      text: {
        defaultStyle: { fontFamily: "Inter", fontWeight: 400, size: 64, color: WHITE, ...style },
        defaultParagraph: { alignment: extra.align ?? "center" },
        verticalAlign: "middle",
        paragraphs: lines.map((l) => ({ runs: [{ text: l }] })),
      },
    },
    ...(extra.el ?? {}),
  };
}
/** A text object with chords over its lines (batch 16): `[[at, chord], …]` per line. */
function withChords(el, lines) {
  el.kind.text.paragraphs.forEach((p, i) => {
    const c = lines[i];
    if (c?.length) p.chords = c.map(([at, chord]) => ({ at, chord }));
  });
  return el;
}
function shape(n, name, bounds, type, color, extra = {}) {
  return { id: id(n), name, bounds, kind: { type: "shape", shape: { type } }, fill: { enabled: true, kind: { type: "color", color } }, ...extra };
}

function build() {
  return {
    id: editStub.id,
    name: "Morning Light",
    canvasSize: { width: 1920, height: 1080 },
    transition: { kind: { type: "dissolve" }, duration: 0.5 },
    songInfo: { title: "Morning Light", authors: "A. Sample (words and music)", publisher: "Example Songs", copyrightYear: 2024, originalKey: "G", transpose: 2 },
    sections: [
      {
        id: sid(1),
        name: "Title",
        color: C(0.23, 0.24, 0.27),
        slides: [
          {
            id: sid(11),
            background: C(0.06, 0.07, 0.12),
            elements: [
              shape(1, "Backdrop glow", { x: 0, y: 0, width: 1920, height: 1080 }, "rectangle", { r: 0.18, g: 0.16, b: 0.42, a: 0.55 }),
              shape(2, "Sun", { x: 1380, y: 120, width: 360, height: 360 }, "ellipse", C(0.96, 0.65, 0.14), { opacity: 0.9 }),
              text(3, "Title", { x: 160, y: 330, width: 1600, height: 220 }, ["Morning Light"], { size: 150, fontWeight: 700 }),
              text(4, "Subtitle", { x: 360, y: 560, width: 1200, height: 110 }, ["Words and music by A. Sample"], { size: 54, color: C(0.79, 0.75, 1) }),
              // Shown only while the playlist has a next item (a visibility condition, plan G16).
              shape(5, "Lower bar", { x: 0, y: 900, width: 1920, height: 110 }, "rectangle", C(0.43, 0.36, 0.9), { visibility: [{ type: "playlistItem", which: "next" }] }),
              text(6, "Lower bar text", { x: 80, y: 905, width: 1100, height: 100 }, ["Welcome — find a seat"], { size: 48, fontWeight: 600 }, { align: "left" }),
              {
                id: id(7),
                name: "Colour bars",
                bounds: { x: 140, y: 120, width: 400, height: 225 },
                aspectLocked: true,
                kind: { type: "shape", shape: { type: "rectangle" } },
                fill: { enabled: true, kind: { type: "media", asset: "5a6cab40-8854-44e7-940c-fa780a2a1fc0", display: { scale: "fit" } } },
              },
            ],
            notes: { paragraphs: [{ runs: [{ text: "Band starts on the second chord." }] }] },
            // Object builds (plan G18): the subtitle on Next, then the lower bar pushed up with its text after it.
            builds: [
              { element: id(4), transition: { kind: { type: "dissolve" }, duration: 0.5 } },
              { element: id(5), transition: { kind: { type: "push", direction: "up" }, duration: 0.6, easing: "easeInOut" } },
              { element: id(6), start: "afterPrevious", delay: 0.2, transition: { kind: { type: "dissolve" }, duration: 0.4 } },
            ],
          },
        ],
      },
      {
        id: sid(2),
        name: "Verse 1",
        color: C(0.18, 0.37, 0.72),
        slides: [
          {
            id: sid(21),
            elements: [
              withChords(text(11, "Lyrics", { x: 160, y: 240, width: 1600, height: 600 }, ["The hills are waking slowly", "A lantern on the shore"], { size: 84 }), [
                [
                  [0, "G"],
                  [10, "C"],
                  [18, "Em7"],
                ],
                [
                  [2, "D/F#"],
                  [13, "G"],
                ],
              ]),
              shape(12, "Rule", { x: 760, y: 860, width: 400, height: 8 }, "rectangle", C(0.96, 0.65, 0.14)),
            ],
          },
          {
            id: sid(22),
            elements: [text(13, "Lyrics", { x: 160, y: 240, width: 1600, height: 600 }, ["We follow every footstep", "Into the open door"], { size: 84 })],
          },
        ],
      },
      {
        id: sid(3),
        name: "Chorus",
        color: C(0.12, 0.62, 0.56),
        slides: [
          {
            id: sid(31),
            elements: [text(21, "Lyrics", { x: 160, y: 240, width: 1600, height: 600 }, ["Morning light, morning light", "Carry us home tonight"], { size: 84, fontWeight: 600 })],
          },
        ],
      },
    ],
  };
}

const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const rgba = (c, o = 1) => (c ? `rgba(${Math.round(c.r * 255)},${Math.round(c.g * 255)},${Math.round(c.b * 255)},${(c.a ?? 1) * o})` : "none");

/** A slide drawn from its objects (rectangles, ellipses, text lines, colour bars for images). */
function render(slide, w, hide) {
  const k = w / 1920;
  const h = Math.round(1080 * k);
  const out = [`<rect width="${w}" height="${h}" fill="${rgba(slide.background ?? C(0, 0, 0))}"/>`];
  for (const e of slide.elements ?? []) {
    if (hide.has(e.id) || e.hidden) continue;
    const b = e.bounds ?? { x: 0, y: 0, width: 0, height: 0 };
    const [x, y, bw, bh] = [b.x * k, b.y * k, b.width * k, b.height * k];
    const op = e.opacity ?? 1;
    const rot = e.rotation ? ` transform="rotate(${e.rotation} ${x + bw / 2} ${y + bh / 2})"` : "";
    const fill = e.fill?.enabled === true ? e.fill?.kind : null;
    const stroke = e.stroke?.enabled ? ` stroke="${rgba(e.stroke.color)}" stroke-width="${(e.stroke.width ?? 3) * k}"` : "";
    if (fill?.type === "media") {
      const cols = ["#C0C0C0", "#C0C000", "#00C0C0", "#00C000", "#C000C0", "#C00000", "#0000C0"];
      out.push(`<g opacity="${op}"${rot}>${cols.map((c, i) => `<rect x="${x + (i * bw) / 7}" y="${y}" width="${bw / 7 + 0.5}" height="${bh}" fill="${c}"/>`).join("")}</g>`);
      continue;
    }
    const shapeType = e.kind?.shape?.type ?? "rectangle";
    if (fill?.type === "color" || stroke) {
      const f = fill?.type === "color" ? rgba(fill.color) : "none";
      out.push(
        shapeType === "ellipse"
          ? `<ellipse cx="${x + bw / 2}" cy="${y + bh / 2}" rx="${bw / 2}" ry="${bh / 2}" fill="${f}" opacity="${op}"${stroke}${rot}/>`
          : `<rect x="${x}" y="${y}" width="${bw}" height="${bh}" fill="${f}" opacity="${op}"${stroke}${rot}/>`,
      );
    }
    if (e.kind?.type === "text") {
      const tb = e.kind.text ?? {};
      const paras = tb.paragraphs ?? [];
      const lineH = (p) => ((p.runs?.[0]?.style?.size ?? tb.defaultStyle?.size ?? 64) * 1.25 * k);
      const total = paras.reduce((s, p) => s + lineH(p), 0);
      let at = y + (bh - total) / 2;
      for (const p of paras) {
        const st = { ...(tb.defaultStyle ?? {}), ...(p.runs?.[0]?.style ?? {}) };
        const al = p.style?.alignment ?? tb.defaultParagraph?.alignment ?? "center";
        const tx = al === "left" ? x : al === "right" ? x + bw : x + bw / 2;
        const anchor = al === "left" ? "start" : al === "right" ? "end" : "middle";
        const size = (st.size ?? 64) * k;
        let t = (p.runs ?? []).map((r) => r.text ?? "").join("");
        if (st.capitalization === "allCaps") t = t.toUpperCase();
        const deco = [st.underline && st.underline !== "none" ? "underline" : "", st.strikethrough && st.strikethrough !== "none" ? "line-through" : ""].filter(Boolean).join(" ");
        const outline = tb.outline?.enabled ? ` stroke="${rgba(tb.outline.color)}" stroke-width="${(tb.outline.width ?? 3) * k}" paint-order="stroke"` : "";
        at += lineH(p);
        out.push(
          `<text x="${tx}" y="${at - size * 0.32}" text-anchor="${anchor}" font-family="Inter, 'DejaVu Sans', sans-serif" font-size="${size}" font-weight="${st.fontWeight ?? 400}"${st.italic ? ' font-style="italic"' : ""}${deco ? ` text-decoration="${deco}"` : ""} fill="${rgba(st.color ?? WHITE)}" opacity="${op}"${outline}${rot}>${esc(t)}</text>`,
        );
      }
    }
  }
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">${out.join("")}</svg>`;
}

export default function editRoutes({ json, svg, apiError }) {
  let doc = build();
  let rev = 1;
  editStub.reset = () => {
    doc = build();
    rev += 1;
    editStub.edits = [];
  };

  const slides = () => doc.sections.flatMap((s) => s.slides);
  const find = (s) => slides().find((x) => x.id === s);
  const elOf = (op) => find(op.slide)?.elements.find((e) => e.id === op.element);
  const merge = (to, from) => {
    for (const [key, v] of Object.entries(from ?? {})) {
      if (v === null) delete to[key];
      else if (v !== undefined) to[key] = v;
    }
    asServerSends(to);
  };
  /** As the server sends an element: a switched-off fill without `enabled` (H09-1). */
  const asServerSends = (e) => {
    if (e?.fill && e.fill.enabled === false) {
      e.fill = { ...e.fill };
      delete e.fill.enabled;
    }
  };

  /** Applies one op; returns an error message for an op the stub can't do. */
  function apply(op) {
    switch (op.type) {
      case "setElement": {
        const e = elOf(op);
        if (!e) return "no such element";
        merge(e, op.patch);
        return null;
      }
      case "addElement": {
        const s = find(op.slide);
        if (!s) return "no such slide";
        s.elements.push(structuredClone(op.element));
        asServerSends(s.elements.at(-1));
        return null;
      }
      case "deleteElement": {
        const s = find(op.slide);
        if (!s) return "no such slide";
        s.elements = s.elements.filter((e) => e.id !== op.element);
        return null;
      }
      case "reorderElement": {
        const s = find(op.slide);
        const at = s?.elements.findIndex((e) => e.id === op.element) ?? -1;
        if (at < 0) return "no such element";
        const [e] = s.elements.splice(at, 1);
        s.elements.splice(Math.min(op.index, s.elements.length), 0, e);
        return null;
      }
      case "setTextStyle": {
        const e = elOf(op);
        if (e?.kind?.type !== "text") return "not a text element";
        const tb = e.kind.text;
        if (!op.range) {
          tb.defaultStyle = { ...(tb.defaultStyle ?? {}), ...(op.character ?? {}) };
          tb.defaultParagraph = { ...(tb.defaultParagraph ?? {}), ...(op.paragraph ?? {}) };
        } else {
          for (const p of tb.paragraphs ?? []) {
            if (op.paragraph) p.style = { ...(p.style ?? {}), ...op.paragraph };
            for (const r of p.runs ?? []) if (op.character) r.style = { ...(r.style ?? {}), ...op.character };
          }
        }
        return null;
      }
      case "setTextBox": {
        const e = elOf(op);
        if (e?.kind?.type !== "text") return "not a text element";
        const { type: _t, slide: _s, element: _e, ...fields } = op;
        merge(e.kind.text, fields);
        return null;
      }
      case "replaceTextBox": {
        const e = elOf(op);
        if (e?.kind?.type !== "text") return "not a text element";
        e.kind.text = structuredClone(op.textBox);
        return null;
      }
      case "setSlideText":
        return null; // the precondition of a text save; the stub doesn't check it
      case "deleteSlide": {
        const sec = doc.sections.find((x) => x.slides.some((y) => y.id === op.slide));
        if (!sec) return "no such slide";
        sec.slides = sec.slides.filter((y) => y.id !== op.slide);
        return null;
      }
      case "setSlide": {
        const s = find(op.slide);
        if (!s) return "no such slide";
        const { type: _t, slide: _s, ...fields } = op;
        merge(s, fields);
        return null;
      }
      case "setPresentation": {
        const { type: _t, ...fields } = op;
        merge(doc, fields);
        return null;
      }
      case "setSection": {
        const s = doc.sections.find((x) => x.id === op.section);
        if (!s) return "no such section";
        const { type: _t, section: _s, ...fields } = op;
        merge(s, fields);
        return null;
      }
      case "applyChordPro":
        // The server puts the chords on matching lines; the stub keeps the song as it is.
        return null;
      default:
        return `the stub can't apply ${op.type}`;
    }
  }

  return (req, res, { parts, query }) => {
    if (!editStub.on || parts[0] !== "presentations" || parts[1] !== editStub.id) return false;
    if (req.method === "GET" && parts.length === 2) {
      json(res, 200, doc);
      return true;
    }
    if (req.method === "GET" && parts[2] === "slides" && (parts[4] === "thumbnail" || parts[4] === "render")) {
      const w = Math.min(1920, Math.max(16, Number(query.get("w")) || 320));
      const s = find(parts[3]);
      const hide = new Set((query.get("hide") ?? "").split(",").filter(Boolean));
      svg(res, s ? render(s, w, hide) : `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${Math.round((w * 9) / 16)}"/>`);
      return true;
    }
    if (req.method === "POST" && parts[2] === "thumbnails") {
      const n = slides().length;
      json(res, 202, { slides: n, cached: n, queued: 0 });
      return true;
    }
    if (req.method === "POST" && parts[2] === "edits") {
      let body = "";
      req.on("data", (d) => (body += d));
      req.on("end", () => {
        let edit;
        try {
          edit = JSON.parse(body);
        } catch {
          return apiError(res, 400, "badRequest", "not JSON");
        }
        editStub.edits.push({ ops: edit.ops ?? [] });
        const before = structuredClone(doc);
        for (const op of edit.ops ?? []) {
          const err = apply(op);
          if (err) {
            doc = before;
            return apiError(res, 400, "badRequest", err);
          }
        }
        rev += 1;
        json(res, 200, { revision: `r${rev}`, applied: edit.ops ?? [], inverse: [] });
      });
      return true;
    }
    return false;
  };
}
