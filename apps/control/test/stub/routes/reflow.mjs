// The Reflow page's stub routes: a synthetic song with several groups, hotkeys and media cues
// (`fixtures/reflow/reflow-song.json`), editable in memory, and a scripted edit conflict.
//
// Off by default, so nothing else changes. `scripts/shots/reflow.mjs` imports `reflowStub` and
// turns it on: while `on`, the song stands in for the presentation it names (the one the Show
// page opens), with its own thumbnails and edits (applied to a copy; `inverse` is empty, so
// undo does nothing here). With `conflictText` set, the next `setSlideText` that carries
// `expectText` answers `409 conflict` with that text as the slide's current text (and keeps it).

export const reflowStub = {
  on: false,
  /** The text "someone else" saved: the next checked `setSlideText` conflicts with it. */
  conflictText: null,
  /** The song's ID (the presentation it stands in for). */
  id: null,
  /** Back to the song as in the fixture. */
  reset: () => {},
  /** The song as edited now (routes/translate.mjs reads its slides). */
  doc: () => null,
};

const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const hex = (c) => (c ? "#" + [c.r, c.g, c.b].map((v) => Math.round(v * 255).toString(16).padStart(2, "0")).join("") : "#3A3D46");

function primary(slide) {
  return (slide?.elements ?? []).find((e) => e.kind?.type === "text") ?? null;
}
function lines(slide) {
  return (primary(slide)?.kind.text.paragraphs ?? []).map((p) => (p.runs ?? []).map((r) => r.text ?? "").join(""));
}
function setLines(slide, ls) {
  let el = primary(slide);
  if (!el) {
    el = { id: `${slide.id.slice(0, 24)}text00000001`, name: "Text", bounds: { x: 96, y: 54, width: 1728, height: 972 }, kind: { type: "text", text: { paragraphs: [] } } };
    slide.elements = [...(slide.elements ?? []), el];
  }
  el.kind.text.paragraphs = ls.map((l) => (l ? { runs: [{ text: l }] } : {}));
}

/** A thumbnail: the slide's lines in white on a dark gradient tinted by its group's colour. */
function picture(ls, colour, w) {
  const h = Math.round((w * 9) / 16);
  const size = Math.max(10, Math.round(w / 22));
  const gap = size * 1.3;
  const top = h / 2 - ((ls.length - 1) * gap) / 2 + size * 0.35;
  const text = ls.map((l, i) => `<text x="${w / 2}" y="${(top + i * gap).toFixed(1)}" text-anchor="middle">${esc(l)}</text>`).join("");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${colour}" stop-opacity="0.55"/><stop offset="0.6" stop-color="#121317"/><stop offset="1" stop-color="#0B0C0F"/></linearGradient></defs><rect width="${w}" height="${h}" fill="url(#g)"/><g fill="#FFFFFF" font-family="Inter, 'DejaVu Sans', sans-serif" font-size="${size}" font-weight="600">${text}</g></svg>`;
}

