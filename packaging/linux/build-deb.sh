#!/usr/bin/env bash
# Builds Vesper's Debian packages (Ubuntu LTS, amd64) into packaging/out/:
#
#   vesper_<version>-<rev>_amd64.deb                the server (needs vesper-launcher)
#   vesper-launcher_<version>-<rev>_amd64.deb       the Vesper Launcher service, alone on a
#                                                   machine without a server (a display box,
#                                                   an Oriel control PC)
#   vesper-launcher-tray_<version>-<rev>_amd64.deb  its tray icon and window (desktops), started
#                                                   at every log-in
#   vesper-idn-node_<idn version>-<rev>_amd64.deb   the IDN Node and tools the Launcher bundles
#                                                   (display boxes, diagnostics)
#
#   packaging/linux/build-deb.sh [--no-build]
#
# --no-build packages what is already built: the binaries in $CARGO_TARGET_DIR/release (default
# target/) and the IDN programs in packaging/out/idn-node-linux (packaging/idn-node/build.sh).
# The ProPresenter 7 schemas the build needs come from scripts/fetch-propresenter-proto.sh.
# The bundled Bible translations come from scripts/fetch-scripture.sh ($CARGO_TARGET_DIR/scripture,
# or $VESPER_SCRIPTURE_DIR): run when building, else they must be there already.
# The packages are built for the Ubuntu release they are built on: their library dependencies
# (FFmpeg's sonames above all) come from this machine through dpkg-shlibdeps.
# Needs: cargo, npm, dpkg-dev (dpkg-shlibdeps), the build dependencies in docs/PACKAGING.md, and
# for the tray WebKitGTK 4.1 and libayatana-appindicator3-dev.
set -euo pipefail
umask 022

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
here="$root/packaging/linux"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
build=1
for arg in "$@"; do
  case "$arg" in
    --no-build) build=0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/vesper-server/Cargo.toml" | head -n1)"
launcher_version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/vesper-launcher/Cargo.toml" | head -n1)"
# Revision 2: the Launcher moved out of the server's package into its own.
revision="${VESPER_DEB_REVISION:-2}"
# The oldest Launcher the server's package works with (what it needs of the Launcher: its
# launcher.d product file), not the Launcher's own current version: a server update from the
# site's channel must never need the Launcher's newest package (the Launcher installs one
# product at a time; it installs a needed product's package with it when the bound is raised).
# Raise it only when the server needs something a newer Launcher brings.
min_launcher="0.1.0"
arch="$(dpkg --print-architecture)"
# Reproducible timestamps: the last commit's time.
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct)}"

if [[ $build == 1 ]]; then
  # The ProPresenter 7 schemas the server's importer is built from (not in the repository).
  "$root/scripts/fetch-propresenter-proto.sh"
  (cd "$root" && cargo build --release --locked -p vesper-server --bin vesper -p vesper-launcher --bin vesper-launcher)
  # (The tray is its own workspace: built into the same target folder, where it is looked for.)
  (cd "$root/apps/launcher-tray/src-tauri" && cargo build --release --locked --target-dir "$target_dir")
  (cd "$root/apps/control" && { [[ -d node_modules ]] || npm ci --prefer-offline; } && npm run build)
  [[ -n "${VESPER_SCRIPTURE_DIR:-}" ]] || "$root/scripts/fetch-scripture.sh"
  "$root/packaging/idn-node/build.sh" linux
fi
bin="$target_dir/release/vesper"
launcher="$target_dir/release/vesper-launcher"
tray="$target_dir/release/vesper-launcher-tray"
idn="$root/packaging/out/idn-node-linux"
ui="$root/apps/control/dist"
scripture="${VESPER_SCRIPTURE_DIR:-$target_dir/scripture}"
for f in "$bin" "$launcher" "$tray"; do
  [[ -x "$f" ]] || { echo "missing $f (build first)" >&2; exit 1; }
done
[[ -x "$idn/idn-node" ]] || { echo "missing $idn (packaging/idn-node/build.sh linux)" >&2; exit 1; }
[[ -f "$ui/index.html" ]] || { echo "missing $ui (npm run build in apps/control)" >&2; exit 1; }
compgen -G "$scripture/*.vbible" > /dev/null \
  || { echo "no Bible translations in $scripture (scripts/fetch-scripture.sh)" >&2; exit 1; }

out="$root/packaging/out"
work="$out/deb"
rm -rf -- "$work"
mkdir -p "$work"

