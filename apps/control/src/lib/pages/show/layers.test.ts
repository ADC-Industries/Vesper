import { describe, expect, it } from "vitest";
import { layerAction } from "./layers";

const quiet = { slide: false, announcements: false, media: false, props: false, messages: false, audio: false };

describe("layer buttons", () => {
  it("clear what is on the layer (the Slide button also clears announcements)", () => {
    expect(layerAction("media", { ...quiet, media: true }, ["media"])).toEqual({ kind: "clear", commands: [{ type: "clearLayer", layer: "media" }] });
    expect(layerAction("slide", { ...quiet, slide: true, announcements: true }, [])).toEqual({
      kind: "clear",
      commands: [
        { type: "clearLayer", layer: "slide" },
        { type: "clearLayer", layer: "announcements" },
      ],
    });
    expect(layerAction("slide", { ...quiet, announcements: true }, ["slide"]).commands).toEqual([{ type: "clearLayer", layer: "announcements" }]);
  });

  it("bring back what was cleared while the layer is empty", () => {
    expect(layerAction("props", quiet, ["props", "audio"])).toEqual({ kind: "restore", commands: [{ type: "restoreLayer", layer: "props" }] });
    expect(layerAction("slide", quiet, ["announcements", "slide"]).commands).toEqual([
      { type: "restoreLayer", layer: "slide" },
      { type: "restoreLayer", layer: "announcements" },
    ]);
  });

  it("do nothing on an empty layer with nothing kept", () => {
    expect(layerAction("audio", quiet, ["media"])).toEqual({ kind: "none", commands: [] });
  });
});
