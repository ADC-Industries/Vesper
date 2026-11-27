// The Site page's stub routes (see server.mjs's header for the interface): a synthetic two-box
// site. "Booth A" (this stub) is the site's server and holds the Codex; "Chapel Box" is a show
// box; the Codex's own membership ("builtIn"), an IDN display node that needs attention and one
// that is offline (an older Node: no machine details; the others report theirs). A machine asks to join, an Oriel computer waits for approval, the site code
// is up. Settings backups (four sets), restores (one queued for the show box), nightly exports,
// a manual export download and an import with its preview. Names and hosts are made up;
// addresses are documentation ones (192.0.2.x).
//
// Stub control (not part of the API): `POST /api/v1/__stub/site` with
//   {"reset": true}                 everything as at start
//   {"access": "open"|"pin"|"users"} the access level; with "pin" admin actions answer 403
//                                    until `POST /site/unlock {"pin": "1234"}`
//   {"exportDelay": 3000}           the manual export answers after that many ms
//   {"apiError": "…"|null}          the Site Manager runs without its API (admin panel), for
//                                    that reason (null: with it again)
//   {"settingsDamaged": true|false} the server started from a damaged site.json (until a
//                                    settings save, as on the server)

// Times count back from the moment the stub (or a reset) set the site up.
let NOW = Date.now();
const at = (minAgo) => new Date(NOW - minAgo * 60_000).toISOString();
const later = (min) => new Date(NOW + min * 60_000).toISOString();
const SERVER_ID = "ef405d09-d621-4bc9-8846-beefcf530b86";
const CHAPEL_ID = "a7c3e512-60d4-4f0b-9b8e-3c21d5f0e7aa";
const PIN = "1234";

