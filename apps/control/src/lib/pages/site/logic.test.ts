import { describe, expect, it } from "vitest";
import type { ImportCollection } from "$protocol";
import {
  apiWarning,
  canRestartService,
  damagedWarning,
  defaultChoices,
  deviceKindLine,
  deviceName,
  deviceSummary,
  healthText,
  importNote,
  isVesperBox,
  relayToggle,
  importOptions,
  importRequest,
  inOtherSite,
  isJoinEvent,
  isThisBox,
  membershipText,
  mmss,
  passphraseProblem,
  serviceState,
  spacedCode,
  withSiteEvent,
} from "./logic";
import type { SiteEvent } from "$protocol";

const col = (id: string, comparison: ImportCollection["comparison"], extra: Partial<ImportCollection> = {}): ImportCollection => ({
  collection: { id, name: id, documents: 3, seq: 1 },
  comparison,
  active: false,
  tooNew: false,
  ...extra,
});

describe("devices", () => {
  it("summarises the site, leaving removed machines out", () => {
    expect(deviceSummary([{ health: "online" }, { health: "needsAttention" }, { health: "offline" }, { health: "removed" }])).toBe(
      "3 devices · 2 online · 1 needs attention",
    );
    expect(deviceSummary([{ health: "online" }])).toBe("1 device · 1 online");
  });
  it("names the Codex's own membership", () => {
    expect(deviceName({ name: "Codex", builtIn: true })).toBe("Codex (built-in)");
    expect(deviceName({ name: "Chapel Box" })).toBe("Chapel Box");
  });
  it("says what a device is, and that it relays pairings", () => {
    const d = { kind: "app", node: false, admin: false };
    expect(deviceKindLine(d)).toBe("App");
    expect(deviceKindLine({ ...d, node: true, relay: true })).toBe("App · IDN Node · Relays pairings");
    expect(deviceKindLine({ ...d, kind: "output", admin: true })).toBe("Output · site admin");
    expect(deviceKindLine({ ...d, builtIn: true, relay: true })).toBe("Part of Vesper on this server");
  });
  it("offers the relay toggle only for a Vesper box, or to take a mark off (M-6)", () => {
    const d = { kind: "app", name: "LOBBY (Vesper)", health: "online" as const };
    expect(isVesperBox(d)).toBe(true);
    expect(relayToggle(d)).toBe(true);
    // Another program's app, a display box or camera PC named like one, only the suffix.
    expect(relayToggle({ ...d, name: "Lighting desk" })).toBe(false);
    expect(relayToggle({ ...d, kind: "output" })).toBe(false);
    expect(relayToggle({ ...d, kind: "input" })).toBe(false);
    expect(isVesperBox({ kind: "app", name: " (Vesper)" })).toBe(false);
    expect(isVesperBox({ kind: "app", name: "BOX (Vesper) spare" })).toBe(false);
    // A machine marked anyway can have its mark taken off.
    expect(relayToggle({ ...d, kind: "output", relay: true })).toBe(true);
    // Never the Codex's own membership or a removed machine.
    expect(relayToggle({ ...d, builtIn: true })).toBe(false);
    expect(relayToggle({ ...d, health: "removed" })).toBe(false);
  });
  it("puts the reason after the health", () => {
    expect(healthText({ health: "needsAttention", reason: "Too warm" })).toBe("Needs attention: Too warm");
    expect(healthText({ health: "online" })).toBe("Online");
  });
  it("names a service's state, an unknown one as it is", () => {
    expect(serviceState("running")).toEqual({ label: "Running", status: "ok" });
    expect(serviceState("failed").status).toBe("error");
    expect(serviceState("no_session").label).toBe("Waiting for a log-in");
    expect(serviceState("sleeping")).toEqual({ label: "sleeping", status: "off" });
    expect(serviceState("").label).toBe("Unknown");
  });
  it("offers Restart unless the service isn't there or restarts already", () => {
    expect(canRestartService({ id: "display", state: "running" })).toBe(true);
    expect(canRestartService({ id: "display", state: "failed" })).toBe(true);
    expect(canRestartService({ id: "display", state: "unsupported" })).toBe(false);
    expect(canRestartService({ id: "display", state: "restarting" })).toBe(false);
    expect(canRestartService({ id: "display", state: "running" }, ["display"])).toBe(false);
  });
});

describe("codes", () => {
  it("spaces six digits and counts down", () => {
    expect(spacedCode("482913")).toBe("482 913");
    expect(spacedCode(" 482 913 ")).toBe("482 913");
    expect(mmss(581)).toBe("9:41");
    expect(mmss(-3)).toBe("0:00");
  });
  it("lists join, device and pairing events under Recently", () => {
    expect(isJoinEvent({ name: "join_requested" })).toBe(true);
    expect(isJoinEvent({ name: "key_requested" })).toBe(true);
    expect(isJoinEvent({ name: "device_offline" })).toBe(true);
    expect(isJoinEvent({ name: "two_sites" })).toBe(false);
  });
});

