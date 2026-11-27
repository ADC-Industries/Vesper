// Shots of the Screens & Looks page (see scripts/shots.mjs for `ctx`): a test pattern on the
// audience screen (TEST), its name edited but not applied (EDITED, Apply / Revert), the test
// pattern panel, an output's Alpha Key, Color and Corner Pin tabs, the stage screen sending to an
// attached display, a new display output on the audience screen, then the Looks tab with the live
// look, a look's matrix edited (theme picker, unsaved edits, the prompt before another look
// is picked), the mask picker and the mask
// editor (a keyhole, an overlay with a new polygon), a look with its transition and mask, a
// new look and its delete confirmation; a stage screen's mask (the picker, a mask applied, a
// deleted one refused on Apply); a Fill/Key pair on two displays (the fill's card, "Add the key
// output", the key's card); then the same box as an operator's PC (window displays): a new output in a rehearsal
// window on the projector, then on the main monitor with the confirmation before covering it;
// screen layouts (an Edge Blend wall with a placeholder, Grouped with bezels, a placeholder
// added from an empty tile, identifying a free display) (test/stub/routes/screens.mjs).
import { bibleStub } from "../../test/stub/routes/bible.mjs";
import { screensStub } from "../../test/stub/routes/screens.mjs";

export default async function ({ page, openPage, settle, shot, step, stub }) {
  screensStub.start();
  stub.push({ type: "documentChanged", kind: "look", id: "5eed0000-0000-4000-8000-0000000c0001", change: "changed" });
  try {
    await settle(400);
    await openPage("Screens");
    await shot("page-screens");
    await step("screens edited", async () => {
      const name = page.getByTestId("screen-name");
      await name.click();
      await name.press("End");
      await name.type(" (Left)");
      await page.getByTestId("tp-button").click();
      await settle(300);
      await page.mouse.move(5, 300);
      await shot("screens-edited");
    });
    await step("output tab", async () => {
      await page.getByTestId("output-tab-alpha").first().click();
      await settle(200);
      await shot("screens-output-alpha");
      await page.getByTestId("output-tab-hw").first().click();
    });
    await step("colour and corner pin tabs", async () => {
      await page.getByTestId("output-tab-color").first().click();
      await settle(150);
      for (const [k, v] of [["gamma", "1.2"], ["saturation", "0.85"], ["blue", "1.1"]]) {
        await page.getByTestId(`color-${k}-value`).first().fill(v);
        await page.getByTestId(`color-${k}-value`).first().press("Tab");
      }
      await settle(400);
      await page.mouse.move(5, 300);
      await shot("screens-output-color");
      await page.getByTestId("output-tab-pin").first().click();
      await settle(150);
      await page.getByTestId("pin-enabled").first().check();
      for (const [k, v] of [["topLeft-x", "-120"], ["topLeft-y", "-40"], ["topRight-x", "-80"], ["bottomLeft-y", "30"]]) {
        await page.getByTestId(`pin-${k}`).first().fill(v);
        await page.getByTestId(`pin-${k}`).first().press("Tab");
      }
      // Drag the bottom-right corner up and in.
      const h = await page.getByTestId("pin-handle-bottomRight").first().boundingBox();
      if (h) {
        await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
        await page.mouse.down();
        await page.mouse.move(h.x + h.width / 2 - 25, h.y + h.height / 2 - 15, { steps: 5 });
        await page.mouse.up();
      }
      await settle(400);
      await page.mouse.move(5, 300);
      await shot("screens-output-pin");
      await page.getByTestId("pin-reset").first().click();
      await page.getByTestId("pin-enabled").first().uncheck();
      await page.getByTestId("output-tab-color").first().click();
      await page.getByTestId("color-reset").first().click();
      await settle(300);
      await page.getByTestId("output-tab-hw").first().click();
    });
    await step("unsaved prompt closes on Esc", async () => {
      // Another screen with the name edited: the in-page prompt; Esc with the focus outside it
      // (the list) closes it and keeps the edits.
      await page.getByTestId("screen-item").filter({ hasText: "Stage" }).first().click();
      await page.getByTestId("unsaved-prompt").waitFor();
      await page.evaluate(() => document.activeElement?.blur?.());
      await page.keyboard.press("Escape");
      await settle(150);
      if (await page.getByTestId("unsaved-prompt").isVisible()) throw new Error("Esc left the unsaved-changes prompt open");
    });
    await step("revert edits", async () => {
      await page.getByTestId("revert-btn").click();
      await settle(200);
    });
    await step("display output", async () => {
      await page.getByTestId("screen-item").filter({ hasText: "Stage" }).first().click();
      await settle(300);
      await shot("screens-display");
    });
    await step("new display output", async () => {
      await page.getByTestId("screen-item").first().click();
      await settle(200);
      await page.getByTestId("add-output").click();
      await settle(100);
      await page.getByTestId("transport-select").last().selectOption("display");
      await settle(200);
      await page.getByTestId("output-card").last().scrollIntoViewIfNeeded();
      await shot("screens-display-new");
    });
    await step("new OMT output", async () => {
      // OMT is built in (docs/OMT-PLAN.md): selectable, with embedded audio like IDN.
      const sel = page.getByTestId("transport-select").last();
      const label = await sel.locator('option[value="omt"]').textContent();
      if (/unavailable|not available/.test(label ?? "")) throw new Error(`OMT is not offered: ${label}`);
      await sel.selectOption("omt");
      await settle(200);
      await page.getByTestId("output-embed-audio").last().check();
      await page.getByTestId("output-card").last().scrollIntoViewIfNeeded();
      await shot("screens-omt-new");
    });
    await step("looks", async () => {
      await page.getByRole("radio", { name: "Looks" }).click();
      await settle(400);
      await shot("page-looks");
    });
    await step("looks: matrix edits", async () => {
      // Another look: props off and a theme override on the first audience screen (the theme
      // list: the Bible stub's two made-up themes).
      bibleStub.on = true;
      await page.getByTestId("look-item").filter({ hasText: "Full Service" }).click();
      await settle(300);
      await page.getByTestId("look-cell").nth(1).uncheck();
      await page.getByTestId("look-theme").first().click();
      await page.getByTestId("look-theme-menu").waitFor();
      await settle(400);
      await shot("looks-theme-picker");
      await page.locator("[data-testid=look-theme-menu] .tslide").first().click();
      await settle(200);
      await page.mouse.move(5, 300);
      await shot("looks-edited");
      bibleStub.on = false;
      // Another look picked with unsaved edits: asked first (D08-3); keep editing.
      await page.getByTestId("look-item").filter({ hasNotText: "Full Service" }).first().click();
      await page.getByTestId("look-discard").waitFor();
      await settle(200);
      await shot("looks-unsaved-prompt");
      await page.getByRole("button", { name: "Keep editing" }).click();
      await settle(200);
    });
    await step("looks: masks", async () => {
      await page.getByTestId("look-revert").click();
      await page.getByTestId("look-mask").first().click();
      await settle(200);
      await shot("looks-mask-picker");
      await page.getByRole("menuitem", { name: "Edit masks…" }).click();
      await page.getByTestId("mask-editor").waitFor();
      await settle(300);
      await shot("mask-editor-keyhole");
      await page.getByTestId("mask-item").filter({ hasText: "Corner bug" }).click();
      await page.getByTestId("mask-add-polygon").click();
      await settle(300);
      await shot("mask-editor-overlay");
      // A new mask, edited, deleted by another operator before Save: the editor moves to
      // another mask and says why (E08-3).
      await page.getByTestId("mask-new").click();
      await settle(300);
      await page.getByTestId("mask-name").fill("Side panel");
      await page.getByTestId("mask-add-rectangle").click();
      // (Stored as "Mask 1": the new name isn't saved yet.)
      screensStub.deleteMaskElsewhere("Mask 1");
      await page.getByTestId("mask-save").click();
      await page.getByText("“Side panel” was deleted by another operator").waitFor({ timeout: 3000 });
      await settle(300);
      await shot("mask-editor-deleted-elsewhere");
      await page.getByRole("button", { name: "Close" }).last().click();
      await settle(200);
    });
    await step("looks: second language", async () => {
      // Dual-language slides (batch 16): one screen side by side, another first language only.
      const sel = page.getByTestId("look-language");
      await sel.first().selectOption("sideBySide");
      if ((await sel.count()) > 1) await sel.nth(1).selectOption("primary");
      await settle(300);
      await shot("looks-second-language");
      await page.getByTestId("look-revert").click();
      await settle(200);
    });
    await step("looks: transition", async () => {
      await page.getByTestId("look-item").filter({ hasText: "Announcements" }).click();
      await settle(300);
      await shot("looks-transition");
    });
    await step("looks: new and delete", async () => {
      await page.getByTestId("look-new").click();
      await settle(400);
      await shot("looks-new");
      await page.getByTestId("look-delete").click();
      await settle(200);
      await shot("looks-delete");
      await page.getByTestId("look-delete-ok").click();
      await settle(300);
      await page.getByRole("radio", { name: "Screens" }).click();
    });
    await step("revert", async () => {
      await page.getByTestId("revert-btn").click();
      await settle(200);
    });
    // A stage screen's own mask: the picker, a mask chosen and applied, then one deleted in the
    // mask editor before Apply (the server refuses it; the error shows under the row).
    await step("stage mask: picker", async () => {
      await page.getByTestId("screen-item").filter({ hasText: "Stage" }).first().click();
      await settle(300);
      await page.getByTestId("screen-mask").click();
      await page.getByRole("menuitem", { name: "Edit masks…" }).waitFor();
      await settle(200);
      await shot("screens-stage-mask-picker");
    });
    await step("stage mask: chosen and applied", async () => {
      await page.getByRole("menuitemcheckbox", { name: "Center 4:3" }).click();
      await settle(200);
      await page.getByTestId("apply-btn").click();
      await settle(500);
      if ((await page.getByTestId("screen-mask").innerText()).trim() !== "Center 4:3") throw new Error("the stage mask was not applied");
      await page.mouse.move(5, 300);
      await shot("screens-stage-mask");
    });
    await step("stage mask: refused", async () => {
      await page.getByTestId("screen-mask").click();
      await page.getByRole("menuitemcheckbox", { name: "Corner bug" }).click();
      await page.getByTestId("screen-mask").click();
      await page.getByRole("menuitem", { name: "Edit masks…" }).click();
      await page.getByTestId("mask-editor").waitFor();
      await settle(300);
      await page.getByTestId("mask-item").filter({ hasText: "Corner bug" }).click();
      await page.getByTestId("mask-delete").click();
      await page.getByTestId("mask-delete-ok").click();
      await settle(300);
      await page.getByRole("button", { name: "Close" }).last().click();
      await settle(200);
      await page.getByTestId("apply-btn").click();
      await page.getByTestId("screen-mask-error").waitFor();
      await settle(300);
      await page.mouse.move(5, 300);
      await shot("screens-stage-mask-refused");
      await page.getByTestId("revert-btn").click();
      await settle(200);
    });
    await step("stage mask: deleted by another operator", async () => {
      // The applied mask deleted elsewhere (detached from the screen): the mask list reloads on
      // its event (the name turns to "Missing mask"), the screen on its own.
      const maskId = "5eed0000-0000-4000-8000-0000000c0021";
      const r = await fetch(`${stub.url}/api/v1/masks/${maskId}?detach=true`, { method: "DELETE" });
      if (!r.ok || !(r.headers.get("content-type") ?? "").includes("json")) throw new Error(`deleting the mask: HTTP ${r.status}`);
      stub.push({ type: "documentChanged", kind: "mask", id: maskId, change: "deleted" });
      await page.getByTestId("screen-mask").filter({ hasText: "Missing mask" }).waitFor({ timeout: 3000 });
      stub.push({ type: "documentChanged", kind: "screen", id: "6703b24e-2f82-47a2-9986-b8043c7fb592", change: "changed" });
      await page.getByTestId("screen-mask").filter({ hasText: "None" }).waitFor({ timeout: 3000 });
    });
    // A Fill/Key pair for a hardware keyer: the audience screen with alpha, a display output
    // targeting Fill on the keyer's fill input, then "Add the key output" (on the next free
    // display, then moved to the keyer's key input), each card with its own display picker
    // and the keyer hint.
    await step("fill and key", async () => {
      screensStub.stop();
      screensStub.keyer = true;
      screensStub.start();
      await settle(300);
      await page.getByRole("radio", { name: "Looks" }).click();
      await settle(300);
      await page.getByRole("radio", { name: "Screens" }).click();
      await settle(400);
      await page.getByTestId("screen-item").first().click();
      await settle(200);
      await page.getByTestId("screen-alpha").selectOption("straight");
      await page.getByTestId("add-output").click();
      await settle(100);
      await page.getByTestId("transport-select").last().selectOption("display");
      await settle(200);
      await page.getByTestId("display-select").last().selectOption("DP-2");
      await page.getByTestId("output-target").last().selectOption("fill");
      await settle(150);
      await page.getByTestId("add-key-output").scrollIntoViewIfNeeded();
      await page.mouse.move(5, 300);
      await shot("screens-fill-key-add");
      await page.getByTestId("add-key-output").click();
      await settle(300);
      const cards = page.getByTestId("output-card");
      const n = await cards.count();
      if ((await cards.nth(n - 1).getByTestId("output-target").inputValue()) !== "key") throw new Error("the key output was not added");
      // It lands on the next free display (DP-1); the operator picks the keyer's key input.
      if ((await cards.nth(n - 1).getByTestId("display-select").inputValue()) !== "DP-1") throw new Error("the key output is not on the next free display");
      await cards.nth(n - 1).getByTestId("display-select").selectOption("DP-3");
      await settle(150);
      await cards.nth(n - 1).scrollIntoViewIfNeeded();
      await page.mouse.move(5, 300);
      await shot("screens-fill-key");
      await page.getByTestId("revert-btn").click();
      await settle(200);
      screensStub.stop();
      screensStub.start();
      await settle(300);
    });
    // Screen layouts: the audience screen as a three-projector Edge Blend wall (an IDN output on
    // the left tile, a placeholder in the middle, the right tile empty), the same as Grouped,
    // the placeholder's card, and identifying a free display.
    await step("layouts: edge blend wall", async () => {
      screensStub.stop();
      screensStub.wall = true;
      screensStub.start();
      await settle(300);
      // The screen and its outputs changed on the "server": Oriel loads them again.
      stub.push({ type: "documentChanged", kind: "screen", id: "3b86def4-cbe1-439e-87d7-c837c454913c", change: "changed" });
      stub.push({ type: "documentChanged", kind: "outputBinding", id: "5eed0000-0000-4000-8000-0000000d0002", change: "created" });
      await settle(400);
      await page.getByTestId("screen-item").first().click();
      await settle(300);
      await page.getByTestId("arrangement-canvas").waitFor();
      if ((await page.getByTestId("arrangement-tile").count()) !== 3) throw new Error("the wall has not three tiles");
      await page.getByTestId("screen-name").scrollIntoViewIfNeeded();
      await page.mouse.move(5, 300);
      await shot("screens-layout-blend");
    });
    await step("layouts: grouped", async () => {
      await page.getByTestId("arrangement-grouped").click();
      await page.getByTestId("arrangement-bezel-x").fill("64");
      await page.getByTestId("arrangement-bezel-x").press("Tab");
      await page.getByTestId("arrangement-rows").fill("2");
      await page.getByTestId("arrangement-rows").press("Tab");
      await settle(200);
      if ((await page.getByTestId("arrangement-tile").count()) !== 6) throw new Error("the grid has not six tiles");
      await page.getByTestId("screen-name").scrollIntoViewIfNeeded();
      await page.mouse.move(5, 300);
      await shot("screens-layout-grouped");
      // An empty tile: a click adds a placeholder for it.
      await page.getByTestId("arrangement-tile").last().click();
      await settle(200);
      await page.getByTestId("output-card").last().scrollIntoViewIfNeeded();
      await page.mouse.move(5, 300);
      await shot("screens-layout-placeholder");
      await page.getByTestId("revert-btn").click();
      await settle(200);
    });
    await step("layouts: identify a free display", async () => {
      await page.getByTestId("add-output").click();
      await settle(100);
      await page.getByTestId("transport-select").last().selectOption("display");
      await settle(200);
      await page.getByTestId("display-select").last().selectOption("DP-1");
      await page.getByTestId("output-tile").last().selectOption("2");
      await settle(150);
      await page.getByTestId("display-identify").click();
      await settle(200);
      await page.getByTestId("output-card").last().scrollIntoViewIfNeeded();
      await page.mouse.move(5, 300);
      await shot("screens-layout-identify");
      await page.getByTestId("revert-btn").click();
      await settle(200);
      screensStub.stop();
      screensStub.start();
      await settle(300);
    });
    // The operator's own PC: monitors shown as windows; a new output in a window on the
    // projector, then full screen on the main monitor (Oriel's), which asks first.
    await step("window displays: reload", async () => {
      screensStub.stop();
      screensStub.source = "window";
      screensStub.start();
      await settle(300);
      // Back and forth between the tabs: the Screens tab loads the outputs again.
      await page.getByRole("radio", { name: "Looks" }).click();
      await settle(300);
      await page.getByRole("radio", { name: "Screens" }).click();
      await settle(400);
    });
    await step("window displays: a rehearsal window", async () => {
      await page.getByTestId("screen-item").first().click();
      await settle(200);
      await page.getByTestId("add-output").click();
      await settle(100);
      await page.getByTestId("transport-select").last().selectOption("display");
      await settle(200);
      await page.getByTestId("display-select").last().selectOption("\\\\.\\DISPLAY2");
      await page.getByTestId("display-fullscreen").last().selectOption("window");
      await settle(200);
      await page.getByTestId("output-card").last().scrollIntoViewIfNeeded();
      await shot("screens-window-display");
    });
    await step("window displays: the main monitor", async () => {
      await page.getByTestId("display-select").last().selectOption("\\\\.\\DISPLAY1");
      await page.getByTestId("display-fullscreen").last().selectOption("full");
      await settle(200);
      await page.getByTestId("output-card").last().scrollIntoViewIfNeeded();
      await shot("screens-window-risk");
      await page.getByTestId("apply-btn").click();
      await settle(300);
      await shot("screens-window-confirm");
      await page.getByTestId("prompt-cancel").click();
      await settle(100);
    });
    await step("window displays: revert", async () => {
      if (await page.getByTestId("prompt-cancel").isVisible()) await page.getByTestId("prompt-cancel").click();
      await page.getByTestId("revert-btn").click();
      await settle(200);
    });
  } finally {
    bibleStub.on = false;
    screensStub.stop();
    await settle(300);
  }
}
