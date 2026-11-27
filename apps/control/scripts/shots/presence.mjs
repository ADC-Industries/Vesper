// Shots of presence and edit locks (G8; see scripts/shots.mjs for `ctx`), with the stub's
// presence list (test/stub/routes/presence.mjs; synthetic people and computers): in Reflow, the
// "2 others" badge with its list, the song locked by someone else (banner, read-only text),
// and this Oriel losing the lock while it typed (its typing kept: "Take back editing", "Copy
// my typing"); in the Library, "… is editing" on a song's row and the banner on the song; in
// Edit, the badge and the banner over a read-only editor; a song locked by someone on another
// Vesper of the site (site-wide locks, its box named); browsing: "3 others have this collection
// open" on the Library's connection card, the Show page's playlist badge, and a timer locked by
// someone else on the Timers page (read-only settings). The list is emptied at the end.
import { presenceStub } from "../../test/stub/routes/presence.mjs";
import { reflowStub } from "../../test/stub/routes/reflow.mjs";
import { editStub } from "../../test/stub/routes/edit.mjs";
import { timersStub } from "../../test/stub/routes/timers.mjs";

const SINCE = "2026-11-24T10:15:00Z";
const person = (session, clientId, computer, open, user) => ({ session, clientId, name: `Oriel – ${computer}`, computer, ...(user ? { user } : {}), open });
const lockBy = (kind, id, clientId, computer, user) => ({ kind, id, clientId, name: `Oriel – ${computer}`, computer, ...(user ? { user } : {}), since: SINCE });
const others = (kind, id) => [
  person("s2", "booth-pc", "Booth PC", [{ kind, id, place: "library" }], "sam"),
  person("s3", "lobby", "Lobby laptop", [{ kind, id, place: "edit" }]),
];

