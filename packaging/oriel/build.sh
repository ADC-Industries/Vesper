#!/usr/bin/env bash
# Builds Vesper Oriel's installers with Tauri's bundler into packaging/out/:
#
#   packaging/oriel/build.sh linux     # .deb and .AppImage (amd64)
#   packaging/oriel/build.sh windows   # NSIS setup .exe, cross-built (x86_64-pc-windows-gnu)
#
# Linux needs the WebKitGTK development packages (apps/control/README.md, "Desktop app").
# Windows needs rustup's x86_64-pc-windows-gnu target, gcc/g++-mingw-w64-x86-64 and nsis;
# Tauri's bundler downloads its NSIS plugins on the first run.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
app="$root/apps/control"
out="$root/packaging/out"
# The Tauri shell is outside the root workspace: give it its own target folder.
target_dir="${ORIEL_TARGET_DIR:-${CARGO_TARGET_DIR:-$root/target}/oriel}"
export CARGO_TARGET_DIR="$target_dir"

cd "$app"
[[ -d node_modules ]] || npm ci --prefer-offline
mkdir -p "$out"

case "${1:-}" in
  linux)
    npx tauri build --bundles deb,appimage -- --locked
    cp "$target_dir"/release/bundle/deb/*.deb "$target_dir"/release/bundle/appimage/*.AppImage "$out/"
    ;;
  windows)
    t=x86_64-pc-windows-gnu
    export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc-posix
    export CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++-posix
    export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc-posix
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C link-arg=-static-libgcc -C link-arg=-static-libstdc++ -C link-arg=-Wl,-Bstatic -C link-arg=-lwinpthread -C link-arg=-Wl,-Bdynamic"
    npx tauri build --target "$t" --bundles nsis -- --locked
    cp "$target_dir/$t"/release/bundle/nsis/*.exe "$out/"
    ;;
  *)
    echo "usage: $0 linux|windows" >&2
    exit 2
    ;;
esac
ls -l "$out"
