# Contributing to Vesper

Thank you for helping. Vesper is pre-release software under active development, so please open an
issue to discuss larger changes before you start on them. Security problems go through
[`SECURITY.md`](SECURITY.md), not public issues.

## Building and testing

```sh
scripts/fetch-propresenter-proto.sh  # once after cloning: the ProPresenter 7 schemas pro-import builds from
export CARGO_INCREMENTAL=0          # incremental builds make target/ very large
cargo test --workspace              # GPU tests need access to the render node (the `render` group)
cargo clippy --workspace --all-targets -- -D warnings
UPDATE_BINDINGS=1 cargo test -p protocol --test bindings   # after changing protocol/model types
cd apps/control && npm run check && npx vitest run && npm run build
```

Before you open a pull request:

- run clippy (no warnings) and the tests for what you changed, plus the frontend checks if you
  touched Oriel;
- format with **`cargo fmt -p <crate>`** for each crate you changed. Don't run `cargo fmt --all`
  or other workspace-wide commands that write into dependencies, such as `cargo fix`;
- after changing `protocol` or `model` types, regenerate the TypeScript bindings (above) and
  commit them;
- for UI or rendering changes, look at the result and include screenshots in the pull request.

Server integration tests are one test binary (`crates/vesper-server/tests/server/`, one module per
area): add new server tests as a module there rather than a new `tests/*.rs` file.

## Rules

- **Synthetic test data only.** Tests, fixtures, screenshots and docs must never contain a real
  church's data: song titles, lyrics, names, IP addresses, paths or screenshots from a real
  library. Build test data in the test, or invent it.
- **Product naming.** Never use "ProPresenter" in product, binary, type or UI names. Vesper reads
  ProPresenter 7 files for migration; that is the only place the name belongs. Network video is
  described as "IDN Open Protocol (compatible with NDI®)".
- **IDN Open Protocol** is an external dependency, pinned to a release tag. Don't patch, fork or
  work around library problems in this repository: report them to the IDN maintainers (steps,
  observed, expected). Bugs in Vesper's own code around the library are fixed here as usual.
- Don't commit `vendor/`, `reference/`, `data/`, `target/` or build output.

## Licence

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
the work by you, as defined in the Apache-2.0 license, shall be dual licensed under MIT OR
Apache-2.0, without any additional terms or conditions.
