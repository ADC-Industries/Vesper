// The Stage page's stub routes: while `stageStub.on`, a stage message is up and the stage
// layouts can be edited in memory (`POST /stage-layouts/{id}/edits`: the element ops the Stage
// page's checkboxes send; `inverse` is empty, so undo does nothing here). Off by default, so
// nothing else changes; `scripts/shots/stage.mjs` turns it on. The text is made up.

// Every stub started in this process (a shot may start a second one, e.g. the Library's Codex
// box): start / stop act on all of them.
const instances = new Set();
export const stageStub = {
  on: false,
  /** More layouts while on (the chords shots add a musician's layout, batch 16). */
  extra: [],
  start: () => instances.forEach((i) => i.start()),
  stop: () => instances.forEach((i) => i.stop()),
};

export default function stageRoutes({ fx, state, bump, json, apiError }) {
  let docs = structuredClone(fx.get("stage-layouts"));
  let rev = 1;

  const inst = {};
  instances.add(inst);
  inst.start = () => {
    stageStub.on = true;
    docs = structuredClone(fx.get("stage-layouts")).concat(structuredClone(stageStub.extra));
    state.state.stageMessage = { text: "Bridge twice, then out", flash: false };
    bump();
  };
  inst.stop = () => {
    stageStub.on = false;
    state.state.stageMessage = null;
    bump();
  };

  function apply(slide, op) {
    const els = (slide.elements ??= []);
    const at = els.findIndex((e) => e.id === op.element || e.id === op.element?.id);
    switch (op.type) {
      case "addElement":
        els.splice(op.index ?? els.length, 0, op.element);
        return true;
      case "deleteElement":
        if (at >= 0) els.splice(at, 1);
        return at >= 0;
      case "setElement":
        if (at >= 0) Object.assign(els[at], op.patch);
        return at >= 0;
      case "setTextStyle":
        if (at >= 0 && els[at].kind?.text) els[at].kind.text.defaultStyle = { ...els[at].kind.text.defaultStyle, ...op.character };
        return at >= 0;
      default:
        return false;
    }
  }

  return (req, res, { parts }) => {
    if (!stageStub.on || parts[0] !== "stage-layouts") return false;
    if (req.method === "GET" && parts.length === 1) {
      json(res, 200, docs);
      return true;
    }
    if (req.method === "PUT" && parts.length === 2) {
      // A whole-layout update (rename, the chord view): kept in memory.
      let body = "";
      req.on("data", (d) => (body += d));
      req.on("end", () => {
        const at = docs.findIndex((d) => d.id === parts[1]);
        if (at < 0) return apiError(res, 404, "notFound", "no such stage layout");
        docs[at] = { ...JSON.parse(body), revision: `stub-${(rev += 1)}` };
        json(res, 200, docs[at]);
      });
      return true;
    }
    if (req.method === "POST" && parts[2] === "edits") {
      let body = "";
      req.on("data", (d) => (body += d));
      req.on("end", () => {
        const doc = docs.find((d) => d.id === parts[1]);
        if (!doc) return apiError(res, 404, "notFound", "no such stage layout");
        const { ops } = JSON.parse(body);
        const slide = structuredClone(doc.slide);
        for (const op of ops) if (!apply(slide, op)) return apiError(res, 400, "badRequest", `the stub can't apply ${op.type}`);
        doc.slide = slide;
        doc.revision = `stub-${(rev += 1)}`;
        json(res, 200, { revision: doc.revision, applied: ops, inverse: [] });
      });
      return true;
    }
    return false;
  };
}