function initial() {
  NOW = Date.now();
  const snap = (set, seq, minAgo, reason, items, by = { client: "server machine", machine: SERVER_ID }) => ({
    id: `${set}-s${seq}`,
    set,
    seq,
    takenAt: at(minAgo),
    reason,
    by,
    digest: `d${set}${seq}`,
    items,
    bytes: items * 2100,
  });
  const sets = [
    { set: { schemaVersion: 1, id: "set-booth", product: "vesper", machine: SERVER_ID, name: "Booth A", fingerprint: "fp-booth", createdAt: at(90 * 1440) }, thisBox: true, yours: false },
    { set: { schemaVersion: 1, id: "set-chapel", product: "vesper", machine: CHAPEL_ID, name: "Chapel Box", fingerprint: "fp-chapel", createdAt: at(60 * 1440) }, thisBox: false, yours: false },
    { set: { schemaVersion: 1, id: "set-oriel-booth", product: "oriel", machine: "local", name: "Booth PC", createdAt: at(30 * 1440) }, thisBox: false, yours: true },
    { set: { schemaVersion: 1, id: "set-oriel-stage", product: "oriel", machine: "k7", name: "Stage Laptop", createdAt: at(20 * 1440) }, thisBox: false, yours: false },
  ];
  const snapshots = {
    "set-booth": [snap("set-booth", 41, 480, "scheduled", 23), snap("set-booth", 40, 1920, "change", 23), snap("set-booth", 39, 3360, "manual", 22)],
    "set-chapel": [snap("set-chapel", 18, 500, "scheduled", 15, { client: "server machine", machine: CHAPEL_ID }), snap("set-chapel", 17, 2000, "change", 15)],
    "set-oriel-booth": [snap("set-oriel-booth", 9, 35, "upload", 1, { client: "Oriel – Booth PC" })],
    "set-oriel-stage": [snap("set-oriel-stage", 4, 4400, "upload", 1, { client: "Oriel – Stage Laptop" })],
  };
  return {
    access: "open",
    unlocked: false,
    unlockedUntil: 0,
    exportDelay: 0,
    apiError: null,
    settingsDamaged: false,
    joinMode: "code_or_approval",
    pairingOpen: false,
    revision: "r12",
    devices: [
      { fingerprint: "fp-codex", name: "Codex", kind: "app", admin: false, health: "online", problems: [], restarting: [], services: [], node: false, builtIn: true },
      {
        fingerprint: "fp-chapel",
        // A Vesper box joins its site as "<name> (Vesper)".
        name: "Chapel Box (Vesper)",
        kind: "app",
        admin: false,
        health: "online",
        problems: [],
        version: "1.4.2",
        restarting: [],
        services: [],
        node: true,
        // A Vesper box: the site's server marks it a pairing relay (output boxes never are).
        relay: true,
        nodeStartedAt: at(3 * 1440 + 238),
        machine: {
          cpu: "Example CPU 8-Core",
          cpus: 16,
          gpus: ["Example GPU 4070"],
          os: "Ubuntu",
          osVersion: "24.04",
          kernel: "6.8.0-49-generic",
          bootedAt: at(3 * 1440 + 240),
          memoryTotal: 32 * 2 ** 30,
          memoryFree: 21.4 * 2 ** 30,
          reportedAt: at(0.2),
        },
      },
      {
        fingerprint: "fp-lobby",
        name: "Lobby Display",
        kind: "output",
        admin: false,
        health: "needsAttention",
        reason: "Running at 1280×720; the source is 1920×1080",
        problems: [{ problem: "resolution_mismatch", level: "warning", text: "Running at 1280×720; the source is 1920×1080", fields: {} }],
        version: "1.4.1",
        updateManagedBy: "Vesper",
        restarting: [],
        services: [
          { id: "display", name: "Display", state: "running" },
          { id: "audio", name: "Audio", state: "failed" },
          { id: "switcher", name: "Switcher", state: "off" },
          { id: "desktop", name: "Desktop capture", state: "no_session" },
          { id: "recorder", name: "Recorder", state: "unsupported" },
        ],
        node: true,
        // Its Node restarted since the machine started.
        nodeStartedAt: at(125),
        machine: {
          cpu: "Example Mobile CPU 4-Core",
          cpus: 8,
          gpus: ["Example Integrated Graphics", "Example GPU 3060"],
          os: "Windows 11 Pro",
          osVersion: "24H2",
          kernel: "26100.2314",
          bootedAt: at(9 * 60 + 12),
          memoryTotal: 16 * 2 ** 30,
          memoryFree: 5.2 * 2 ** 30,
          reportedAt: at(0.4),
        },
      },
      { fingerprint: "fp-side", name: "Side Screen Node", kind: "output", admin: false, health: "offline", lastSeen: at(3 * 1440 + 200), problems: [], version: "1.3.9", restarting: [], services: [{ id: "display", name: "Display", state: "running" }], node: true },
    ],
    joinRequests: [{ id: 7, name: "node-7f3a", kind: "output", fingerprint: "fp-7f3a", code: "527180", address: "192.0.2.71", byCode: false, askedAt: at(2) }],
    pairings: [
      { id: "p3", client: "Youth Laptop", via: "Booth A", address: "192.0.2.88", relayed: false, askedAt: at(4) },
      { id: "p4", client: "Vesper Launcher – Chapel Box", via: "Booth A", address: "192.0.2.89", relayed: false, askedAt: at(3), launcher: "6d6f636b-0000-4000-8000-000000000001" },
    ],
    joinCode: { id: 12, code: "482913", expiresAt: later(9.7), expiresInSeconds: 581, triesLeft: 5, issuer: "Oriel – Booth PC" },
    events: [
      { name: "join_code_issued", level: "info", text: "A new site code was made", at: at(0.5), codeId: 12, detail: {} },
      { name: "join_requested", level: "info", text: "node-7f3a asks to join the site", at: at(2), fingerprint: "fp-7f3a", requestId: 7, detail: {} },
      { name: "key_requested", level: "info", text: "Youth Laptop asks to pair with Booth A", at: at(4), machine: "Booth A", detail: { pairing: "p3" } },
      { name: "device_offline", level: "warning", text: "Side Screen Node went offline", at: at(3 * 1440 + 200), fingerprint: "fp-side", detail: {} },
      { name: "device_joined", level: "ok", text: "Chapel Box joined the site", at: at(60 * 1440), fingerprint: "fp-chapel", detail: {} },
    ],
    apiKeys: [
      { id: "k1", name: "Oriel – Booth PC", engine: "app:vesper", machine: "*", scope: "control", createdAt: at(30 * 1440), by: "admin", expiresAt: later(3650 * 1440), revoked: false, lastUsed: at(1), lastMachine: "Booth A" },
      { id: "k7", name: "Oriel – Stage Laptop", engine: "app:vesper", machine: "*", scope: "control", createdAt: at(20 * 1440), by: "admin (via Chapel Box)", expiresAt: later(3650 * 1440), revoked: false, lastUsed: at(2 * 1440), lastMachine: "Chapel Box" },
      { id: "k9", name: "Oriel – Browser", engine: "app:vesper", machine: "*", scope: "control", createdAt: at(12 * 1440), by: "admin", expiresAt: later(353 * 1440), revoked: true, revokedAt: at(5 * 1440) },
    ],
    users: [
      { name: "pat.admin", role: "admin", revision: "u1" },
      { name: "sam.ops", role: "operator", revision: "u2" },
    ],
    audit: [
      { at: at(0.5), user: "admin", peer: "192.0.2.15", client: "Oriel – Booth PC", action: "join_code", ok: true, detail: {}, source: "siteManager" },
      { at: at(300), user: "admin", peer: "192.0.2.15", client: "Oriel – Booth PC", action: "codex.backupNow", ok: true, detail: {}, source: "vesper" },
      { at: at(2 * 1440), user: "viewer", peer: "192.0.2.40", client: "Oriel – Stage Laptop", action: "unlock", ok: false, error: "wrong PIN", detail: {}, source: "vesper" },
      { at: at(60 * 1440), user: "admin", peer: "192.0.2.15", client: "Oriel – Booth PC", action: "approve", ok: true, detail: { machine: "Chapel Box" }, source: "siteManager" },
    ],
    membership: { state: "none", connected: false },
    sets,
    snapshots,
    restores: [
      {
        id: "q2",
        set: "set-chapel",
        snapshot: "set-chapel-s17",
        target: CHAPEL_ID,
        targetName: "Chapel Box",
        state: "applied",
        queuedAt: at(3000),
        updatedAt: at(2990),
        expiresAt: at(3000 - 1440),
        by: { client: "Oriel – Booth PC" },
        result: { beforeRestore: snapshots["set-chapel"][1], applied: [], summary: "1 screen changed", restart: [] },
      },
    ],
    exportSettings: { enabled: true, time: "02:00", keepNightly: 7, keepWeekly: 4 },
    passphraseSet: true,
    scheduled: [
      { file: "vesper-codex-2026-11-23.vcodex", createdAt: at(720), bytes: 18_400_000 },
      { file: "vesper-codex-2026-11-22.vcodex", createdAt: at(720 + 1440), bytes: 18_350_000 },
      { file: "vesper-codex-2026-11-21.vcodex", createdAt: at(720 + 2880), bytes: 18_100_000 },
    ],
    imports: {},
    oriel: null,
  };
}

