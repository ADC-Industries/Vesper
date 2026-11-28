# Packaging

How to build Vesper's packages, what each one installs, and how to test the Windows builds.
Everything is built on Linux (Ubuntu LTS, amd64); the Windows packages are cross-built.

| Package | Built by | Output (in `packaging/out/`) |
|---|---|---|
| Vesper server, Launcher, tray and bundled IDN Node, Linux | `packaging/linux/build-deb.sh` | `vesper_<version>-2_amd64.deb`, `vesper-launcher_<version>-2_amd64.deb`, `vesper-launcher-tray_<version>-2_amd64.deb`, `vesper-idn-node_<idn version>-1_amd64.deb` |
| Vesper server, Windows | `packaging/windows/build-server.sh` | `vesper-server-<version>-setup.exe` |
| Vesper Launcher (with its tray and the IDN Node), Windows | `packaging/windows/build-launcher.sh` | `vesper-launcher-<version>-setup.exe` |
| The bundled IDN Node's programs | `packaging/idn-node/build.sh linux\|windows` | `idn-node-linux/`, `idn-node-windows/` (programs + licences) |
| Vesper Oriel, Linux | `packaging/oriel/build.sh linux` | `Vesper Oriel_<version>_amd64.deb`, `Vesper Oriel_<version>_amd64.AppImage` |
| Vesper Oriel, Windows | `packaging/oriel/build.sh windows` | `Vesper Oriel_<version>_x64-setup.exe` |

`packaging/out/` and `packaging/downloads/` are gitignored: never commit packages or downloads.
The scripts honour `CARGO_TARGET_DIR`. They are heavy builds; on a machine shared with other
builds, run one at a time. The ones that build the server first run
`scripts/fetch-propresenter-proto.sh`: the ProPresenter 7 schemas the importer is built from are
not in the repository; the script downloads them at a pinned commit, checks every file's SHA-256,
and puts them in `target/propresenter-proto/` (or `$PROPRESENTER_PROTO_DIR`), downloading nothing
when they are already there (`crates/pro-import/PROTO.md`).

## Vesper server for Linux (`.deb`)

```sh
sudo apt install build-essential pkg-config clang cmake dpkg-dev libasound2-dev \
  libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev \
  libvulkan-dev glslc
packaging/linux/build-deb.sh            # the schemas, cargo build --release, npm run build, the IDN Node, then the .debs
packaging/linux/build-deb.sh --no-build # package what is already built
packaging/linux/smoke-test.sh           # install them in throwaway containers (podman) and run them
```

The package is built for the Ubuntu release it is built on: its library dependencies come from
that machine (`dpkg-shlibdeps`), FFmpeg's sonames above all (`libavcodec62` = FFmpeg 8 on Ubuntu
26.04). Build it on each Ubuntu LTS you support. Ubuntu 24.04's FFmpeg 6 is older than the
server's FFmpeg bindings support, so 24.04 isn't one of them.

What it installs:

