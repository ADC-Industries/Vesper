import { describe, expect, it } from "vitest";
import type { StreamingStatus } from "$protocol";
import { RestClient } from "../rest";
import { StreamingClient, formatBytes, formatDuration, hasHiddenValue, indicator, serverKey, serverOf } from "../streaming";

describe("StreamingClient", () => {
  it("asks the recording and streaming routes, keys only when given", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    const fetchFake = (async (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return new Response(init.method === "DELETE" ? null : "{}", { status: init.method === "DELETE" ? 204 : 200 });
    }) as unknown as typeof fetch;
    const c = new StreamingClient(new RestClient({ baseUrl: "http://booth:5555", token: () => "tok", fetch: fetchFake }));
    await c.status();
    await c.startRecording();
    await c.splitRecording();
    await c.stopRecording();
    await c.updateTarget({ id: "t1", name: "Main", kind: "rtmp", url: "rtmp://192.0.2.5/live", hasKey: true, screen: null, codec: "h264", videoMbps: 6, audioKbps: 160 });
    await c.startStream("t1");
    await c.deleteTarget("t1");
    expect(calls.map((x) => `${x.init.method ?? "GET"} ${x.url}`)).toEqual([
      "GET http://booth:5555/api/v1/streaming/status",
      "POST http://booth:5555/api/v1/recording/start",
      "POST http://booth:5555/api/v1/recording/split",
      "POST http://booth:5555/api/v1/recording/stop",
      "PUT http://booth:5555/api/v1/streaming/targets/t1",
      "POST http://booth:5555/api/v1/streaming/targets/t1/start",
      "DELETE http://booth:5555/api/v1/streaming/targets/t1",
    ]);
    // No key in the edit: the server keeps the saved one.
    expect(JSON.parse(calls[4]!.init.body as string)).not.toHaveProperty("key");
  });
});

describe("formatting", () => {
  it("shows durations and sizes", () => {
    expect(formatDuration(0)).toBe("0:00");
    expect(formatDuration(75.9)).toBe("1:15");
    expect(formatDuration(3 * 3600 + 62)).toBe("3:01:02");
    expect(formatBytes(812_000_000)).toBe("812 MB");
    expect(formatBytes(1_400_000_000)).toBe("1.4 GB");
  });

  it("sums up the status for the Show page", () => {
    expect(indicator(null)).toEqual({ recording: false, streaming: 0, trouble: false, seconds: 0 });
    const base = { id: "t", name: "T", screen: "s", startedAt: 0, bytes: 0, sentKbps: 0, videoKbps: 0, reconnects: 0, dropped: 0, skippedPictures: 0, lagMs: 0 };
    const st: StreamingStatus = {
      recording: null,
      streams: [
        { ...base, state: "live", seconds: 30 },
        { ...base, id: "u", state: "retrying", seconds: 90 },
      ],
    };
    expect(indicator(st)).toEqual({ recording: false, streaming: 2, trouble: true, seconds: 90 });
  });
});

describe("hasHiddenValue", () => {
  it("finds the values the server hides in an SRT URL", () => {
    expect(hasHiddenValue("srt://192.0.2.60:9000?passphrase=***")).toBe(true);
    expect(hasHiddenValue("srt://192.0.2.60:9001?latency=200&streamid=***")).toBe(true);
    expect(hasHiddenValue("srt://192.0.2.60:9000?passphrase=madeuppass99")).toBe(false);
    expect(hasHiddenValue("srt://192.0.2.60:9000?passphrase=a***")).toBe(false);
    expect(hasHiddenValue("srt://192.0.2.60:9000")).toBe(false);
    expect(hasHiddenValue("")).toBe(false);
  });
});

describe("serverKey (R1-2)", () => {
  it("is another server when the kind changes, as the server says", () => {
    expect(serverKey("rtmp", "rtmps://a.example.com/live")).toBe(serverKey("rtmp", "rtmps://A.example.com/other"));
    expect(serverKey("srt", "srt://192.0.2.7:9000")).not.toBe(serverKey("rtmp", "srt://192.0.2.7:9000"));
  });
});

describe("serverOf", () => {
  it("names a URL's scheme, host and port the way the server keeps a saved key", () => {
    expect(serverOf("rtmp://a.rtmp.example.com/live2")).toBe("rtmp://a.rtmp.example.com");
    expect(serverOf("rtmp://A.RTMP.example.com/other?x=1")).toBe("rtmp://a.rtmp.example.com");
    expect(serverOf("rtmps://a.rtmp.example.com/live2")).toBe("rtmps://a.rtmp.example.com");
    expect(serverOf("rtmp://a.rtmp.example.com:1935/live2")).toBe("rtmp://a.rtmp.example.com:1935");
    expect(serverOf("rtmp://user:pw@192.0.2.5/live")).toBe("rtmp://192.0.2.5");
    expect(serverOf("srt://192.0.2.7:9000?passphrase=***")).toBe("srt://192.0.2.7:9000");
  });
});
