// Screenshots of the Vesper Launcher's window (the tray app shows the Launcher's own page) and
// its first-run wizard, for design reviews. It runs the real Launcher service (a debug build)
// against a tiny stand-in for the site's server, with synthetic data only.
//
//   cargo build -p vesper-launcher
//   node apps/launcher-tray/shots.mjs [<output folder>]
//
// Uses Oriel's Playwright (apps/control: `npm ci` there first) and the same headless Chromium
// (ORIEL_CHROMIUM overrides it). The PNGs (440×640, the tray window's size) are never committed.

import { spawn } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "../control/node_modules/playwright-core/index.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const OUT = resolve(process.argv[2] ?? join(tmpdir(), "launcher-shots"));
const TARGET = process.env.CARGO_TARGET_DIR ?? join(ROOT, "target");
const CHROMIUM =
  process.env.ORIEL_CHROMIUM ??
  join(homedir(), ".cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
mkdirSync(OUT, { recursive: true });

// The site's server, as far as the Launcher sees it: its info, a pairing that waits for
// approval until `approve()`, reports, a newer package in the channel, and a product's health.
let approved = false;
const json = (res, status, body) => {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
};
const site = createServer((req, res) => {
  const u = req.url;
  if (u === "/api/v1/server") return json(res, 200, { name: "Main Server", version: "0.1.0" });
  if (u === "/api/v1/pair" && req.method === "POST") return json(res, 202, { pairingId: "0123456789abcdef", expiresInSeconds: 600 });
  if (u.startsWith("/api/v1/pair/"))
    return approved ? json(res, 200, { token: "idn1.synthetic.key", clientId: "a1b2c3d4" }) : json(res, 202, { pairingId: "0123456789abcdef", expiresInSeconds: 540 });
  if (u.startsWith("/api/v1/updates/launchers/")) return json(res, 200, { actions: [] });
  if (u === "/api/v1/updates/packages")
    return json(res, 200, [
      { id: "p1", product: "vesper", version: "0.2.0", platform: "linux-x86_64", fileName: "vesper_0.2.0-1_amd64.deb", size: 1, sha256: "", keyId: "", trustedComment: "", uploadedAt: "2026-11-25T10:00:00Z" },
    ]);
  json(res, 404, { error: { code: "notFound", message: "no such route" } });
});
await new Promise((r) => site.listen(0, "127.0.0.1", r));
const sitePort = site.address().port;
// A stand-in for GitHub: Vesper's releases ("Add app" lists the ones this machine can run).
// Packages aren't served: the shots don't install anything.
const ghRelease = (tag, files) => ({ tag_name: tag, prerelease: false, draft: false, assets: files.map((name) => ({ name, size: 1, browser_download_url: `http://127.0.0.1:${gh.address().port}/download/${tag}/${name}` })) });
const ghManifests = {
  "oriel-v0.2.0": { assets: [{ product: "vesper-oriel", platform: "linux-x86_64", file: "vesper-oriel_0.2.0_amd64.deb" }] },
  "idn-node-v0.3.8-beta18": { assets: [{ product: "vesper-idn-node", platform: "linux-x86_64", file: "vesper-idn-node_0.3.8~beta18-1_amd64.deb" }] },
};
const gh = createServer((req, res) => {
  const u = req.url;
  if (u.startsWith("/repos/"))
    return json(res, 200, Object.entries(ghManifests).map(([tag, m]) => ghRelease(tag, ["manifest.json", ...m.assets.flatMap((a) => [a.file, a.file + ".minisig"])])));
  const m = /^\/download\/([^/]+)\/manifest\.json$/.exec(u);
  if (m && ghManifests[m[1]]) return json(res, 200, ghManifests[m[1]]);
  json(res, 404, { message: "Not Found" });
});
await new Promise((r) => gh.listen(0, "127.0.0.1", r));
// The product's health check: the installed Vesper answers with its version.
const product = createServer((req, res) => json(res, 200, { version: "0.1.0" }));
await new Promise((r) => product.listen(0, "127.0.0.1", r));

const dir = mkdtempSync(join(tmpdir(), "launcher-shots-"));
const state = join(dir, "state");
mkdirSync(state, { recursive: true });
writeFileSync(join(state, "manifest.json"), JSON.stringify({ products: { vesper: { version: "0.1.0", previous: "0.0.9" } } }));
writeFileSync(
  join(dir, "vesper.log"),
  ["Vesper 0.1.0 starting", "data folder: /var/lib/vesper", "GPU: software renderer", "API on 0.0.0.0:5555", "Site: joined Main Server", "screen 1: output started"].map((l, i) => `2026-11-25T09:0${i}:00Z  INFO ${l}`).join("\n"),
);
// A stand-in for the bundled `idn doctor` (synthetic output, documentation addresses), and
// `systemctl` (the roles' steps succeed without touching this machine).
const idnDir = join(dir, "idn");
mkdirSync(idnDir, { recursive: true });
writeFileSync(
  join(idnDir, "idn"),
  `#!/bin/sh
sleep 1
cat <<'OUT'
Network
    2 virtual interface(s) (containers, VPN) ignored
  ✔ eth0 192.0.2.20 (1000 Mbit/s): wired
  ✔ routing table has a default or multicast route

Shared network check
  ⚠ 14 devices announce themselves here (TVs, speakers): video may stutter on Wi-Fi

Sources
  ✔ 3 sources found (Main Server: Screen 1, Main Server: Stage, Lobby Camera)

No problems, 1 thing(s) worth checking.
OUT
`,
);
chmodSync(join(idnDir, "idn"), 0o755);
const apiPort = 15000 + Math.floor(Math.random() * 20000);
writeFileSync(
  join(dir, "launcher.json"),
  JSON.stringify({
    stateDir: state,
    listen: `127.0.0.1:${apiPort}`,
    keyDir: null,
    machineName: "Sanctuary Box",
    platform: "linux-x86_64",
    healthTimeoutSecs: 5,
    checkIntervalSecs: 1,
    products: [
      { product: "vesper", name: "Vesper", install: { kind: "deb", units: ["vesper.service"] }, healthUrl: `http://127.0.0.1:${product.address().port}/api/v1/server`, logFile: join(dir, "vesper.log") },
      { product: "oriel", name: "Vesper Oriel", install: { kind: "dir" }, healthUrl: "http://127.0.0.1:9/api/v1/server" },
    ],
    system: { systemctl: "/bin/true", idnDir },
    github: { api: `http://127.0.0.1:${gh.address().port}`, repository: "example/vesper" },
  }),
);
const launcher = spawn(join(TARGET, "debug/vesper-launcher"), ["--config", join(dir, "launcher.json")], { stdio: ["ignore", "ignore", "inherit"] });
const base = `http://127.0.0.1:${apiPort}/`;
for (let i = 0; ; i++) {
  try { if ((await fetch(base + "launcher/v1/status")).ok) break; } catch {}
  if (i > 100) throw new Error("the Launcher didn't start");
  await sleep(100);
}

const browser = await chromium.launch({ executablePath: CHROMIUM });
const page = await browser.newPage({ viewport: { width: 440, height: 640 } });
page.on("console", (m) => m.type() === "error" && console.error("console:", m.text()));
page.on("pageerror", (e) => console.error("page error:", e.message));
const shot = async (name) => {
  await sleep(300);
  await page.screenshot({ path: join(OUT, `launcher-${name}.png`) });
  console.log(join(OUT, `launcher-${name}.png`));
};
try {
  await page.goto(base);
  await page.waitForSelector(".role");
  await shot("1-wizard-roles");
  // (A machine with the server shows "Presentation server" chosen already.)
  if (!(await page.isChecked('[data-role="presentation"]'))) await page.click('[data-role="presentation"]');
  await shot("1-wizard-roles-chosen");
  await page.click('[data-act="roles-next"]');
  await page.waitForSelector("#server");
  await sleep(2000); // the network search
  await page.fill("#server", `127.0.0.1:${sitePort}`);
  await shot("2-wizard-join");
  await page.click('[data-act="join"]');
  await page.waitForSelector(".spinner");
  await shot("3-wizard-waiting");
  approved = true;
  await page.waitForSelector(".big-ok", { timeout: 10000 });
  await shot("4-wizard-joined");
  await page.click('[data-act="finish"]');
  await page.waitForSelector(".product");
  await sleep(2500); // a report, with the channel's newer version
  await shot("5-products");
  await page.click('[data-product="vesper"] [data-act="logs"]');
  await page.waitForSelector("#logtext");
  await shot("6-logs");
  await page.click('[data-act="logs-close"]');
  await page.waitForSelector(".product");
  // "Set up this machine" again, at the smallest window size.
  await page.click('[data-act="wizard"]');
  await page.waitForSelector(".big-ok");
  await page.setViewportSize({ width: 380, height: 520 });
  await shot("7-wizard-small");
  await page.setViewportSize({ width: 440, height: 640 });
  await page.click('[data-act="finish"]');
  await page.waitForSelector(".product");
  // Diagnostics: idn doctor explained, the machine's status, Copy report.
  await page.click('[data-act="diagnostics"]');
  await page.waitForSelector(".headline", { timeout: 15000 });
  await shot("8-diagnostics");
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  await shot("8-diagnostics-bottom");
  await page.click('[data-act="diag-close"]');
  await page.waitForSelector(".product");
  // A display box whose IDN Node isn't installed: the roles' steps say what to do.
  await fetch(base + "launcher/v1/setup", { method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ roles: ["display"] }) });
  await page.waitForSelector(".rolesteps .failed", { timeout: 10000 });
  await shot("9-roles-display-needs-node");
  // Add app: what this display box can run and doesn't have, from GitHub and the site's server.
  await page.click('[data-act="add-open"]');
  await page.waitForSelector('[data-act="add-install"]', { timeout: 10000 });
  await shot("10-add-app");
} finally {
  await browser.close();
  launcher.kill();
  site.close();
  product.close();
  gh.close();
}
