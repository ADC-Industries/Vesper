// Presence and edit locks (G8): `GET /presence`, `POST /presence/locks` (take / take over) and
// `DELETE /presence/locks/{kind}/{id}`, with a list the shots set (`presenceStub.set(list)`,
// sent as `presenceChanged`). Empty by default, so nothing else changes.
//
// Like the server, an edit (`POST /presentations/{id}/edits`, `/themes/{id}/edits`,
// `/stage-layouts/{id}/edits`) of a document whose lock another client holds answers
// `423 locked`, and so does a `PUT` / `DELETE` of a timer or prop someone else holds; this file's
// routes run before the Reflow, Edit and Timers stubs (file name order).
// `presenceStub.me` is the `X-Client-Id` Oriel sends (its edits' clientId), seen on any request;
// `presenceStub.reported` is what Oriel last said it has open (the stub's WebSocket keeps it).

export const presenceStub = {
  list: { clients: [], locks: [] },
  /** Oriel's clientId (from its requests). */
  me: null,
  /** What Oriel last reported open (its `presence` message's `open`). */
  reported: [],
  /** Replaces the list and tells every client. */
  set: (_list) => {},
};

const KIND = { presentations: "presentation", themes: "theme", "stage-layouts": "stageLayout", props: "prop" };

export default function presenceRoutes({ json, push, apiError }) {
  presenceStub.list = { clients: [], locks: [] };
  presenceStub.set = (list) => {
    presenceStub.list = list;
    push({ type: "presenceChanged", presence: list });
  };
  const label = (l) => (l.user && l.computer ? `${l.user} on ${l.computer}` : (l.computer ?? l.name));

  return (req, res, { path, parts }) => {
    const me = req.headers["x-client-id"];
    if (me) presenceStub.me = me;
    if (req.method === "GET" && path === "/presence") {
      json(res, 200, presenceStub.list);
      return true;
    }
    if (req.method === "POST" && path === "/presence/locks") {
      let body = "";
      req.on("data", (d) => (body += d));
      req.on("end", () => {
        const b = JSON.parse(body || "{}");
        const held = presenceStub.list.locks.find((l) => l.kind === b.kind && l.id === b.id);
        if (held && held.clientId !== me && !b.takeOver) return apiError(res, 423, "locked", `${label(held)} is editing this ${b.kind}`);
        const lock = { kind: b.kind, id: b.id, clientId: me ?? "oriel", name: "Oriel – Booth A", computer: "Booth A", since: new Date().toISOString() };
        presenceStub.set({ ...presenceStub.list, locks: [...presenceStub.list.locks.filter((l) => l !== held), lock] });
        json(res, 200, lock);
      });
      return true;
    }
    if (req.method === "DELETE" && parts[0] === "presence" && parts[1] === "locks") {
      presenceStub.set({ ...presenceStub.list, locks: presenceStub.list.locks.filter((l) => !(l.kind === parts[2] && l.id === parts[3] && l.clientId === me)) });
      res.writeHead(204, { "Access-Control-Allow-Origin": "*" });
      res.end();
      return true;
    }
    // Edits of a document someone else holds: refused, as by the server.
    if (req.method === "POST" && KIND[parts[0]] && parts[2] === "edits") {
      const held = presenceStub.list.locks.find((l) => l.kind === KIND[parts[0]] && l.id === parts[1]);
      if (held && held.clientId !== me) {
        req.resume();
        apiError(res, 423, "locked", `${label(held)} is editing this presentation: take over editing to change it`);
        return true;
      }
    }
    // A timer's or prop's settings while someone else holds its lock: refused too.
    if ((req.method === "PUT" || req.method === "DELETE") && (parts[0] === "timers" || parts[0] === "props") && parts.length === 2) {
      const kind = parts[0] === "timers" ? "timer" : "prop";
      const held = presenceStub.list.locks.find((l) => l.kind === kind && l.id === parts[1]);
      if (held && held.clientId !== me) {
        req.resume();
        apiError(res, 423, "locked", `${label(held)} is editing this ${kind}: take over editing to change it`);
        return true;
      }
    }
    return false;
  };
}
