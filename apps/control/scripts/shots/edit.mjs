// Shots of the Edit page (see scripts/shots.mjs for `ctx`), with a synthetic presentation whose
// slides have several objects (test/stub/routes/edit.mjs): nothing selected (Presentation tab
// and its transitions grid; a push upwards, then a zoom), the Slide tab (group select), the Actions tab, a text object (Text
// tab, then Copy / Paste Style; a scrolling text ticker and its Scrolling section), a shape (Shape tab; a double stroke; a media fill's video settings; a live source fill, then a video input as the fill) and several objects (Arrange tab);
// the Build tab (build order, step preview, a build out added) and the Show page's build progress;
// the song's artwork (Presentation tab), the Live text menu, an object's visibility conditions (a
// video input among them).
import { editStub } from "../../test/stub/routes/edit.mjs";

export default async function ({ page, settle, shot, stub, step }) {
  const changed = () => stub.push({ type: "documentChanged", kind: "presentation", id: editStub.id, change: "changed" });
  const row = (name) => page.locator("[data-object-row]").filter({ has: page.locator(".name").getByText(name, { exact: true }) });
  const tab = (name) => page.getByRole("tab", { name, exact: true });
  editStub.reset();
  editStub.on = true;
  try {
    changed();
    await settle(600);
    await page.keyboard.press("Control+e");
    await settle(1500);
    await page.mouse.move(5, 300);
    await shot("page-edit");

    await step("edit transitions grid", async () => {
      await page.locator("[data-transition-grid]").first().scrollIntoViewIfNeeded();
      await settle(200);
      await shot("edit-transitions");
    });

    await step("edit transition push: direction, duration and easing", async () => {
      const grid = page.locator("[data-transition-grid]").first();
      await grid.locator('[data-transition="push"]').click();
      await settle(400);
      await page.locator('[data-transition-direction] [data-direction="up"]').first().click();
      await settle(400);
      await page.locator("[data-transition-direction]").first().scrollIntoViewIfNeeded();
      await shot("edit-transition-push");
      await grid.locator('[data-transition="zoom"]').click();
      await settle(400);
      await shot("edit-transition-zoom");
    });

    await step("edit song artwork: picked from the media bins", async () => {
      const artRow = page.locator('[data-field="song-artwork"]');
      await artRow.scrollIntoViewIfNeeded();
      await artRow.getByRole("button", { name: "Choose…" }).click();
      await settle(800);
      // The picker opens on the Show page's bin: go to one with images.
      const items = page.locator("[data-media-item]:not([disabled])");
      const binSelect = page.getByRole("dialog", { name: "Choose an image" }).getByRole("combobox", { name: "Media bin" });
      for (const v of await binSelect.locator("option").evaluateAll((os) => os.map((o) => o.value))) {
        if (await items.count()) break;
        await binSelect.selectOption(v);
        await settle(600);
      }
      await items.first().click();
      await settle(900);
      const art = editStub.edits.flatMap((e) => e.ops).filter((o) => o.type === "setPresentation" && o.songInfo?.artwork);
      if (!art.length) throw new Error("no artwork was sent");
      if (art.at(-1).songInfo.title !== "Morning Light") throw new Error("the artwork edit lost the song's other fields");
      await artRow.scrollIntoViewIfNeeded();
      await shot("edit-song-artwork");
    });

    await step("edit live text menu: presentation info and song artwork", async () => {
      await page.locator('[data-insert="link"]').click();
      await settle(400);
      for (const name of ["Presentation Name", "CCLI Copyright", "Song Artwork"]) {
        if (!(await page.getByRole("menuitem", { name, exact: true }).isVisible())) throw new Error(`no ${name} in the live text menu`);
      }
      await shot("edit-live-text-menu");
      await page.keyboard.press("Escape");
      await settle(300);
    });

    await step("edit slide tab", async () => {
      await tab("Slide").click();
      await settle(300);
      await shot("edit-slide");
    });

    await step("edit actions tab", async () => {
      await tab("Actions").click();
      await settle(300);
      await shot("edit-actions");
    });

    await step("edit build tab: the build order, the step preview, a build out added", async () => {
      await tab("Build").click();
      await settle(400);
      if ((await page.locator("[data-build-row]").count()) !== 3) throw new Error("the title slide's three builds are not listed");
      await shot("edit-builds");
      await page.locator('[data-action="build-preview"]').click();
      await settle(800);
      await shot("edit-builds-preview-0");
      await page.getByRole("button", { name: "Next step" }).click();
      await settle(800);
      await shot("edit-builds-preview-1");
      await page.locator('[data-action="build-preview"]').click();
      await settle(300);
      await row("Sun").click();
      await settle(300);
      await tab("Build").click();
      await settle(200);
      const before = editStub.edits.length;
      await page.locator('[data-action="build-out"]').click();
      await settle(800);
      const ops = editStub.edits.slice(before).flatMap((e) => e.ops);
      const set = ops.find((o) => o.type === "setSlide" && Array.isArray(o.builds));
      if (!set || set.builds.length !== 4 || set.builds[3].mode !== "out") throw new Error(`no build out sent: ${JSON.stringify(ops)}`);
      await page.locator("[data-build-row]").last().scrollIntoViewIfNeeded();
      await shot("edit-builds-added");
      await page.keyboard.press("Escape");
      await settle(200);
      // Back to the tab the next steps start from.
      await tab("Actions").click();
      await settle(200);
    });

    await step("edit text object", async () => {
      await row("Subtitle").click();
      await settle(800);
      await shot("edit-text");
    });

    await step("edit scrolling text: insert a ticker, its Scrolling section", async () => {
      await page.locator('[data-insert="scroll"]').click();
      await settle(800);
      await page.keyboard.press("Escape");
      await settle(400);
      await row("Scrolling text").click();
      await settle(600);
      await page.locator('[data-scroll-dir="up"]').click();
      await settle(500);
      await page.locator("[data-scroll-section]").scrollIntoViewIfNeeded();
      await settle(200);
      await shot("edit-scrolling");
      await page.keyboard.press("Delete");
      await settle(600);
    });

    // Selects its own text object: the scrolling step before it deletes its ticker, which leaves
    // nothing selected (and so no Text tab).
    await step("edit copy / paste style", async () => {
      await row("Subtitle").click();
      await settle(400);
      await page.getByRole("button", { name: "Copy Style" }).click();
      await row("Lower bar text").click();
      await settle(300);
      await page.getByRole("button", { name: "Paste Style" }).click();
      await settle(1200);
      await shot("edit-paste-style");
    });

    await step("edit shape", async () => {
      await row("Sun").click();
      await settle(800);
      await shot("edit-shape");
    });

    await step("edit media fill: video loop and start offset", async () => {
      await row("Colour bars").click();
      await settle(800);
      await page.locator("[data-video-sound]").scrollIntoViewIfNeeded();
      await shot("edit-media-fill");
      await row("Sun").click();
      await settle(600);
    });

    await step("edit stroke style: double", async () => {
      await page.locator('[data-field="stroke-enabled"]').check();
      await settle(500);
      await page.locator('[data-field="stroke-style"]').selectOption("double");
      await settle(600);
      await page.locator('[data-field="stroke-style"]').scrollIntoViewIfNeeded();
      await shot("edit-stroke-double");
      await page.locator('[data-field="stroke-enabled"]').uncheck();
      await settle(500);
    });

    await step("edit arrange", async () => {
      await row("Colour bars").click({ modifiers: ["Control"] });
      await row("Lower bar").click({ modifiers: ["Control"] });
      await settle(800);
      await shot("edit-arrange");
      await page.getByRole("button", { name: "Align top on the slide" }).click();
      await settle(1200);
      await shot("edit-arrange-aligned");
    });

    // Checks (no shots): what the editor sends (test/stub/routes/edit.mjs logs the edits).
    const sent = () => editStub.edits.flatMap((e) => e.ops);
    await step("edit visibility conditions: listed, added, removed", async () => {
      await row("Lower bar").click();
      await settle(600);
      await tab("Shape").click();
      await settle(300);
      const vis = page.locator("[data-visibility]");
      await vis.scrollIntoViewIfNeeded();
      if ((await vis.locator("[data-condition]").count()) !== 1) throw new Error("the imported condition is not listed");
      await shot("edit-visibility");
      await vis.locator('[data-field="add-condition"]').selectOption("message");
      await settle(900);
      const adds = sent().filter((o) => o.type === "setElement" && o.patch?.visibility);
      if (adds.at(-1)?.patch.visibility.length !== 2) throw new Error("the new condition was not sent with the list");
      await vis.locator('[data-condition="message"] select').nth(1).selectOption("no");
      await settle(900);
      if (sent().filter((o) => o.patch?.visibility).at(-1).patch.visibility[1].active !== false) throw new Error("'not showing' was not sent");
      await vis.scrollIntoViewIfNeeded();
      await shot("edit-visibility-added");
      await vis.getByRole("button", { name: /^Remove condition: Next item/ }).click();
      await settle(900);
      const left = sent().filter((o) => o.patch?.visibility).at(-1).patch.visibility;
      if (left.length !== 1 || left[0].type !== "message") throw new Error("the wrong condition was removed");
      // "Video input live" offers the show's video inputs (GET /video-inputs).
      await vis.locator('[data-field="add-condition"]').selectOption("videoInput");
      await settle(900);
      const withInput = sent().filter((o) => o.patch?.visibility).at(-1).patch.visibility;
      if (withInput.at(-1)?.type !== "videoInput" || withInput.at(-1).input.name !== "Wide shot") throw new Error("the video input condition did not name the first input");
      await vis.locator('[data-field="condition-input"]').selectOption({ label: "Pulpit camera" });
      await settle(600);
      await vis.locator('[data-condition="videoInput"] select').nth(1).selectOption("no");
      await settle(900);
      const picked = sent().filter((o) => o.patch?.visibility).at(-1).patch.visibility.at(-1);
      if (picked.input.name !== "Pulpit camera" || picked.active !== false) throw new Error("the chosen input / 'not live' was not sent");
      await vis.scrollIntoViewIfNeeded();
      await shot("edit-visibility-input");
      await vis.getByRole("button", { name: /^Remove condition: Pulpit camera/ }).click();
      await settle(900);
    });

    await step("edit live source fill: a camera in the box", async () => {
      await row("Sun").click();
      await settle(600);
      await page.locator('[data-field="fill-kind"]').selectOption("live");
      await settle(900);
      await page.locator('[data-field="live-source"]').selectOption("BOOTH-PC (Cam 2 Pulpit)");
      await settle(900);
      const fills = sent().filter((o) => o.type === "setElement" && o.patch?.fill?.kind?.type === "liveSource");
      if (!fills.length) throw new Error("no live source fill was sent");
      if (fills.at(-1).patch.fill.kind.source !== "BOOTH-PC (Cam 2 Pulpit)") throw new Error("the chosen source was not sent");
      await page.locator("[data-live-fill-note]").scrollIntoViewIfNeeded();
      await shot("edit-live-source-fill");
    });
    await step("edit live source fill: a video input", async () => {
      await page.locator('[data-field="live-source"]').selectOption("input:1a9e0000-0000-4000-8000-0000000000c2");
      await settle(900);
      const fills = sent().filter((o) => o.type === "setElement" && o.patch?.fill?.kind?.type === "liveSource");
      const k = fills.at(-1)?.patch.fill.kind;
      if (k?.input !== "1a9e0000-0000-4000-8000-0000000000c2" || k.source !== "BOOTH-PC (Cam 2 Pulpit)") throw new Error(`the input was not sent: ${JSON.stringify(k)}`);
      await page.locator('[data-field="live-source"]').scrollIntoViewIfNeeded();
      await shot("edit-live-input-fill");
    });

    await step("edit check: Escape cancels an object rename, Enter renames once", async () => {
      await row("Sun").dblclick();
      const input = page.getByRole("textbox", { name: "Object name" });
      await input.fill("Cancelled name");
      await input.press("Escape");
      await settle(400);
      if (sent().some((o) => o.patch?.name === "Cancelled name")) throw new Error("Escape renamed the object");
      await row("Sun").dblclick();
      await input.fill("Sun again");
      await input.press("Enter");
      await settle(600);
      const n = sent().filter((o) => o.patch?.name === "Sun again").length;
      if (n !== 1) throw new Error(`Enter sent ${n} renames`);
    });

    await step("edit check: text typed under the canvas is saved to its object on a click away", async () => {
      await page.keyboard.press("Escape");
      await row("Subtitle").click();
      const id = await row("Subtitle").getAttribute("data-object-row");
      await settle(300);
      const field = page.locator("[data-text-field]");
      await field.click();
      await field.press("End");
      await field.type(" typed here");
      // Click away: an empty part of the canvas.
      const c = await page.locator("[data-edit-view] .viewport").first().boundingBox();
      await page.mouse.click(c.x + 8, c.y + c.height - 8);
      await settle(800);
      const saves = sent().filter((o) => o.type === "replaceTextBox" && JSON.stringify(o.textBox).includes("typed here"));
      if (!saves.length) throw new Error("the typed text was not saved");
      if (saves.some((o) => o.element !== id)) throw new Error("the typed text was saved into another object");
      // Again, clicking another object (the middle of the slide) instead.
      await row("Subtitle").click();
      await settle(300);
      await field.click();
      await field.press("End");
      await field.type(" and more");
      await page.mouse.click(c.x + c.width / 2, c.y + c.height / 2);
      await settle(800);
      const more = sent().filter((o) => o.type === "replaceTextBox" && JSON.stringify(o.textBox).includes("and more"));
      if (!more.length) throw new Error("the typed text was not saved (another object clicked)");
      if (more.some((o) => o.element !== id)) throw new Error("the typed text was saved into the object clicked");
    });

    await step("edit check: a number typed in the inspector goes to its object on a click away", async () => {
      await row("Sun again").click();
      const id = await row("Sun again").getAttribute("data-object-row");
      await tab("Shape").click();
      await settle(300);
      const x = page.getByRole("spinbutton", { name: "X", exact: true });
      await x.fill("123");
      // Another object on the canvas (the middle of the slide) without pressing Enter.
      const c = await page.locator("[data-edit-view] .viewport").first().boundingBox();
      await page.mouse.click(c.x + c.width / 2, c.y + c.height / 2);
      await settle(800);
      const moves = sent().filter((o) => o.type === "setElement" && JSON.stringify(o.patch).includes("123"));
      if (!moves.length) throw new Error("the number was not saved");
      if (moves.some((o) => o.element !== id)) throw new Error("the number was saved to the object clicked");
    });

    await step("edit check: deleting the edited slide selects its neighbour", async () => {
      const rows = page.locator("[data-slide-row]");
      const ids = await rows.evaluateAll((els) => els.map((e) => e.getAttribute("data-slide-row")));
      if (ids.length < 3) throw new Error("not enough slides");
      await rows.nth(1).click();
      await settle(300);
      await rows.nth(1).click({ button: "right" });
      await page.getByRole("menuitem", { name: "Delete" }).click();
      await settle(800);
      const cur = await page.locator("[data-slide-row][aria-current=true]").getAttribute("data-slide-row");
      if (cur !== ids[2]) throw new Error(`slide ${ids.indexOf(cur) + 1} selected, not the neighbour`);
      if (await page.getByText("The slide you were editing was deleted.").isVisible()) throw new Error("a false 'deleted' note");
    });

    await step("edit back to Show", async () => {
      await page.keyboard.press("Escape");
      await page.getByRole("button", { name: "Done" }).click();
      await settle(400);
    });

    await step("show build progress on the live slide", async () => {
      editStub.on = false;
      changed();
      await settle(400);
      // A slide live (earlier shots may have cleared the slide layer), then its builds.
      await page.locator("button[data-slide]").first().click();
      await settle(500);
      stub.setBuild({ step: 1, steps: 3 });
      await settle(600);
      const chip = page.locator("[data-build-progress]").first();
      if ((await chip.textContent())?.trim() !== "Build 1/3") throw new Error("no build progress on the live tile");
      await chip.scrollIntoViewIfNeeded();
      await shot("show-build-progress");
      stub.setBuild(null);
      await settle(300);
    });
  } finally {
    editStub.on = false;
    changed();
    await settle(400);
  }
}
