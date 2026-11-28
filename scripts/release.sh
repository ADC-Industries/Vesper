#!/usr/bin/env bash
# Builds, signs and publishes one release of a Vesper product on GitHub (docs/PACKAGING.md,
# "Releases"):
#
#   scripts/release.sh <product> <version> [--beta] [--dry-run] [--no-build]
#
#   <product>   server | oriel | launcher | idn-node
#   <version>   the product's version (what its sources say; refused otherwise). The IDN Node's
#               is the pinned IDN release's, as its package names it (0.3.8~beta18) or as the
#               tag does (0.3.8-beta18).
#   --beta      a beta: published as a GitHub pre-release (the "beta" channel)
#   --dry-run   everything but publishing: builds, signs, writes the manifest, prints the
#               gh command it would run
#   --no-build  packages already in packaging/out/ (built by the scripts below)
#
# One release line per product on one repository, told apart by the tag's prefix:
# server-v<version>, oriel-v<version>, launcher-v<version>, idn-node-v<version>.
#
# Packages come from the existing scripts: packaging/linux/build-deb.sh (the .debs),
# packaging/windows/build-server.sh and build-launcher.sh (the Windows installers),
# packaging/oriel/build.sh linux|windows (Oriel). Each asset is signed with the release
# automation key in minisign's format, with the trusted comment the Launcher checks
# ("vesper-package product=… version=… platform=…", crates/vesper-releases/src/package.rs), and
# described in manifest.json (product, version, channel; per asset its product, platform, file,
# size and SHA-256). The assets, their .minisig files and the manifest are put together in
# packaging/out/release/<tag>/ and published with `gh release create`.
#
# Environment:
#   VESPER_RELEASE_REPO   the GitHub repository (default ADC-Industries/Vesper)
#   VESPER_RELEASE_KEY    the private signing key (default ~/.config/vesper-release/vesper-automation.key);
#                         only its path is passed to minisign, nothing here reads it
#   MINISIGN              the minisign program (default: ~/.local/bin/minisign, else on PATH)
#   VESPER_PACKAGES_DIR   where the packages are (default packaging/out; with --no-build)
#
# The tag is made by GitHub on the repository's default branch: publish the code first.
set -euo pipefail
umask 022

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo="${VESPER_RELEASE_REPO:-ADC-Industries/Vesper}"
key="${VESPER_RELEASE_KEY:-$HOME/.config/vesper-release/vesper-automation.key}"
pubkey="$root/packaging/keys/vesper-automation.pub"
minisign="${MINISIGN:-}"
if [[ -z "$minisign" ]]; then
  if [[ -x "$HOME/.local/bin/minisign" ]]; then minisign="$HOME/.local/bin/minisign"; else minisign="minisign"; fi
fi
out="${VESPER_PACKAGES_DIR:-$root/packaging/out}"
target_dir="${CARGO_TARGET_DIR:-$root/target}"

usage() { echo "usage: $0 server|oriel|launcher|idn-node <version> [--beta] [--dry-run] [--no-build]" >&2; exit 2; }
[[ $# -ge 2 ]] || usage
product="$1"; version="$2"; shift 2
beta=0; dry=0; build=1
for arg in "$@"; do
  case "$arg" in
    --beta) beta=1 ;;
    --dry-run) dry=1 ;;
    --no-build) build=0 ;;
    *) echo "unknown option: $arg" >&2; usage ;;
  esac
done
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-~][0-9A-Za-z.]+)?$ ]] \
  || { echo "\"$version\" isn't a version (1.2.3, 1.2.3-beta1)" >&2; exit 2; }
channel=stable; [[ $beta == 1 ]] && channel=beta

cargo_version() { sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/$1/Cargo.toml" | head -n1; }
arch_platform() { case "$1" in amd64) echo linux-x86_64 ;; arm64) echo linux-aarch64 ;; *) echo "linux-$1" ;; esac; }

# The newest revision of a .deb in packaging/out ($1: the file name up to the revision).
newest_deb() {
  local f
  f="$(compgen -G "$out/$1-*_amd64.deb" | sort -V | tail -n1 || true)"
  [[ -n "$f" ]] || { echo "no $1-*_amd64.deb in $out (build first, or drop --no-build)" >&2; exit 1; }
  printf '%s' "$f"
}

# What the sources say, and the tag's version (a "~" can't be in a git tag).
case "$product" in
  server) name="Vesper"; want="$(cargo_version vesper-server)" ;;
  launcher) name="Vesper Launcher"; want="$(cargo_version vesper-launcher)" ;;
  oriel) name="Vesper Oriel"; want="$(sed -n 's/^  "version": "\(.*\)",/\1/p' "$root/apps/control/src-tauri/tauri.conf.json" | head -n1)" ;;
  idn-node)
    name="IDN Node (bundled)"
    idn_tag="$(sed -n 's/.*idn-open-protocol", tag = "\([^"]*\)".*/\1/p' "$root/crates/audio/Cargo.toml" | head -n1)"
    # v0.3.8-Beta21 → 0.3.8~beta21, as build-deb.sh names the package.
    want="$(printf '%s' "${idn_tag#v}" | sed 's/-\(.*\)$/~\1/' | tr '[:upper:]' '[:lower:]')"
    version="$(printf '%s' "$version" | sed 's/-/~/' | tr '[:upper:]' '[:lower:]')"
    ;;
  *) usage ;;
esac
[[ "$version" == "$want" ]] \
  || { echo "$product's sources are at $want, not $version: change the version there first" >&2; exit 1; }
