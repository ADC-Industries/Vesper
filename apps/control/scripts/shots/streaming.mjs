// Shots of recording and streaming (docs/RECORD-STREAM-PLAN.md; see scripts/shots.mjs for
// `ctx`): the Show page's indicator while the stub records and streams (test/stub/routes/
// streaming.mjs: 42 minutes in, the RTMPS target live) and its popover, then Settings ▸
// Recording & streaming with the status and a target's editor (the key field never filled; a new
// server asks for the key again), and the SRT target's editor (its hidden URL kept, or typed again in full).
// The stub goes back to idle at the end.
import { streamingStub } from "../../test/stub/routes/streaming.mjs";

export default async function ({ page, settle, shot, step, esc }) {
  streamingStub.reset();
  streamingStub.live();
  try {
    await step("indicator", async () => {
      // The indicator polls every 2 s.
      await settle(2600);
      await shot("show-rec-indicator");
      await page.getByTestId("rec-indicator").click();
      await settle(500);
      await shot("show-rec-popover");
      await esc();
      await settle(200);
    });
    await step("settings", async () => {
      await page.keyboard.press("Control+Comma");
      await settle(600);
      await page.getByRole("navigation", { name: "Settings sections" }).getByRole("button", { name: "Recording & streaming", exact: true }).click();
      await settle(800);
      await shot("settings-streaming");
      await page.getByTestId("stream-target").first().getByRole("button", { name: "Edit" }).click();
      await settle(300);
      await page.getByTestId("target-editor").scrollIntoViewIfNeeded();
      await shot("settings-streaming-edit");
      // Another server for a target with a saved key: the key is asked for again before Save
      // (H05-1: the server keeps a key only for the same server).
      await page.locator("#t-url").fill("rtmps://other.example.com/app");
      await settle(200);
      await page.getByTestId("key-again").waitFor();
      await shot("settings-streaming-key-again");
      // The SRT target: its URL shown with the passphrase hidden, kept or typed again in full
      // (F10-1); an edited copy that still has *** is refused before it is sent.
      await page.getByTestId("target-editor").locator("..").getByRole("button", { name: "Cancel" }).click();
      await settle(200);
      await page.getByTestId("stream-target").nth(1).getByRole("button", { name: "Edit" }).click();
      await settle(300);
      await page.getByTestId("target-editor").scrollIntoViewIfNeeded();
      await shot("settings-streaming-edit-srt");
      await page.getByRole("button", { name: "Replace URL" }).click();
      await page.locator("#t-url").fill("srt://192.0.2.60:9001?passphrase=***");
      await settle(200);
      await shot("settings-streaming-srt-hidden");
      await page.getByTestId("target-editor").locator("..").getByRole("button", { name: "Cancel" }).click();
      await settle(200);
    });
  } finally {
    streamingStub.reset();
    const close = page.getByRole("toolbar", { name: "Settings" }).getByRole("button", { name: "Close" });
    if (await close.count()) {
      await close.click();
      await settle(200);
    }
  }
}
