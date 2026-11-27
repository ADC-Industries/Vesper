# Vesper module for Bitfocus Companion

Drives a Vesper server from a Stream Deck (or any surface Companion supports) over Vesper's own
API: commands with `POST /api/v1/commands`, the show state from the WebSocket. What it offers is
listed in [`companion/HELP.md`](companion/HELP.md) (Companion shows that page too).

Older button profiles made for the presentation remote REST API can also talk to Vesper
directly: switch on **Remote API compatibility** in Oriel ▸ Settings ▸ Network & IDN (see
`docs/CONTROL-SURFACES-PLAN.md`). This module is the better choice for new setups: it has
feedbacks, variables and every Vesper command.

## Requirements

- Companion 4.1 or later (the module uses `@companion-module/base` 1.13.6, whose secrets store
  keeps the key, and the `node22` runtime).
- Node.js 22 and npm on the computer you prepare it on (only for `npm ci`).
- A Vesper server; its API port is 5555 unless it was started with another `--listen`.

## Install

1. Copy this folder (`integrations/companion/`) to a folder for Companion's own modules, e.g.
   `C:\CompanionModules\vesper-server` or `~/companion-modules/vesper-server`.
2. In that copy run `npm ci --omit=dev` (it installs the two pinned packages listed in
   `package-lock.json`).
3. In the Companion launcher open the settings (the cog), set **Developer modules path** to
   the parent folder (`C:\CompanionModules`), and restart Companion.
4. In Companion ▸ Connections add **Vesper**, enter the server's address and port.
5. Pairing:
   - Companion on the **Vesper machine itself** (address `127.0.0.1`): nothing to do.
   - From **another computer**: the connection asks Vesper to pair. Approve "Companion" in
     Oriel ▸ Site ▸ Join requests. Or type the pairing code from the server's log into
     **Pairing code** and save. The module keeps the site key in Companion's secrets store and
     clears the code. Revoke it in Oriel ▸ Site ▸ Security ▸ Paired computers.
6. Drag presets from the connection's **Presets** tab (next / previous, clear, looks, timers,
   macros) or build buttons with its actions, feedbacks and `$(vesper:…)` variables.

Dropdowns (looks, timers, messages, props, macros, playlists, presentations) follow Vesper: an
edit in Oriel updates them within a second.

## Develop

```sh
npm ci
npm test          # node --test against a mocked Vesper (REST + WebSocket)
```

`src/client.js` is the API client (no Companion types), `src/actions.js`, `feedbacks.js`,
`variables.js` and `presets.js` build the definitions from the lists, `src/main.js` is the
Companion instance. `test/mock-vesper.js` mocks the routes the module calls, with made-up
content. Package versions are exact; `node_modules/` is never committed.

Before publishing it to Companion's module list, fill in `repository`, `bugs` and `maintainers`
in `companion/manifest.json`.
