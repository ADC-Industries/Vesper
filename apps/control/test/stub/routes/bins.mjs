// Media bins, bin playlists and global groups as documents (G17), for the Library's bins and
// groups screenshots: the bin tree and its bins (seeded from the captured fixtures) with their
// writes (`POST/PUT/DELETE /media-bins…`, item edits), a media playlist, and four made-up global
// groups with their writes (`/groups`). Off by default, so the other pages' screenshots keep the
// captured lists; `scripts/shots/bins.mjs` turns it on (`binsStub.on = true`) and off again.
// `binsStub.reset()` puts everything back as at start.

export const binsStub = { on: false, reset: () => {} };

const MEDIA_PLAYLIST = "5eed0000-0000-4000-8000-0000000b0001";
const groupId = (n) => `5eed0000-0000-4000-8000-0000000b10${String(n).padStart(2, "0")}`;
const rgb = (hex) => ({ r: parseInt(hex.slice(1, 3), 16) / 255, g: parseInt(hex.slice(3, 5), 16) / 255, b: parseInt(hex.slice(5, 7), 16) / 255 });
let seq = 0;
const newId = () => `5eed0000-0000-4000-8000-0000000b${String(2000 + ++seq).padStart(4, "0")}`;

const body = (req) =>
  new Promise((resolve) => {
    let b = "";
    req.on("data", (d) => (b += d));
    req.on("end", () => {
      try {
        resolve(JSON.parse(b || "{}"));
      } catch {
        resolve({});
      }
    });
  });