export default async function ({ page, settle, shot, stub, step, openPage }) {
  const changed = (id) => stub.push({ type: "documentChanged", kind: "presentation", id, change: "changed" });
  try {
    // ---- Reflow ----
    reflowStub.reset();
    reflowStub.on = true;
    const song = reflowStub.id;
    changed(song);
    await settle(600);
    await page.keyboard.press("Control+r");
    await settle(1200);

    await step("presence reflow others", async () => {
      presenceStub.set({ clients: others("presentation", song), locks: [] });
      await settle(300);
      await page.locator("[data-presence-badge]").first().click();
      await settle(300);
      await shot("presence-reflow-others");
      await page.locator("[data-presence-badge]").first().click();
      await settle(200);
    });

    await step("presence reflow locked", async () => {
      presenceStub.set({ clients: others("presentation", song), locks: [lockBy("presentation", song, "booth-pc", "Booth PC", "sam")] });
      await settle(400);
      await page.mouse.move(5, 300);
      await shot("presence-reflow-locked");
    });

    await step("presence reflow lost", async () => {
      // This Oriel holds the lock and types; sam takes over before the typing is saved.
      presenceStub.set({ clients: others("presentation", song), locks: [lockBy("presentation", song, presenceStub.me, "Booth A")] });
      await settle(300);
      const text = page.locator("textarea[data-reflow-text]").nth(2);
      await text.click();
      await page.keyboard.press("End");
      await page.keyboard.type(" again");
      presenceStub.set({ clients: others("presentation", song), locks: [lockBy("presentation", song, "booth-pc", "Booth PC", "sam")] });
      await page.locator("[data-save-status='locked']").waitFor({ timeout: 3000 });
      await settle(300);
      await shot("presence-reflow-lost");
      // Take it back: the kept typing is saved.
      await page.getByRole("button", { name: "Take back editing" }).click();
      await page.locator("[data-save-status='saved']").waitFor({ timeout: 3000 });
      await settle(200);
    });
    reflowStub.on = false;
    changed(song);
    await settle(400);

    // ---- Library ----
    await step("presence library", async () => {
      await openPage("Library");
      const row = page.locator(".what .row").nth(1);
      const id = await row.getAttribute("data-id");
      presenceStub.set({ clients: others("presentation", id), locks: [lockBy("presentation", id, "booth-pc", "Booth PC", "sam")] });
      await row.click();
      await settle(800);
      await page.mouse.move(5, 300);
      await shot("presence-library-locked");
      await page.keyboard.press("Control+s");
      await settle(400);
    });

    // Site-wide (stage 3): the song locked by someone on another Vesper of the site.
    await step("presence library other box", async () => {
      await openPage("Library");
      await settle(400);
      const row = page.locator(".what .row").nth(2);
      const id = await row.getAttribute("data-id");
      const remote = { ...person("ab12cd34-s4", "chapel", "Chapel laptop", [{ kind: "presentation", id, place: "edit" }], "jo"), server: "Chapel Booth" };
      presenceStub.set({ clients: [...others("presentation", id), remote], locks: [{ ...lockBy("presentation", id, "chapel", "Chapel laptop", "jo"), server: "Chapel Booth" }] });
      await row.click();
      await settle(800);
      await page.mouse.move(5, 300);
      await shot("presence-library-locked-other-box");
      await page.keyboard.press("Control+s");
      await settle(400);
    });

    // ---- Browsing (presence for the collection, the Show page and timers) ----
    // What this Oriel reports open, copied into made-up people's lists.
    const reported = async (kind, place) => {
      for (let i = 0; i < 30; i++) {
        const o = presenceStub.reported.find((x) => x.kind === kind && x.place === place);
        if (o) return o.id;
        await settle(100);
      }
      throw new Error(`Oriel never reported a ${kind} open (${place})`);
    };

    await step("presence library collection", async () => {
      await openPage("Library");
      await settle(400);
      const col = await reported("collection", "library");
      const row = page.locator(".what .row").nth(1);
      const song = await row.getAttribute("data-id");
      const open = (extra) => [{ kind: "collection", id: col, place: "library" }, ...extra];
      presenceStub.set({
        clients: [
          person("s2", "booth-pc", "Booth PC", open([{ kind: "presentation", id: song, place: "library" }]), "sam"),
          person("s3", "lobby", "Lobby laptop", open([])),
          person("s4", "office", "Office iMac", open([]), "kim"),
        ],
        locks: [lockBy("presentation", song, "booth-pc", "Booth PC", "sam")],
      });
      await page.locator("[data-collection-presence]").click();
      await settle(400);
      await shot("presence-library-collection");
      await page.keyboard.press("Escape");
      await settle(200);
    });

    await step("presence show", async () => {
      await page.keyboard.press("Control+s");
      await settle(800);
      const playlist = await reported("playlistNode", "show");
      presenceStub.set({
        clients: [
          person("s2", "booth-pc", "Booth PC", [{ kind: "playlistNode", id: playlist, place: "show" }], "sam"),
          person("s3", "lobby", "Lobby laptop", [{ kind: "playlistNode", id: playlist, place: "show" }]),
        ],
        locks: [],
      });
      await settle(300);
      await page.locator("[data-presence-badge]").first().click();
      await settle(300);
      await shot("presence-show");
      await page.keyboard.press("Escape");
      await settle(200);
    });

    await step("presence timers locked", async () => {
      timersStub.start();
      stub.push({ type: "documentChanged", kind: "timer", id: "5eed0000-0000-4000-8000-00000000a001", change: "changed" });
      await openPage("Timers");
      await settle(800);
      const timer = await reported("timer", "timers");
      presenceStub.set({
        clients: [person("s2", "booth-pc", "Booth PC", [{ kind: "timer", id: timer, place: "timers" }], "sam")],
        locks: [lockBy("timer", timer, "booth-pc", "Booth PC", "sam")],
      });
      await settle(400);
      if (!(await page.locator("[data-timer-settings] input[type=text]").first().isDisabled())) throw new Error("the locked timer's settings can be changed");
      await page.mouse.move(5, 300);
      await shot("presence-timers-locked");
    });
    timersStub.stop();
    stub.push({ type: "documentChanged", kind: "timer", id: "5eed0000-0000-4000-8000-00000000a001", change: "changed" });

    // ---- Edit ----
    await step("presence edit", async () => {
      editStub.reset();
      editStub.on = true;
      changed(editStub.id);
      await settle(600);
      presenceStub.set({ clients: others("presentation", editStub.id), locks: [lockBy("presentation", editStub.id, "booth-pc", "Booth PC", "sam")] });
      await page.keyboard.press("Control+e");
      await settle(1500);
      await page.mouse.move(5, 300);
      await shot("presence-edit-locked");
    });
  } finally {
    presenceStub.set({ clients: [], locks: [] });
    reflowStub.on = false;
    editStub.on = false;
    changed(reflowStub.id);
    changed(editStub.id);
    await settle(400);
  }
}
