#!/usr/bin/env bash
# Builds the IDN Node programs the Vesper Launcher bundles for display boxes (idn-node,
# idn-session, idn, idn-display) at the IDN release Vesper is pinned to, into
# packaging/out/idn-node-<platform>/ (programs + licences), ready for build-deb.sh and the
# Launcher's Windows installer.
#
#   packaging/idn-node/build.sh linux|windows [--no-build]
#
# The source is a fresh shallow clone of the public IDN repository at the pinned tag (read from
# crates/audio/Cargo.toml, checked against Cargo.lock's commit) in
# packaging/downloads/idn-src-<tag>/ (gitignored), built there so IDN's own [patch.crates-io]
# and .cargo/config.toml apply. Never a local IDN checkout. Its own target folder
# ($IDN_TARGET_DIR, default packaging/downloads/idn-target): the IDN workspace's dependency
# versions differ from Vesper's. Two cargo runs (IDN's build instructions): the node and
# the CLI first, then the display program, so wgpu stays out of the others.
#
# Heavy: on a machine shared with other builds, run one at a time.
# Linux needs: build-essential pkg-config clang libclang-dev cmake glslc libasound2-dev
#   libxkbcommon-dev libwayland-dev libudev-dev (python3 for the crate list).
# Windows (cross, x86_64-pc-windows-gnu) also needs: gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64,
#   the Vulkan headers (libvulkan-dev) and glslc (`idn` builds whisper.cpp with Vulkan).
set -euo pipefail
umask 022

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
platform="${1:-}"
build=1
for arg in "${@:2}"; do
  case "$arg" in
    --no-build) build=0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done
case "$platform" in
  linux) target=""; exe="" ;;
  windows) target=x86_64-pc-windows-gnu; exe=".exe" ;;
  *) echo "usage: $0 linux|windows [--no-build]" >&2; exit 2 ;;
esac

repo="https://github.com/ADC-Industries/idn-open-protocol"
tag="$(sed -n 's/.*idn-open-protocol", tag = "\([^"]*\)".*/\1/p' "$root/crates/audio/Cargo.toml" | head -n1)"
[[ -n "$tag" ]] || { echo "no IDN tag in crates/audio/Cargo.toml" >&2; exit 1; }
# The commit Vesper's own build uses for that tag: the clone must be the same.
commit="$(grep -o "idn-open-protocol?tag=${tag}#[0-9a-f]*" "$root/Cargo.lock" | head -n1 | sed 's/.*#//')"
[[ -n "$commit" ]] || { echo "no commit for $tag in Cargo.lock" >&2; exit 1; }

src="$root/packaging/downloads/idn-src-$tag"
if [[ ! -d "$src/.git" ]]; then
  rm -rf -- "$src"
  mkdir -p "$root/packaging/downloads"
  git -c advice.detachedHead=false clone -q --depth 1 --branch "$tag" "$repo" "$src"
fi
head="$(git -C "$src" rev-parse HEAD)"
[[ "$head" == "$commit" ]] || {
  echo "$src is at $head, but Cargo.lock pins $tag to $commit; delete the folder to clone again" >&2
  exit 1
}
idn_version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$src/media/idn-node/Cargo.toml" | head -n1)"

idn_target="${IDN_TARGET_DIR:-$root/packaging/downloads/idn-target}"
export CARGO_TARGET_DIR="$idn_target"
args=(--release --locked)
if [[ -n "$target" ]]; then
  args+=(--target "$target" --bins)
  # IDN's .cargo/config.toml sets CC/CXX and the bindgen target for the GNU target; the
  # linker and the static C/C++ runtimes are as for Vesper's own Windows build.
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER:-x86_64-w64-mingw32-gcc}"
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="${CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS:--C link-arg=-static-libgcc -C link-arg=-static-libstdc++ -C link-arg=-Wl,-Bstatic -C link-arg=-lwinpthread -C link-arg=-Wl,-Bdynamic}"
  # The Vulkan headers alone (not the rest of /usr/include, whose glibc headers would shadow
  # MinGW's): a folder with links to the system's vulkan/ and vk_video/.
  if [[ -z "${Vulkan_INCLUDE_DIR:-}" ]]; then
    Vulkan_INCLUDE_DIR="$root/packaging/downloads/vulkan-include"
    mkdir -p "$Vulkan_INCLUDE_DIR"
    for d in vulkan vk_video; do
      [[ -d "/usr/include/$d" ]] && ln -sfn "/usr/include/$d" "$Vulkan_INCLUDE_DIR/$d"
    done
    [[ -f "$Vulkan_INCLUDE_DIR/vulkan/vulkan.h" ]] || { echo "the Vulkan headers are needed (apt install libvulkan-dev)" >&2; exit 1; }
  fi
  export Vulkan_INCLUDE_DIR
  command -v glslc > /dev/null || { echo "glslc is needed (apt install glslc)" >&2; exit 1; }
