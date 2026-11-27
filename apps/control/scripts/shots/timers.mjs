// Shots of the Timers page (see scripts/shots.mjs for `ctx`): timers running, paused and over
// time (test/stub/routes/timers.mjs), the selected countdown with its settings, then the timer
// over time selected.
import { timersStub } from "../../test/stub/routes/timers.mjs";

export default async function ({ page, openPage, settle, shot, step, stub }) {
  // The timer list is a document list: tell Oriel it changed (the values come with the state).
  const changed = () => stub.push({ type: "documentChanged", kind: "timer", id: "5eed0000-0000-4000-8000-00000000a001", change: "changed" });
  timersStub.start();
  changed();
  try {
    await settle(400);
    await openPage("Timers");
    await shot("page-timers");
    await step("timer over time", async () => {
      await page.getByTestId("timer-card").filter({ hasText: "Walk-in Countdown" }).locator("button").first().click();
      await settle(300);
      await shot("timers-overrun");
    });
  } finally {
    timersStub.stop();
    changed();
    await settle(300);
  }
}
