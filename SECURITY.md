# Security policy

## Reporting a vulnerability

Please report security problems **privately**, not in a public issue or pull request:
use GitHub's private vulnerability reporting on this repository (the **Security** tab ▸
**Report a vulnerability**, which opens a draft security advisory only the maintainers can see).

Include what you can of:

- the product and version (or commit) affected, and the platform (Linux or Windows);
- the steps to reproduce, what happened and what you expected;
- the impact as you see it (who can do what, from where on the network).

We will acknowledge the report, keep you informed while we work on a fix, and credit you in the
advisory unless you prefer otherwise. Vesper is pre-release software; there are no supported
release branches yet, so fixes go into the main branch and the next release.

## Scope

In scope:

- **Vesper**, the server (`crates/vesper-server` and the crates it is built from): its REST and
  WebSocket API, pairing and site API keys, the site's device and user management, the Codex
  (content store, uploads, backups and encrypted exports), media handling and the importers.
- **Vesper Oriel**, the control app (`apps/control`), including its Tauri shell.
- **Vesper Launcher** (`crates/vesper-launcher`, `apps/launcher-tray`): its local API, package
  verification, installation, updates and rollback.
- **Packaging** (`packaging/`): the Linux `.deb` packages and systemd units, the Windows
  installers and startup tasks, and the update-package signing described below.
- The Bitfocus Companion module (`integrations/companion`).

Out of scope here: problems in **IDN Open Protocol** itself (network video, discovery, the Site
Manager, its media and file channels) belong to that project; report them to its maintainers at
`github.com/ADC-Industries/idn-open-protocol`. If you are not sure which side a problem is on,
report it here and we will pass it on. Problems in other third-party dependencies (FFmpeg, wgpu,
Tauri, …) should go to those projects.

## How update packages are signed

The Launcher installs only packages with a valid signature:

- Signatures are Ed25519 in [minisign](https://jedisct1.github.io/minisign/)'s format, over the
  package's BLAKE2b-512 hash. The signed *trusted comment* carries the package's product, version
  and platform, so a renamed or relabelled package fails verification.
- The public keys are in [`packaging/keys/`](packaging/keys/) and are compiled into the Launcher
  (`vesper_launcher::sign::RELEASE_KEYS`): an automation key that signs every release, and a
  maintainer key, kept offline, that signs only a release replacing the automation key. Private
  keys are never in this repository.
- Administrators can add keys in an admin-only folder (`/etc/vesper/update-keys/*.pub` on Linux,
  `%ProgramData%\Vesper\update-keys\*.pub` on Windows).
- The Vesper server checks uploaded packages with the same keys and refuses a bad one, but the
  Launcher is the gate: it verifies every package again before installing it.

Details: [`docs/LAUNCHER-PLAN.md`](docs/LAUNCHER-PLAN.md) and `crates/vesper-launcher/src/sign.rs`.
