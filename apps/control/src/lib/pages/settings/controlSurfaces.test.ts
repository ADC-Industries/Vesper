import { describe, expect, it } from "vitest";
import { parsePort, remoteCompatLine } from "./controlSurfaces";

describe("remote API compatibility card", () => {
  it("says whether the layer listens", () => {
    expect(remoteCompatLine(null).text).toBe("Reading…");
    expect(remoteCompatLine({ enabled: false, port: 50001, listening: false })).toEqual({ status: "off", text: "Off" });
    expect(remoteCompatLine({ enabled: true, port: 50001, listening: true, listeningPort: 50001 })).toEqual({ status: "ok", text: "Listening on port 50001" });
    expect(remoteCompatLine({ enabled: true, port: 80, listening: false, problem: "port 80 can't be opened: permission denied" })).toEqual({
      status: "error",
      text: "Not listening: port 80 can't be opened: permission denied",
    });
  });
  it("takes ports 1–65535 only", () => {
    expect(parsePort(" 50001 ")).toBe(50001);
    for (const bad of ["", "0", "65536", "12a", "-1", "1.5"]) expect(parsePort(bad)).toBeNull();
  });
});
