// Site ▸ Updates' stub routes (G10; see server.mjs's header for the interface): the site's
// update channel and three machines' Vesper Launchers. "Booth A" (this stub, the site's server)
// has an update available, "Chapel Box" (a display box) was updated and can roll back, "Lobby PC"
// (Windows) is offline after an update that rolled back by itself. Names are made up; key IDs
// and hashes are synthetic.
//
// Uploads are checked like the server does, minus the cryptography: the `X-Vesper-Signature`
// header must be base64 of a minisign file whose trusted comment is
// `vesper-package product=… version=… platform=…`.
//
// Releases from GitHub: the server's check (on, stable, checked 42 minutes ago; one package came
// from it) and the catalogue "Add app" offers (Vesper, Vesper Oriel, the bundled IDN Node),
// filtered per machine as the server does (platform, role, what it has).
//
// Stub control (not part of the API): `POST /api/v1/__stub/updates` with
//   {"reset": true}   everything as at start
//   {"apply": true}   every machine's queued actions "happen" (as its Launcher would)

let NOW = Date.now();
const at = (minAgo) => new Date(NOW - minAgo * 60_000).toISOString();
const KEY = "D4A61C0E5B7F2398";

function pkg(id, product, version, platform, size, minAgo) {
  return {
    id,
    product,
    version,
    platform,
    fileName: platform.startsWith("windows") ? `vesper-server-${version}-setup.exe` : `vesper_${version}-1_amd64.deb`,
    size,
    sha256: id.padEnd(64, "0"),
    keyId: KEY,
    trustedComment: `vesper-package product=${product} version=${version} platform=${platform}`,
    uploadedAt: at(minAgo),
  };
}

const health = (cpu, memFree, mem, diskFree, disk, gpus) => ({
  cpus: 8,
  cpuPercent: cpu,
  memoryTotal: mem * 2 ** 30,
  memoryAvailable: memFree * 2 ** 30,
  diskTotal: disk * 2 ** 30,
  diskFree: diskFree * 2 ** 30,
  gpus,
});

const CATALOGUE = [
  { product: "vesper", name: "Vesper", stable: "0.2.0", beta: "0.3.0-beta1", platforms: ["linux-x86_64", "windows-x86_64"], sources: ["channel", "github"] },
  { product: "vesper-idn-node", name: "IDN Node (bundled)", stable: "0.3.8-beta18", platforms: ["linux-x86_64"], sources: ["github"] },
  { product: "vesper-oriel", name: "Vesper Oriel", stable: "0.2.0", platforms: ["linux-x86_64", "linux-x86_64-appimage", "windows-x86_64"], sources: ["github"] },
];

/** What a machine can add (as the server's `offered`). */
function offered(k) {
  const { platform, roles, products } = k.report;
  const linux = platform.startsWith("linux-");
  return CATALOGUE.filter(
    (c) =>
      c.platforms.includes(platform) &&
      !products.some((p) => p.product === c.product && p.version) &&
      (c.product === "vesper" || linux) &&
      !(c.product === "vesper" && roles.includes("display")),
  );
}

