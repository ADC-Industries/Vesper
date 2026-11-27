# CLAUDE.md

**Vesper** is the product family of an open-source church presentation system.
The headless Rust render/playout server is **Vesper** (crate `crates/vesper-server`, package
`vesper-server`, binary `vesper`); the Svelte/Tauri control app is **Vesper Oriel** ("Oriel" for short,
in `apps/control`). Future products are named "Vesper <name>".
Start with `ARCHITECTURE.md`. The protocol is documented in `crates/protocol/README.md`.

Maintainers' machine-specific rules (not in the repository; ignored where the file doesn't exist):
@~/.claude/vesper-maintainer.md

## Hard rules

1. **IDN Open Protocol is an external dependency.** The `idn-open-protocol` crates come from
   `github.com/ADC-Industries/idn-open-protocol` (its public repository) as a git dependency pinned
   to a release tag (the same
   `tag = "…"` in every crate's `Cargo.toml`, recorded in `Cargo.lock`). Move to a new release only on
   the maintainer's word: change every tag, then `cargo update -p idn-open-protocol`.
   - Use only its README files, public API (signatures and doc comments) and examples. Never patch,
     fork or work around a library problem in this repo: report it to the IDN maintainers as a
     black-box report (steps / observed / expected) and leave failing IDN tests failing until a fixed
     release arrives. Bugs in our own adapter code around a library that behaves as documented are
     ours to fix.
   - **`cargo fmt -p <crate>` only. Never `cargo fmt --all`**, and avoid other workspace-wide commands
     that write into dependencies, such as `cargo fix`.
2. **Test data is synthetic.** Never commit real church data: song titles, lyrics, names, IPs,
   paths or screenshots from a real library. Tests build their data themselves.
3. **Never use "ProPresenter" in product, binary, type or UI names.** Network video is described as
   "IDN Open Protocol (compatible with NDI®)".
4. **Never commit `vendor/`, `reference/` (third-party UI screenshots), `data/` or `target/`.**

## Building and testing

```sh
scripts/fetch-propresenter-proto.sh # once per checkout (and after `cargo clean`): pro-import's schemas
export CARGO_INCREMENTAL=0          # incremental builds make target/ very large
cargo test --workspace              # GPU tests need access to the render node (the `render` group)
cargo clippy --workspace --all-targets -- -D warnings
UPDATE_BINDINGS=1 cargo test -p protocol --test bindings   # after changing protocol/model types
cd apps/control && npm run check && npx vitest run && npm run build
```

- Platforms: every UI runs on Windows and Linux. The server runs on Linux (Ubuntu LTS, primary) and
  Windows. On Linux, system FFmpeg comes from apt, dynamically linked, never bundled; VA-API decode.
  On Windows the server bundles a shared, LGPL-only FFmpeg build next to `vesper.exe`. Packages and
  installers: `docs/PACKAGING.md`.
- The server's integration tests are ONE test binary (`crates/vesper-server/tests/server/`, one
  module per area). Add new server tests as a module there, not as a new `tests/*.rs` file: every
  extra binary links the whole server again. Run one area with
  `cargo test -p vesper-server --test server site::`.
- The server is split into crates, `vesper-core` ← `vesper-site` ← `vesper-codex` ←
  `vesper-server` (see `ARCHITECTURE.md`); unit tests live with their crate
  (`cargo test -p vesper-codex`). The Codex reaches the server only through the
  `codex::Server` and `codex::Show` traits; its routes stay in `vesper-server/src/codex/`.
- Dev builds keep line-number debug info only (`[profile.dev]` in `Cargo.toml`); don't turn full
  debug info back on workspace-wide.
- Only one wgpu device per process (see `crates/render` docs: Vulkan loader race).
- Parallel builds in git worktrees each need their own target dir; a shared one makes their test
  binaries overwrite each other.

## Working style

- Verify every change yourself before committing: tests, clippy, frontend checks. For UI or rendering
  changes, LOOK at the screenshots.
