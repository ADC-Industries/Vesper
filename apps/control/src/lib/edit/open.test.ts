import { describe, expect, it } from "vitest";
import { ApiRequestError } from "../server";
import { openFailure } from "./open";

describe("opening a presentation that fails", () => {
  it("a deleted presentation is cleared quietly, never a red error", () => {
    const gone = new ApiRequestError("notFound", "presentation cf2bd1ac not found", 404);
    expect(openFailure(gone, true)).toEqual({ missing: true, notice: null });
    const asked = openFailure(gone, false);
    expect(asked.missing).toBe(true);
    expect(asked.notice?.level).toBe("info");
    expect(asked.notice?.message).not.toContain("cf2bd1ac");
  });

  it("real failures still show an error, even for background opens", () => {
    for (const e of [
      new ApiRequestError("network", "Could not reach the server", 0),
      new ApiRequestError("internal", "disk full", 500),
      new Error("boom"),
    ]) {
      for (const quiet of [true, false]) {
        const f = openFailure(e, quiet);
        expect(f.missing).toBe(false);
        expect(f.notice?.level).toBe("error");
      }
    }
  });
});