tag="$product-v${version//\~/-}"

if [[ $build == 1 ]]; then
  [[ "$out" == "$root/packaging/out" ]] || { echo "VESPER_PACKAGES_DIR needs --no-build" >&2; exit 2; }
  case "$product" in
    server)
      "$root/packaging/linux/build-deb.sh"
      "$root/packaging/windows/build-server.sh" ;;
    launcher)
      "$root/packaging/linux/build-deb.sh"
      "$root/packaging/windows/build-launcher.sh" ;;
    idn-node)
      "$root/packaging/idn-node/build.sh" linux
      "$root/packaging/linux/build-deb.sh" --no-build ;;
    oriel)
      "$root/packaging/oriel/build.sh" linux
      "$root/packaging/oriel/build.sh" windows ;;
  esac
fi

# The assets: "source file|published name|signed product|platform".
arch="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
linux="$(arch_platform "$arch")"
assets=()
add_deb() { # $1: package name, $2: signed product
  local f; f="$(newest_deb "$1_$version")"
  assets+=("$f|$(basename "$f")|$2|$linux")
}
case "$product" in
  server)
    add_deb vesper vesper
    assets+=("$out/vesper-server-$version-setup.exe|vesper-server-$version-setup.exe|vesper|windows-x86_64") ;;
  launcher)
    add_deb vesper-launcher vesper-launcher
    add_deb vesper-launcher-tray vesper-launcher-tray
    assets+=("$out/vesper-launcher-$version-setup.exe|vesper-launcher-$version-setup.exe|vesper-launcher|windows-x86_64") ;;
  idn-node)
    add_deb vesper-idn-node vesper-idn-node ;;
  oriel)
    # Tauri's names have a space; GitHub would rename them, so they are published without one.
    assets+=("$out/Vesper Oriel_${version}_amd64.deb|vesper-oriel_${version}_amd64.deb|vesper-oriel|$linux")
    assets+=("$out/Vesper Oriel_${version}_amd64.AppImage|vesper-oriel_${version}_amd64.AppImage|vesper-oriel|$linux-appimage")
    assets+=("$out/Vesper Oriel_${version}_x64-setup.exe|vesper-oriel_${version}_x64-setup.exe|vesper-oriel|windows-x86_64") ;;
esac
for a in "${assets[@]}"; do
  f="${a%%|*}"
  [[ -f "$f" ]] || { echo "missing $f (build first, or drop --no-build)" >&2; exit 1; }
done

command -v "$minisign" > /dev/null || { echo "minisign not found ($minisign)" >&2; exit 1; }
[[ -f "$key" ]] || { echo "no signing key at the configured path" >&2; exit 1; }

stage="$out/release/$tag"
rm -rf -- "$stage"
mkdir -p "$stage"
manifest_rows=()
for a in "${assets[@]}"; do
  IFS='|' read -r src file signed platform <<< "$a"
  cp -- "$src" "$stage/$file"
  comment="vesper-package product=$signed version=$version platform=$platform"
  # Only the key's path goes to minisign; stdin closed so a passphrase prompt can't hang.
  "$minisign" -S -s "$key" -m "$stage/$file" -x "$stage/$file.minisig" -t "$comment" < /dev/null > /dev/null
  "$minisign" -V -q -p "$pubkey" -m "$stage/$file" -x "$stage/$file.minisig" > /dev/null \
    || { echo "the signature of $file doesn't verify with $pubkey" >&2; exit 1; }
  size="$(stat -c %s "$stage/$file")"
  sha="$(sha256sum "$stage/$file" | cut -d' ' -f1)"
  manifest_rows+=("$signed|$platform|$file|$size|$sha")
  echo "signed $file ($comment)"
done

python3 -I - "$stage/manifest.json" "$product" "$name" "$version" "$channel" "$tag" "${manifest_rows[@]}" <<'EOF'
import json, sys
path, product, name, version, channel, tag, *rows = sys.argv[1:]
assets = []
for r in rows:
    p, platform, file, size, sha = r.split("|")
    assets.append({"product": p, "platform": platform, "file": file,
                   "size": int(size), "sha256": sha, "signature": file + ".minisig"})
with open(path, "w") as f:
    json.dump({"format": 1, "product": product, "name": name, "version": version,
               "channel": channel, "tag": tag, "assets": assets}, f, indent=2)
    f.write("\n")
EOF

files=("$stage/manifest.json")
for a in "${assets[@]}"; do
  IFS='|' read -r _ file _ _ <<< "$a"
  files+=("$stage/$file" "$stage/$file.minisig")
done
title="$name ${version}"
[[ $beta == 1 ]] && title="$title (beta)"
notes="$name $version. Every file is signed with the Vesper release key (minisign; \`.minisig\` beside it, public key in packaging/keys/vesper-automation.pub); manifest.json lists each file's platform, size and SHA-256."
cmd=(gh release create "$tag" --repo "$repo" --title "$title" --notes "$notes")
[[ $beta == 1 ]] && cmd+=(--prerelease)
cmd+=("${files[@]}")

if [[ $dry == 1 ]]; then
  echo "dry run: would publish $tag ($channel) to $repo with:"
  printf '  %q' "${cmd[@]}"; echo
  echo "the release's files are in $stage"
  exit 0
fi
if gh release view "$tag" --repo "$repo" > /dev/null 2>&1; then
  echo "$repo already has a release $tag" >&2
  exit 1
fi
"${cmd[@]}"