describe("export form", () => {
  it("needs 8 characters, typed twice the same", () => {
    expect(passphraseProblem("short", "short")).toMatch(/8 characters/);
    expect(passphraseProblem("long enough", "long enougj")).toMatch(/don't match/);
    expect(passphraseProblem("long enough", "long enough")).toBeNull();
    expect(passphraseProblem("long enough")).toBeNull();
  });
});

describe("import choices", () => {
  const preview = {
    collections: [
      col("new", "new"),
      col("same", "same"),
      col("newer", "newer"),
      col("active", "older", { active: true }),
      col("trash", "inTrash"),
      col("future", "new", { tooNew: true }),
    ],
  };
  it("offers what the server can do with each", () => {
    expect(importOptions(preview.collections[0]!)).toEqual(["add", "skip"]);
    expect(importOptions(preview.collections[2]!)).toEqual(["keep", "replace"]);
    expect(importOptions(preview.collections[3]!)).toEqual(["keep"]);
    expect(importOptions(preview.collections[4]!)).toEqual(["skip"]);
    expect(importOptions(preview.collections[5]!)).toEqual(["skip"]);
    expect(importNote(preview.collections[3]!)).toMatch(/assign another/);
    expect(importNote(preview.collections[0]!)).toBeNull();
  });
  it("defaults to the server's defaults: add new ones, keep the rest", () => {
    expect(defaultChoices(preview)).toEqual({ new: "add", same: "keep", newer: "keep", active: "keep", trash: "skip", future: "skip" });
    expect(importRequest("pass phrase", preview, defaultChoices(preview), { backups: true, site: false })).toEqual({ passphrase: "pass phrase", skip: ["future"] });
  });
  it("sends replace and skip for the operator's choices, never replacing the shown one", () => {
    const req = importRequest("pass phrase", preview, { new: "skip", newer: "replace", active: "replace" }, { backups: false, site: true });
    expect(req).toEqual({ passphrase: "pass phrase", replace: ["newer"], skip: ["new", "future"], backups: false, site: true });
  });
});

describe("membership", () => {
  it("says where this server stands", () => {
    expect(membershipText({ state: "none", connected: false })).toMatch(/its own site/);
    expect(membershipText({ state: "enrolled", server: "hall.example", connected: true })).toBe("In the site at hall.example · connected");
    expect(membershipText({ state: "wrongCode", triesLeft: 2, connected: false })).toBe("Wrong code. 2 tries left.");
    expect(inOtherSite({ state: "waitingForApproval", connected: false })).toBe(true);
    expect(inOtherSite({ state: "none", connected: false })).toBe(false);
  });
});

describe("backups", () => {
  it("says which set is this box's, also when the list came from the Codex box", () => {
    const codexBox = { thisBox: true, set: { machine: "codex-box" } };
    const showBox = { thisBox: false, set: { machine: "show-box" } };
    expect(isThisBox(codexBox, "codex-box", false)).toBe(true);
    expect(isThisBox(showBox, "codex-box", false)).toBe(false);
    // Oriel on the show box, the list read from the Codex box through the side connection.
    expect(isThisBox(codexBox, "show-box", true)).toBe(false);
    expect(isThisBox(showBox, "show-box", true)).toBe(true);
    expect(isThisBox(showBox, "", true)).toBe(false);
  });
});

describe("Recently", () => {
  const ev = (text: string, at = "2026-11-23T10:41:00Z"): SiteEvent => ({ name: "device_joined", level: "ok", text, at, detail: {} });

  it("puts a live siteEvent first and keeps at most the limit", () => {
    const list = [ev("b", "2026-11-23T10:40:00Z"), ev("a", "2026-11-23T10:39:00Z")];
    expect(withSiteEvent(list, ev("c")).map((e) => e.text)).toEqual(["c", "b", "a"]);
    expect(withSiteEvent(list, ev("c"), 2).map((e) => e.text)).toEqual(["c", "b"]);
  });

  it("doesn't add an event a refetch already brought", () => {
    const list = [ev("c"), ev("b", "2026-11-23T10:40:00Z")];
    expect(withSiteEvent(list, ev("c"))).toBe(list);
  });
});

describe("site manager status", () => {
  it("warns while running without the admin panel", () => {
    expect(apiWarning({ running: true, apiError: "address in use (API port 29202)" })).toBe("Running, but the admin panel isn't available: address in use (API port 29202)");
    expect(apiWarning({ running: true })).toBeNull();
    expect(apiWarning({ running: false, apiError: "stale" })).toBeNull();
    expect(apiWarning(null)).toBeNull();
  });
  it("warns after a start from damaged site settings, until the access is set again", () => {
    const you = (role: "viewer" | "admin") => ({ access: "users" as const, role, signedIn: false, unlocked: false });
    expect(damagedWarning({ settingsDamaged: true, you: you("viewer") })).toBe(
      "The site settings were damaged, so this server started locked down: paired computers can only view. Set the access again in Security, on the server machine or signed in as a site admin, to fix it.",
    );
    expect(damagedWarning({ settingsDamaged: true, you: you("admin") })).toContain("keeping Users is enough");
    expect(damagedWarning({ you: you("admin") })).toBeNull();
    expect(damagedWarning({ settingsDamaged: false, you: you("viewer") })).toBeNull();
    expect(damagedWarning(null)).toBeNull();
  });
});
