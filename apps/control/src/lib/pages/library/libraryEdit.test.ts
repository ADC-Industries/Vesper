import { afterEach, describe, expect, it, vi } from "vitest";
import type { CodexStatus, CollectionSummary } from "$protocol";
import type { Connection } from "../../server";
import { libraryPage } from "../../state/library.svelte";
import { session, type OpenPresentation } from "../../state/session.svelte";
import { shell } from "../../state/shell.svelte";
import { siteState } from "../../state/site.svelte";

// A connection whose lists are all empty (the Library page's load asks for many).
const emptyConn = () =>
  ({
    rest: new Proxy({ presentation: (id: string) => Promise.resolve({ id, sections: [] }) } as Record<string, unknown>, {
      get: (t, k) => t[k as string] ?? (() => Promise.resolve([])),
    }),
    socket: { isOnline: true },
    pushNotice: () => {},
  }) as unknown as Connection;

afterEach(() => {
  session.open = null;
  session.conn = null;
  siteState.codex = null;
  shell.reset();
});

describe("a song in Edit on the Library page", () => {
  it("is done when another Oriel switches the box's collection (A15-3)", async () => {
    session.conn = emptyConn();
    session.open = { presentation: { id: "A" }, slides: [], playlistItem: null } as unknown as OpenPresentation;
    await libraryPage.startEdit("B");
    expect(libraryPage.mode).toBe("edit");
    const finished = vi.fn(async () => {});
    libraryPage.editState!.finishText = finished;
    libraryPage.loadedCollection = "c1";
    siteState.codex = { collection: "c2" } as unknown as CodexStatus;
    libraryPage.followCollection();
    await vi.waitFor(() => expect(libraryPage.mode).toBe("view"));
    expect(finished).toHaveBeenCalled();
    expect(shell.pageEditing).toBe(false);
    expect(libraryPage.editState).toBeNull();
    // Show's presentation comes back.
    await vi.waitFor(() => expect(session.open?.presentation.id).toBe("A"));
  });
});

describe("typing kept from a lost edit lock (D09-1)", () => {
  it("is reported with its text when the Library leaves Edit, not dropped", async () => {
    const notices: [string, string][] = [];
    session.conn = { ...emptyConn(), pushNotice: (kind: string, text: string) => notices.push([kind, text]) } as unknown as Connection;
    session.open = { presentation: { id: "A" }, slides: [], playlistItem: null } as unknown as OpenPresentation;
    await libraryPage.startEdit("B");
    libraryPage.editState!.lockedEdits = [{ label: "Typing", ops: [], presentation: "B", text: "The lantern sways" }];
    await libraryPage.endEdit();
    expect(libraryPage.editState).toBeNull();
    expect(notices).toContainEqual(["error", "Typing not saved: someone else took over editing. What you typed: “The lantern sways”"]);
  });
});

describe("a collection's details", () => {
  it("are saved one after the other, each with the newest revision (A15-4)", async () => {
    let rev = 1;
    const sent: string[] = [];
    const summary = () => ({ collection: { id: "c1", name: "Example" }, revision: `r${rev}` }) as unknown as CollectionSummary;
    session.conn = {
      rest: new Proxy(
        {
          request: async (method: string, _path: string, _body: unknown, _q: unknown, opts?: { ifMatch?: string }) => {
            if (method !== "PUT") return undefined;
            sent.push(opts?.ifMatch ?? "");
            if (opts?.ifMatch !== `r${rev}`) throw new Error("stale revision");
            rev += 1;
            await new Promise((r) => setTimeout(r, 5));
            return summary();
          },
          get: async (path: string) => (path === "/codex/collections" ? [summary()] : []),
        } as Record<string, unknown>,
        { get: (t, k) => t[k as string] ?? (() => Promise.resolve([])) },
      ),
      socket: { isOnline: true },
      pushNotice: () => {},
    } as unknown as Connection;
    libraryPage.collections = [summary()];
    const stale = libraryPage.collections[0]!;
    // A rename on blur, then a colour clicked at once (both with the summary shown then).
    const a = libraryPage.updateCollection(stale, { name: "Renamed" }, "Renaming");
    const b = libraryPage.updateCollection(stale, { color: null }, "Changing the colour");
    expect(await a).toBe(true);
    expect(await b).toBe(true);
    expect(sent).toEqual(["r1", "r2"]);
  });
});
