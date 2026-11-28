# Vesper

Vesper is an open-source presentation system for churches: lyrics, scripture, media and
announcements on projectors, stage monitors, lobby screens and the network.

- **Vesper** (`crates/vesper-server`, binary `vesper`) is the headless render and playout server.
  It holds the show state, renders every screen on the GPU (wgpu) and sends it to attached
  displays and to the network. It runs on a box with no keyboard, mouse or monitor.
- **Vesper Oriel** (`apps/control`) is the control app (Svelte + Tauri). It talks to the server
  only through the public API (REST + WebSocket, mDNS discovery), so a browser, a Bitfocus
  Companion module (`integrations/companion`) or a script can do the same.
- **Vesper Launcher** (`crates/vesper-launcher`, tray app in `apps/launcher-tray`) installs,
  updates (signed packages, with rollback), starts and monitors the Vesper products and their
  roles on each machine.

Also part of the server: presentations, playlists, themes and looks; stage displays with timers
and messages; video and audio playback (system FFmpeg, hardware decode first); recording and
streaming; a site-wide content store (Vesper Codex) with settings backups and encrypted exports;
signage channels shown on **Vesper Mosaic** display boxes; live captions; offline machine
translation for second-language slides; Bible scripture; and importers for ProPresenter 7 and
PowerPoint (`.pptx`) files so a church can move its existing library over.

The design is described in [`ARCHITECTURE.md`](ARCHITECTURE.md), the product family in
[`docs/PRODUCTS.md`](docs/PRODUCTS.md) and the control protocol in
[`crates/protocol/README.md`](crates/protocol/README.md).

## Status

Pre-release and under active development. Vesper has not been deployed at a church yet. APIs,
the control protocol and on-disk file formats may still change between versions.

## Platforms

- **Server:** Linux (Ubuntu LTS, x86_64) is the primary target; Windows is supported.
- **User interfaces** (Oriel, Launcher tray): Windows and Linux.

On Linux the server uses the system FFmpeg from `apt` (linked dynamically, never bundled) with
VA-API hardware decode. On Windows the server ships a shared, LGPL-only FFmpeg build next to
`vesper.exe`. Packages and installers are described in [`docs/PACKAGING.md`](docs/PACKAGING.md).

## Releases

Releases are published on this repository's GitHub Releases page, one release line per product,
told apart by the tag's prefix: `server-v<version>` (the Vesper server), `oriel-v<version>`
(Vesper Oriel), `launcher-v<version>` (the Vesper Launcher and its tray) and
`idn-node-v<version>` (the IDN Node the Launcher bundles). A normal release is the **stable**
channel; a GitHub pre-release is a **beta**. Every file is signed with the Vesper release key
in [minisign](https://jedisct1.github.io/minisign/)'s format (its `.minisig` beside it; the
public key is `packaging/keys/vesper-automation.pub`), and each release's `manifest.json` lists
its files with their platform, size and SHA-256. Check a download by hand with
`minisign -V -p packaging/keys/vesper-automation.pub -m <file>`. The Vesper Launcher and the
church's Vesper server find, check and install new releases themselves
([`docs/PACKAGING.md`](docs/PACKAGING.md), "Releases").

## Building and testing

You need a recent stable Rust toolchain, Node.js with npm (for Oriel), the FFmpeg development
libraries, and a Vulkan-capable GPU driver (or a software Vulkan driver such as lavapipe) for the
render tests. The ProPresenter 7 schemas the importer is built from aren't in the repository:
`scripts/fetch-propresenter-proto.sh` downloads them at a pinned commit and checks each file
(needs `curl`, `tar` and `sha256sum`; on Windows run it from Git Bash). Run it once after
cloning and after `cargo clean`; without them the build stops and names the script
([`crates/pro-import/PROTO.md`](crates/pro-import/PROTO.md)).

```sh
scripts/fetch-propresenter-proto.sh  # once after cloning: the ProPresenter 7 schemas pro-import builds from
export CARGO_INCREMENTAL=0          # incremental builds make target/ very large
cargo test --workspace              # GPU tests need access to the render node (the `render` group)
cargo clippy --workspace --all-targets -- -D warnings
UPDATE_BINDINGS=1 cargo test -p protocol --test bindings   # after changing protocol/model types
cd apps/control && npm run check && npx vitest run && npm run build
```

- The server's integration tests are one test binary (`crates/vesper-server/tests/server/`, one
  module per area). Run one area with `cargo test -p vesper-server --test server site::`.
- Unit tests live with their crate, e.g. `cargo test -p vesper-codex`.
- Format per crate with `cargo fmt -p <crate>`.

See [`CONTRIBUTING.md`](CONTRIBUTING.md) before opening a pull request.

## Repository layout

| Path | Contents |
|---|---|
| `crates/` | The Rust workspace: the server (`vesper-server`, split into `vesper-core` ← `vesper-site` ← `vesper-codex` ← `vesper-server`), data model (`model`), protocol types and TypeScript bindings (`protocol`), renderer (`render`), media decoding (`media`), audio bus (`audio`), outputs (`output`), importers (`pro-import`, `pptx-import`, `rtf`), scripture (`vesper-scripture`), translation (`vesper-translate`), the Launcher (`vesper-launcher`) and a soak-test harness (`vesper-soak`) |
| `apps/` | Desktop apps: Vesper Oriel (`control`) and the Launcher tray (`launcher-tray`) |
| `integrations/` | Third-party integrations: a Bitfocus Companion module (`companion`) |
| `packaging/` | Linux `.deb` and Windows installer scripts, systemd units, the public release-signing keys (`keys/`) |
| `docs/` | Plans and design notes per feature area, packaging and interoperability checklists |
| `scripts/` | Helper scripts (e.g. fetching the public-domain Bible translations) |

## IDN Open Protocol

Network video, audio, captions, discovery, the site and device management and several media
components come from **IDN Open Protocol (compatible with NDI®)**, a separate, open-source
(MIT OR Apache-2.0) Rust library developed independently of Vesper. It is compiled in, so there
is no runtime to install. Vesper uses it as a git dependency
(`github.com/ADC-Industries/idn-open-protocol`) pinned to a release tag: the same `tag = "…"` in
every crate's `Cargo.toml`, recorded in `Cargo.lock`. It is never vendored or modified here;
problems in the library are reported to its maintainers. Details:
[`crates/output/README.md`](crates/output/README.md).

OMT (Open Media Transport) is supported too, through the library's `idn-omt` crate.

## Licence

Vesper is licensed under either of

- Apache License, Version 2.0 ([`LICENSE-APACHE`](LICENSE-APACHE)), or
- MIT license ([`LICENSE-MIT`](LICENSE-MIT))

at your option. Third-party components in this repository keep their own licences; they are
listed in [`COPYRIGHT`](COPYRIGHT). Unless you explicitly state otherwise, any contribution
intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license,
shall be dual licensed as above, without any additional terms or conditions.

## Trademarks

NDI® is a registered trademark of Vizrt NDI AB. ProPresenter is a trademark of Renewed Vision.
Neither Vesper nor IDN Open Protocol is affiliated with or endorsed by Vizrt, and Vesper is not
affiliated with or endorsed by Renewed Vision. Network video is described only as "compatible with NDI®". Vesper's importer reads ProPresenter 7
files so a church can migrate its existing library; interoperability comes from reading those
file formats, never from decompiling ProPresenter or copying its code or assets.
