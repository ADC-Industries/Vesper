// Screenshots of Oriel's screens against the stub server (test/stub/), for design reviews.
//
//   npm run shots [-- <output folder>]
//
// Linux only for now: it drives the headless Chromium shell that Playwright installed on the
// build machine. Set ORIEL_CHROMIUM to another Chromium / Chrome binary elsewhere.
//
// - Builds `dist/` first unless ORIEL_SHOTS_NO_BUILD=1.
// - Output folder: the first argument, else ORIEL_SHOTS_DIR, else <tmp>/oriel-shots. The PNGs are
//   never committed.
// - Every screen at 1920×1080 and 1366×768: connect (unreachable, pairing, key revoked), the Show
//   page (header and status bar), the Oriel menu's panes, the Look picker, the connection pill,
//   the "?" list, Edit, Reflow and every page button's page, toasts, the offline and reconnected
//   banners. Synthetic data only (test/stub/fixtures and the notices below).
// - Browser console errors and page errors are printed to stderr.
// - Each page's own shots are in `scripts/shots/<page>.mjs` (one file per page, so parallel lanes
//   don't edit the same file). Every file there default-exports `async (ctx) => {}`; it starts on
//   the Show page (playlist open, a presentation selected) and goes back there afterwards (Ctrl+S).
//   `ctx`: `page` (Playwright), `w`, `h`, `stub`, `shot(name)` (writes `<w>-<name>.png`),
//   `settle(ms)`, `esc()`, `step(label, fn)` and `openPage(name)` (clicks a header page button).

import { execSync } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium } from "playwright-core";
import { slideSvg, startStub } from "../test/stub/server.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = resolve(process.argv[2] ?? process.env.ORIEL_SHOTS_DIR ?? join(tmpdir(), "oriel-shots"));
const CHROMIUM =
  process.env.ORIEL_CHROMIUM ??
  join(homedir(), ".cache/ms-playwright/chromium_headless_shell-1223/chrome-headless-shell-linux64/chrome-headless-shell");
const SIZES = [
  [1920, 1080],
  [1366, 768],
];

if (process.env.ORIEL_SHOTS_NO_BUILD !== "1") execSync("npm run build", { cwd: ROOT, stdio: ["ignore", "ignore", "inherit"] });
mkdirSync(OUT, { recursive: true });

const browser = await chromium.launch({ executablePath: CHROMIUM });
const written = [];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The live preview: the live slide of the fixtures drawn as a JPEG by the browser.
async function makePreviewJpeg() {
  const page = await browser.newPage({ viewport: { width: 640, height: 360 } });
  const state = JSON.parse(readFileSync(join(ROOT, "test/stub/fixtures/ws__state.json"), "utf8")).state;
  const live = state.layers.slide;
  const doc = JSON.parse(readFileSync(join(ROOT, `test/stub/fixtures/presentations__${live.presentation}.json`), "utf8"));
  const slide = doc.sections.flatMap((s) => s.slides).find((s) => s.id === live.slide);
  const lines = slide.elements.flatMap((e) => e.kind?.text?.paragraphs ?? []).map((p) => p.runs.map((r) => r.text).join(""));
  await page.setContent(`<body style="margin:0;background:#000">${slideSvg(lines, "#2F5FB8", 640, 360)}</body>`);
  const jpeg = await page.screenshot({ type: "jpeg", quality: 85 });
  await page.close();
  return jpeg;
}

// The pages' own shots (scripts/shots/*.mjs), in file name order.
const PAGE_SHOTS = await Promise.all(
  readdirSync(join(ROOT, "scripts/shots"))
    .filter((f) => f.endsWith(".mjs"))
    // ORIEL_SHOTS_ONLY=library,site: only those pages' own shots (while working on them).
    .filter((f) => !process.env.ORIEL_SHOTS_ONLY || process.env.ORIEL_SHOTS_ONLY.split(",").includes(f.slice(0, -4)))
    .sort()
    .map(async (f) => ({ name: f.slice(0, -4), run: (await import(pathToFileURL(join(ROOT, "scripts/shots", f)).href)).default })),
);

const stub = await startStub({ dist: join(ROOT, "dist"), previewJpeg: await makePreviewJpeg() });

async function settle(page, ms = 600) {
  await page.waitForLoadState("networkidle").catch(() => {});
  await page.evaluate(() => document.fonts.ready);
  await sleep(ms);
}

async function shot(page, w, name) {
  const file = join(OUT, `${w}-${name}.png`);
  await page.screenshot({ path: file });
  written.push(file);
}

/** Runs a step; a selector that moved in a redesign logs a warning instead of stopping. */
async function step(label, fn) {
  try {
    await fn();
  } catch (e) {
    // The first line, and which element Playwright was waiting for.
    const lines = e.message.split("\n");
    const what = lines.find((l) => l.includes("waiting for"))?.trim();
    console.warn(`shots: ${label}: ${lines[0]}${what ? ` (${what})` : ""}`);
  }
}