let site = initial();

const NOTE = "Media files aren't included: back up the media folder separately.";

function preview(set, snapshot, target = null) {
  const snap = (site.snapshots[set] ?? []).find((s) => s.id === snapshot) ?? site.snapshots[set]?.[0];
  // Onto this box (`?target=`, a replacement for another box): compared with this box's
  // settings now, which the old box's snapshot doesn't hold all of.
  if (target && !set.startsWith("set-oriel"))
    return {
      snapshot: snap,
      changes: [
        { key: "portable/screen/main.json", kind: "screen", id: "main", name: "Main Screen", change: "changed" },
        { key: "portable/look/lyrics.json", kind: "look", id: "lyrics", name: "Lyrics Only", change: "added" },
        { key: "machine/outputBinding/hall.json", kind: "outputBinding", id: "hall", name: "Hall projector", change: "removed" },
        { key: "machine/server.json", kind: "server", name: "Server name", change: "changed", restart: true },
      ],
      unchanged: 9,
      summary: "1 screen changed; 1 look added; 1 output binding removed; the server's name changed",
      restart: ["the server's name"],
    };
  if (set.startsWith("set-oriel"))
    return { snapshot: snap, changes: [{ key: "oriel/preferences.json", kind: "orielPreferences", change: "changed" }], unchanged: 0, summary: "Oriel's preferences changed", restart: [] };
  return {
    snapshot: snap,
    currentAsOf: set === "set-booth" ? undefined : site.snapshots[set]?.[0]?.takenAt,
    changes: [
      { key: "portable/screen/main.json", kind: "screen", id: "main", name: "Main Screen", change: "changed" },
      { key: "portable/screen/confidence.json", kind: "screen", id: "confidence", name: "Confidence", change: "changed" },
      { key: "portable/screen/side.json", kind: "screen", id: "side", name: "Side Wall", change: "changed" },
      { key: "portable/look/lyrics.json", kind: "look", id: "lyrics", name: "Lyrics Only", change: "changed" },
      { key: "portable/look/lower.json", kind: "look", id: "lower", name: "Lower Third", change: "changed" },
      { key: "machine/output/ndi-1.json", kind: "output", id: "ndi-1", name: "Stream feed", change: "added" },
      { key: "machine/server.json", kind: "server", name: "Server name", change: "changed", restart: true },
    ],
    unchanged: 16,
    summary: "3 screens, 2 looks changed; 1 output binding added; the server's name changed",
    restart: ["the server's name"],
  };
}