function initial() {
  NOW = Date.now();
  return {
    github: {
      settings: { enabled: true, repository: "ADC-Industries/Vesper", channel: "stable", intervalHours: 6 },
      lastCheck: at(42),
      lastSuccess: at(42),
      nextCheck: new Date(NOW + 318 * 60_000).toISOString(),
      added: ["vesper 0.2.0 for linux-x86_64", "vesper-idn-node 0.3.8 for linux-x86_64"],
      checking: false,
    },
    packages: [
      { ...pkg("7c1e93a0b24d5f61", "vesper", "0.2.0", "linux-x86_64", 48_211_456, 90), source: "github" },
      pkg("2f0d6e8b9a137c45", "vesper", "0.1.0", "linux-x86_64", 47_002_112, 30 * 1440),
      pkg("b83a45c1d09e2f76", "vesper", "0.2.0", "windows-x86_64", 61_884_416, 88),
    ],
    launchers: [
      {
        report: {
          machineId: "m-booth",
          machineName: "Booth A",
          launcherVersion: "0.1.0",
          platform: "linux-x86_64",
          roles: ["presentation", "siteManager"],
          products: [{ product: "vesper", name: "Vesper", version: "0.1.0", state: "running", restarts: 0 }],
          health: health(14, 9.2, 16, 312, 476, ["amdgpu"]),
        },
        seen: 0.2,
        pending: [],
      },
      {
        report: {
          machineId: "m-chapel",
          machineName: "Chapel Box",
          launcherVersion: "0.1.0",
          platform: "linux-x86_64",
          roles: ["display"],
          products: [
            {
              product: "vesper",
              name: "Vesper",
              version: "0.2.0",
              previous: "0.1.0",
              state: "running",
              restarts: 1,
              lastUpdate: { from: "0.1.0", to: "0.2.0", result: "installed", message: "", at: at(70) },
            },
          ],
          // An "Add app" of Oriel that failed there (apt couldn't bring a library).
          failedInstalls: [
            {
              product: "vesper-oriel",
              name: "Vesper Oriel",
              outcome: { to: "0.2.0", result: "failed", message: "apt couldn't install it (exit status: 100): vesper-oriel : Depends: libwebkit2gtk-4.1-0 but it is not installable / E: Unable to correct problems, you have held broken packages.", at: at(12) },
            },
          ],
          health: health(6, 5.4, 8, 88, 118, ["i915"]),
        },
        seen: 0.4,
        pending: [],
      },
      {
        report: {
          machineId: "m-lobby",
          machineName: "Lobby PC",
          launcherVersion: "0.1.0",
          platform: "windows-x86_64",
          roles: ["display"],
          products: [
            {
              product: "vesper",
              name: "Vesper",
              version: "0.1.0",
              state: "running",
              restarts: 0,
              lastUpdate: {
                from: "0.1.0",
                to: "0.2.0",
                result: "rolledBack",
                message: "version 0.2.0 wasn't healthy within 60 s: its health check failed; 0.1.0 is back",
                at: at(80),
              },
            },
          ],
          health: { cpus: 4, gpus: [] },
        },
        seen: 185,
        pending: [],
      },
    ],
  };
}

let st = initial();

const cmp = (a, b) => {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) if ((pa[i] ?? 0) !== (pb[i] ?? 0)) return (pa[i] ?? 0) - (pb[i] ?? 0);
  return 0;
};

function entry(k) {
  const updates = [];
  for (const p of k.report.products) {
    const newest = st.packages.filter((c) => c.product === p.product && c.platform === k.report.platform).sort((a, b) => cmp(b.version, a.version))[0];
    if (newest && (!p.version || cmp(newest.version, p.version) > 0)) updates.push({ product: p.product, version: newest.version, package: newest.id });
  }
  return { report: k.report, lastSeenSeconds: Math.round(k.seen * 60), online: k.seen * 60 < 90, pending: k.pending, updates };
}

function apply() {
  for (const k of st.launchers) {
    for (const a of k.pending) {
      if (a.type === "install") {
        const c = CATALOGUE.find((x) => x.product === a.product);
        const to = { from: undefined, to: a.version, result: "installed", message: "", at: new Date().toISOString() };
        k.report.products.push({ product: a.product, name: c?.name ?? a.product, version: a.version, state: "running", restarts: 0, lastUpdate: to });
        continue;
      }
      const p = k.report.products.find((x) => x.product === a.product);
      if (!p) continue;
      if (a.type === "update") {
        p.lastUpdate = { from: p.version, to: a.version, result: "installed", message: "", at: new Date().toISOString() };
        p.previous = p.version;
        p.version = a.version;
      } else if (p.previous) {
        p.lastUpdate = { from: p.version, to: p.previous, result: "installed", message: "rolled back by hand", at: new Date().toISOString() };
        [p.version, p.previous] = [p.previous, p.version];
      }
    }
    k.pending = [];
  }
}

