// The Timers page's stub routes: while `timersStub.on`, the timers run as during a service (a
// countdown running, one inside its amber warning time, an elapsed timer counting up, a
// countdown over time) and timer documents
// can be created, changed and deleted in memory. Off by default, so nothing else changes;
// `scripts/shots/timers.mjs` turns it on. Names are made up.

// Every stub started in this process (a shot may start a second one, e.g. the Library's Codex
// box): start / stop act on all of them.
const instances = new Set();
export const timersStub = {
  on: false,
  start: () => instances.forEach((i) => i.start()),
  stop: () => instances.forEach((i) => i.stop()),
};

const WALK_IN = "5eed0000-0000-4000-8000-00000000a001";
const OFFERING = "5eed0000-0000-4000-8000-00000000a002";

export default function timersRoutes({ fx, state, bump, json, apiError }) {
  let docs = structuredClone(fx.get("timers"));
  let saved = null;
  let rev = 1;
  const nextRev = () => `stub-${(rev += 1)}`;

  const inst = {};
  instances.add(inst);
  inst.start = () => {
    timersStub.on = true;
    docs = structuredClone(fx.get("timers"));
    docs.push({ revision: nextRev(), schemaVersion: 2, id: WALK_IN, name: "Walk-in Countdown", sortKey: "z", kind: { type: "countdown", seconds: 300 }, allowOverrun: true });
    docs.push({ revision: nextRev(), schemaVersion: 2, id: OFFERING, name: "Offering Song", sortKey: "zz", kind: { type: "countdown", seconds: 240 }, warningSeconds: 120 });
    saved = structuredClone(state.state.timers);
    for (const t of state.state.timers) {
      if (t.kind.type === "elapsed") Object.assign(t, { state: "paused", currentSeconds: 728, display: "12:08" });
      if (t.kind.type === "countdown") Object.assign(t, { state: "running", currentSeconds: 1695, display: "28:15" });
    }
    state.state.timers.push({ id: WALK_IN, name: "Walk-in Countdown", kind: { type: "countdown", seconds: 300 }, allowOverrun: true, state: "overrun", currentSeconds: -42, display: "-0:42", warningSeconds: null });
    state.state.timers.push({ id: OFFERING, name: "Offering Song", kind: { type: "countdown", seconds: 240 }, allowOverrun: false, state: "running", currentSeconds: 95, display: "1:35", warningSeconds: 120 });
    bump();
  };
  inst.stop = () => {
    timersStub.on = false;
    if (saved) state.state.timers = saved;
    saved = null;
    bump();
  };

  return (req, res, { parts }) => {
    if (!timersStub.on || parts[0] !== "timers") return false;
    if (req.method === "GET" && parts.length === 1) {
      json(res, 200, docs);
      return true;
    }
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      const doc = body ? JSON.parse(body) : {};
      if (req.method === "POST" && parts.length === 1) {
        const e = { schemaVersion: 2, ...doc, id: `5eed0000-0000-4000-8000-${String(Date.now()).slice(-12)}`, revision: nextRev() };
        docs.push(e);
        state.state.timers.push({ id: e.id, name: e.name, kind: e.kind ?? { type: "countdown", seconds: 300 }, allowOverrun: !!e.allowOverrun, state: "stopped", currentSeconds: 300, display: "5:00" });
        bump();
        return json(res, 201, e);
      }
      const i = docs.findIndex((d) => d.id === parts[1]);
      if (i < 0) return apiError(res, 404, "notFound", "no such timer");
      if (req.method === "PUT") {
        docs[i] = { ...doc, revision: nextRev() };
        const s = state.state.timers.find((t) => t.id === parts[1]);
        if (s) Object.assign(s, { name: docs[i].name, kind: docs[i].kind, allowOverrun: !!docs[i].allowOverrun, warningSeconds: docs[i].warningSeconds ?? null });
        bump();
        return json(res, 200, docs[i]);
      }
      if (req.method === "DELETE") {
        docs.splice(i, 1);
        state.state.timers = state.state.timers.filter((t) => t.id !== parts[1]);
        bump();
        return json(res, 204);
      }
      return apiError(res, 405, "methodNotAllowed", "not here");
    });
    return true;
  };
}