fi
if [[ $build == 1 ]]; then
  (cd "$src" && cargo build "${args[@]}" -p idn-node -p idn-cli)
  (cd "$src" && cargo build "${args[@]}" -p idn-display)
fi
bindir="$idn_target/${target:+$target/}release"
programs=(idn-node idn-session idn idn-display)
for p in "${programs[@]}"; do
  [[ -f "$bindir/$p$exe" ]] || { echo "missing $bindir/$p$exe (build first)" >&2; exit 1; }
done

out="$root/packaging/out/idn-node-$platform"
rm -rf -- "$out"
mkdir -p "$out/licenses"
for p in "${programs[@]}"; do
  install -m 0755 "$bindir/$p$exe" "$out/$p$exe"
  if [[ -n "$target" ]]; then x86_64-w64-mingw32-strip "$out/$p$exe"; else strip --strip-unneeded "$out/$p$exe"; fi
done
printf '%s\n' "IDN Open Protocol (compatible with NDI®) $idn_version, release $tag ($commit)" \
  "Built from $repo at that tag by Vesper's packaging/idn-node/build.sh." > "$out/VERSION.txt"

# Licences: the IDN workspace's own (MIT OR Apache-2.0, © ADC Industries), its notes on what
# it builds in, and a per-crate list of every crate compiled into these four programs, from
# cargo's metadata (name, version, licence expression, source). cargo-about / cargo-deny would
# add each crate's full licence text; they aren't installed on the build machine, so the list
# names the licences and the texts of the ones that aren't MIT/Apache are still to be added.
install -m 0644 "$src/LICENSE-MIT" "$out/licenses/IDN-LICENSE-MIT.txt"
install -m 0644 "$src/LICENSE-APACHE" "$out/licenses/IDN-LICENSE-APACHE.txt"
metadata="$(mktemp)"
trap 'rm -f -- "$metadata"' EXIT
(cd "$src" && cargo metadata --format-version 1 --locked \
  --filter-platform "${target:-x86_64-unknown-linux-gnu}" > "$metadata")
python3 -I - "$metadata" "$out/licenses/THIRD-PARTY-CRATES.txt" "$tag" <<'PY'
import json, sys
meta = json.load(open(sys.argv[1]))
pkgs = {p["id"]: p for p in meta["packages"]}
nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
roots = [p["id"] for p in meta["packages"] if p["name"] in ("idn-node", "idn-cli", "idn-display") and p["source"] is None]
seen, todo = set(), list(roots)
while todo:
    i = todo.pop()
    if i in seen:
        continue
    seen.add(i)
    for d in nodes[i]["deps"]:
        # Normal (run-time) dependencies only: build scripts and tests don't ship.
        if any(k.get("kind") in (None, "normal") for k in d["dep_kinds"]):
            todo.append(d["pkg"])
rows = sorted((pkgs[i]["name"], pkgs[i]["version"], pkgs[i].get("license") or "see its files",
               "IDN workspace" if pkgs[i]["source"] is None else (pkgs[i]["source"] or "").split("+")[0]) for i in seen)
with open(sys.argv[2], "w") as f:
    f.write(f"Crates compiled into idn-node, idn-session, idn and idn-display (IDN {sys.argv[3]}).\n")
    f.write("Generated from cargo metadata: crate, version, licence (SPDX expression), source.\n")
    f.write("Not MIT/Apache-2.0 among them are listed by their own expression below.\n\n")
    w = max(len(r[0]) for r in rows)
    for name, ver, lic, srcname in rows:
        f.write(f"{name:<{w}}  {ver:<12}  {lic}  ({srcname})\n")
    other = sorted({r[2] for r in rows if r[2] not in ("MIT OR Apache-2.0", "Apache-2.0 OR MIT", "MIT/Apache-2.0", "MIT", "Apache-2.0")})
    f.write("\nOther licence expressions present:\n")
    for o in other:
        f.write(f"  {o}: " + ", ".join(r[0] for r in rows if r[2] == o) + "\n")
print(f"{len(rows)} crates listed", file=sys.stderr)
PY
cat > "$out/licenses/IDN-NOTICE.txt" <<EOF
IDN Open Protocol (compatible with NDI®), release $tag
Copyright ADC Industries. Licensed under the MIT License or the Apache License 2.0, at your
option (IDN-LICENSE-MIT.txt, IDN-LICENSE-APACHE.txt). Source: $repo (tag $tag).

These programs (idn-node, idn-session, idn, idn-display) contain no GPL code and no FFmpeg.
On Linux they use the system's ALSA library (libasound, LGPL-2.1) dynamically.
Third-party crates compiled in, each under its own licence: THIRD-PARTY-CRATES.txt. Among
them are licences other than MIT/Apache-2.0 (for example BSD-3-Clause, ISC, Unlicense,
Unicode-3.0, CDLA-Permissive-2.0); their full texts are in each crate's source.

NDI® is a registered trademark of Vizrt NDI AB. Neither Vesper nor IDN Open Protocol is
affiliated with or endorsed by Vizrt.
EOF
echo "$out"
