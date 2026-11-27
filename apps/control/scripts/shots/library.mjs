// Shots of the Library page (see scripts/shots.mjs for `ctx`): Booth A's songs, the collections
// list, the "something is live" prompt when switching to Main Hall, its songs with the A–Z rail,
// a song, search over every collection, images, videos, a theme, a prop, Recently deleted, a show
// box's "pair with the Codex box" prompt (Oriel on the box itself has no key) and its "managed
// on" prompt (a second stub stands in for the Codex box) and the Codex
// unreachable line. Round 4: uploads in progress (then the same file again: "Already in this
// collection"), the Missing filter and the relink dialog, the
// all-collections view, the collection trash with the "Delete for good" confirmation, and a
// song in Edit. Linked themes: a theme with its template slides and used-by, deleting one songs
// follow, a new theme, a song that follows its theme (with an override, in Edit too) and a
// theme in the slide editor, with its template slides added (roles shown), an object's role and
// the delete that names the songs following it. The stub's library routes are reset first and the box shows Booth A again
// at the end.
import { readFileSync } from "node:fs";
import { presenceStub } from "../../test/stub/routes/presence.mjs";
import { fileURLToPath } from "node:url";
import { startStub } from "../../test/stub/server.mjs";
import { CODEX_BOX_ID } from "../../test/stub/routes/library.mjs";

const ctl = (url, body) => fetch(`${url}/api/v1/__stub/library`, { method: "POST", body: JSON.stringify(body) });

// The collection switches below need something live (the "Switch anyway" prompt), and the pages
// before this one (or the other screen size's Show page, with Clear All) may have cleared it:
// the stub's starting live slide is put back first.
const START = JSON.parse(readFileSync(fileURLToPath(new URL("../../test/stub/fixtures/ws__state.json", import.meta.url)), "utf8"));
const startSlide = (START.state ?? START).layers.slide;
const goLive = (url) =>
  fetch(`${url}/api/v1/commands`, {
    method: "POST",
    body: JSON.stringify({ type: "triggerSlide", presentation: startSlide.presentation, slide: startSlide.slide, playlistItem: startSlide.playlist }),
  });