export default function binsRoutes({ fx, json, apiError }) {
  /** Bins as flat records {id, name, folder, browser, kind, items: MediaBinItemInfo[]}. */
  let bins = [];
  let groups = [];
  let playlist = null;

  function seed() {
    bins = [];
    const walk = (nodes) => {
      for (const n of nodes ?? []) {
        if (n.kind === "folder") walk(n.children);
        else if (n.id) {
          const d = fx.get(`media-bins__${n.id}`);
          bins.push({ id: n.id, name: n.name, folder: [...n.path], browser: n.browser, kind: n.kind, items: structuredClone(d?.items ?? []) });
        }
      }
    };
    walk(fx.get("media-bins"));
    bins.push({ id: "5eed0000-0000-4000-8000-0000000b0002", name: "Countdowns", folder: [], browser: "media", kind: "manual", items: [] });
    groups = [
      ["Verse", "#3f6fd8", "KeyV"],
      ["Chorus", "#c2453f", "KeyC"],
      ["Bridge", "#b7791f", "KeyB"],
      ["Tag", "#56606e", null],
    ].map(([name, color, code], i) => ({ schemaVersion: 2, id: groupId(i + 1), name, color: rgb(color), ...(code ? { hotkey: { code } } : {}), sortKey: String.fromCharCode(98 + i) }));
    playlist = {
      id: MEDIA_PLAYLIST,
      name: "Pre-service Loops",
      parent: null,
      purpose: "media",
      items: (bins.find((b) => b.items.some((i) => i.kind === "video"))?.items ?? []).slice(0, 2).map((i, k) => ({
        id: `5eed0000-0000-4000-8000-0000000b03${String(k).padStart(2, "0")}`,
        displayName: i.name,
        enabled: true,
        kind: { type: "media", asset: i.asset, layer: "background" },
        presentation: null,
      })),
    };
  }
  seed();
  binsStub.reset = seed;

  /** The tree as `GET /media-bins` sends it. */
  function tree() {
    const build = (path) => {
      const depth = path.length;
      const folders = [...new Set(bins.filter((b) => b.folder.length > depth && path.every((p, i) => b.folder[i] === p)).map((b) => b.folder[depth]))].sort();
      const out = [];
      for (const f of folders) {
        const children = build([...path, f]);
        out.push({ id: null, name: f, path: [...path, f], kind: "folder", browser: "media", itemCount: children.reduce((a, c) => a + c.itemCount, 0), children });
      }
      for (const b of bins.filter((b) => b.folder.length === depth && path.every((p, i) => b.folder[i] === p)))
        out.push({ id: b.id, name: b.name, path: b.folder, kind: b.kind, browser: b.browser, itemCount: b.items.length, children: [] });
      return out;
    };
    return build([]);
  }
  const detail = (b) => ({ id: b.id, name: b.name, path: b.folder, kind: b.kind, browser: b.browser, items: b.items });
  const rev = () => ({ revision: String(++seq) });
  /** Moves `list[i]` after the element with id `after` (null = first). */
  const place = (list, item, after) => {
    const i = list.indexOf(item);
    if (i >= 0) list.splice(i, 1);
    const at = after ? list.findIndex((x) => x.id === after) + 1 : 0;
    list.splice(at, 0, item);
  };
  /** The playlist tree with the media playlist added at the end. */
  const playlistTree = () => [...(fx.get("playlists") ?? []), { id: MEDIA_PLAYLIST, name: playlist.name, kind: "playlist", purpose: "media", itemCount: playlist.items.length, children: [] }];

  return (req, res, { parts, path }) => {
    if (!binsStub.on) return false;
    const m = req.method;
    if (path === "/media-bins" && m === "GET") return json(res, 200, tree()), true;
    if (path === "/media-bins" && m === "POST") {
      void body(req).then((b) => {
        const name = (b.name ?? "Bin").trim();
        if (!name) return apiError(res, 400, "badRequest", "a name is needed");
        const bin = { id: newId(), name, folder: b.folder ?? [], browser: b.browser ?? "media", kind: "manual", items: [] };
        bins.push(bin);
        json(res, 201, { kind: "mediaBin", id: bin.id, ...rev() });
      });
      return true;
    }
    if (parts[0] === "media-bins" && parts.length >= 2) {
      const bin = bins.find((b) => b.id === parts[1]);
      if (!bin) return apiError(res, 404, "notFound", "media bin not found"), true;
      if (parts.length === 2 && m === "GET") return json(res, 200, detail(bin)), true;
      if (parts.length === 2 && m === "PUT") {
        void body(req).then((b) => {
          if (b.name) bin.name = b.name.trim();
          if (b.folder) bin.folder = b.folder;
          if ("after" in b || b.folder) {
            const sibs = bins.filter((x) => x !== bin && x.folder.join("/") === bin.folder.join("/"));
            const after = "after" in b ? b.after : (sibs.at(-1)?.id ?? null);
            bins.splice(bins.indexOf(bin), 1);
            const at = after ? bins.findIndex((x) => x.id === after) + 1 : bins.indexOf(sibs[0] ?? bins[0]);
            bins.splice(Math.max(0, at), 0, bin);
          }
          json(res, 200, { kind: "mediaBin", id: bin.id, ...rev() });
        });
        return true;
      }
      if (parts.length === 2 && m === "DELETE") {
        bins.splice(bins.indexOf(bin), 1);
        return json(res, 200, { kind: "mediaBin", documents: [{ id: bin.id, name: bin.name }] }), true;
      }
      if (parts[2] === "edits" && m === "POST") {
        void body(req).then((b) => {
          const inverse = [];
          for (const op of b.ops ?? []) {
            if (op.type === "addItem") {
              const name = bins.flatMap((x) => x.items).find((i) => i.asset === op.item.trigger.asset)?.name ?? "Added file";
              const it = { id: op.item.id, name, kind: "image", asset: op.item.trigger.asset, availability: "online", enabled: true, duration: null };
              place(bin.items, it, op.after ?? null);
              inverse.unshift({ type: "removeItem", item: it.id });
            } else if (op.type === "removeItem") {
              const i = bin.items.findIndex((x) => x.id === op.item);
              if (i >= 0) bin.items.splice(i, 1);
            }
          }
          json(res, 200, { ...rev(), applied: b.ops ?? [], inverse });
        });
        return true;
      }
    }
    if (path === "/groups" && m === "GET") return json(res, 200, groups), true;
    if (path === "/groups" && m === "POST") {
      void body(req).then((b) => {
        const g = { schemaVersion: 2, id: newId(), name: (b.name ?? "Group").trim(), ...(b.color ? { color: b.color } : {}), sortKey: "z" };
        groups.push(g);
        json(res, 201, { kind: "globalGroup", id: g.id, ...rev() });
      });
      return true;
    }
    if (parts[0] === "groups" && parts.length === 2) {
      const g = groups.find((x) => x.id === parts[1]);
      if (!g) return apiError(res, 404, "notFound", "group not found"), true;
      if (m === "DELETE") {
        groups.splice(groups.indexOf(g), 1);
        return json(res, 200, { kind: "globalGroup", documents: [g] }), true;
      }
      if (m === "PUT") {
        void body(req).then((b) => {
          if (b.name) g.name = b.name.trim();
          if ("color" in b) {
            if (b.color) g.color = b.color;
            else delete g.color;
          }
          if ("after" in b) place(groups, g, b.after);
          json(res, 200, { kind: "globalGroup", id: g.id, ...rev() });
        });
        return true;
      }
    }
    if (path === "/playlists" && m === "GET") return json(res, 200, playlistTree()), true;
    if (parts[0] === "playlists" && parts[1] === MEDIA_PLAYLIST) {
      if (parts.length === 2 && m === "GET") return json(res, 200, playlist), true;
      if (parts[2] === "edits" && m === "POST") {
        void body(req).then((b) => {
          for (const op of b.ops ?? []) {
            if (op.type === "removeItem") playlist.items = playlist.items.filter((i) => i.id !== op.item);
            if (op.type === "addItem") playlist.items.push({ id: op.item.id, displayName: "Added file", enabled: true, kind: op.item.kind, presentation: null });
          }
          json(res, 200, { ...rev(), applied: b.ops ?? [], inverse: [] });
        });
        return true;
      }
    }
    return false;
  };
}