# The library dependencies of a package's programs (ldd → owning packages, with minimum
# versions from their symbols files). dpkg-shlibdeps wants a debian/control next to it.
shlibs() {
  local s="$work/shlibdeps" r
  rm -rf -- "$s"
  mkdir -p "$s/debian"
  printf 'Source: vesper\n\nPackage: vesper\nArchitecture: any\n' > "$s/debian/control"
  r="$(cd "$s" && dpkg-shlibdeps -O "$@" 2>/dev/null | sed -n 's/^shlibs:Depends=//p')"
  rm -rf -- "$s"
  [[ -n "$r" ]] || { echo "dpkg-shlibdeps found no dependencies for $*" >&2; exit 1; }
  printf '%s' "$r"
}

# Finishes a staged package $1 into the .deb $2: its control file (stdin, ending with the
# Description; Installed-Size added), md5sums, timestamps.
finish() {
  local pkg="$1" deb="$2" size control
  size="$(du -sk --exclude=DEBIAN "$pkg" | cut -f1)"
  control="$(cat)"
  {
    printf '%s\n' "$control" | sed '/^Description:/,$d'
    echo "Installed-Size: ${size}"
    printf '%s\n' "$control" | sed -n '/^Description:/,$p'
  } > "$pkg/DEBIAN/control"
  chmod -R u=rwX,go=rX "$pkg"
  (cd "$pkg" && find . -path ./DEBIAN -prune -o -type f -printf '%P\0' | LC_ALL=C sort -z | xargs -0 -r md5sum > DEBIAN/md5sums)
  find "$pkg" -exec touch -h -d "@$SOURCE_DATE_EPOCH" {} +
  dpkg-deb --root-owner-group -Zxz --build "$pkg" "$deb" >/dev/null
  rm -rf -- "$pkg"
  echo "$deb"
}