| Path | What |
|---|---|
| `/usr/bin/vesper` | the server |
| `/usr/share/vesper/ui/` | Oriel's web build, served to browsers at `http://<box>:5555/` |
| `/usr/share/vesper/scripture/` | the bundled Bible translations (`*.vbible`, [below](#bible-translations)); sources in `/usr/share/doc/vesper/SCRIPTURE-SOURCES.txt` |
| `/usr/lib/systemd/system/vesper.service` | the service (below), enabled and started on install |
| `/usr/lib/sysusers.d/vesper.conf` | the `vesper` system user, in the `render`, `video` and `audio` groups |
| `/etc/default/vesper` | extra options (`VESPER_ARGS`, `RUST_LOG`); a conffile, kept on upgrades |
| `/var/lib/vesper/` | the data folder (libraries, media, settings), the `vesper` user's home |
| `/var/cache/vesper/` | caches (Mesa's shader cache) |
| `/etc/vesper/launcher.d/vesper.json` | the server as one of the Launcher's products (a conffile) |

The server's package depends on **`vesper-launcher`** ([below](#vesper-launcher)), which the
Launcher-only machines install alone:

| Package | Path | What |
|---|---|---|
| `vesper-launcher` | `/usr/bin/vesper-launcher`, `/usr/lib/systemd/system/vesper-launcher.service` | the Launcher, enabled on install, started on every install and upgrade |
| | `/etc/vesper/launcher.json` | its configuration (itself as a product, for its self-update); a conffile; more products in `/etc/vesper/launcher.d/*.json` |
| | `/var/lib/vesper-launcher/` | its state: kept packages, its manifest, the site's Launcher key, `roles.json` |
| `vesper-launcher-tray` | `/usr/bin/vesper-launcher-tray`, `/etc/xdg/autostart/vesper-launcher-tray.desktop` | the tray icon and window, started at every log-in (desktops) |
| `vesper-idn-node` | `/usr/lib/vesper-idn-node/` (`idn-node`, `idn-session`, `idn`, `idn-display`, the marker `idn-updates-managed-by`) | the IDN Node and tools the Launcher bundles (display boxes, diagnostics); licences in `/usr/share/doc/vesper-idn-node/` |

**Dependencies.** `Depends` is what the binary links, found by `dpkg-shlibdeps` (`ldd` mapped to
packages, with minimum versions): FFmpeg's runtime libraries (`libavcodec`, `libavformat`,
`libavutil`, `libswscale`, `libswresample`, from Ubuntu's archive, never bundled), ALSA
(`libasound2t64`), libc and libgcc. Added by hand, because they are loaded at run time where
`ldd` can't see them: `libvulkan1` and `mesa-vulkan-drivers` (wgpu; Mesa's lavapipe is the
software fallback on a box without a GPU; since GPU captions the binary also links
`libvulkan.so.1` itself, for whisper's Vulkan backend, so `dpkg-shlibdeps` finds `libvulkan1` too), `mesa-va-drivers` (VA-API decode on AMD),
`avahi-daemon` (the box's `<host>.local` name; the server announces `_vesper._tcp` over mDNS
itself), `fonts-dejavu-core` (slide text needs at least one system font), and `systemd` /
`init-system-helpers` for the maintainer scripts. `Recommends`: `intel-media-va-driver` (VA-API
on Intel) and `fonts-liberation`. `Suggests`: `libreoffice-impress` (slide pictures for PowerPoint
imports, see below).

**The service** (`vesper.service`) is for network-only boxes: no display attached, every screen
out over IDN Open Protocol (compatible with NDI®). It runs
`vesper --data /var/lib/vesper --ui-dir /usr/share/vesper/ui $VESPER_ARGS` as `vesper`, restarts
after a crash, and stops cleanly on SIGTERM. Logs: `journalctl -u vesper` (the pairing code is in
it). Hardening: a read-only system (`ProtectSystem=strict`; only `/var/lib/vesper`,
`/var/cache/vesper` and a private `/tmp` are writable), no home folders, no capabilities, no new
privileges, kernel tunables/modules/logs/clock protected, `DevicePolicy=closed` with only the GPU
(`char-drm`) and the sound cards (`char-alsa`) allowed, address families limited to IP, netlink
and Unix sockets. Left off on purpose: `MemoryDenyWriteExecute` (the GPU drivers' shader
compilers generate code), `PrivateDevices` (it hides `/dev/dri` and `/dev/snd`),
`RestrictRealtime` (audio threads) and any network restriction (it is a network server: API on
5555/tcp, the site on 5959 and 29202, mDNS on 5353/udp, IDN's own ports).

**Upgrades and removal.** An upgrade restarts the service and keeps `/etc/default/vesper`, and an
admin's `systemctl disable vesper` stays in force. `apt remove` stops it; installing again
afterwards starts whichever unit was enabled (a display box keeps `vesper-display`). `apt purge` deletes the
cache but **keeps `/var/lib/vesper` and the `vesper` user**: that folder holds the church's
libraries and media, so it is deleted by hand once it is backed up.

**Display boxes** (screens on attached monitors or projectors, HDMI / DisplayPort) run
`vesper-display.service` instead of `vesper.service`: the same server, user, hardening and
`/etc/default/vesper`, ordered after the boot splash (`plymouth-quit-wait.service`) and in
conflict with `vesper.service`. The server drives the displays itself with kernel mode setting
(no desktop, compositor or auto-login user: see `docs/DISPLAY-OUTPUTS-PLAN.md`), so nothing else
is installed. The package ships the unit but doesn't enable it:

```sh
sudo systemctl disable --now vesper && sudo systemctl enable --now vesper-display
```

The box must not run a desktop session on the same graphics card (the server has to be the
card's DRM master); a display output says so when it can't drive its display. Upgrades restart
whichever of the two units is enabled. On a box with displays that should stay off, add
`--no-displays` to `VESPER_ARGS`. The server creates `portable/` in an empty data folder itself
(the installers still create it).

**Smoke test** (`packaging/linux/smoke-test.sh`, podman, never the host): in a clean container of
the build machine's Ubuntu release, `apt` installs the `.deb` with its dependencies, `vesper
--help` runs, the user, groups and data folder exist, `systemd-analyze verify` accepts the unit,
and nothing in `ldd` is missing; under the Launcher's package repacked as 0.1.0, dpkg refuses the
IDN Node's package and apt installs the two together. Then a container boots systemd (the
Launcher configured with GitHub checks off, so it makes no network calls of its own): the
service starts by itself (Mesa's software renderer, no GPU there), serves the UI and `GET
/api/v1/server` on 5555, the Launcher installs signed packages from its local API, and `apt
purge` keeps `/var/lib/vesper`.

## Bible translations

The server ships public-domain Bible translations so the Bible page works offline on every box.
`scripts/fetch-scripture.sh` downloads them, each **pinned by URL and SHA-256**, into
`target/scripture-download/` and converts them (`vesper-scripture-convert`, in
`crates/vesper-scripture`) into Vesper's translation format in `$CARGO_TARGET_DIR/scripture/`
(default `target/scripture/`), with `SOURCES.txt` recording each source and licence. Neither
folder is committed. Both package scripts run it when they build (or take
`VESPER_SCRIPTURE_DIR`), and refuse to package without at least one translation.

| ID | Translation | Source | Licence |
|---|---|---|---|
| KJV | King James Version | ebible.org `eng-kjv_vpl.zip` | public domain (in the UK, Crown patent) |
| ASV | American Standard Version (1901) | ebible.org `eng-asv_vpl.zip` | public domain |
| WEB | World English Bible | ebible.org `eng-web_vpl.zip` | public domain |
| BSB | Berean Standard Bible | bereanbible.com `bsb.txt` | public domain (dedicated, 2023) |

Only the 66 books of the Protestant canon are kept; supplied-word brackets and paragraph marks
are removed. A source that can't be reached, or whose file no longer matches its pin (ebible.org
refreshes its files now and then), is skipped with a warning and the others still ship: check
the new file and move its pin in the script. None of the four needs an attribution line; the
format carries one for translations that do (it is put on the last generated slide).

Where the server finds them: `--scripture-dir <dir>`, by default the `scripture` folder next to
the UI folder (`/usr/share/vesper/scripture`, `<install>\scripture` on Windows), else
`target/scripture` in a source checkout; plus the data folder's `scripture/` for a church's own
files (an ID there replaces a bundled one).

## Live captions: speech models

Live captions (`docs/CAPTIONS-PLAN.md`) run whisper.cpp in a worker process that is the server's
own binary (`vesper --idn-captions-worker`), **compiled in** (IDN's `idn-captions`, built from
source by `whisper-rs-sys` with CMake: no DLL). **GPU captions** (whisper on the GPU through
Vulkan, IDN's `gpu` feature):
- **Linux:** always built in (used only when Settings ▸ Captions ▸ Processor is GPU; the CPU is
  the default). Building needs `libvulkan-dev` and `glslc` (ggml compiles its Vulkan
  shaders at build time, into the binary); running needs only the Vulkan loader (`libvulkan1`) and
  a Vulkan driver (`mesa-vulkan-drivers` for AMD and Intel, NVIDIA's own driver), which the
  package already depends on for rendering. No new package dependency. A box without a Vulkan
  GPU runs whisper on the CPU (the device IDN's worker chooses is shown in Settings ▸ Captions).
- **Windows:** not built yet: the cross build would need the Vulkan SDK's headers and loader
  import library for MinGW and `glslc` on the build machine (feature `gpu-captions` of
  `vesper-server`); the Windows server runs whisper on the CPU, and Settings ▸ Captions says so
  when GPU is chosen. The speech **models are not packaged**: they are 75 MB
(`tiny.en`) to 1.6 GB (`large-v3-turbo`) and the right one depends on the box. The server fetches
the one it needs the first time captions start, from whisper.cpp's own files on Hugging Face at a
pinned commit, checked against its pinned size and SHA-256, into the data folder's
`cache/whisper/` (`/var/lib/vesper/cache/whisper/`, `%ProgramData%\Vesper\cache\whisper\`); the
cache is left out of backups. A box without internet takes the file (`ggml-<model>.bin`) copied
into that folder by hand; it is checked against the same pin before use.

## Machine translation: translation models

Machine translation of second-language slides (`docs/DUAL-LANGUAGE-PLAN.md` §5) runs OPUS-MT
models on `candle` inside the server, **compiled in** (pure Rust: no C++ build, no DLL, no new
package dependency on either platform). The **models are not packaged**: one per language pair
(300–470 MB each), fetched the first time a pair is used (or from Settings ▸ Translation) from
Helsinki-NLP's repositories on Hugging Face at a pinned commit, each file checked against its
pinned size and SHA-256, into the data folder's `cache/translate/<pair>/`
(`/var/lib/vesper/cache/translate/`, `%ProgramData%\Vesper\cache\translate\`); the cache is left
out of backups. A box without internet takes the pair's files copied into that folder by hand
together with its `ready.json` (as written by a box that downloaded it).

## PowerPoint import: slide pictures (optional LibreOffice)

PowerPoint imports (`.pptx`, and Keynote / Google Slides exported as `.pptx`; protocol README,
"PowerPoint import") bring in what Vesper can edit and draw the rest (charts, tables, SmartArt,
other shapes, lines, EMF/WMF pictures) from a **picture of the slide**. The pictures come from
**LibreOffice**, an **optional** program on the box that holds the Codex: never bundled, never a
`Depends`. Without it an import still works: those objects are approximated (a shape as a
rectangle with its text) or left out, the first slide may use the small thumbnail the file
carries, and the import report says so slide by slide.

- **Linux:** `sudo apt install libreoffice-impress` (the `.deb` lists it under `Suggests`; about
  300 MB with its dependencies). The server finds `soffice` on the `PATH`. It runs headless with
  a profile of its own in the server's temporary folder (the service's private `/tmp`), so it
  needs no home folder and doesn't touch a LibreOffice the operator has open; fonts the slides
  use should be installed on the box (`fonts-liberation`, `fonts-crosextra-carlito` for Calibri,
  `fonts-crosextra-caladea` for Cambria) or LibreOffice substitutes them.
- **Windows:** install LibreOffice (any 7.4 or later; the default install folder,
  `C:\Program Files\LibreOffice`). The server finds `program\soffice.exe` there; no PATH change.
- **Elsewhere / another copy:** `vesper --soffice <file>` or the environment variable
  `VESPER_SOFFICE`. `--no-slide-pictures` turns the pictures off.

LibreOffice 7.4 or later is needed for the picture size option (1920 px wide); one conversion
runs at a time, with a time limit (60 s plus 10 s per slide), killed when it runs over.
**Not verified on this machine** (no LibreOffice installed here): the server's tests stand a
script in for `soffice`; the owner's checklist below has a real-deck check.

## Vesper server for Windows

```sh
rustup target add x86_64-pc-windows-gnu
sudo apt install gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 clang cmake nsis
packaging/windows/build-server.sh      # fetches FFmpeg and the schemas, cross-builds, packs the installer
```

**Toolchain: `x86_64-pc-windows-gnu`** (MinGW-w64), not `-msvc` through `cargo-xwin`. The GNU
target builds with Ubuntu's own packages, needs no Microsoft SDK download (whose licence someone
would have to accept), and is the target IDN documents for its own Windows cross-builds. The C
and C++ runtimes are linked statically, so only Windows' own DLLs and FFmpeg's are needed; the
build script checks `vesper.exe`'s imports for anything else. OMT (`idn-omt`, batch 13) adds
no runtime file on either platform: its VMX codec (libvmx, MIT, vendored in the IDN release) is
C++ compiled by the `cc` crate with the same MinGW `g++` (Linux: `build-essential`) and linked
into the binary. The FFmpeg bindings are generated
by clang for the MinGW target, with clang's AMX intrinsics headers skipped (bindgen can't lay
out their tile type, and FFmpeg doesn't use it).

Live captions' whisper.cpp is built by CMake with the same MinGW compilers and linked in
statically (no libgomp: OpenMP is off; no libstdc++ or libwinpthread DLL). One fix-up is needed:
ggml drops the `lib` prefix from its static libraries on every Windows build (`ggml.a`), while
rustc looks for `libggml.a` for the GNU target. `build-server.sh` sets
`CMAKE_PROJECT_ggml_INCLUDE` to `packaging/windows/ggml-lib-prefix.cmake` (whisper-rs-sys hands
`CMAKE_*` variables on to CMake), which puts the prefix back. `whisper-rs-sys` doesn't rebuild for
a changed variable, so the script cleans a cached build that still has `ggml.a` or was built with
other ggml flags (a stamp file in the target folder). `GGML_NATIVE` can't apply when
cross-compiling, so the script turns on **AVX, AVX2, FMA and F16C** (owner, 2026-11-25): live
captions need that speed, and the Windows server therefore needs a CPU with AVX2 (Intel Haswell /
AMD Excavator, 2013–2015, or later). Older CPUs can't run it.

**FFmpeg.** Windows has no system FFmpeg, so the server ships an **LGPL-only shared build** next
to `vesper.exe` (Linux keeps Ubuntu's, never bundled). `packaging/windows/fetch-ffmpeg.sh`
downloads BtbN's `win64-lgpl-shared-8.1` build, pinned by URL and SHA-256 (a month-end autobuild,
which that project keeps for about two years), into `packaging/downloads/ffmpeg-win64/`, and
refuses a build configured with `--enable-gpl`. The server links its import libraries
(`FFMPEG_DIR`) and the installer ships its DLLs (avcodec, avformat, avutil, swresample, swscale,
avdevice, avfilter). Not its `ffmpeg.exe` and `ffprobe.exe`: since IDN Beta14, media preparation
probes and converts through IDN's FFmpeg adapter on these DLLs (`docs/MEDIA-PREP-PLAN.md`; a
conversion runs as a child process of `vesper.exe` itself, `vesper media-convert`, in a lower
priority class and without a console window: nothing more to ship), and the installer deletes the
copies batch 16 put there. On Linux
the `vesper` package no longer recommends Ubuntu's `ffmpeg` package either; nothing in the server
runs an FFmpeg tool (the `--preview` development option still opens `ffplay`). Conversions encode
H.264 on the GPU (NVENC, AMF, Quick Sync) or with OpenH264 on the CPU where the build has it.
To move the pin, see the script's header; stay on FFmpeg 8 (the major Ubuntu's build has).

Recording and streaming (`docs/RECORD-STREAM-PLAN.md`) need no new files: they encode through
the same DLLs (avcodec's NVENC, AMF and Quick Sync encoders on the GPU, OpenH264 on the CPU
where the build has it) and mux with avformat (MP4, MKV, FLV over RTMP(S), MPEG-TS over SRT).
Not yet checked on a Windows box: which of these encoders and protocols the pinned build has
(`GET /api/v1/streaming` lists the encoders the server found). On Linux they come from
Ubuntu's FFmpeg (VA-API on the church boxes), as decoding does. FFmpeg's Vulkan Video encoders
are off: `vesper` sets `IDN_VULKAN_ENCODE=off` at start (unless the environment already sets
it), because the renderer owns Vulkan in the process and a second Vulkan instance or device
made beside it can crash it (the Vulkan loader race in the `render` crate docs). VA-API, NVENC
and the CPU encoders are unaffected.

**What the installer does** (`vesper-server-<version>-setup.exe`, NSIS, needs administrator):

- installs `vesper.exe`, FFmpeg's DLLs, the browser UI (`ui\`), the Bible translations
  (`scripture\`), `vesper-task.ps1` and the
  licence texts (`licenses\`: Vesper's, FFmpeg's LGPL and a notice naming the build, its source
  and the offer to provide it) into `C:\Program Files\Vesper`;
- creates the data folder `%ProgramData%\Vesper` (and `logs\`), readable and writable only
  by SYSTEM, Administrators and (with screens on this PC's monitors) the task's user: the
  SYSTEM task must not pick up files other users drop there, and the folder holds the site's
  and sign-ins' state (open it as an administrator to read the log);
- adds the firewall rule "Vesper Server" (inbound, the program `vesper.exe`, every profile);
- registers and starts the **scheduled task "Vesper"**: at system startup, before anyone logs
  in, as SYSTEM, no window, no time limit, normal priority. Its trigger repeats every minute
  (a no-op while the server runs), so the server is back about a minute after a crash: Task
  Scheduler's own "restart on failure" only covers a failure to launch. It runs `vesper.exe
  --data %ProgramData%\Vesper --ui-dir <install>\ui --log-file
  %ProgramData%\Vesper\logs\vesper.log --displays none` in the install folder (the previous
  run's log is kept as `vesper.log.old`). No display outputs: a SYSTEM process in session 0 has
  no desktop anyone sees (that is the next option). To keep the server stopped, disable the
  task (`vesper-task.ps1 -Stop` does, and the installer enables it again; `-Resume` enables and
  starts it by hand); ending the process isn't enough. An upgrade that fails after stopping the
  server (a file that can't be written, an account name Windows doesn't know) leaves the
  previous task as it was, enabled, so the old server still starts.

**"Show screens on this PC's monitors"** (the installer's Screens page, off by default; `/DESKTOP`
for a silent install; remembered for upgrades in `HKLM\Software\Vesper\Server\Desktop`): a
SYSTEM process started at boot cannot draw on anyone's desktop, so for display outputs on this
PC's own monitors the task instead runs **as one Windows account, at its logon, in its
session** (`vesper-task.ps1 -Install -Desktop [-User NAME]`), with `--displays window` and no
administrator rights. The server then runs only while that user is signed in. The account is
on the same page (`/USER=NAME` for a silent install): an upgrade shows and keeps the account
the existing task runs for (say an auto-login booth account, even when an administrator runs
the upgrade from their own account or over Remote Desktop); with the field left empty (a first
install, or an upgrade where it was cleared) it takes the user signed in at the console
(`-ConsoleUser`). An account name Windows doesn't know is refused before anything changes. The task script gives that user Modify
rights on `%ProgramData%\Vesper` (files the SYSTEM task wrote before stay writable).
Running the installer again with the box unticked goes back to the headless startup task. Display outputs there are borderless windows on the chosen monitor that
never take the keyboard or mouse (`docs/DISPLAY-OUTPUTS-PLAN.md`, stage 1b).

**Fonts.** Slide text is drawn with the fonts installed on the PC (and those in
`%ProgramData%\Vesper\fonts`). Install a church's fonts **for all users**: right-click the font
file, "Install for all users". A double click and "Install" puts the font in that user's own
font folder only (Windows 10 1809 and later). The server, running as SYSTEM, still finds
those: at start-up it also loads every user's own font folder. But a font installed for all
users is there for every program and account. A font added while the server runs is picked
up when the server restarts. Until then, slides that use it show the fallback font, and Oriel
lists it in the fonts notice.

A scheduled task rather than a Windows service: a service must answer the service control
manager, which `vesper.exe` doesn't do (yet), while a startup task runs any program headless.
The cost: Windows stops the task by ending the process, so the server gets no clean-shutdown
signal there (its documents are written as they change; the Site Manager and mDNS just don't
say goodbye). A real service is a later step.

Uninstalling stops and removes the task and the firewall rule and deletes the program folder;
**`%ProgramData%\Vesper` is kept** (the church's data).

`--data` defaults to `%ProgramData%\Vesper` on Windows, so `vesper.exe` run by hand from the
install folder uses the same data (stop the task first: `vesper-task.ps1 -Stop` in an
administrator PowerShell; it disables the task too, so `Enable-ScheduledTask Vesper` or the
installer starts it again).

## Vesper Oriel

```sh
packaging/oriel/build.sh linux      # .deb + .AppImage (needs the WebKitGTK dev packages)
packaging/oriel/build.sh windows    # NSIS installer, cross-built (MinGW-w64 + nsis)
```

Product name "Vesper Oriel", identifier `org.vesper.oriel`, program `vesper-oriel`. The Linux
`.deb` depends on WebKitGTK 4.1 and GTK 3 (added by Tauri's bundler); the AppImage carries them.
The Windows installer installs per machine (administrator prompt; `C:\Program Files\Vesper Oriel`,
Start menu entry for every account), so one install serves every Windows account at the church;
each account keeps its own settings (its app-data folder). Tauri's `installMode: "perMachine"`,
plus our hook `apps/control/src-tauri/windows/installer-hooks.nsh`: a per-user install of an
earlier test build under the installing account (`%LOCALAPPDATA%\Vesper Oriel`) is uninstalled
silently first, its settings kept; one under another account (elevated with a different
administrator's password) isn't seen and stays until that account removes it in Settings ▸ Apps.
It uses the WebView2 runtime that Windows 10 and 11 already have (it downloads it if missing). It is built
for `x86_64-pc-windows-gnu` like the server, so it carries `WebView2Loader.dll` next to
`vesper-oriel.exe` (Tauri's bundler adds it); the C runtimes are linked statically. The shell's
library is built as an `rlib` only: the template's `cdylib` (for Android/iOS) can't link for
the GNU target (too many exports).

## Unsigned installers

Neither Windows installer is code-signed yet. Windows SmartScreen shows "Windows protected your
PC" the first time each one runs: click **More info**, then **Run anyway**. Browsers may also warn
about downloading an uncommon `.exe`. Signing needs a code-signing certificate (an owner
decision); until then hand the installers out from a place the church trusts, with their SHA-256
(`sha256sum packaging/out/*.exe`).

## Windows test checklist (for the owner)

Nobody can run Windows on the build machine; please try these on a Windows 10 or 11 PC and note
what fails (with `%ProgramData%\Vesper\logs\vesper.log` for the server).

Server (`vesper-server-<version>-setup.exe`):

1. Run the installer. SmartScreen warns (unsigned): More info → Run anyway. Accept UAC.
2. Task Scheduler → Task Scheduler Library → "Vesper": status Running, trigger "At system
   startup" repeating every minute, runs as SYSTEM. `icacls %ProgramData%\Vesper` lists only
   SYSTEM and Administrators (no Users). Oriel ▸ Screens ▸ Add output: display outputs are
   turned off.
3. `%ProgramData%\Vesper\logs\vesper.log` shows the server starting: the FFmpeg version line, the
   GPU adapter (D3D12 or Vulkan), the pairing code, no errors.
4. On the same PC, `http://localhost:5555/` in a browser shows the control UI.
5. From another PC on the LAN: Oriel's connect screen lists this server (mDNS), and pairing with
   the code from the log works. If it isn't listed, connect by IP address and note it. No
   firewall prompt should appear (the installer's rule covers `vesper.exe`).
6. Show a slide on a screen sent over IDN Open Protocol (compatible with NDI®) and receive it on
   another machine.
7. Play a video (H.264 MP4) as a background: it plays smoothly (the log says whether decoding is
   on the GPU, D3D11VA, or in software).
8. Audio: Settings ▸ Audio lists the PC's sound cards; a video's sound plays on the chosen one
   (WASAPI). Check this again with nobody logged in (step 9).
9. Restart Windows and don't log in: from another PC the server is reachable again within a
   minute or two.
10. Log in and end `vesper.exe` in Task Manager (Details tab): it starts again within about a
    minute.
11. Run the same installer again (an upgrade): it replaces the files and the server runs again.
    Install once to another folder (say `D:\Vesper`): the next upgrade proposes that folder.
    Start `vesper.exe` by hand from the install folder, then upgrade: the installer ends it
    (no "Error opening file for writing").
12. Uninstall (Settings ▸ Apps ▸ Vesper Server): the task, the firewall rule and
    `C:\Program Files\Vesper` are gone; `%ProgramData%\Vesper` is still there.

Server with screens on this PC's monitors (a projector as the second monitor):

13. Run the installer again and tick **Show screens on this PC's monitors** on the Screens page.
    Task Scheduler ▸ "Vesper": trigger "At log on of <you>", runs as you, not as SYSTEM; the
    server is running now (no restart needed).
14. Oriel ▸ Screens ▸ a screen ▸ Add output ▸ System display: both monitors are listed
    ("Display 1 … main monitor", "Display 2"). Pick the projector, Apply: the screen fills the
    projector. Then the checks in `docs/DISPLAY-OUTPUTS-PLAN.md` ("Hardware test checklist,
    windows").
15. Sign out and in again: the server starts with your session, the screen comes back on the
    projector. Restart Windows and don't sign in: the server is not running (expected in this
    mode).
16. Run the installer again with the box unticked: the task is back to "At system startup" as
    SYSTEM.
17. Tick the box again for another account (the Screens page's account field, say a booth
    account), then run the installer once more from your own account: the page shows the booth
    account, and after the upgrade the task still runs "At log on of" the booth account.

PowerPoint import (any real deck with a chart or a table in it):

18. Without LibreOffice: Oriel ▸ Import ▸ PowerPoint / Keynote / Google Slides…, pick the
    `.pptx`: it comes in; the report names the chart / table as "left out" and says LibreOffice
    isn't installed. Text, pictures and backgrounds look like the deck.
19. Install LibreOffice (default folder), import the same deck again (no server restart): the
    chart / table now show as pictures in their places (the report: "drawn from the slide's
    picture", pictures by LibreOffice), and the slides' text is still editable. Also with the
    server running as SYSTEM (the default install) and with nobody logged in.

Oriel (`Vesper Oriel_<version>_x64-setup.exe`):

1. Run the installer (SmartScreen: More info → Run anyway); it asks for an administrator and
   installs in `C:\Program Files\Vesper Oriel`.
2. Start "Vesper Oriel" from the Start menu: the window opens. Sign in as another (standard)
   account: "Vesper Oriel" is in its Start menu too and opens.
3. The connect screen lists the servers on the LAN; connect and pair; the library, slide grid
   and live view work; click a slide to go live.
4. Over an earlier per-user test install (`%LOCALAPPDATA%\Vesper Oriel`, same account): after
   the install it is gone (folder, Start menu entry, Settings ▸ Apps entry), only one
   "Vesper Oriel" is listed, and the saved servers are still there.
5. Uninstall from Settings ▸ Apps (as an administrator).

## Vesper Launcher

The Launcher (`docs/LAUNCHER-PLAN.md`) installs, updates and monitors the Vesper products on a
machine, acts on its role (presentation server, display box) and runs IDN's diagnostics.

**Linux:** its own package, `vesper-launcher`, which the server's package depends on; alone it
makes a display box or a control PC (`apt install ./vesper-launcher_*.deb ./vesper-idn-node_*.deb`).
`vesper-launcher.service` runs as root (it installs packages with apt and starts and stops the
products' units), with `KillMode=process` so an install it started finishes even if the Launcher itself is
stopped meanwhile (the Launcher waits for that install at its next start). The package enables it on
first install and starts it on every install, upgrade and reinstall (an admin's disable is kept),
but never restarts it: an update from the site's channel restarts it through its self-update
helper (a transient `systemd-run` unit that installs, restarts, checks the new version and goes
back to the kept one when it doesn't come up); after an upgrade by hand, `systemctl restart
vesper-launcher`. The Launcher installs with `apt-get install -y -q --allow-downgrades --no-remove
-o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold ./<package>.deb`
(`DEBIAN_FRONTEND=noninteractive`, `NEEDRESTART_SUSPEND=1`): the system packages it needs come
along from the machine's repositories (apt's lists are refreshed once when one can't be found
or fetched), a rollback may install an older version, apt never removes a package (it stops and
the install fails with apt's reason), an edited `/etc/default/vesper` or
`/etc/vesper/launcher.json` is kept, and a changed default from the package is left beside it
as `*.dpkg-dist`. Revision 2 of the packages moved the Launcher out of
`vesper` (`Replaces`/`Breaks: vesper (<< 0.1.0-2)`): upgrading from revision 1 installs both.
The released revision 1 (`vesper_0.1.0-1`) has no Launcher, so such a machine is upgraded by hand once
(`apt install ./vesper_*.deb ./vesper-launcher_*.deb ./vesper-idn-node_*.deb`); from then on the
Launcher updates it from the site's channel. `vesper` needs `vesper-launcher (>= 0.1.0)`, the
oldest Launcher it works with, never the Launcher's own newest version, so a server update never
waits for a Launcher update. When a product's package needs another product of the Launcher's at
a newer version than installed (its `Depends`), the Launcher downloads the newest one meeting it
from the channel and installs both in one apt run (refused, saying what it needs, when the
channel has none). Versions for the first release: the Launcher (and its tray) 0.1.1, every
other product 0.1.0. The bundled IDN Node's package `Breaks: vesper-launcher (<< 0.1.1)`
(`idn_min_launcher` in `build-deb.sh`): under an older Launcher dpkg refuses it, so the Launcher
is updated first (by hand, `apt install` both packages together: apt orders them).

**Products** come from `/etc/vesper/launcher.json` (the Launcher itself) and
`/etc/vesper/launcher.d/*.json`, one product per file, which each product's package installs (the
server's `vesper.json`: its units `vesper.service` and `vesper-display.service`; the Launcher's own
package `vesper-idn-node.json`, the bundled IDN Node); a product in the main file wins over a file
with the same ID. A product file read at the Launcher's start: a product added by hand counts
after `systemctl restart vesper-launcher`. `/etc/vesper/update-keys/*.pub` adds trusted
signing keys.

**The bundled IDN Node** (`vesper-idn-node`, recommended by the Launcher): IDN's `idn-node`,
`idn-session`, `idn` and `idn-display` at the release Vesper is pinned to, built by
`packaging/idn-node/build.sh linux` from a fresh shallow clone of the public IDN repository at
that tag (`packaging/downloads/idn-src-<tag>/`, its own target folder `packaging/downloads/
idn-target/`). The package only installs the programs: the
Launcher runs `idn-node install` when the machine becomes a display box (and `idn-node
uninstall` when it stops being one); removing the package removes a Node service that runs its
copy (the machine's IDN state in `/var/lib/idn-node` stays). Its build needs `build-essential
pkg-config clang libclang-dev cmake glslc libasound2-dev libxkbcommon-dev libwayland-dev
libudev-dev`. **Vesper updates this Node, not the Site Manager** (lane I, IDN Beta14): the package
also installs `idn-updates-managed-by` (first line `Vesper`) beside `idn-node`, so the Node reports
its updates as managed by Vesper and the Site Manager doesn't push IDN Media updates to it (they
would overwrite the package's programs); the Launcher puts the marker back if it goes missing
(`docs/LAUNCHER-PLAN.md`). Windows: the Launcher's installer puts the same marker in `idn-node\`.
**Its updates come through Vesper's update channel** (bug run 7, H04-2 / R3-2): `vesper-idn-node`
is one of the Launcher's products (`/etc/vesper/launcher.d/vesper-idn-node.json`, shipped by
`vesper-launcher` so a box's Launcher learns it with its own update, whatever Node package the box
has): a package of programs, with `"healthUrl": ""` (no health URL: up while dpkg has it
installed, healthy once dpkg has the new version, else put back; no Launch or Stop; its Logs are
`idn-node.service`'s). Sign a newer `vesper-idn-node_*.deb` as product `vesper-idn-node` and upload
it to Site ▸ Updates: each box's Launcher installs it like the server's package (its postinst
restarts a Node service running its copy). Windows: the Node is part of the Launcher's installer,
so a newer Launcher setup uploaded to Site ▸ Updates updates it (the installer stops the Node's
service and starts it again on the new copy).

**A server installed with apt.** The Launcher takes the version dpkg has installed as the
server's (also after an `apt install` by hand), and never offers or runs an update to the same
or an older version. To roll back automatically it needs the installed version's package: it
keeps every package it installs itself, and for a version installed outside it, it downloads
that version from the site's update channel when the channel has it, or uses apt's cached
download. Otherwise the outcome of a failed first update says so, and the version before is
installed again by hand (`apt install ./vesper_<version>_amd64.deb`). Upload the installed
version to Site ▸ Updates before the first update to have the automatic rollback. `apt purge
vesper-launcher` removes `/var/lib/vesper-launcher` (its kept packages and site key), never
`/var/lib/vesper`.

**Joining the site.** On a machine with a desktop, the tray app opens the first-run wizard
(roles, the site's server, wait for approval). On a headless box, from a shell on it:

```sh
sudo vesper-launcher join 123456 --server main-server.local --role display
vesper-launcher join                 # the only server on the network; waits for an admin in Oriel
```

The code is the pairing code the site's server shows (Oriel ▸ Site); without it the join waits
for an admin's approval in Oriel ▸ Site (the pairing is named `Vesper Launcher – <machine>`).
`--role display` makes the box a display box: no Vesper server, the IDN Node installed and
joined to the site with a code the site's server makes for it, its display service on from boot
without a desktop (Linux). The same over the local API: `POST /launcher/v1/setup/join`
`{"server", "code"}`, `PUT /launcher/v1/setup` `{"roles": ["display"]}`.

**Diagnostics:** the window's "Diagnostics" (and the tray's) runs the bundled `idn doctor` and
explains it, with the Launcher's status and "Copy report" (keys and tokens redacted);
`curl -s http://127.0.0.1:5556/launcher/v1/diagnostics` on a headless box.

**Tray app** (`apps/launcher-tray`, Tauri, excluded from the workspace like Oriel's shell): a
tray icon in the Launcher's status colour, "Open", "Diagnostics" and "Quit", and a window showing
the Launcher's own page (`http://127.0.0.1:5556/`). Linux: `vesper-launcher-tray` (WebKitGTK 4.1,
`libayatana-appindicator3-1`), started at every log-in by `/etc/xdg/autostart`. Build it with
`cargo build --release` in `apps/launcher-tray/src-tauri` (build-deb.sh does). Screenshots: `node
apps/launcher-tray/shots.mjs <folder>` after `cargo build -p vesper-launcher`.

**Windows** (`packaging/windows/build-launcher.sh`, cross-built like the server):
`vesper-launcher-<version>-setup.exe` (NSIS, administrator) installs `vesper-launcher.exe`, the
tray `vesper-launcher-tray.exe` (with `WebView2Loader.dll`), `launcher-task.ps1` and the IDN
programs (`idn-node\`, with their licences) into `C:\Program Files\Vesper Launcher`; registers the
scheduled task **"Vesper Launcher"** (SYSTEM, at startup, started again every minute, `--config
%ProgramData%\Vesper Launcher\launcher.json`); writes that configuration if missing (the Launcher
itself, `"install": {"kind": "nsis", "task": "Vesper Launcher"}`); makes `%ProgramData%\Vesper
Launcher` SYSTEM- and Administrators-only (the site's key is there); starts the tray at every
sign-in (HKLM `Run`); adds the firewall rule "Vesper IDN Node" for `idn-node.exe`, `idn.exe` and
`idn-display.exe`. An upgrade ends the Launcher, the trays, the IDN programs of its folder and a
Node service running them, and starts them again: the trays and the IDN session agent (the
display service on a display box) in the sessions of the users who ran them (a one-time task as
that user). Before it writes the configuration, the installer makes the state folder safe
(`launcher-task.ps1 -Secure`): links in it are removed, a file another user owns (Users may
create folders under ProgramData before the Launcher is installed) is renamed `<name>.untrusted`
and never loaded, and everything is given to Administrators. The server's installer makes
`launcher.d` SYSTEM- and Administrators-only too. The server's installer adds the server as a
product (`launcher.d\vesper.json`, its task "Vesper"). A display box on Windows runs IDN's display
service in a signed-in session (`apps-display=on`): set the PC to sign in automatically.
Uninstalling removes a Node service that runs its copy (`launcher-task.ps1 -Remove`), the task,
the `Run` value and the rule,
and keeps `%ProgramData%\Vesper Launcher`. The Launcher's self-update on Windows runs the new setup
silently from the one-time task "Vesper Launcher Update" (SYSTEM) and goes back to the kept one
when the new Launcher doesn't answer. Not tested on a Windows PC yet (cross-built, imports checked;
see the checklist below).

Windows checklist (Launcher):

1. Run `vesper-launcher-<version>-setup.exe` (SmartScreen: More info → Run anyway). Task Scheduler
   ▸ "Vesper Launcher": running as SYSTEM, at startup, repeating every minute.
   `http://127.0.0.1:5556/` in a browser shows the Launcher's window.
2. Sign out and in: the tray icon appears and opens the first-run wizard.
3. Diagnostics (tray menu): `idn doctor` runs and is explained; "Copy report" copies text.
4. Choose "Display box", join the site with the server's code: the window shows the steps done
   (IDN Node service "idn-node" installed and running, joined), and Oriel ▸ Displays lists the
   box. Turn on automatic sign-in, restart: the display service shows the box's screen.
5. Switch the role back to none/presentation: the display service stops, the Node service is
   removed.
6. Upload a newer signed Launcher setup to Site ▸ Updates and update it from Oriel: the Launcher
   restarts on the new version (its window shows it); with a broken one, it goes back.
7. Uninstall: the task, the tray's `Run` value, the firewall rule and the program folder are
   gone; `%ProgramData%\Vesper Launcher` stays.

## Releases

Releases live on GitHub (`github.com/ADC-Industries/Vesper`, its Releases page), one release line
per product told apart by the tag's prefix:

| Product | Tag | Assets (signed product, platform) |
|---|---|---|
| Vesper server | `server-v<version>` | `vesper_<version>-<rev>_amd64.deb` (`vesper`, `linux-x86_64`), `vesper-server-<version>-setup.exe` (`vesper`, `windows-x86_64`) |
| Vesper Launcher | `launcher-v<version>` | `vesper-launcher_…deb` (`vesper-launcher`, `linux-x86_64`), `vesper-launcher-tray_…deb` (`vesper-launcher-tray`, `linux-x86_64`), `vesper-launcher-<version>-setup.exe` (`vesper-launcher`, `windows-x86_64`) |
| Bundled IDN Node | `idn-node-v<version>` | `vesper-idn-node_<version>-<rev>_amd64.deb` (`vesper-idn-node`, `linux-x86_64`; on Windows the Node is in the Launcher's installer) |
| Vesper Oriel | `oriel-v<version>` | `vesper-oriel_<version>_amd64.deb` (`vesper-oriel`, `linux-x86_64`), `…_amd64.AppImage` (`linux-x86_64-appimage`, to download by hand), `…_x64-setup.exe` (`windows-x86_64`) |

A normal release is the **stable** channel, a GitHub pre-release the **beta** channel. Every asset
has its minisign signature beside it (`<file>.minisig`), made with the **automation key**
(A05D75D53CB6819A, public half `packaging/keys/vesper-automation.pub`, compiled into the
Launcher's and the server's trusted keys) and the trusted comment the Launcher checks:
`vesper-package product=<product> version=<version> platform=<platform>`. The IDN Node's version
is its package's (`0.3.8~beta21`); its tag spells the `~` as `-` (`idn-node-v0.3.8-beta21`), which
git allows. Each release also has `manifest.json`: the product, version and channel, and per
asset its signed product, platform, file name, size and SHA-256.

**Making a release** (on the build machine that holds the automation key):

```sh
scripts/release.sh server 0.2.0 --dry-run    # build, sign, write the manifest, print the gh command
scripts/release.sh server 0.2.0              # the same, then publish (gh release create)
scripts/release.sh oriel 0.2.0 --beta        # a beta: a GitHub pre-release
scripts/release.sh idn-node 0.3.8-beta18 --no-build   # packages already in packaging/out/
```

The version must be what the product's sources say (`crates/vesper-server/Cargo.toml`,
`crates/vesper-launcher/Cargo.toml`, `apps/control/src-tauri/tauri.conf.json`, the IDN tag in
`crates/audio/Cargo.toml`): change it there first. The script builds with the scripts above
(`--no-build` takes what is in `packaging/out/`, the newest revision of each `.deb`), signs each
asset with `minisign -S -s ~/.config/vesper-release/vesper-automation.key` (another key with
`VESPER_RELEASE_KEY`; only its path is passed to minisign, nothing reads, prints or copies it),
checks every signature against the public key, writes the manifest, puts the release together in
`packaging/out/release/<tag>/` and publishes it with `gh release create <tag> [--prerelease]`
(another repository with `VESPER_RELEASE_REPO`). GitHub makes the tag on the repository's default
branch: publish the code first. An existing tag is refused.

**How releases reach the machines.** The church's Vesper server checks the repository every 6
hours and brings the newest release of each product its machines run into the site's update
channel, signature checked (Oriel ▸ Site ▸ Updates ▸ "Releases from GitHub": on/off, stable or
beta, how often, the repository); installing stays an admin's action. A Launcher with no church
server checks GitHub itself (`"github"` in `launcher.json`, `docs/LAUNCHER-PLAN.md`, "Releases
from GitHub"). "Add app" (the Launcher's window, or Oriel's "Add app to <machine>…") installs a
product a machine doesn't have yet from the channel or GitHub. A site without internet keeps
uploading the release's files (the package and its `.minisig`) in Site ▸ Updates.

## Licences

- Vesper and Oriel: MIT OR Apache-2.0 (`COPYRIGHT`, `LICENSE-MIT`, `LICENSE-APACHE`), installed
  in `/usr/share/doc/vesper/` and `licenses\` on Windows.
- IDN Open Protocol (compatible with NDI®): MIT OR Apache-2.0, compiled in. NDI® is a registered
  trademark of Vizrt NDI AB; neither this project nor the library is affiliated with or endorsed
  by Vizrt.
- FFmpeg: LGPL. On Linux it is Ubuntu's, a dependency we don't ship. On Windows we ship its
  unmodified LGPL-only DLLs, linked dynamically and replaceable, with the LGPL text
  (`licenses\FFMPEG-LICENSE.txt`) and `licenses\FFMPEG-NOTICE.txt` (which build, where its source
  is, the offer to provide it). No GPL or non-free FFmpeg components are shipped.
- mimalloc (the server's Rust allocator, cargo feature `mimalloc`, off by default; only in builds that turn it on): MIT
  (© Microsoft Corporation), its C source compiled in statically on Linux and Windows by the
  `mimalloc` crate (MIT); no runtime file, nothing for the installers to ship besides the
  notice in `COPYRIGHT`.
- whisper.cpp (live captions): MIT, compiled in; the speech models it downloads are whisper.cpp's
  conversions of OpenAI's Whisper models (MIT).
- candle and Hugging Face `tokenizers` (machine translation): MIT OR Apache-2.0 / Apache-2.0,
  compiled in; the OPUS-MT models it downloads are Helsinki-NLP's, each under the licence its model
  card gives (Apache-2.0 or CC-BY-4.0; Settings ▸ Translation shows each pair's licence and
  attribution).
- Bible translations: public domain texts (see [Bible translations](#bible-translations)), their
  sources listed in `SCRIPTURE-SOURCES.txt`.
- The bundled IDN Node programs (`vesper-idn-node`, the Launcher's `idn-node\` on Windows): IDN
  Open Protocol, MIT OR Apache-2.0 (© ADC Industries), no GPL and no FFmpeg; on Linux they use
  the system's ALSA (LGPL-2.1) dynamically. They ship IDN's licence texts, a notice and
  `THIRD-PARTY-CRATES.txt` (every crate compiled in, with its licence expression, from `cargo
  metadata`). Some are not MIT/Apache (BSD-2/3-Clause, ISC, Unlicense, Zlib, Unicode-3.0,
  CDLA-Permissive-2.0, BSL-1.0; the list names them); their full licence texts aren't gathered
  into the packages yet (cargo-about or cargo-deny would; neither is installed on the build
  machine). `dxcompiler.dll` isn't shipped on Windows (wgpu falls back to FXC).
- The Rust crates compiled into Vesper itself keep their own permissive licences; a generated
  list of them for the packages is still to come.
