import { afterEach, describe, expect, it } from "vitest";
import type { ServerInfo } from "$protocol";
import { session } from "./session.svelte";
import { shell } from "./shell.svelte";
import { screensState } from "./screens.svelte";

const server = { id: "s", name: "Test", version: "0", protocolVersion: 1, capabilities: ["codex", "site", "screenConfig"], auth: "local" } as ServerInfo;

describe("shell routing", () => {
  afterEach(() => {
    screensState.guard = null;
    shell.reset();
    session.server = null;
  });

  it("goes to pages and modes, and Close / Done go back the way the operator came", () => {
    session.server = server;
    expect(shell.route).toBe("show");
    shell.go("library");
    expect([shell.page, shell.mode]).toEqual(["library", "show"]);
    shell.go("timers");
    shell.go("edit");
    expect([shell.page, shell.mode]).toEqual(["show", "edit"]);
    shell.close();
    expect(shell.route).toBe("timers");
    shell.close();
    expect(shell.route).toBe("library");
    shell.close();
    expect(shell.route).toBe("show");
  });

  it("Ctrl+E / Ctrl+R toggle back to Show; Ctrl+S always goes to Show", () => {
    shell.appAction("viewEdit");
    expect(shell.route).toBe("edit");
    shell.appAction("viewEdit");
    expect(shell.route).toBe("show");
    shell.appAction("viewReflow");
    expect(shell.route).toBe("reflow");
    shell.appAction("viewShow");
    expect(shell.route).toBe("show");
  });

  it("doesn't open a page the server can't show", () => {
    session.server = { ...server, capabilities: [] };
    expect(shell.go("site")).toBe(false);
    expect(shell.route).toBe("show");
    shell.appAction("pageBible");
    expect(shell.route).toBe("show");
    shell.appAction("pageTimers");
    expect(shell.route).toBe("timers");
  });

  it("asks the Screens page before leaving it, then goes on where the operator was going", () => {
    session.server = server;
    shell.go("screens");
    let dirty = true;
    screensState.guard = () => !dirty;
    expect(shell.go("library")).toBe(false);
    expect(shell.route).toBe("screens");
    dirty = false;
    shell.resumeAfterGuard();
    expect(shell.route).toBe("library");
  });

  it("Settings is a page: its own route, Close goes back, Esc stays", () => {
    session.server = server;
    shell.go("library");
    shell.appAction("settings");
    expect([shell.page, shell.route, shell.settingsSection]).toEqual(["settings", "settings", "General"]);
    expect(shell.dialogOpen).toBe(false);
    shell.openSettings("Audio");
    expect([shell.route, shell.settingsSection]).toEqual(["settings", "Audio"]);
    shell.closeDialogs();
    expect(shell.route).toBe("settings");
    shell.close();
    expect(shell.route).toBe("library");
  });

  it("Esc closes dialogs but never leaves a page", () => {
    session.server = server;
    shell.go("timers");
    shell.showHelp = true;
    shell.closeDialogs();
    expect(shell.showHelp).toBe(false);
    expect(shell.route).toBe("timers");
  });
});