export default function reflowRoutes({ fx, json, svg, apiError }) {
  const original = fx.get("reflow-song");
  let doc = structuredClone(original);
  let rev = 1;
  reflowStub.id = original.id;
  reflowStub.reset = () => {
    doc = structuredClone(original);
    rev += 1;
  };
  reflowStub.doc = () => doc;

  const find = (sid) => {
    for (const sec of doc.sections) {
      const k = sec.slides.findIndex((s) => s.id === sid);
      if (k >= 0) return { sec, k, slide: sec.slides[k] };
    }
    return null;
  };
  const sectionOf = (id) => doc.sections.find((s) => s.id === id);

  /** Applies one op; returns an error message for an op the stub can't do. */
  function apply(op, i) {
    switch (op.type) {
      case "setSlideText": {
        const hit = find(op.slide);
        if (!hit) return "no such slide";
        const now = lines(hit.slide).join("\n");
        if (op.expectText !== undefined && reflowStub.conflictText !== null) {
          const theirs = reflowStub.conflictText;
          reflowStub.conflictText = null;
          rev += 1;
          return { conflict: { revision: `r${rev}`, opIndex: i, slide: op.slide, text: theirs }, theirs };
        }
        if (op.expectText !== undefined && op.expectText !== now) return { conflict: { revision: `r${rev}`, opIndex: i, slide: op.slide, text: now } };
        setLines(hit.slide, op.text.split("\n"));
        return null;
      }
      case "splitSlide": {
        const hit = find(op.slide);
        if (!hit) return "no such slide";
        const ls = lines(hit.slide);
        const fresh = { id: `${op.slide.slice(0, 24)}${String(rev * 100 + i).padStart(12, "0")}`, elements: [] };
        setLines(fresh, ls.slice(op.atParagraph));
        setLines(hit.slide, ls.slice(0, op.atParagraph));
        hit.sec.slides.splice(hit.k + 1, 0, fresh);
        return null;
      }
      case "mergeSlides": {
        const a = find(op.first);
        const b = find(op.second);
        if (!a || !b) return "no such slide";
        setLines(a.slide, [...lines(a.slide), ...lines(b.slide)]);
        a.slide.actions = [...(a.slide.actions ?? []), ...(b.slide.actions ?? [])];
        b.sec.slides.splice(b.k, 1);
        return null;
      }
      case "deleteSlide": {
        const hit = find(op.slide);
        if (!hit) return "no such slide";
        hit.sec.slides.splice(hit.k, 1);
        return null;
      }
      case "moveSlide": {
        const hit = find(op.slide);
        const to = sectionOf(op.toSection);
        if (!hit || !to) return "no such slide or section";
        hit.sec.slides.splice(hit.k, 1);
        const at = op.after ? to.slides.findIndex((s) => s.id === op.after) + 1 : 0;
        to.slides.splice(at, 0, hit.slide);
        return null;
      }
      case "addSlide": {
        const to = sectionOf(op.section);
        if (!to) return "no such section";
        const at = op.after ? to.slides.findIndex((s) => s.id === op.after) + 1 : to.slides.length;
        to.slides.splice(at, 0, op.slide ?? { id: `${op.section.slice(0, 24)}${String(rev * 100 + i).padStart(12, "0")}`, elements: [] });
        return null;
      }
      case "addSection": {
        const at = op.after ? doc.sections.findIndex((s) => s.id === op.after) + 1 : doc.sections.length;
        doc.sections.splice(at, 0, { slides: [], ...op.section });
        return null;
      }
      case "deleteSection": {
        const at = doc.sections.findIndex((s) => s.id === op.section);
        if (at < 0) return "no such section";
        doc.sections.splice(at, 1);
        return null;
      }
      case "setSection": {
        const s = sectionOf(op.section);
        if (!s) return "no such section";
        for (const k of ["name", "color", "hotkey", "globalGroup"]) {
          if (op[k] === null) delete s[k];
          else if (op[k] !== undefined) s[k] = op[k];
        }
        return null;
      }
      case "removeSlideAction": {
        const hit = find(op.slide);
        if (!hit) return "no such slide";
        hit.slide.actions = (hit.slide.actions ?? []).filter((a) => a.id !== op.action);
        return null;
      }
      case "setPresentation": {
        // Presentation fields (the Inspector's Language): `null` clears.
        for (const [k, v] of Object.entries(op)) {
          if (k === "type" || v === undefined) continue;
          if (v === null) delete doc[k];
          else doc[k] = v;
        }
        return null;
      }
      case "setSlideTranslation": {
        const hit = find(op.slide);
        if (!hit) return "no such slide";
        if (op.translation) hit.slide.translation = op.translation;
        else delete hit.slide.translation;
        return null;
      }
      default:
        return `the stub can't apply ${op.type}`;
    }
  }

  return (req, res, { parts, query }) => {
    if (!reflowStub.on || parts[0] !== "presentations" || parts[1] !== original.id) return false;
    if (req.method === "GET" && parts.length === 2) {
      json(res, 200, doc);
      return true;
    }
    if (req.method === "GET" && parts[2] === "slides" && (parts[4] === "thumbnail" || parts[4] === "render")) {
      const w = Math.min(1920, Math.max(16, Number(query.get("w")) || 320));
      const hit = find(parts[3]);
      svg(res, picture(lines(hit?.slide), hex(hit?.sec.color), w));
      return true;
    }
    if (req.method === "POST" && parts[2] === "thumbnails") {
      const n = doc.sections.reduce((a, s) => a + s.slides.length, 0);
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
        const before = structuredClone(doc);
        for (const [i, op] of (edit.ops ?? []).entries()) {
          const err = apply(op, i);
          if (err?.conflict) {
            // All or nothing; "someone else's" text stays.
            doc = before;
            if (err.theirs !== undefined) setLines(find(op.slide).slide, err.theirs.split("\n"));
            return json(res, 409, { error: { code: "conflict", message: "the slide's text changed" }, current: err.conflict });
          }
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
