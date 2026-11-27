import { afterEach, describe, expect, it, vi } from "vitest";
import type { Presentation } from "$protocol";
import { EditState } from "./editState.svelte";
import { session } from "../state/session.svelte";
import { ApiRequestError } from "../server/rest";

const pres = (...ids: string[]) => ({ id: "p", sections: [{ id: "s", slides: ids.map((id) => ({ id })) }] }) as unknown as Presentation;

afterEach(() => {
  session.conn = null;
  session.editor = null;
  session.open = null;
});

describe("the Build tab's step preview (D10-7)", () => {
  it("is dropped when the slide shown changes outside selectSlide", () => {
    const st = new EditState();
    st.reconcile(pres("a", "b"));
    st.selectSlide("b");
    st.buildPreview = 0;
    // Slide b was deleted: the editor moves to slide a, without b's step preview.
    st.reconcile(pres("a"));
    expect(st.slideId).toBe("a");
    expect(st.buildPreview).toBeNull();
    // Another presentation opened (EditView starts over).
    st.buildPreview = 1;
    st.startOver();
    expect(st.slideId).toBeNull();
    expect(st.buildPreview).toBeNull();
  });
});

describe("typing refused by a lost edit lock (E10-1)", () => {
  it("is reported with its text when it's refused after the editor closed", async () => {
    const pushNotice = vi.fn();
    const p = pres("a");
    session.conn = { pushNotice, rest: { presentation: async () => p } } as never;
    session.editor = {
      editPresentation: async () => {
        throw new ApiRequestError("locked", "Booth PC is editing this presentation", 423);
      },
    } as never;
    session.open = { presentation: p, slides: [] } as never;
    const st = new EditState();
    // The page closes (its teardown reports), then the in-place editor's onDestroy saves the
    // typing, which the server refuses: someone took over meanwhile.
    st.reportLocked();
    expect(pushNotice).not.toHaveBeenCalled();
    const ok = await st.run("Typing", [{ type: "setSlideText", slide: "a", element: "e", text: "the second line, fixed", before: "" } as never]);
    expect(ok).toBe(false);
    expect(pushNotice).toHaveBeenCalledTimes(1);
    expect(pushNotice.mock.calls[0]![1]).toContain("the second line, fixed");
    expect(st.lockedEdits).toEqual([]);
  });

  it("is kept to send again while the editor is open", async () => {
    const pushNotice = vi.fn();
    const p = pres("a");
    session.conn = { pushNotice, rest: { presentation: async () => p } } as never;
    session.editor = {
      editPresentation: async () => {
        throw new ApiRequestError("locked", "Booth PC is editing this presentation", 423);
      },
    } as never;
    session.open = { presentation: p, slides: [] } as never;
    const st = new EditState();
    await st.run("Typing", [{ type: "setSlideText", slide: "a", element: "e", text: "kept", before: "" } as never]);
    expect(pushNotice).not.toHaveBeenCalled();
    expect(st.lockedEdits.map((k) => k.text)).toEqual(["kept"]);
  });
});
