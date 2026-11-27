// Shots of the Show page (see scripts/shots.mjs for `ctx`). Left column and centre (round 5): the
// long synthetic playlist (test/stub/routes/show.mjs) at the top and in the middle, a slide live
// with the next one marked, a song in its arrangement (the second chorus live, the groups it
// leaves out opened), the Text and List views, the slide menu (Shift+F10) and a library
// item shown on its own with "Add to <playlist>". Right column and bin (round 6): the page during
// playback (a video with its transport, a track on the Audio layer, the meter moving, a message
// with its token, a prop), each bin tab, each compact tab, the Audio transport, the bin hidden;
// after Clear All, the layer buttons that bring a cleared layer back (G4). The Video Inputs tab
// with a camera on the Video Input layer and its strip under the layer buttons (G7), its sound muted;
// a logical input live, an input's menu and a new input (stage 4). A sound card lost and being
// reconnected in the status bar (batch 15).
export default async function (ctx) {
  await leftAndCentre(ctx);
  await rightAndBin(ctx);
}

async function rightAndBin({ page, w, h, settle, shot, step, stub }) {
  const post = (what) => fetch(`${stub.url}/api/v1/stub/${what}`, { method: "POST" });
  const bin = page.getByTestId("bottom-bin");
  const side = page.getByRole("tablist", { name: "Controls" });
  const away = async () => {
    await page.mouse.move(w / 2, 8);
    await settle(400);
  };
  await step("playback on", async () => {
    await post("show-playback");
    await settle(900);
  });
  await step("bin Media, Timers tab", async () => {
    await bin.getByRole("tab", { name: "Media", exact: true }).click();
    await page.getByRole("list", { name: "Media bins" }).getByText("Backgrounds").click();
    await side.getByRole("tab", { name: "Timers" }).click();
    await away();
    await shot("show-playback");
  });
  await step("Messages tab", async () => {
    await side.getByRole("tab", { name: "Messages" }).click();
    await away();
    await shot("show-tab-messages");
  });
  await step("Stage tab", async () => {
    await side.getByRole("tab", { name: "Stage" }).click();
    await away();
    await shot("show-tab-stage");
  });
  await step("sound card lost, reconnecting", async () => {
    await post("audio-lost");
    await settle(600);
    await away();
    await shot("show-audio-lost");
    await post("audio-back");
    await settle(400);
  });
  await step("bin Audio, Audio transport", async () => {
    await side.getByRole("tab", { name: "Timers" }).click();
    await bin.getByRole("tab", { name: "Audio", exact: true }).click();
    await page.getByRole("group", { name: "Transport target" }).getByRole("button", { name: "Audio" }).click();
    await away();
    await shot("show-bin-audio");
    await page.getByRole("group", { name: "Transport target" }).getByRole("button", { name: "Video" }).click();
  });
  for (const tab of ["Props", "Macros"]) {
    await step(`bin ${tab}`, async () => {
      await bin.getByRole("tab", { name: tab, exact: true }).click();
      await away();
      await shot(`show-bin-${tab.toLowerCase()}`);
    });
  }
  await step("bin Video Inputs, a camera live", async () => {
    await bin.getByRole("tab", { name: "Video Inputs", exact: true }).click();
    await settle(500);
    await page.getByRole("button", { name: "Show Cam 2 Pulpit" }).click();
    await away();
    await settle(600);
    await shot("show-bin-inputs");
  });
  await step("bin Video Inputs: an input live, its menu, a new input", async () => {
    // Stage 4: logical inputs above the sources (test/stub/routes/sources.mjs).
    await page.getByRole("button", { name: "Show Wide shot" }).click();
    await settle(600);
    const live = page.getByTestId("input-card").filter({ hasText: "Wide shot" });
    if (!(await live.getByText("LIVE").isVisible())) throw new Error("the input is not marked live");
    await page.getByRole("button", { name: "Pulpit camera options" }).click();
    await settle(300);
    await shot("show-bin-input-menu");
    await page.keyboard.press("Escape");
    const sides = page.getByTestId("input-card").filter({ hasText: "Side camera" });
    const before = await sides.count();
    await page.getByTestId("add-input").click();
    await page.getByRole("dialog").locator("input").fill("Side camera");
    await page.getByRole("dialog").getByRole("button", { name: "Add" }).click();
    await settle(600);
    if ((await sides.count()) !== before + 1) throw new Error("the new input is not listed");
    await away();
    await shot("show-bin-inputs-live");
    const all = await (await fetch(`${stub.url}/api/v1/video-inputs`)).json();
    for (const v of all.filter((x) => x.name === "Side camera")) await fetch(`${stub.url}/api/v1/video-inputs/${v.id}`, { method: "DELETE" });
  });
  await step("bin Video Inputs: an OMT source live, an input bound to it", async () => {
    // OMT sources (docs/OMT-PLAN.md) are listed with the IDN ones, tagged "OMT".
    const card = page.getByTestId("source-card").filter({ hasText: "Lower thirds" });
    await card.scrollIntoViewIfNeeded();
    if ((await card.getByTestId("source-transport").textContent()) !== "OMT") throw new Error("the OMT source is not tagged");
    await page.getByRole("button", { name: "Show Lower thirds" }).click();
    await settle(600);
    const live = (await (await fetch(`${stub.url}/api/v1/state`)).json()).state.layers.videoInput;
    if (live?.source !== "omt:GFX-PC (Lower thirds)") throw new Error(`the OMT source is not live: ${JSON.stringify(live)}`);
    await page.getByRole("button", { name: "Baptistry options" }).click();
    await settle(300);
    await page.getByRole("menuitem", { name: "Lower thirds · OMT" }).click();
    await settle(600);
    const all = await (await fetch(`${stub.url}/api/v1/video-inputs`)).json();
    const b = all.find((x) => x.name === "Baptistry");
    if (b?.source?.type !== "omt" || b.source.name !== "GFX-PC (Lower thirds)") throw new Error(`not bound to the OMT source: ${JSON.stringify(b?.source)}`);
    if (!(await page.getByTestId("input-card").filter({ hasText: "Baptistry" }).textContent()).includes("OMT · Lower thirds")) throw new Error("the input line does not name the OMT source");
    await away();
    await shot("show-bin-inputs-omt");
    await fetch(`${stub.url}/api/v1/video-inputs/${b.id}`, { method: "PUT", headers: { "content-type": "application/json", "if-match": b.revision }, body: JSON.stringify({ ...b, revision: undefined, source: null }) });
  });
  await step("live source sound muted", async () => {
    await page.getByTestId("video-input-mute").click();
    await away();
    await settle(400);
    await shot("show-input-muted");
    await page.getByTestId("video-input-mute").click();
    await settle(300);
  });
  await step("bin hidden, outputs off", async () => {
    await bin.getByRole("button", { name: "Hide Media Bin" }).click();
    await page.getByTestId("output-stage").click();
    await away();
    await shot("show-bin-hidden");
    await page.getByTestId("output-stage").click();
    await page.getByRole("button", { name: "Show Media Bin" }).click();
    await bin.getByRole("tab", { name: "Media", exact: true }).click();
  });
  await step("cleared layers can come back", async () => {
    await page.getByTestId("clear-all").click();
    await away();
    await shot("show-restore");
    await page.getByTestId("layer-media").click();
    await away();
    await shot("show-restore-media");
  });
  await step("playback off", async () => {
    await post("show-reset");
    await settle(500);
  });
}