# ---- vesper: the server ---------------------------------------------------------------------
pkg="$work/vesper"
mkdir -p "$pkg/DEBIAN"
install -D -m 0755 "$bin" "$pkg/usr/bin/vesper"
strip --strip-unneeded "$pkg/usr/bin/vesper"
install -D -m 0644 "$here/vesper.service" "$pkg/usr/lib/systemd/system/vesper.service"
install -D -m 0644 "$here/vesper-display.service" "$pkg/usr/lib/systemd/system/vesper-display.service"
install -D -m 0644 "$here/vesper.sysusers" "$pkg/usr/lib/sysusers.d/vesper.conf"
install -D -m 0644 "$here/vesper.default" "$pkg/etc/default/vesper"
# The server as one of the Launcher's products (docs/LAUNCHER-PLAN.md).
install -D -m 0644 "$here/launcher.d/vesper.json" "$pkg/etc/vesper/launcher.d/vesper.json"
mkdir -p "$pkg/usr/share/vesper"
cp -r "$ui" "$pkg/usr/share/vesper/ui"
# The bundled translations: next to the UI, where vesper looks by default (--scripture-dir).
install -d "$pkg/usr/share/vesper/scripture"
install -m 0644 "$scripture"/*.vbible "$pkg/usr/share/vesper/scripture/"
[[ -f "$scripture/SOURCES.txt" ]] && install -D -m 0644 "$scripture/SOURCES.txt" "$pkg/usr/share/doc/vesper/SCRIPTURE-SOURCES.txt"
install -D -m 0644 "$root/COPYRIGHT" "$pkg/usr/share/doc/vesper/copyright"
install -m 0644 "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$pkg/usr/share/doc/vesper/"
# The ProPresenter 7 schemas compiled into the importer: their source and MIT licence.
install -m 0644 "$root/crates/pro-import/PROTO.md" "$pkg/usr/share/doc/vesper/PROPRESENTER-SCHEMAS.md"
install -m 0644 "$root/docs/PACKAGING.md" "$pkg/usr/share/doc/vesper/"
for s in postinst prerm postrm; do install -m 0755 "$here/debian/$s" "$pkg/DEBIAN/$s"; done
printf '%s\n' /etc/default/vesper /etc/vesper/launcher.d/vesper.json > "$pkg/DEBIAN/conffiles"
deps="$(shlibs "$pkg/usr/bin/vesper")"
# Loaded at run time, so invisible to ldd: the Vulkan loader and Mesa's drivers (wgpu; Mesa's
# lavapipe is the software fallback), the VA-API drivers (hardware video decode, AMD through
# mesa-va-drivers, Intel through intel-media-va-driver), avahi (the box's <host>.local name;
# the server announces _vesper._tcp itself), fonts for slide text, and the tools the
# maintainer scripts use. The Launcher updates and monitors the server. FFmpeg's command-line
# tools aren't needed (IDN Beta14): media preparation probes and converts in the server through
# IDN's FFmpeg adapter, on the same libraries (`dpkg-shlibdeps` finds them).
extra="libvulkan1, mesa-vulkan-drivers, mesa-va-drivers, avahi-daemon, fonts-dejavu-core, systemd, init-system-helpers, vesper-launcher (>= ${min_launcher})"
finish "$pkg" "$out/vesper_${version}-${revision}_${arch}.deb" <<EOF
Package: vesper
Version: ${version}-${revision}
Architecture: ${arch}
Maintainer: Dylan Charpentier <info@adcnetwork.org>
Depends: ${deps}, ${extra}
Recommends: intel-media-va-driver, fonts-liberation
Suggests: libreoffice-impress
Section: video
Priority: optional
Description: Vesper presentation server (headless render and playout)
 Vesper renders slides, media and stage displays on the GPU and sends every
 screen over IDN Open Protocol (compatible with NDI®). It is controlled over
 the network by Vesper Oriel or a browser (served at http://<box>:5555/).
 .
 This package runs it as the systemd service vesper.service, as the vesper
 user, with its data in /var/lib/vesper. The Vesper Launcher (its own
 package) updates it from the site's update channel.
EOF

# ---- vesper-launcher: the Launcher service ---------------------------------------------------
pkg="$work/vesper-launcher"
mkdir -p "$pkg/DEBIAN"
install -D -m 0755 "$launcher" "$pkg/usr/bin/vesper-launcher"
strip --strip-unneeded "$pkg/usr/bin/vesper-launcher"
install -D -m 0644 "$here/vesper-launcher.service" "$pkg/usr/lib/systemd/system/vesper-launcher.service"
install -D -m 0644 "$here/launcher.json" "$pkg/etc/vesper/launcher.json"
install -d "$pkg/etc/vesper/launcher.d"
# The bundled IDN Node (the vesper-idn-node package) as one of the Launcher's products: a
# package of programs (no health URL, nothing to Launch or Stop), so a newer vesper-idn-node
# uploaded to Site ▸ Updates is installed by the Launcher like the server's package (Vesper
# owns this Node's updates: H04-2 / R3-2). Shipped here, not in vesper-idn-node: a box's
# Launcher learns it with its own update from the channel, whatever IDN Node package it has.
install -m 0644 "$here/launcher.d/vesper-idn-node.json" "$pkg/etc/vesper/launcher.d/vesper-idn-node.json"
install -D -m 0644 "$root/COPYRIGHT" "$pkg/usr/share/doc/vesper-launcher/copyright"
install -m 0644 "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$pkg/usr/share/doc/vesper-launcher/"
for s in postinst prerm postrm; do install -m 0755 "$here/debian/vesper-launcher.$s" "$pkg/DEBIAN/$s"; done
printf '%s\n' /etc/vesper/launcher.json /etc/vesper/launcher.d/vesper-idn-node.json > "$pkg/DEBIAN/conffiles"
deps="$(shlibs "$pkg/usr/bin/vesper-launcher")"
finish "$pkg" "$out/vesper-launcher_${launcher_version}-${revision}_${arch}.deb" <<EOF
Package: vesper-launcher
Version: ${launcher_version}-${revision}
Architecture: ${arch}
Maintainer: Dylan Charpentier <info@adcnetwork.org>
Depends: ${deps}, systemd, init-system-helpers
Recommends: vesper-idn-node
Replaces: vesper (<< 0.1.0-2)
Breaks: vesper (<< 0.1.0-2)
Section: admin
Priority: optional
Description: Vesper Launcher (installs, updates and monitors Vesper)
 The Vesper Launcher runs on every Vesper machine: it installs and updates
 the Vesper products there from the site's update channel (signed packages,
 rolled back automatically when a new version doesn't come up), starts and
 monitors them, acts on the machine's role (presentation server, display box)
 and reports to the site. Its window (the tray app, or a browser on this
 machine) is at http://127.0.0.1:5556/.
 .
 Alone, this package makes a display box or a control PC; the vesper package
 adds the server.
EOF

# ---- vesper-launcher-tray: the tray icon and window ------------------------------------------
pkg="$work/vesper-launcher-tray"
mkdir -p "$pkg/DEBIAN"
install -D -m 0755 "$tray" "$pkg/usr/bin/vesper-launcher-tray"
strip --strip-unneeded "$pkg/usr/bin/vesper-launcher-tray"
install -D -m 0644 "$here/vesper-launcher-tray.desktop" "$pkg/usr/share/applications/vesper-launcher-tray.desktop"
# Started at every log-in (XDG autostart): the tray's colour and the first-run wizard.
install -D -m 0644 "$here/vesper-launcher-tray.desktop" "$pkg/etc/xdg/autostart/vesper-launcher-tray.desktop"
install -D -m 0644 "$root/apps/launcher-tray/src-tauri/icons/128x128.png" "$pkg/usr/share/icons/hicolor/128x128/apps/vesper-launcher.png"
install -D -m 0644 "$root/apps/launcher-tray/src-tauri/icons/32x32.png" "$pkg/usr/share/icons/hicolor/32x32/apps/vesper-launcher.png"
install -D -m 0644 "$root/COPYRIGHT" "$pkg/usr/share/doc/vesper-launcher-tray/copyright"
install -m 0644 "$root/LICENSE-MIT" "$root/LICENSE-APACHE" "$pkg/usr/share/doc/vesper-launcher-tray/"
printf '%s\n' /etc/xdg/autostart/vesper-launcher-tray.desktop > "$pkg/DEBIAN/conffiles"
deps="$(shlibs "$pkg/usr/bin/vesper-launcher-tray")"
# The tray icon's library is loaded at run time (libayatana-appindicator).
finish "$pkg" "$out/vesper-launcher-tray_${launcher_version}-${revision}_${arch}.deb" <<EOF
Package: vesper-launcher-tray
Version: ${launcher_version}-${revision}
Architecture: ${arch}
Maintainer: Dylan Charpentier <info@adcnetwork.org>
Depends: ${deps}, libayatana-appindicator3-1, vesper-launcher
Section: admin
Priority: optional
Description: Vesper Launcher tray icon and window
 A tray icon in the Vesper Launcher's status colour and a small window with the
 products on this machine (Launch, Stop, Update, Logs), the first-run setup
 (role, joining the site) and the diagnostics page. Started at every log-in.
EOF

# ---- vesper-idn-node: the bundled IDN Node and tools -----------------------------------------
idn_tag="$(sed -n 's/^IDN Open Protocol.*, release \([^ ]*\) .*/\1/p' "$idn/VERSION.txt")"
[[ -n "$idn_tag" ]] || { echo "no release in $idn/VERSION.txt" >&2; exit 1; }
# v0.3.8-Beta18 → 0.3.8~beta18 (a pre-release sorts before 0.3.8).
idn_version="$(printf '%s' "${idn_tag#v}" | sed 's/-\(.*\)$/~\1/' | tr '[:upper:]' '[:lower:]')"
idn_revision="${VESPER_IDN_DEB_REVISION:-1}"
pkg="$work/vesper-idn-node"
mkdir -p "$pkg/DEBIAN"
for p in idn-node idn-session idn idn-display; do
  install -D -m 0755 "$idn/$p" "$pkg/usr/lib/vesper-idn-node/$p"
done
# Vesper updates this IDN Node, not the Site Manager (IDN Beta14: the marker beside idn-node,
# first line the product; the Node reports it and the Site Manager leaves the machine out of its
# updates). A package file: removed with the package; the Launcher puts it back if it goes
# missing. The package is one of the Launcher's products (vesper-launcher ships its product
# file), so a newer vesper-idn-node uploaded to Site ▸ Updates is installed by the Launcher.
printf 'Vesper\n' > "$pkg/usr/lib/vesper-idn-node/idn-updates-managed-by"
chmod 0644 "$pkg/usr/lib/vesper-idn-node/idn-updates-managed-by"
doc="$pkg/usr/share/doc/vesper-idn-node"
install -d "$doc"
install -m 0644 "$idn/licenses/"* "$idn/VERSION.txt" "$doc/"
{
  cat "$idn/licenses/IDN-NOTICE.txt"
  printf '\nThe licence texts and the list of the crates compiled in are in this folder.\n'
} > "$doc/copyright"
chmod 0644 "$doc/copyright"
for s in postinst prerm; do install -m 0755 "$here/debian/vesper-idn-node.$s" "$pkg/DEBIAN/$s"; done
deps="$(shlibs "$pkg"/usr/lib/vesper-idn-node/*)"
# Loaded at run time: the Vulkan loader and Mesa's drivers (idn-display draws with wgpu).
finish "$pkg" "$out/vesper-idn-node_${idn_version}-${idn_revision}_${arch}.deb" <<EOF
Package: vesper-idn-node
Version: ${idn_version}-${idn_revision}
Architecture: ${arch}
Maintainer: Dylan Charpentier <info@adcnetwork.org>
Depends: ${deps}, libvulkan1, mesa-vulkan-drivers, systemd
Section: video
Priority: optional
Description: IDN Node and tools, as bundled with the Vesper Launcher
 IDN Open Protocol (compatible with NDI®) ${idn_tag}: the IDN Node (idn-node,
 idn-session), the command-line tools (idn: doctor, find, recv, ptz, …) and
 the display service (idn-display), in /usr/lib/vesper-idn-node. The Vesper
 Launcher installs the Node's service when the machine becomes a display box,
 and runs idn doctor on its diagnostics page. Vesper updates this Node (the
 Launcher installs newer vesper-idn-node packages from the site's update
 channel): the site's IDN Site Manager doesn't (idn-updates-managed-by).
EOF

rmdir "$work" 2>/dev/null || true
