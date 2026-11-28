// Shots of the Settings page (see scripts/shots.mjs for `ctx`): General with Oriel's settings
// backup (test/stub/routes/site.mjs keeps what Oriel saved), Captions (and a model downloading,
// routes/captions.mjs's stub control), Network & IDN (with Control
// surfaces: listening, and a port that can't be opened, test/stub/routes/settings.mjs), CCLI (no
// licence number yet, the imported songs' one offered; the watcher's switch is the desktop app's,
// see songselectWatch.mjs), Keyboard and About.
// Settings is a page (`#/settings`): Esc stays on it, Close goes back.
const captions = (url, body) => fetch(`${url}/api/v1/__stub/captions`, { method: "POST", body: JSON.stringify(body) });

export default async function ({ page, stub, settle, shot, step }) {
  await page.keyboard.press("Control+Comma");
  await settle(600);
  try {
    await shot("settings-general");
    const nav = page.getByRole("navigation", { name: "Settings sections" });
    await step("settings control surfaces", async () => {
      // Network & IDN ▸ Control surfaces: listening (the stub's start), then a port that can't
      // be opened (the stub refuses ports below 1024), then back.
      await nav.getByRole("button", { name: "Network & IDN", exact: true }).click();
      await settle(300);
      const card = page.getByTestId("control-surfaces");
      await card.scrollIntoViewIfNeeded();
      await shot("settings-control-surfaces");
      const port = card.getByLabel("Port");
      await port.fill("80");
      await card.getByRole("button", { name: "Use this port" }).click();
      await settle(300);
      await shot("settings-control-surfaces-problem");
      await port.fill("50001");
      await card.getByRole("button", { name: "Use this port" }).click();
      await settle(200);
    });
    for (const [name, file] of [
      ["Captions", "settings-captions"],
      ["Network & IDN", "settings-network"],
      ["CCLI", "settings-ccli"],
      ["Keyboard", "settings-keyboard"],
      ["About", "settings-about"],
    ]) {
      await step(`settings ${name}`, async () => {
        await nav.getByRole("button", { name, exact: true }).click();
        await settle(300);
        await shot(file);
        if (file === "settings-captions") {
          // The engine fetching a model: the status says which, the bar how far.
          try {
            await captions(stub.url, { downloading: { model: "small.en", downloaded: 233_000_000, size: 466_000_000 } });
            await settle(400);
            await shot("settings-captions-download");
            // The GPU failed: captions go on on the CPU, the note says why.
            await captions(stub.url, { gpuFailed: true });
            await settle(400);
            await shot("settings-captions-gpu-fallback");
            // The GPU chosen and in use: the warning about frames under the choice.
            await captions(stub.url, { onGpu: true });
            await page.getByTestId("captions-processor").selectOption("gpu");
            await settle(400);
            if (!(await page.getByTestId("captions-gpu-hint").isVisible())) throw new Error("no GPU warning");
            await shot("settings-captions-gpu");
            await page.getByTestId("captions-processor").selectOption("cpu");
          } finally {
            await captions(stub.url, { reset: true });
            await settle(200);
          }
        }
      });
    }
  } finally {
    await page.getByRole("toolbar", { name: "Settings" }).getByRole("button", { name: "Close" }).click();
    await settle(200);
  }
}
