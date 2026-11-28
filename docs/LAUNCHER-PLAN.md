# Vesper Launcher: plan (G10)

Requirements: `docs/PRODUCTS.md` "Vesper Launcher". Packaging today: `docs/PACKAGING.md`.
Round 1 (this plan's first stage, 2026-11-25) builds the service, its API, the site's update
channel and Oriel's Site ▸ Updates. Round 2 (batch 14) adds per-machine Launcher keys (the join
round), the first-run wizard, the tray app and the Launcher in the Linux package. Round 3
(batch 15): the bundled IDN Node, roles that act, the diagnostics page, the Launcher's
self-update, the headless join and the rest of the packaging ([below](#round-3)).

## Architecture

One **Launcher service per machine** (`crates/vesper-launcher`, binary `vesper-launcher`), always
running, plus a **tray/window UI** (a small Tauri app, round 2) that only talks to the
service's local API. Servers have no tray: the same service reports to the site, and Oriel's
**Site ▸ Updates** shows every machine (the "headless mode" of `PRODUCTS.md`).

```
 owner ──signed package──▶ Oriel ──upload──▶ site's Vesper server (update channel)
                                                 ▲ report + actions   │ packages
                                                 │ (every 30 s)        ▼
                     tray UI ──local API──▶ Launcher service ──installs, health-checks──▶ products
```

- **Local API** on `127.0.0.1:5556` only (loopback; a request from another address or whose
  `Host` isn't a loopback name is refused, against DNS rebinding): status, products, update,
  roll back, logs. Same-machine programs are trusted, as with the Vesper server's
  `trust_localhost`, but not a web page open in a browser there: a request carrying `Origin`,
  `Referer` or a `Sec-Fetch-Site` other than `none` is refused, and a `POST` needs a JSON body
  (or the signed package's header), which a page can't send without a CORS preflight.
- **Reporting is a pull** from the Launcher: it `PUT`s its report (machine, roles, products,
  versions, health) to the site's server and gets back the actions queued for it (update to a
  version, roll back). No inbound port on the machine, nothing to traverse NAT, and an offline
  machine simply picks its actions up when it is back.
- It is its own crate and binary (not a module of the Vesper server) because it must outlive the
  products it restarts and replaces, and it runs on machines without a Vesper server (an Oriel
  control PC, later a Cantor mixer). It shares `protocol` types with the server.

## Update channel and signing

- **Where updates come from:** the site's Vesper server (the Site Manager box) is the channel,
  LAN-only. An admin uploads signed packages in Oriel (Site ▸ Updates), the server keeps them in
  `<data>/updates/`, lists them by product, platform and version, and serves the files to the
  site's Launchers. Since the GitHub releases (2026-11-27, [below](#releases-from-github)) the
  server also brings Vesper's signed releases from GitHub into the channel by itself, and a
  Launcher without a church server checks GitHub directly; an admin's upload still works the
  same (a site without internet, a USB stick).
- **Signing (recommended): Ed25519 in minisign's format.** The owner generates the key pair once
  with `minisign -G` on their own machine; **the private key never enters the repository or the
  build machine** (keep it offline, with a backup). Releases are signed with
  `minisign -S -m <package> -t "vesper-package product=vesper version=0.2.0 platform=linux-x86_64"`.
  The signature is over the package's BLAKE2b-512 hash (minisign's default prehashed mode), and
  the *trusted comment* carries the product, version and platform, so they are signed too: a
  renamed or relabelled package fails verification.
- **Public keys:** compiled into the Launcher (`vesper_launcher::sign::RELEASE_KEYS`, from
  `packaging/keys/`): the **automation key** A05D75D53CB6819A signs every release (the release
  automation on the build machine holds its private key, outside the repository; owner's
  decision 2026-11-26), and the **owner's key** FB82D64871794689 (generated on the owner's
  machine, kept offline) signs only a release that replaces the automation key. More keys are read
  from an admin-only folder
  (`/etc/vesper/update-keys/*.pub`, `%ProgramData%\Vesper\update-keys\*.pub`) that the installers
  fill. Key rotation: a release signed with the old key ships the new public key. The Vesper
  server checks uploads with the same keys (`--update-key`), so a bad package is refused at
  upload, but **the Launcher is the gate**: it verifies again before installing.

## Install, update and rollback

| | Linux (Ubuntu LTS) | Windows | Portable / dev / tests |
|---|---|---|---|
| Package | the existing `.deb` | the existing NSIS `setup.exe` | one executable |
| Launcher runs as | root, `vesper-launcher.service` | SYSTEM, scheduled task "Vesper Launcher" | the user |
| Install | `apt-get install ./<pkg>.deb` (dependencies from the repositories; postinst restarts the enabled unit) | `setup.exe /S` (options kept from the registry) | versioned folder, Launcher-supervised process |
| Restart on crash | systemd (`Restart=`), counted via `NRestarts` | the task's one-minute trigger | the Launcher, back-off 1 s → 60 s |
| Kept | `/var/lib/vesper-launcher/packages/<product>/<version>/` | `%ProgramData%\Vesper Launcher\packages\…` | the same, under its state folder |

An update: verify signature and trusted comment (product, platform) → store → install → wait for
**health** (the product's `GET /api/v1/server` answers 200 with the new version within 60 s) →
done, and older packages beyond the previous one are pruned. If health fails, the Launcher
**reinstalls the previous version automatically** (apt with `--allow-downgrades`, the previous setup.exe,
the previous folder) and waits for it to be healthy again; the outcome (`installed`,
`rolledBack` with the reason, `failed`) is reported. "Roll back" by hand does the same with the
kept previous version (for the Launcher itself, through its self-update helper, as an update). The update in progress (`from`, `to`) is saved in the manifest before the
installer runs and cleared with the outcome: a Launcher stopped in between (a restart, a power
cut during the health wait) finds it at its next start, checks the new version again and keeps
it or puts the one before back, so the manifest never names a version that isn't installed.
A `.deb` install the stopped Launcher started may still be running then (`KillMode=process`):
the Launcher waits for dpkg's lock (any install it runs does, up to 15 minutes) and only
checks a new version dpkg already installed instead of installing it again. A product
installed outside the Launcher (apt, the Windows installer) is adopted: dpkg's installed
version for a `.deb`, else the version its health check reports (also over a version the
Launcher installed; never one differing only by a packaging revision, `0.2.0` = `0.2.0-1`); an
update to the same or an
older version is refused, and its package is downloaded from the channel (or found in apt's
cache) so a failed first update can go back to it; without one, the outcome says so. A
product that was running and goes down shows `starting` for the health timeout (systemd or the
Launcher restarting it), then `failed`. Data folders are never touched; a release that migrates data must keep
reading its previous format for one release (the server's existing rule for documents).

## Roles

A machine's roles are which Vesper modules and products run there: **presentation server**
(`vesper.service`; or `vesper-display.service` when the server drives screens on its own
monitors), **display box** (Mosaic, owner's option A, `docs/MOSAIC-PLAN.md`: the box runs the
**IDN Node with its display service** (`apps.display` on), managed by the site's Vesper server;
**no Vesper server** is installed there), **Site Manager + Codex** (modules of the server, on the
site's server), and later **mixer** (Cantor), **switcher**, **intercom station**. Round 1 reports
the configured roles; round 2's wizard records the roles chosen; round 3 acts on them
([Round 3](#round-3)).

## Health

Each report carries: products with version, previous version, state (running, stopped,
starting, updating, rolling back, failed), restarts, the last update's outcome; and the machine's
CPUs and load, memory, disk of the data folder, GPUs (Linux: `/sys/class/drm`). Oriel shows
machines that stopped reporting for 90 s as offline.

## The join round: per-machine Launcher keys (round 2)

A Launcher joins the site by **pairing with the site's server**, the same pairing Oriel uses
(`POST /api/v1/pair`, with `launcher: <machineId>`): with the server's pairing code it pairs at
once; without one the pairing waits for an admin's approval in Oriel ▸ Site ▸ Pairings, where it
is named `Vesper Launcher – <machine name>`. What it gets is not a client's key but **its
machine's Launcher key** (`crates/vesper-site/src/launchers.rs`):

- **From IDN:** a named site API key with its own audience, `app:vesper-launcher` (IDN's
  per-program audience, `app:<name>`), scope `control`, made by the Site Manager (in its audit)
  and revocable in Site ▸ API keys like any key. The server's other routes take only
  `app:vesper` keys, so a Launcher key controls nothing there.
- **Here:** IDN has no binding of a key to the machine that holds it (a token's `aud.machine` is
  the machine it may be *used on*), and a Launcher isn't an IDN device (it may run where no
  Vesper server is, an Oriel PC), so the site's server binds **key ID → machine ID** when it
  makes the key (`site.json`, `launcherKeys`). A key reaches only the package list and files
  and `PUT /updates/launchers/{its machine}`: a report as another machine is `403`, so it can
  neither report as another machine nor take another machine's queued actions. A machine that
  joins again gets a new key and its key from before is revoked; a revoked or deleted key's
  binding is forgotten at the hourly sweep, and a key still unused a day after it was made is
  revoked, as an Oriel pairing's is.
- The round-1 stopgap (reports only from the server machine or an admin's key) stays for
  machines without their own key; admins' keys and the server machine keep working.
- A Launcher pairs at the site's server only (the update channel is there); at another
  machine of the site the pairing is refused with the site's server's address. A damaged
  `site.json` loses the bindings: those machines join again.

The Launcher keeps the site and its key in `site.json` in its state folder (mode 0600) and
reports there from then on; the configuration file's `channel` is used until it joins.

## First-run wizard and the tray app (round 2)

- **The wizard** is part of the Launcher's own page, served by its local API at
  `http://127.0.0.1:5556/` (one HTML file, Oriel's design tokens, no build step). Three steps:
  **role** (presentation server; display box, Mosaic: the IDN Node's display service, recorded
  as a role flag only in round 2; mixer, switcher,
  intercom listed as "later"), **join the site** (the Vesper servers found on the network over
  mDNS, or a name or address; the pairing code, optional), **wait for approval** (until an admin
  approves, or "Cancel"); then "Done". "Not now" skips joining. The choices are kept in
  `setup.json`; the window opens the wizard until it was finished.
- **The window** shows the machine (name, roles), the site (reporting, waiting, or "Join the
  site"), and a card per product: state (running, stopped, starting, updating, failed), version,
  "Update available" with **Update to <version>** (the newest one the site's channel has for
  this machine), **Launch**, **Stop** (a product stopped by hand shows "Stopped", not failed:
  its unit is stopped, or its scheduled task ended and disabled), and **Logs**.
- **Browser rules of the local API, round 2:** the page is same-origin, so its requests (an
  `Origin` or `Referer` of the API's own address, `Sec-Fetch-Site: same-origin`) are answered;
  anything from another origin is still refused, a change still needs a JSON body, the page
  can't be framed.
- **The tray app** (`apps/launcher-tray`, Tauri): a tray icon in the status colour (green all
  good, amber working, red attention: a product failed, the channel can't be reached, the
  join failed, or setting up the machine's role failed; grey the service isn't answering), "Open" and "Quit", and a 440×640 window that
  shows the Launcher's page. It talks only to the local service (`GET
  /launcher/v1/overview` every 2 s, as a program on the machine) and opens the window by itself
  on the first run. Servers have no tray: `curl` against the local API does the same
  (`docs/PACKAGING.md`).

## Round 3

Done in batch 15 (owner: **the IDN Node is bundled with the Launcher**).

- **Bundled IDN Node.** `packaging/idn-node/build.sh linux|windows` builds `idn-node`,
  `idn-session`, `idn` and `idn-display` at the IDN tag Vesper pins (read from
  `crates/audio/Cargo.toml`, checked against `Cargo.lock`'s commit) from a fresh shallow clone of
  the public repository in `packaging/downloads/idn-src-<tag>/`, in its own target folder, two cargo
  runs (IDN's build instructions, so IDN's own `[patch.crates-io]` and `.cargo/config.toml`
  apply). Licences: IDN's MIT/Apache texts, a notice, and a per-crate list from `cargo metadata`
  (`THIRD-PARTY-CRATES.txt`; cargo-about/cargo-deny aren't installed, so the full texts of the
  non-MIT/Apache crates aren't gathered yet). Linux: its own package, `vesper-idn-node`
  (`/usr/lib/vesper-idn-node/`), recommended by `vesper-launcher`; Windows: `idn-node\` in the
  Launcher's installer. Its own package rather than inside the Launcher's: the Launcher's update
  stays small, the Node's version follows IDN's releases, and a machine that never needs the Node
  (an Oriel PC) can leave it out.
- **Roles act** (`crates/vesper-launcher/src/roles.rs`). Only roles chosen in the wizard (or
  `PUT /launcher/v1/setup`, `vesper-launcher join --role`) are acted on, never the
  configuration's default, so an admin's own unit setup is left alone until someone picks a
  role. **Presentation server:** the server's unit enabled and started (`vesper.service`, or
  `vesper-display.service` when that one is enabled; Windows: the "Vesper" task). **Display box:**
  the server stopped and disabled; `idn-node install`; a join code the site's server makes for
  this machine (`POST /api/v1/updates/launchers/{machineId}/idn-join`, with the machine's own
  Launcher key; the server runs `SiteCommand::JoinCode` as the site's admin, in the Site
  Manager's audit as `Vesper Launcher – <machine>`; one waiting code per machine) and `idn-node
  set server=<site's server>`, `idn-node join CODE`, so the volunteer types the server's pairing
  code once; then `idn-node set apps-display=on` and `apps-display-boot=on` on Linux (the
  headless KMS display service from boot, no desktop: it needs both, as Mosaic sets them) or
  `apps-display=on` on Windows, with a note that the PC must sign in
  automatically. Taken away, a role is undone: display service off (both settings off on
  Linux), `idn-node uninstall` when the Launcher installed it (the Node's state stays), the
  server's unit disabled. Each step is
  recorded in `roles.json` as it succeeds; a run that failed (no site yet, the server
  unreachable) is tried again a minute later, and at once when the machine joins the site. A
  role leaves `applied` as soon as its undo begins, and a Node service the Launcher installed is
  removed whenever the display role isn't wanted, so a run that stopped partway is set right when
  the roles change back. A failed `idn-node join` never shows its code (`join ******`). The
  window shows the steps.
- **Updates managed by Vesper** (lane I, owner decision (a) 2026-11-26; IDN Beta14). Vesper
  owns IDN Node updates where its Node is installed: the IDN Node is updated with Vesper's
  packages through Vesper's update channel, never by the site's Site Manager. Linux (bug run 7,
  H04-2 / R3-2): `vesper-idn-node` is one of the Launcher's products
  (`launcher.d/vesper-idn-node.json`, shipped by `vesper-launcher`), a *package of programs*
  (`"healthUrl": ""`: up while dpkg has it installed, healthy once dpkg has the new version, no
  Launch or Stop, kind `package` in the window), so a newer signed `vesper-idn-node` package in
  Site ▸ Updates is installed by the Launcher like the server's (the smoke test does it through
  the local API). Windows: the Node is part of the Launcher's installer, updated with the
  Launcher's self-update. The package ships the marker `idn-updates-managed-by` (first line `Vesper`) beside
  `idn-node` in `/usr/lib/vesper-idn-node/` (a package file: removed with the package; Windows:
  the Launcher's installer puts it in `idn-node\`, its uninstaller removes the folder). The Node
  reports `<update managed_by="Vesper" managed_by_source="package"/>`; the Site Manager leaves the
  machine out of `POST update`, and the machine refuses an offer or a rollback. Oriel shows
  "Updates managed by Vesper" in Site ▸ Devices (under the IDN version) and on a display box's
  panel in Site ▸ Displays (`SiteDevice.updateManagedBy`, `MosaicBox.updateManagedBy`). At every
  check (`idn_node.rs`) the Launcher puts the marker back when it is missing, names another
  product or isn't text, with a warning in its log and on the diagnostics page ("IDN Node" row:
  how this Node is updated, flagged while the marker can't be written or the Node's service runs
  another program as of the last check, gone when there is no bundled Node). **Guard**:
  when the Launcher installed the system's IDN Node service (the display box role) and the
  service runs another program than the bundled `idn-node` (another IDN install replaced it), the
  Launcher runs the bundled `idn-node install` again (at most every 10 minutes, never during a
  role run) and warns: remove the other IDN Node package (Windows: the other IDN Media install)
  from the display box. Linux reads the program from the service's unit
  (`/etc/systemd/system/idn-node.service`, its `ExecStart`); Windows from the service manager
  (the service `idn-node`'s image path through `QueryServiceConfigW`, so a path in any language
  reads right; quoted or not), compared as Windows compares paths (any case, either slash).
  `idn-node.exe install` points an existing service at this copy and restarts it (IDN's README),
  the same step the display box role runs. The parsing and the decision are pure functions with
  unit tests (`idn_node.rs`); the Windows reading is cross-clippied only, not run on a real PC.
- **Diagnostics** (`GET /launcher/v1/diagnostics`, the window's footer and the tray's
  "Diagnostics"): the bundled `idn doctor --seconds 3` parsed into its sections (each with a
  sentence on what it checks) and marks, a headline in plain words, the Launcher's and the
  products' status, the role run, the site and the disk, and "Copy report" (keys, tokens,
  passphrases and long key-like strings redacted).
- **Self-update.** The Launcher is a product of its own (`"self": true`, health
  `GET /launcher/v1/version`). Its update goes through a helper (`selfupdate.rs`): a copy of the
  running Launcher started outside its service (Linux: `systemd-run`; Windows: the one-time task
  "Vesper Launcher Update" as SYSTEM) that installs the package, restarts the Launcher
  (`systemctl restart`; Windows: the installer ends and starts the task), waits for the new
  version's health, and otherwise installs the kept package again. The Launcher that starts
  finds its update under way in the manifest and records the helper's outcome
  (`self-update/result.json`). The package's postinst still never restarts the Launcher itself.
- **Packaging.** Linux: `vesper` (server, depends on `vesper-launcher`), `vesper-launcher` (alone:
  a display box or a control PC), `vesper-launcher-tray` (autostart at log-in,
  `/etc/xdg/autostart`), `vesper-idn-node`. Products come from `/etc/vesper/launcher.json` (the
  Launcher itself) and `/etc/vesper/launcher.d/*.json` (the server's package adds
  `vesper.json`). Windows: `vesper-launcher-<version>-setup.exe` (`packaging/windows/
  build-launcher.sh`): the Launcher, the tray (HKLM `Run`), the IDN programs (firewall rules), the
  task "Vesper Launcher" (SYSTEM, at boot, every minute); the server's installer adds
  `launcher.d\vesper.json`.
- **Headless join:** `vesper-launcher join [CODE] [--server HOST] [--role display|presentation]
  [--no-wait]`, through the running service's local API; without `--server`, the only Vesper
  server found on the network.

## Releases from GitHub

Owner decisions 2026-11-27: releases live on `github.com/ADC-Industries/Vesper`, one release line
per product told apart by the tag prefix (`server-v`, `oriel-v`, `launcher-v`, `idn-node-v`;
stable = a normal release, beta = a GitHub pre-release), every asset signed with the automation
key. `scripts/release.sh` makes them (`docs/PACKAGING.md`, "Releases"). The shared checker is
`crates/vesper-releases` (`github`; `sign` and `package` moved there from the Launcher, which
re-exports them): the unauthenticated GitHub API with the last list's ETag, a rate limit
remembered until it ends, "offline" told apart, the channel and tag-prefix filter, the
release's `manifest.json` to find a product's package for a platform (only a hint: nothing in it
is trusted), and the download of the package and its `.minisig` into a staging folder with size
limits, verified with the trusted keys; the trusted comment must name the product, the
release's version (`0.3.8~beta18` is `idn-node-v0.3.8-beta18`'s) and the platform, or the
package is refused and removed.

- **The church server** (`crates/vesper-server/src/releases.rs`) checks every 6 hours by
  default (failures back off from 5 minutes, at most to the interval), for each product and
  platform the site has (its Launchers' reports, the server itself, what "Add app" asked for):
  the newest release in the channel the channel doesn't have yet comes in as if an admin
  uploaded it, with "from GitHub" as its source; the two newest per product and platform are
  kept. Oriel's Site ▸ Updates shows the last check, the error, the next check, what came in,
  and the settings (on/off, stable/beta, how often, the repository; admins). Installing stays an
  admin's action (or the install window), with the Launcher's rules and rollback. Errors are
  logged once per kind, not at every try.
- **The Launcher directly** (`crates/vesper-launcher/src/github.rs`): with no church server (not
  joined, no channel configured, or no report reached the site's server for a day,
  `github.directAfterSecs`), it checks GitHub itself every 6 hours for its own products; a newer
  version shows as "Update available" in its window, and the update downloads and verifies the
  package exactly as a channel package would be, then installs with the same rules. An update
  or install the channel can't serve (no channel, the channel doesn't have it, the server
  can't be reached) is fetched from GitHub when checking is on; a package refused for its
  signature is never fetched elsewhere. Configuration: `"github": {"enabled", "repository",
  "channel", "intervalHours", "api", "directAfterSecs"}` in `launcher.json` (all optional).
- **"Add app".** The catalogue (`GET /api/v1/updates/catalogue`, the Launcher's `GET
  /launcher/v1/catalogue`): per product the newest stable version, a newer beta, the platforms,
  and where it comes from (the site's channel, GitHub). A machine is offered only what it can
  run and doesn't have: a package for its platform, not the Launcher or its tray (they come with
  the Launcher), no Vesper server on a display box, and what the Launcher knows how to install
  there (`github::template`: the server as `.deb` or NSIS, the bundled IDN Node and Oriel as
  `.deb` packages of programs; Oriel's Windows installer now installs per machine, but the
  Launcher can't yet tell which version of a Windows program without a health check is
  installed, so it doesn't offer Oriel on Windows: it is installed by hand there). In the Launcher's window: Add app → pick → it
  downloads (the church server if it has the package, else GitHub), verifies, installs, and
  manages the product from then on (`added-products.json` in its state folder; a product whose
  first install failed is forgotten again). In Oriel: "Add app to <machine>…" queues an
  `install` action for the machine's Launcher; a version only GitHub has is brought into the
  channel at once.

Packages are installed with apt (`apt-get install ./<pkg>.deb`, non-interactive, never removing
a package), so adding the Vesper server to a machine without FFmpeg's libraries brings them from
the repositories; offline, it fails with apt's reason (trimmed to its errors and the unmet
dependency) and the previous version is put back. Not covered here: the first real release (the owner decides when) and a real download from github.com.

## Deferred (said so in Oriel where it matters)

Windows install tested on a real PC (the Launcher's installer, its self-update through the
scheduled helper task, a Windows display box with auto sign-in); `dxcompiler.dll` next to the
Windows `idn-display` (without it wgpu falls back to FXC); the full licence texts of the
non-MIT/Apache crates in the IDN programs (cargo-about); a real self-update of an installed
`vesper-launcher` `.deb` (tested with a fake dpkg and systemd); a display box joined to a real
site end to end (tested with fakes for the IDN Node and the site, and the server's join code
against the real Site Manager); a real upgrade of an installed `.deb`
(rounds 1 and 2 test the update and rollback logic with a fake product); the tray app run on a
real desktop (round 2 builds it and screenshots its page, not its tray icon).

## Owner questions answered (recommendations)

1. **Signing keys:** Ed25519 / minisign format; owner generates and holds the private key
   offline; public key compiled in and installed with the installer.
2. **Where updates come from:** the site's Vesper server, fed by an admin uploading signed
   packages in Oriel. LAN-only.
3. **New crate/binary:** yes, `vesper-launcher` (service + local API), plus a Tauri tray UI in
   `apps/launcher-tray` (round 2); servers use Oriel's Site ▸ Updates instead.
4. **Per-machine credentials (round 2):** IDN's per-program audience (`app:vesper-launcher`)
   plus a binding of key ID → machine ID at the site's server, made by the existing pairing
   (code or approval). Not an IDN enrollment of the Launcher: that would put a second IDN
   identity on every Vesper box and the whole IDN library into the Launcher, and IDN has no
   way to bind a key to its holder anyway.
5. **The wizard in the Launcher's own page** (not a separate UI in the tray): one UI, the
   same in the tray window and in a browser on the machine; the tray stays a thin shell.
