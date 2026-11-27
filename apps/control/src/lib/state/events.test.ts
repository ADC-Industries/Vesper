import { describe, expect, it, vi } from "vitest";
import { ChangeEvents } from "./events";
import type { DocumentChangedMessage } from "../server";

const doc = (kind: "library" | "presentation", id: string) =>
  ({ type: "documentChanged", kind, id, msg: { type: "documentChanged", kind, id, change: "changed" } as DocumentChangedMessage }) as const;

describe("change events", () => {
  it("delivers each event to the subscribers of its type only", () => {
    const ev = new ChangeEvents();
    const docs = vi.fn();
    const codex = vi.fn();
    const site = vi.fn();
    ev.on("documentChanged", docs);
    ev.on("codexChanged", codex);
    ev.on("siteChanged", site);
    ev.emit(doc("library", "l1"));
    ev.emit({ type: "codexChanged", parts: ["collections"] });
    ev.emit({ type: "siteChanged", parts: ["joinRequests", "devices"] });
    expect(docs).toHaveBeenCalledTimes(1);
    expect(docs.mock.calls[0]![0].id).toBe("l1");
    expect(codex).toHaveBeenCalledWith({ type: "codexChanged", parts: ["collections"] });
    expect(site.mock.calls[0]![0].parts).toEqual(["joinRequests", "devices"]);
  });

  it("stops after unsubscribing, and a failing handler doesn't stop the others", () => {
    const ev = new ChangeEvents();
    const err = vi.spyOn(console, "error").mockImplementation(() => {});
    const bad = vi.fn(() => {
      throw new Error("boom");
    });
    const good = vi.fn();
    const off = ev.on("documentChanged", bad);
    ev.on("documentChanged", good);
    ev.emit(doc("presentation", "p1"));
    expect(good).toHaveBeenCalledTimes(1);
    expect(err).toHaveBeenCalled();
    off();
    ev.emit(doc("presentation", "p2"));
    expect(bad).toHaveBeenCalledTimes(1);
    expect(good).toHaveBeenCalledTimes(2);
    err.mockRestore();
  });
});
