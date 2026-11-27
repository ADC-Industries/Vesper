// Shots of the Library's media bins and groups (G17; see scripts/shots.mjs for `ctx`): the
// stub's bins routes are turned on (bins from the captured fixtures, a media playlist, four
// made-up groups), then a bin with its items, a file dragged from Images onto a bin, the bin
// after the drop, the "New media bin" prompt, a media playlist and Design ▸ Groups; and the
// collection card's "Synced to 2 of 3 Vespers" (G14, the library stub's show boxes). The routes
// are reset and turned off at the end, so the other pages keep the captured lists.
import { binsStub } from "../../test/stub/routes/bins.mjs";

export default async function ({ page, stub, openPage, shot, settle, step, esc }) {
  binsStub.reset();
  binsStub.on = true;
  const changed = () => {
    stub.push({ type: "documentChanged", kind: "mediaBin", id: "5eed0000-0000-4000-8000-0000000b0002", change: "changed" });
    stub.push({ type: "documentChanged", kind: "globalGroup", id: "5eed0000-0000-4000-8000-0000000b1001", change: "changed" });
    stub.push({ type: "documentChanged", kind: "playlistNode", id: "5eed0000-0000-4000-8000-0000000b0001", change: "changed" });
  };
  changed();
  await openPage("Library");
  await settle(800);
  const where = page.getByRole("navigation", { name: "Library" });
  const node = (name) => where.locator(".node", { hasText: name }).first();

  await step("a bin", async () => {
    await node("Motion Loops").click();
    await settle(800);
    await page.mouse.move(5, 300);
    await shot("page-library-bin");
  });

  await step("drag a file onto a bin", async () => {
    await node("Images").click();
    await settle(800);
    const tile = page.locator(".what .tile").first();
    const box = await tile.boundingBox();
    const target = await node("Countdowns").boundingBox();
    if (!box || !target) throw new Error("no tile or bin to drag onto");
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 20, box.y + box.height / 2 + 10, { steps: 4 });
    await page.mouse.move(target.x + target.width / 2, target.y + target.height / 2, { steps: 10 });
    await settle(300);
    await shot("page-library-bin-drag");
    await page.mouse.up();
    await settle(800);
    await node("Countdowns").click();
    await settle(800);
    await page.mouse.move(5, 300);
    await shot("page-library-bin-added");
  });

  await step("new bin", async () => {
    await where.getByRole("button", { name: "New bin" }).click();
    await settle(400);
    await page.getByRole("dialog").locator("input").fill("Announcements");
    await shot("page-library-bin-new");
    await page.getByRole("dialog").getByRole("button", { name: "Create" }).click();
    await settle(800);
    await page.mouse.move(5, 300);
    await shot("page-library-bin-created");
  });

  await step("a media playlist", async () => {
    await node("Pre-service Loops").click();
    await settle(800);
    await shot("page-library-media-playlist");
  });

  await step("groups", async () => {
    await node("Groups").click();
    await settle(600);
    await shot("page-library-groups");
  });

  await step("synced to N of M Vespers", async () => {
    await where.locator(".collection").click();
    await settle(500);
    await shot("page-library-synced");
    await where.locator(".collection").click();
    await settle(200);
  });

  // The Library opens where it was left: back to All songs for the other pages' shots.
  await step("back to All songs", async () => {
    await node("All songs").click();
    await settle(300);
  });
  await esc();
  binsStub.reset();
  binsStub.on = false;
  changed();
  await settle(400);
}
