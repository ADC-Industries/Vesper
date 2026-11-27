# Recording and streaming (batch 13, lane R)

A screen's program (its picture and the program audio) recorded to a file on the server and
streamed live to RTMP(S) and SRT targets, through `idn-ffmpeg`'s gateway (`gateway::Gateway`:
one shared hardware encode per setting, each destination writing and reconnecting on its own
thread, keys hidden in its status). Products row: "streaming and recording (IDN gateways)".

## Model

Machine-local settings in `<data>/machine/streaming.json` (atomic writes). Not a document and
**not in settings backups** (only `MACHINE_KINDS` and `server.json` are): stream keys stay on
the box they were typed into.

- `recording`: `screen` (a screen ID, `null` = the first enabled screen), `container`
  (`mp4` fragmented, the default: opens in every editor and a crash loses at most the last
  fragment; or `mkv`), `codec` (`h264` default, `hevc`, `av1`: whatever the box's encoder
  takes), `videoMbps` (12), `audioKbps` (192), `splitMinutes` (0 = one file), `folder`
  (`null` = `<data>/recordings`), `minFreeGb` (2).
- `targets[]`: `id`, `name`, `kind` (`rtmp` | `srt`), `url` (RTMP: the server URL without
  the key, `rtmp://` or `rtmps://`; SRT: `srt://host:port?…`), `key` (the stream key: written,
  never read back: the API answers `hasKey`), `screen`, `videoMbps` (6), `audioKbps` (160).
  RTMP sends `url/key`; SRT sends the URL as typed (an SRT passphrase or stream ID belongs in
  its query, so the whole SRT URL is treated as secret in status and logs). A saved key or
  SRT secret is kept on edit only while the target points at the same server (scheme, host,
  port); for another server it is entered again, so it is only ever sent where it was typed
  for (an edit that moves a target with a key to another server without one is refused, and
  Oriel asks for the key again: the key is never dropped without a word). The file is created readable by the server's account only (`0600`) from its first
  byte.

## API

- `GET /api/v1/streaming` → `{ recording, targets: [{…, key: null, hasKey}], encoders,
  recordingsFolder }`; `PUT /api/v1/streaming/recording`; `POST /api/v1/streaming/targets`,
  `PUT|DELETE /api/v1/streaming/targets/{id}` (`key` absent keeps it, `""` clears it).
- `GET /api/v1/streaming/status` → the recording (file name, screen, state, started,
  seconds, bytes, kbit/s, dropped and skipped pictures, free space) and each target's stream
  (state, detail, seconds, sent and video kbit/s, reconnects, dropped, lag). Oriel polls it
  every 2 s (no WebSocket message: one less shared protocol change; listed as a follow-up).
- `POST /api/v1/recording/start` (`{ screen? }`), `/recording/stop`, `/recording/split`;
  `POST /api/v1/streaming/targets/{id}/start`, `/stop`. Answers `409 conflict` when already
  running / not running, `507 insufficientStorage` when the folder has less than
  `minFreeGb` free, `409` when no encoder takes the codec.

## Server

`gateways.rs`: one `Gateway` per screen that is recorded or streamed, fed by our own tap:
a thread on the screen's frame grid (the same timecodes as its outputs) reads the screen's
latest frame, converts it once per new frame to 4:2:2 Y'CbCr planes (BT.709 limited, the
output crate's UYVY conversion) and calls `Gateway::send`; the program audio comes from
the existing embedded-audio worker (a master-bus tap, blocks sent when heard) with a sink
that hands planar blocks to the gateway. When the last destination of a screen goes, its
threads stop and the gateway closes (trailers written). Every 5 s a recording checks free
space and stops itself (a warning notice) under half of `minFreeGb`. Splits: `set_split`
(minutes) and a manual split (the file closed and a new numbered part started). A stream that
can't connect keeps retrying (the gateway's own retries); its status says so. URLs with keys
are never logged: logs and status use the gateway's redacted description.

Encoders: the GPU's (VA-API on the church boxes, NVENC/AMF/Quick Sync on Windows); without
one, the CPU's (OpenH264 in the Windows FFmpeg, the system FFmpeg's on Linux).

## Oriel

- Show page: a REC / LIVE indicator in the right column's output row (elapsed time, red when
  recording or streaming, amber while a stream retries); its menu starts and stops the
  recording and each target.
- Settings ▸ Recording & streaming: the recording settings, the targets list (add, edit,
  delete; the key field shows "Key saved" and is never filled from the server), start/stop
  and status per target.

## Deferred

Pause/resume (the gateway has it; no UI yet), adaptive bit rate and a second connection
(`AddOptions`), multitrack stems, a WebSocket status message, per-target screen previews,
recording an input on its own, scheduled starts, uploads of finished recordings.

## Owner questions (answered with a recommendation)

1. **Codec / container default?** H.264 + AAC in fragmented MP4: every editor and platform
   takes it and a crash keeps the file. ARCHITECTURE.md prefers royalty-free AV1 for
   recordings: offered as a choice where the GPU encodes it (the 680M doesn't).
2. **Where do stream keys live?** In `machine/streaming.json` on the box only, never sent to
   clients, never backed up or exported. Re-type them on a replacement box.
3. **Recordings folder?** `<data>/recordings` by default, a setting for another disk.