export default function updatesRoutes(ctx) {
  const { json, apiError } = ctx;
  const readBody = (req, fn) => {
    const chunks = [];
    req.on("data", (d) => chunks.push(d));
    req.on("end", () => fn(Buffer.concat(chunks)));
  };

  return function handle(req, res, { path, parts, query }) {
    const m = req.method;
    if (path === "/__stub/updates" && m === "POST") {
      readBody(req, (b) => {
        const v = b.length ? JSON.parse(b.toString()) : {};
        if (v.reset) st = initial();
        if (v.apply) apply();
        json(res, 200, { ok: true });
      });
      return true;
    }
    if (parts[0] !== "updates") return false;
    if (path === "/updates/packages" && m === "GET") {
      json(res, 200, [...st.packages].sort((a, b) => (a.product + a.platform).localeCompare(b.product + b.platform) || cmp(b.version, a.version)));
      return true;
    }
    if (path === "/updates/launchers" && m === "GET") {
      json(res, 200, st.launchers.map(entry));
      return true;
    }
    if (path === "/updates/packages" && m === "POST") {
      readBody(req, (body) => {
        const sig = Buffer.from(String(req.headers["x-vesper-signature"] ?? ""), "base64").toString();
        const c = /trusted comment: vesper-package product=(\S+) version=(\S+) platform=(\S+)/.exec(sig);
        if (!c) return apiError(res, 400, "badRequest", "the package was refused: this isn't a minisign signature file (no untrusted comment)");
        const [, product, version, platform] = c;
        if (st.packages.some((p) => p.product === product && p.version === version && p.platform === platform)) {
          return apiError(res, 409, "conflict", `${product} ${version} for ${platform} is already in the channel`);
        }
        const id = Math.random().toString(16).slice(2, 18).padEnd(16, "0");
        const p = { ...pkg(id, product, version, platform, body.length, 0), fileName: query.get("file") ?? "package" };
        st.packages.push(p);
        json(res, 201, p);
      });
      return true;
    }
    if (parts[1] === "packages" && parts.length === 3 && m === "DELETE") {
      const i = st.packages.findIndex((p) => p.id === parts[2]);
      if (i < 0) return apiError(res, 404, "notFound", "no such package"), true;
      st.packages.splice(i, 1);
      res.writeHead(204);
      res.end();
      return true;
    }
    if (path === "/updates/github" && m === "GET") {
      json(res, 200, st.github);
      return true;
    }
    if (path === "/updates/github" && m === "PUT") {
      readBody(req, (b) => {
        const v = JSON.parse(b.toString() || "{}");
        if (!/^[\w.-]+\/[\w.-]+$/.test(v.repository ?? "")) return apiError(res, 400, "badRequest", "the repository is owner/name, as on GitHub");
        st.github = { ...st.github, settings: v, error: undefined };
        json(res, 200, st.github);
      });
      return true;
    }
    if (path === "/updates/github/check" && m === "POST") {
      if (!st.github.settings.enabled) return apiError(res, 409, "conflict", "checking GitHub is off: turn it on first"), true;
      st.github = { ...st.github, lastCheck: new Date().toISOString(), lastSuccess: new Date().toISOString() };
      json(res, 202, st.github);
      return true;
    }
    if (path === "/updates/catalogue" && m === "GET") {
      const id = query.get("machine");
      if (!id) return json(res, 200, { products: CATALOGUE }), true;
      const k = st.launchers.find((x) => x.report.machineId === id);
      if (!k) return apiError(res, 404, "notFound", "no Launcher with that machine ID has reported"), true;
      json(res, 200, { products: offered(k) });
      return true;
    }
    if (parts[1] === "launchers" && parts[3] === "actions" && m === "POST") {
      readBody(req, (b) => {
        const a = JSON.parse(b.toString() || "{}");
        const k = st.launchers.find((x) => x.report.machineId === parts[2]);
        if (!k) return apiError(res, 404, "notFound", "no Launcher with that machine ID has reported");
        if (!k.pending.some((p) => JSON.stringify(p) === JSON.stringify(a))) k.pending.push(a);
        json(res, 202, entry(k));
      });
      return true;
    }
    return false;
  };
}