function importPreview(id) {
  const col = (cid, name, documents, seq, minAgo) => ({ id: cid, name, documents, seq, lastChangeAt: at(minAgo) });
  const collections = [
    { collection: col("6c931ace-df87-4f16-960c-43669b655534", "Booth A", 19, 412, 200), comparison: "same", here: col("6c931ace-df87-4f16-960c-43669b655534", "Booth A", 19, 412, 200), active: true, tooNew: false },
    { collection: col("c-main", "Main Hall", 61, 980, 60), comparison: "newer", here: col("c-main", "Main Hall", 58, 951, 1500), active: false, tooNew: false },
    { collection: col("c-youth", "Youth Room", 9, 77, 9000), comparison: "older", here: col("c-youth", "Youth Room", 11, 90, 300), active: false, tooNew: false },
    { collection: col("c-retreat", "Retreat Weekend", 24, 130, 20000), comparison: "new", active: false, tooNew: false },
    { collection: col("c-old", "Old Chapel Songs", 40, 300, 40000), comparison: "inTrash", here: col("c-old", "Old Chapel Songs", 40, 300, 40000), active: false, tooNew: false },
  ];
  return {
    id,
    manifest: {
      format: "vesper-codex",
      formatVersion: 1,
      createdAt: at(1500),
      reason: "manual",
      codexId: "2adb06a5-2d13-4c9f-a48b-59728c980931",
      serverId: SERVER_ID,
      serverName: "Booth A",
      version: "1.4.2",
      collections: collections.map((c) => c.collection),
      backupSets: 3,
      snapshots: 11,
      site: true,
      mediaIncluded: false,
    },
    sealedAt: at(1500),
    collections,
    backupSets: [
      { id: "set-booth", product: "vesper", machine: SERVER_ID, name: "Booth A", snapshots: 6, present: true },
      { id: "set-hall", product: "vesper", machine: "b1d2", name: "Hall Box (removed)", snapshots: 4, present: false },
      { id: "set-oriel-booth", product: "oriel", machine: "local", name: "Booth PC", snapshots: 1, present: true },
    ],
    site: true,
    mediaIncluded: false,
    note: NOTE,
  };
}

const ADMIN_WRITES = /^\/site\/(settings|join-requests|pairing|join-code|join-codes|devices|pairings|api-keys|users|password|join|leave)|^\/codex\/(backups\/.+\/restore|backups\/restores|backups\/snapshots$|export)/;