async function leftAndCentre({ page, w, h, settle, shot, step }) {
  const scroller = page.locator("section.slides .scroll");
  const playlists = page.getByRole("list", { name: "Playlists", exact: true });
  await step("open the long playlist", async () => {
    if (!(await playlists.getByText("Harvest Gathering").isVisible().catch(() => false))) await playlists.getByText("Seasonal").click();
    await playlists.getByText("Harvest Gathering").click();
    await settle(1200);
  });
  await page.mouse.move(w - 5, h - 5);
  await settle(800);
  await shot("show-playlist-top");

  await step("a slide live, the next one marked", async () => {
    const tile = page.getByRole("button", { name: /^Lanterns on the Water, slide 2,/ });
    await tile.scrollIntoViewIfNeeded();
    await tile.click();
    await settle(900);
  });
  await page.mouse.move(w - 5, h - 5);
  await settle(400);
  await shot("show-live-next");

  await step("the second chorus of the arrangement live", async () => {
    // "Full" = V1 C V2 C: slide 10 is the first slide of the second chorus; only it is LIVE.
    const tile = page.getByRole("button", { name: /^Lanterns on the Water, slide 10,/ });
    await tile.scrollIntoViewIfNeeded();
    await tile.click();
    await settle(900);
    await page.mouse.move(w - 5, h - 5);
    await settle(400);
    await shot("show-arrangement-repeat");
  });

  await step("the groups the arrangement leaves out", async () => {
    const sec = page.locator("section.sec", { has: page.getByRole("heading", { name: "Lanterns on the Water", exact: true }) });
    const head = sec.getByRole("button", { name: /^Not in arrangement/ });
    await head.click();
    await settle(700);
    await head.scrollIntoViewIfNeeded();
    await page.mouse.move(w - 5, h - 5);
    await settle(400);
    await shot("show-arrangement-left");
    await head.click();
  });

  await step("the slide menu (Shift+F10)", async () => {
    const tile = page.getByRole("button", { name: /^Lanterns on the Water, slide 4,/ });
    await tile.focus();
    await page.keyboard.press("Shift+F10");
    await settle(300);
    await shot("show-slide-menu");
    await page.keyboard.press("Escape");
  });

  await step("scrolled to the middle", async () => {
    await scroller.evaluate((el) => el.scrollTo({ top: el.scrollHeight / 2 - el.clientHeight / 2 }));
    await settle(1200);
  });
  await page.mouse.move(w - 5, h - 5);
  await shot("show-playlist-middle");

  for (const view of ["Text", "List"]) {
    await step(`${view} view`, async () => {
      await page.getByRole("radio", { name: `${view} view` }).click();
      await settle(500);
      await scroller.evaluate((el) => el.scrollTo({ top: 0 }));
      await settle(900);
      await page.mouse.move(w - 5, h - 5);
      await shot(`show-view-${view.toLowerCase()}`);
    });
  }
  await step("back to Grid", async () => {
    await page.getByRole("radio", { name: "Grid view" }).click();
    await settle(300);
  });

  await step("a library item on its own", async () => {
    const libs = page.getByRole("list", { name: "Libraries" }).getByRole("button");
    await libs.first().click();
    await settle(600);
    await page.getByRole("list", { name: "Presentations" }).getByRole("button").first().click();
    await settle(1200);
    await page.mouse.move(w - 5, h - 5);
    await shot("show-library-item");
  });

  // Back to the fixtures' live slide in "Sunday Morning", so the next screen size starts the same.
  await step("restore the live slide", async () => {
    await playlists.getByText("Sunday Morning").click();
    await settle(900);
    await page.getByRole("button", { name: /^Morning Lantern, slide 2,/ }).click();
    await settle(500);
  });
}