export default async function ({ page, stub, openPage, shot, settle, step }) {
  await ctl(stub.url, { reset: true, role: "normal" });
  stub.push({ type: "codexChanged", parts: ["status", "collections", "trash"] });
  await openPage("Library");
  await shot("page-library");

  const where = page.getByRole("navigation", { name: "Library" });
  const node = (name) => where.locator(".node", { hasText: name }).first();
  const row = (name) => page.locator(".what .row", { hasText: name }).first();
  const dialogButton = (name) => page.getByRole("dialog").getByRole("button", { name });

  await step("collections", async () => {
    await goLive(stub.url);
    await settle(300);
    await where.locator(".collection").click();
    await settle(300);
    await shot("page-library-collections");
    await where.locator(".coll", { hasText: "Main Hall" }).click();
    await settle(300);
    await shot("page-library-liveshow");
    await dialogButton("Switch anyway").click();
    await settle(1200);
    await page.mouse.move(5, 300);
    await shot("page-library-songs");
  });

  await step("a song", async () => {
    await row("Lantern Road").click();
    await settle(1000);
    await shot("page-library-song");
  });

  await step("search", async () => {
    await page.locator(".what input[type=search]").fill("lantern");
    await settle(500);
    await page.getByRole("radio", { name: "All collections" }).click();
    await settle(600);
    await shot("page-library-search");
    await page.locator(".what input[type=search]").fill("");
    await settle(300);
  });

  await step("media", async () => {
    await node("Images").click();
    await settle(400);
    await page.locator(".what .tile").first().click();
    await settle(600);
    await shot("page-library-images");
    await node("Videos").click();
    await settle(400);
    await page.locator(".what .tile").nth(1).click();
    await settle(600);
    await shot("page-library-videos");
  });

  await step("theme and prop", async () => {
    await node("Themes").click();
    await settle(300);
    await row("Evening Lyrics").click();
    await settle(400);
    await shot("page-library-theme");
    await node("Props").click();
    await settle(300);
    await row("Welcome Banner").click();
    await settle(700);
    await shot("page-library-prop");
  });

  // A theme and a prop someone else edits (the presence stub's lock): no "Take over editing"
  // (the Library can't edit them, D09-6, E09-1), and the theme's Rename and Delete wait (E09-2).
  await step("theme and prop locked", async () => {
    const reported = async (kind) => {
      for (let i = 0; i < 30; i++) {
        const o = presenceStub.reported.find((x) => x.kind === kind && x.place === "library");
        if (o) return o.id;
        await settle(100);
      }
      throw new Error(`Oriel never reported a ${kind} open in the Library`);
    };
    const lockBy = (kind, id) => ({ kind, id, clientId: "booth-pc", name: "Oriel – Booth PC", computer: "Booth PC", user: "sam", since: "2026-11-24T10:15:00Z" });
    const editor = (kind, id) => [{ session: "s2", clientId: "booth-pc", name: "Oriel – Booth PC", computer: "Booth PC", user: "sam", open: [{ kind, id, place: "edit" }] }];
    const takeOver = page.getByRole("button", { name: "Take over editing" });
    try {
      await node("Themes").click();
      await settle(300);
      await row("Evening Lyrics").click();
      const theme = await reported("theme");
      presenceStub.set({ clients: editor("theme", theme), locks: [lockBy("theme", theme)] });
      await page.locator("[data-lock-banner]").waitFor({ timeout: 3000 });
      await settle(300);
      if (await takeOver.count()) throw new Error("the Library offers to take over a theme");
      if (!(await page.getByRole("toolbar", { name: "Theme" }).getByRole("button", { name: "Rename" }).isDisabled())) throw new Error("a theme someone edits can be renamed in the Library");
      await page.mouse.move(5, 300);
      await shot("page-library-theme-locked");
      await node("Props").click();
      await settle(300);
      await row("Welcome Banner").click();
      const prop = await reported("prop");
      presenceStub.set({ clients: editor("prop", prop), locks: [lockBy("prop", prop)] });
      await page.locator("[data-lock-banner]").waitFor({ timeout: 3000 });
      await settle(300);
      if (await takeOver.count()) throw new Error("the Library offers to take over a prop");
      await page.mouse.move(5, 300);
      await shot("page-library-prop-locked");
    } finally {
      presenceStub.set({ clients: [], locks: [] });
      await settle(200);
    }
  });

  await step("Recently deleted", async () => {
    await node("Recently deleted").click();
    await settle(300);
    await page.locator(".what .tmain").first().click();
    await settle(300);
    await shot("page-library-trash");
  });

  await step("uploads", async () => {
    await node("Images").click();
    await settle(300);
    await ctl(stub.url, { uploadDelay: 2500 });
    await page.locator(".what").getByRole("button", { name: "+ Import" }).click();
    await settle(200);
    await shot("page-library-import");
    const mb = 1024 * 1024;
    // Playwright takes at most 50 MB of file buffers per call.
    const file = (name, size, seed) => ({ name, mimeType: name.endsWith(".mp4") ? "video/mp4" : "image/jpeg", buffer: Buffer.alloc(size, seed) });
    await page.locator(".what input[type=file][accept]").first().setInputFiles([
      file("Sunrise Hills.jpg", 20 * mb, 1),
      file("Choir Rehearsal Wide.jpg", 14 * mb, 2),
      file("Lobby Welcome Loop.mp4", 12 * mb, 3),
    ]);
    await page.waitForTimeout(3200);
    await page.mouse.move(5, 300);
    await shot("page-library-upload");
    await ctl(stub.url, { uploadDelay: 0 });
    await page.waitForFunction(() => !document.querySelector(".tray .waiting, .tray li:not(.done)"), null, { timeout: 30_000 }).catch(() => {});
    await settle(400);
    // The same file again: the collection's asset is answered (`existing`), no second tile.
    await page.locator(".what input[type=file][accept]").first().setInputFiles([file("Sunrise Hills.jpg", 20 * mb, 1)]);
    await page.waitForFunction(() => document.querySelectorAll(".tray li.done").length >= 4, null, { timeout: 30_000 }).catch(() => {});
    await settle(400);
    await page.mouse.move(5, 300);
    await shot("page-library-upload-existing");
    await page.locator(".tray").getByRole("button", { name: "Clear" }).click();
  });

  await step("missing media", async () => {
    await page.locator(".what").getByRole("radio", { name: /Missing/ }).click();
    await settle(300);
    await page.locator(".what .tile").first().click();
    await settle(500);
    await shot("page-library-missing");
    await page.getByRole("button", { name: "Relink…" }).click();
    await settle(500);
    await page.getByRole("dialog").locator(".tile").first().click();
    await settle(200);
    await shot("page-library-relink");
    await dialogButton("Use this file").click();
    await settle(600);
    await page.locator(".what").getByRole("radio", { name: "All" }).click();
    await settle(200);
  });

  await step("all collections", async () => {
    await where.locator(".collection").click();
    await settle(200);
    await where.getByRole("button", { name: "Manage collections…" }).click();
    await settle(500);
    await page.locator(".what .row", { hasText: "Main Hall" }).first().click();
    await settle(300);
    await shot("page-library-collections-all");
    await page.locator(".what .row", { hasText: "Summer Camp 2025" }).first().click();
    await settle(300);
    await shot("page-library-collection-trash");
    await page.getByRole("button", { name: "Delete for good…" }).click();
    await settle(200);
    await dialogButton("Continue").click();
    await settle(300);
    await shot("page-library-delete-for-good");
    await page.keyboard.press("Escape");
    await settle(200);
  });

  await step("themes (linked)", async () => {
    await node("Themes").click();
    await settle(300);
    await row("Evening Lyrics").click();
    await settle(700);
    // Deleting a theme songs follow: move them to another theme, or keep their look.
    await page.locator(".item").getByRole("button", { name: "Delete…" }).click();
    await settle(300);
    await shot("page-library-theme-delete");
    await page.locator(".item").getByRole("button", { name: "Cancel" }).click();
    await page.locator(".what").getByRole("button", { name: "+ New theme" }).click();
    await settle(300);
    await page.getByRole("dialog").locator("input").fill("Christmas Eve");
    await dialogButton("Create").click();
    await settle(900);
    await shot("page-library-theme-new");
    // A song that follows its theme, one slide with a lyrics box of its own.
    await node("All songs").click();
    await settle(400);
    await row("Amber Fields").click();
    await settle(1000);
    await shot("page-library-song-theme");
    // An imported song (no theme): "Link to theme, keeping the look" (batch 9).
    await row("Northbound Light").click();
    await settle(1000);
    await shot("page-library-song-link-keep-look");
    await page.getByTestId("link-keep-look").click();
    await settle(1000);
    await shot("page-library-song-linked");
    // A passage made on the Bible page, edited, whose theme changed since (batch 9).
    await ctl(stub.url, { stalePassage: "Northbound Light" });
    await row("Amber Fields").click();
    await settle(600);
    await row("Northbound Light").click();
    await settle(1000);
    await shot("page-library-passage-stale");
    await page.getByTestId("passage-stale").getByRole("button", { name: "Split again for the theme" }).click();
    await settle(400);
    await shot("page-library-passage-resplit");
    await dialogButton("Split again").click();
    await settle(800);
    await row("Amber Fields").click();
    await settle(1000);
    await page.locator(".item").getByRole("radio", { name: "Edit" }).click();
    await settle(1500);
    await page.locator(".what [data-slide-row]").nth(1).click();
    await settle(600);
    await page.locator("[data-object-row]").nth(0).click();
    await settle(600);
    await shot("page-library-edit-theme-override");
    await page.keyboard.press("Escape");
    await page.locator(".what").getByRole("button", { name: "Done" }).click();
    await settle(400);
    // The theme's template slides in the slide editor.
    await node("Themes").click();
    await settle(300);
    await row("Evening Lyrics").click();
    await settle(400);
    await page.locator(".item").getByRole("button", { name: "Edit in slide editor" }).click();
    await settle(1500);
    await shot("page-edit-theme");
    // Template slides (batch 9): "+" offers the starter layouts; a new Scripture slide with
    // its roles; an object's role in the inspector; deleting the slide songs follow.
    await page.getByTestId("theme-add-slide").click();
    await settle(300);
    await shot("page-edit-theme-add");
    await page.getByRole("menuitem", { name: "Scripture template slide" }).click();
    await settle(1200);
    await page.locator("[data-object-row]").nth(1).click();
    await settle(600);
    await shot("page-edit-theme-role");
    await page.keyboard.press("Escape");
    await page.locator("[data-slide-row]").first().click({ button: "right" });
    await settle(300);
    await page.getByRole("menuitem", { name: "Delete…" }).click();
    await settle(500);
    await shot("page-edit-theme-delete");
    await dialogButton("Cancel").click();
    await settle(300);
    await page.locator("[data-testid=prop-done]").click();
    await settle(400);
    await openPage("Library");
    await settle(400);
  });

  await step("a song in Edit", async () => {
    await node("All songs").click();
    await settle(400);
    await row("Lantern Road").click();
    await settle(600);
    await page.locator(".item").getByRole("radio", { name: "Edit" }).click();
    await settle(1800);
    await page.mouse.move(900, 600);
    await shot("page-library-edit");
    // The slide editor's inspector in the Library (one editor, two places).
    await page.locator("[data-object-row]").first().click();
    await settle(600);
    await shot("page-library-edit-object");
    await page.keyboard.press("Escape");
    await page.locator(".what").getByRole("button", { name: "Done" }).click();
    await settle(400);
    // The show box's shots start from Recently deleted.
    await node("Recently deleted").click();
    await settle(300);
  });

  // A show box: emptying the trash is managed on the Codex box (a second stub).
  // Only the library routes: the other areas' route state stays as the main stub has it.
  const hub = await startStub({ port: 0, routes: ["library"] });
  const codexBox = { serverId: CODEX_BOX_ID, name: "Codex Hub", addresses: ["http://127.0.0.1:5998", hub.url] };
  try {
    await step("managed on the Codex box", async () => {
      await ctl(hub.url, { role: "codexBox", name: "Codex Hub" });
      await ctl(stub.url, { role: "showBox", codexBox });
      stub.push({ type: "codexChanged", parts: ["status"] });
      await settle(500);
      // The stub is reached locally (no key): the Codex box would refuse this Oriel, which
      // offers to pair it there first.
      await page.locator(".what").getByRole("button", { name: "Empty trash" }).click();
      await settle(200);
      await dialogButton("Delete for good").click();
      await settle(800);
      await shot("page-library-unpaired-prompt");
      await dialogButton("Cancel").click();
      await settle(200);
      // Paired there before: the Codex box's key is in this Oriel's store.
      await page.evaluate((id) => localStorage.setItem(`oriel.token.${id}`, "stub-site-key"), CODEX_BOX_ID);
      await page.locator(".what").getByRole("button", { name: "Empty trash" }).click();
      await settle(200);
      await dialogButton("Delete for good").click();
      await settle(800);
      await shot("page-library-managed-prompt");
      await dialogButton("Use Codex Hub").click();
      await settle(1000);
      await shot("page-library-managed-on");
    });
    await step("Codex unreachable", async () => {
      await ctl(stub.url, { role: "showBox", codexBox, reachable: false });
      stub.push({ type: "codexChanged", parts: ["status"] });
      await settle(600);
      await shot("page-library-unreachable");
    });
    // G15: this show box's own collection, offered for upload to the site's Codex.
    await step("upload the dormant collection", async () => {
      await ctl(stub.url, { role: "showBox", codexBox, dormant: true });
      stub.push({ type: "codexChanged", parts: ["status"] });
      await settle(800);
      const offer = page.getByTestId("dormant-offer");
      await offer.scrollIntoViewIfNeeded();
      await shot("page-library-dormant-offer");
      await offer.getByRole("button", { name: "Upload it" }).click();
      await settle(800);
      await shot("page-library-dormant-copying");
      await ctl(stub.url, { dormantFinish: true });
      stub.push({ type: "codexChanged", parts: ["status"] });
      await settle(1500);
      await shot("page-library-dormant-done");
      await ctl(stub.url, { dormant: false });
    });
  } finally {
    await page.evaluate((id) => localStorage.removeItem(`oriel.token.${id}`), CODEX_BOX_ID).catch(() => {});
    await hub.close();
  }

  // Back as it was: a host showing Booth A, the trash as at start.
  await step("back to Booth A", async () => {
    await ctl(stub.url, { role: "normal" });
    stub.push({ type: "codexChanged", parts: ["status"] });
    await goLive(stub.url);
    await settle(400);
    // (The list may still be open from the switch above.)
    if (!(await where.locator(".coll").first().isVisible())) await where.locator(".collection").click();
    await settle(200);
    await where.locator(".coll", { hasText: "Booth A" }).click();
    await settle(200);
    await dialogButton("Switch anyway").click();
    await settle(800);
  });
  await ctl(stub.url, { reset: true, role: "normal" });
  stub.push({ type: "codexChanged", parts: ["status", "collections", "trash"] });
  await settle(300);
}