export default function siteRoutes(ctx) {
  const { json, apiError } = ctx;

  const readBody = (req, fn) => {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      let v = {};
      try {
        v = body ? JSON.parse(body) : {};
      } catch {
        v = { _raw: body.length };
      }
      fn(v);
    });
  };
  const you = () => ({
    access: site.access,
    signedIn: false,
    unlocked: site.access === "pin" && site.unlocked,
    unlockSecondsLeft: site.access === "pin" && site.unlocked ? 1800 : undefined,
    role: site.access === "open" || site.unlocked ? "admin" : "viewer",
  });
  const status = () => ({
    enabled: true,
    running: true,
    joinMode: site.joinMode,
    pairingOpen: site.pairingOpen,
    siteId: "3f9a0c12d4e5b6a7",
    discoveryPort: 5959,
    ...(site.apiError ? { apiError: site.apiError } : { apiPort: 29202 }),
    ...(site.settingsDamaged ? { settingsDamaged: true } : {}),
    you: you(),
    revision: site.revision,
    membership: site.membership,
  });
  const exportStatus = () => ({
    settings: site.exportSettings,
    folder: site.exportSettings.folder || "/var/lib/vesper/codex/exports",
    passphraseSet: site.passphraseSet,
    active: site.exportSettings.enabled && site.passphraseSet,
    problem: !site.passphraseSet && site.exportSettings.enabled ? "No passphrase is stored: nightly exports don't run until one is set." : undefined,
    last: site.scheduled[0],
    nextAt: site.exportSettings.enabled ? later(12 * 60) : undefined,
    mediaIncluded: false,
    note: NOTE,
  });
  const ok = (res) => {
    res.writeHead(204);
    res.end();
    return true;
  };

  return function handle(req, res, { path, parts, query }) {
    const m = req.method;
    if (path === "/__stub/site" && m === "POST") {
      readBody(req, (b) => {
        if (b.reset) site = initial();
        if (b.access) {
          site.access = b.access;
          site.unlocked = false;
        }
        if (b.exportDelay !== undefined) site.exportDelay = b.exportDelay;
        if (b.apiError !== undefined) site.apiError = b.apiError;
        if (b.settingsDamaged !== undefined) site.settingsDamaged = b.settingsDamaged;
        json(res, 200, { ok: true });
      });
      return true;
    }
    if (parts[0] !== "site" && !(parts[0] === "codex" && (parts[1] === "backups" || parts[1] === "export"))) return false;

    // Admin actions at access `pin`, locked: 403 (the unlock itself is never one).
    if (m !== "GET" && site.access !== "open" && !site.unlocked && ADMIN_WRITES.test(path) && path !== "/site/unlock") {
      apiError(res, 403, "forbidden", "This is an admin action: unlock the site with its PIN first.");
      return true;
    }

    // ---------------------------------------------------------------- site
    if (path === "/site" && m === "GET") return json(res, 200, status()), true;
    if (path === "/site/settings") {
      if (m === "GET") return json(res, 200, { enabled: true, joinMode: site.joinMode, access: site.access, pinSet: site.access === "pin", revision: site.revision }), true;
      readBody(req, (b) => {
        // Like the server: whole settings, or 400.
        const missing = ["enabled", "joinMode", "access"].find((k) => b[k] === undefined);
        if (missing) return apiError(res, 400, "badRequest", `invalid JSON: missing field \`${missing}\``);
        if (b.joinMode) site.joinMode = b.joinMode;
        if (b.access) site.access = b.access;
        // Setting the access again (even the same) ends the lockdown after damaged settings.
        site.settingsDamaged = false;
        site.revision = `r${Number(site.revision.slice(1)) + 1}`;
        json(res, 200, { enabled: true, joinMode: site.joinMode, access: site.access, pinSet: site.access === "pin", revision: site.revision });
      });
      return true;
    }
    if (path === "/site/session") {
      if (m === "GET") return json(res, 200, you()), true;
      if (m === "DELETE") return (site.unlocked = false), ok(res);
      readBody(req, () => apiError(res, 401, "unauthorized", "Wrong user name or password"));
      return true;
    }
    if (path === "/site/unlock") {
      if (m === "DELETE") return (site.unlocked = false), ok(res);
      readBody(req, (b) => {
        if (site.access !== "pin") return apiError(res, 409, "conflict", "The site has no PIN");
        if (b.pin !== PIN) return apiError(res, 401, "unauthorized", "Wrong PIN");
        site.unlocked = true;
        json(res, 200, you());
      });
      return true;
    }
    if (path === "/site/devices" && m === "GET") return json(res, 200, site.devices), true;
    if (parts[1] === "devices" && parts[2]) {
      const d = site.devices.find((x) => x.fingerprint === parts[2]);
      if (!d) return apiError(res, 404, "notFound", "No such device"), true;
      if (parts.length === 3 && m === "DELETE") {
        if (d.builtIn) return apiError(res, 409, "conflict", "The Codex's own membership can't be removed"), true;
        site.devices = site.devices.filter((x) => x !== d);
        site.events = [{ name: "device_revoked", level: "info", text: `${d.name} was removed from the site`, at: new Date().toISOString(), fingerprint: d.fingerprint, detail: {} }, ...site.events];
        return ok(res);
      }
      if (parts[3] === "relay" && m === "POST") {
        if (d.builtIn) return apiError(res, 409, "conflict", "The Codex's own membership relays nothing"), true;
        readBody(req, (b) => {
          // Only a Vesper box (an app joined as "<name> (Vesper)") is marked (M-6).
          if (b.on && !(d.kind === "app" && / \(Vesper\)$/.test(d.name) && d.name.length > " (Vesper)".length))
            return apiError(res, 409, "conflict", `${d.name} isn't a Vesper box: only a Vesper box relays Oriel pairings`);
          if (b.on) d.relay = true;
          else delete d.relay;
          ok(res);
        });
        return true;
      }
      if (parts[3] === "logs" && m === "GET" && !parts[4])
        return (
          json(res, 200, [
            { snapshot: "2026-11-23T13-58-00Z", files: [{ name: "node.log", size: 48_210 }, { name: "display.log", size: 12_004 }] },
            { snapshot: "2026-11-20T09-12-00Z", files: [{ name: "node.log", size: 31_870 }] },
          ]),
          true
        );
      if (parts[3] === "logs" && parts[5] && m === "GET") {
        res.writeHead(200, { "Content-Type": "text/plain; charset=utf-8" });
        res.end(
          [
            `13:52:01 INFO  ${d.name}: node started (1.4.1)`,
            "13:52:02 INFO  joined site 3f9a0c12d4e5b6a7",
            "13:52:04 INFO  output 1: 1280x720 @ 60 Hz",
            "13:52:04 WARN  source is 1920x1080, scaling down",
            "13:57:40 INFO  identify requested (10 s)",
          ].join("\n"),
        );
        return true;
      }
      return ok(res);
    }
    if (path === "/site/join-requests" && m === "GET") return json(res, 200, site.joinRequests), true;
    if (parts[1] === "join-requests" && parts[3]) {
      const id = Number(parts[2]);
      const r = site.joinRequests.find((x) => x.id === id);
      readBody(req, (b) => {
        site.joinRequests = site.joinRequests.filter((x) => x.id !== id);
        if (r && parts[3] === "approve") {
          const name = b.name || r.name;
          site.devices = [...site.devices, { fingerprint: r.fingerprint, name, kind: r.kind, admin: false, health: "online", problems: [], version: "1.4.2", restarting: [], services: [], node: true }];
          site.events = [{ name: "device_joined", level: "ok", text: `${name} joined the site`, at: new Date().toISOString(), detail: {} }, ...site.events];
        } else if (r) site.events = [{ name: "join_denied", level: "info", text: `${r.name} was refused`, at: new Date().toISOString(), detail: {} }, ...site.events];
        ok(res);
      });
      return true;
    }
    if (path === "/site/pairings" && m === "GET") return json(res, 200, site.pairings), true;
    if (parts[1] === "pairings" && parts[3]) {
      const p = site.pairings.find((x) => x.id === parts[2]);
      readBody(req, () => {
        site.pairings = site.pairings.filter((x) => x !== p);
        if (p && parts[3] === "approve") site.apiKeys = [{ id: "k12", name: `Oriel – ${p.client}`, engine: "app:vesper", machine: "*", scope: "control", createdAt: new Date().toISOString(), by: "admin", expiresAt: later(3650 * 1440), revoked: false }, ...site.apiKeys];
        if (p) site.events = [{ name: parts[3] === "approve" ? "key_request_cancelled" : "key_request_cancelled", level: "info", text: `${p.client} was ${parts[3] === "approve" ? "approved" : "refused"}`, at: new Date().toISOString(), detail: {} }, ...site.events];
        ok(res);
      });
      return true;
    }
    if (path === "/site/pairing") {
      site.pairingOpen = m === "POST";
      return ok(res);
    }
    if (path === "/site/join-code") {
      if (m === "GET") return site.joinCode ? json(res, 200, site.joinCode) : apiError(res, 404, "notFound", "No site code"), true;
      const n = String(100000 + Math.floor(Math.random() * 900000));
      site.joinCode = { id: site.joinCode ? site.joinCode.id + 1 : 1, code: n, expiresAt: later(10), expiresInSeconds: 600, triesLeft: 5, issuer: "Oriel – Booth PC" };
      return json(res, 201, site.joinCode), true;
    }
    if (path === "/site/api-keys" && m === "GET") return json(res, 200, site.apiKeys), true;
    if (parts[1] === "api-keys" && parts[2] && m === "DELETE") {
      site.apiKeys = site.apiKeys.map((k) => (k.id === parts[2] ? { ...k, revoked: true, revokedAt: new Date().toISOString() } : k));
      return ok(res);
    }
    if (path === "/site/users" && m === "GET") return json(res, 200, site.users), true;
    if (path === "/site/audit" && m === "GET") return json(res, 200, site.audit), true;
    if (path === "/site/events" && m === "GET") return json(res, 200, site.events.slice(0, Number(query.get?.("n") ?? 50) || 50)), true;
    if (path === "/site/membership" && m === "GET") return json(res, 200, site.membership), true;
    if (path === "/site/join") {
      readBody(req, (b) => {
        site.membership = { state: "waitingForApproval", server: b.server, code: "604118", connected: false };
        json(res, 200, site.membership);
      });
      return true;
    }
    if (path === "/site/leave") {
      site.membership = { state: "none", connected: false };
      return ok(res);
    }

    // ---------------------------------------------------------------- settings backups
    if (path === "/codex/backups" && m === "GET")
      return json(res, 200, site.sets.map((s) => ({ ...s, snapshots: site.snapshots[s.set.id].length, latest: site.snapshots[s.set.id][0] }))), true;
    if (path === "/codex/backups/snapshots" && m === "POST") {
      const list = site.snapshots["set-booth"];
      const s = { ...list[0], id: `set-booth-s${list[0].seq + 1}`, seq: list[0].seq + 1, takenAt: new Date().toISOString(), reason: "manual", by: { client: "Oriel – Booth PC" } };
      list.unshift(s);
      return json(res, 201, { snapshot: s, created: true }), true;
    }
    if (path === "/codex/backups/restores" && m === "GET") return json(res, 200, site.restores), true;
    if (parts[2] === "restores" && parts[3]) {
      site.restores = site.restores.map((r) => (r.id === parts[3] && r.state === "queued" ? { ...r, state: "cancelled", updatedAt: new Date().toISOString() } : r));
      return ok(res);
    }
    if (path === "/codex/backups/oriel") {
      if (m === "GET") return site.oriel ? json(res, 200, site.oriel) : apiError(res, 404, "notFound", "No preferences backed up"), true;
      readBody(req, (b) => {
        const snapshot = { ...site.snapshots["set-oriel-booth"][0], takenAt: new Date().toISOString() };
        site.oriel = { schemaVersion: b.schemaVersion ?? 1, preferences: b.preferences ?? {}, snapshot };
        json(res, 201, { snapshot, created: true });
      });
      return true;
    }
    if (parts[1] === "backups" && parts[3] === "snapshots" && !parts[4] && m === "GET") return json(res, 200, site.snapshots[parts[2]] ?? []), true;
    if (parts[1] === "backups" && parts[5] === "preview" && m === "GET") return json(res, 200, preview(parts[2], parts[4], query.get?.("target") ?? null)), true;
    if (parts[1] === "backups" && parts[5] === "restore" && m === "POST") {
      readBody(req, (b) => {
        const pv = preview(parts[2], parts[4]);
        const applied = b.items ? pv.changes.filter((c) => b.items.includes(c.key)) : pv.changes;
        const result = { beforeRestore: site.snapshots["set-booth"][0], applied, summary: pv.summary, restart: applied.some((c) => c.restart) ? pv.restart : [] };
        if (parts[2] === "set-chapel") {
          const q = {
            id: `q${site.restores.length + 3}`,
            set: parts[2],
            snapshot: parts[4],
            target: CHAPEL_ID,
            targetName: "Chapel Box",
            items: b.items,
            state: "queued",
            queuedAt: new Date().toISOString(),
            updatedAt: new Date().toISOString(),
            expiresAt: later(1440),
            by: { client: "Oriel – Booth PC" },
          };
          site.restores = [q, ...site.restores];
          return json(res, 202, q);
        }
        json(res, 200, result);
      });
      return true;
    }

    // ---------------------------------------------------------------- export and import
    if (path === "/codex/export" && m === "POST") {
      readBody(req, (b) => {
        if (!b.passphrase || b.passphrase.length < 8) return apiError(res, 400, "badRequest", "The passphrase needs at least 8 characters");
        setTimeout(() => {
          res.writeHead(200, {
            "Content-Type": "application/octet-stream",
            "Content-Disposition": 'attachment; filename="vesper-codex-2026-11-23.vcodex"',
            "X-Codex-Media-Included": "false",
            "X-Codex-Export-Note": NOTE,
          });
          res.end(Buffer.from("VCODEX synthetic export for the stub\n"));
        }, site.exportDelay);
      });
      return true;
    }
    if (path === "/codex/export/settings") {
      if (m === "GET") return json(res, 200, exportStatus()), true;
      readBody(req, (b) => {
        site.exportSettings = { ...site.exportSettings, ...b };
        json(res, 200, exportStatus());
      });
      return true;
    }
    if (path === "/codex/export/passphrase") {
      if (m === "DELETE") return (site.passphraseSet = false), json(res, 200, exportStatus()), true;
      readBody(req, () => {
        site.passphraseSet = true;
        json(res, 200, exportStatus());
      });
      return true;
    }
    if (path === "/codex/export/scheduled" && m === "GET") return json(res, 200, site.scheduled), true;
    if (path === "/codex/export/imports" && m === "POST") {
      let n = 0;
      req.on("data", (d) => (n += d.length));
      req.on("end", () => {
        const id = `imp${Object.keys(site.imports).length + 1}`;
        site.imports[id] = { bytes: n };
        json(res, 201, { id, bytes: n, expiresAt: later(60) });
      });
      return true;
    }
    if (parts[2] === "imports" && parts[3]) {
      const id = parts[3];
      if (m === "DELETE") return delete site.imports[id], ok(res);
      readBody(req, (b) => {
        if (b.passphrase !== "correct horse" && b.passphrase?.length < 8) return apiError(res, 400, "badRequest", "Wrong passphrase, or the file is damaged");
        if (b.passphrase === "wrong passphrase") return apiError(res, 400, "badRequest", "Wrong passphrase, or the file is damaged");
        if (parts[4] === "preview") return json(res, 200, importPreview(id));
        const pv = importPreview(id);
        const added = pv.collections.filter((c) => c.comparison === "new" && !(b.skip ?? []).includes(c.collection.id)).map((c) => c.collection.id);
        json(res, 200, {
          added,
          replaced: b.replace ?? [],
          skipped: pv.collections.map((c) => c.collection.id).filter((x) => !added.includes(x) && !(b.replace ?? []).includes(x)),
          backupSets: b.backups === false ? 0 : 1,
          snapshots: b.backups === false ? 0 : 4,
          siteStaged: !!b.site,
          restartNeeded: !!b.site,
          mediaIncluded: false,
          note: NOTE,
        });
      });
      return true;
    }
    return false;
  };
}