const NOTICES = [
  { type: "notice", level: "warning", message: "3 media files are missing in “Wide Open Fields”", key: "missingMedia:a" },
  { type: "notice", level: "error", message: "Couldn't clear the props: the server is restarting", key: "props" },
  { type: "notice", level: "warning", message: "The font “Harbor Sans” isn't installed on the server; another font is used", key: "fonts" },
  { type: "notice", level: "error", message: "Output “Lobby” stopped: the receiver is gone", key: "output:lobby" },
  { type: "notice", level: "info", message: "Timer “Countdown” reset", key: "timer" },
];

for (const [w, h] of SIZES) {
  const context = await browser.newContext({ viewport: { width: w, height: h }, deviceScaleFactor: 1 });
  const page = await context.newPage();
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket|ERR_EMPTY_RESPONSE|ERR_CONNECTION|501|404|423/.test(m.text())) console.error(`[${w}] console: ${m.text()}`);
  });
  page.on("pageerror", (e) => console.error(`[${w}] page error: ${e.message}`));
  const esc = async () => {
    await page.keyboard.press("Escape");
    await settle(page, 250);
  };

  // Connect screen: no server answering, then a server that wants pairing.
  stub.setMode("down");
  await page.goto(stub.url + "/");
  await settle(page);
  await shot(page, w, "connect");
  stub.setMode("unpaired");
  await page.goto(stub.url + "/");
  await settle(page);
  await shot(page, w, "connect-pair");

  // Show page with the playlist open and a presentation selected: header and status bar.
  stub.setMode("local");
  await page.goto(stub.url + "/");
  await settle(page, 1000);
  await step("open the playlist", async () => {
    const playlists = page.getByRole("list", { name: "Playlists" });
    if (!(await playlists.getByText("Sunday Morning").isVisible().catch(() => false))) await playlists.getByText("Sundays").click();
    await playlists.getByText("Sunday Morning").click();
    await page.getByRole("list", { name: "Playlist items" }).getByText("Morning Lantern").click();
  });
  await page.mouse.move(w - 5, h - 5);
  await settle(page, 1500);
  await shot(page, w, "show");

  // The Oriel menu, every pane.
  await step("Oriel menu", async () => {
    await page.locator(".oriel .logo").click();
    await settle(page, 200);
    const panes = ["file", "edit", "view", "import", "export", "settings", "help"];
    for (let i = 0; i < panes.length; i++) {
      await page.locator(`[data-cat="${i}"]`).hover();
      await settle(page, 150);
      await shot(page, w, `menu-${panes[i]}`);
    }
  });
  await esc();
  await page.mouse.move(w - 5, h - 5);

  // Look picker, connection pill, "?".
  await step("Look picker", async () => {
    await page.locator(".look .pick").click();
    await settle(page, 200);
    await shot(page, w, "look-picker");
  });
  await esc();
  await step("connection pill", async () => {
    await page.locator(".conn .pill").click();
    await settle(page, 400);
    await shot(page, w, "connection");
  });
  await esc();
  await step("shortcut list", async () => {
    await page.getByRole("button", { name: "Keyboard shortcuts" }).click();
    await settle(page, 300);
    await shot(page, w, "shortcuts");
  });
  await esc();

  // Edit and Reflow (modes) and every page: scripts/shots/<page>.mjs.
  const ctx = {
    page,
    w,
    h,
    stub,
    shot: (name) => shot(page, w, name),
    settle: (ms) => settle(page, ms),
    esc,
    step,
    async openPage(name) {
      await page.getByRole("navigation", { name: "Pages" }).getByRole("button", { name }).click();
      await settle(page, 900);
      await page.mouse.move(w - 5, h - 5);
    },
  };
  for (const { name, run } of PAGE_SHOTS) {
    await step(`page ${name}`, () => run(ctx));
    await page.keyboard.press("Control+s");
    await settle(page, 300);
  }

  // Toasts: five notices, three lines on screen (two toasts and "3 more").
  for (const n of NOTICES) stub.push(n);
  await settle(page, 300);
  await shot(page, w, "toasts");
  await page.reload();
  await settle(page, 1200);

  // Offline: the stub drops the socket and refuses reconnects.
  stub.setMode("down");
  await sleep(2500);
  await shot(page, w, "offline");

  // Back online: the green banner for 3.5 s.
  stub.setMode("local");
  await page.getByRole("button", { name: "Retry now" }).click({ timeout: 2000 }).catch(() => {});
  await sleep(1200);
  await shot(page, w, "reconnected");
  await sleep(3000);

  // The site removes this computer: the socket closes with 4003, back to the connect screen.
  stub.revoke();
  await settle(page, 800);
  await shot(page, w, "revoked");
  stub.setMode("local");

  await context.close();
}

await browser.close();
await stub.close();
console.log(written.join("\n"));
