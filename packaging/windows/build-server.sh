#!/usr/bin/env bash
# Cross-builds the Vesper server for Windows (x86_64-pc-windows-gnu, MinGW-w64) on Linux and
# packs its installer: packaging/out/vesper-server-<version>-setup.exe.
#
#   packaging/windows/build-server.sh [--no-build]
#
# Needs: rustup target x86_64-pc-windows-gnu, gcc-mingw-w64-x86-64, g++-mingw-w64-x86-64,
# clang (bindgen), nsis, npm, curl. FFmpeg: the pinned LGPL shared build (fetch-ffmpeg.sh); the
# ProPresenter 7 schemas: scripts/fetch-propresenter-proto.sh.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
here="$root/packaging/windows"
target=x86_64-pc-windows-gnu
target_dir="${CARGO_TARGET_DIR:-$root/target}"
build=1
for arg in "$@"; do
  case "$arg" in
    --no-build) build=0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

"$here/fetch-ffmpeg.sh"
ffmpeg="$root/packaging/downloads/ffmpeg-win64"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/vesper-server/Cargo.toml" | head -n1)"

if [[ $build == 1 ]]; then
  # FFmpeg: link the pinned build's import libraries (lib/), bindings from its headers.
  export FFMPEG_DIR="$ffmpeg"
  # C/C++ parts (cpal, wgpu's helpers, …): MinGW-w64 with POSIX threads.
  export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc-posix
  export CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++-posix
  export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc-posix
  # The C and C++ runtimes linked in statically: no libgcc/libstdc++/libwinpthread DLLs to ship.
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C link-arg=-static-libgcc -C link-arg=-static-libstdc++ -C link-arg=-Wl,-Bstatic -C link-arg=-lwinpthread -C link-arg=-Wl,-Bdynamic"
  # Bindings: MinGW's <windows.h> pulls in clang's <immintrin.h>, whose AMX tile types
  # (packed and 64-byte aligned) bindgen can't lay out in Rust. FFmpeg's API doesn't use
  # them, so their headers are skipped by defining their include guards.
  amx_guards="$(grep -l '_tile1024i' "$(clang -print-resource-dir)"/include/*.h \
    | xargs -r grep -h -m1 -o '^#define __[A-Z0-9_]*_H\b' | sed 's/^#define /-D/' | tr '\n' ' ')"
  export BINDGEN_EXTRA_CLANG_ARGS_x86_64_pc_windows_gnu="--target=x86_64-w64-mingw32 $amx_guards"
  # Live captions (whisper.cpp): ggml names its static libraries ggml.a on Windows, rustc
  # looks for libggml.a; this puts the prefix back (see the file).
  export CMAKE_PROJECT_ggml_INCLUDE="$here/ggml-lib-prefix.cmake"
  # A cross-build can't tune whisper.cpp for the build machine's CPU (GGML_NATIVE), so it would
  # be baseline x86-64 (SSE2) and captions slow. AVX2 (with FMA and F16C: Intel Haswell / AMD
  # Excavator, 2013–2015, and later) is the owner's choice (2026-11-25): such a PC is the least
  # that keeps up with live captions anyway; older CPUs can't run this Windows server.
  ggml_flags=(GGML_NATIVE=OFF GGML_AVX=ON GGML_AVX2=ON GGML_FMA=ON GGML_F16C=ON)
  for f in "${ggml_flags[@]}"; do export "${f?}"; done
  # whisper-rs-sys doesn't rerun its build for a changed variable: a cache from a build without
  # the prefix (ggml.a) or with other ggml flags is rebuilt.
  stamp="$target_dir/$target/release/.vesper-ggml-flags"
  if compgen -G "$target_dir/$target/release/build/whisper-rs-sys-*/out/lib/ggml.a" > /dev/null \
    || [[ "$(cat "$stamp" 2>/dev/null)" != "${ggml_flags[*]}" ]]; then
    (cd "$root" && cargo clean --release --target "$target" -p whisper-rs-sys)
  fi
  mkdir -p "$(dirname "$stamp")" && printf '%s' "${ggml_flags[*]}" > "$stamp"
  # The ProPresenter 7 schemas the server's importer is built from (not in the repository).
  "$root/scripts/fetch-propresenter-proto.sh"
  (cd "$root" && cargo build --release --locked --target "$target" -p vesper-server --bin vesper)
  (cd "$root/apps/control" && { [[ -d node_modules ]] || npm ci --prefer-offline; } && npm run build)
  [[ -n "${VESPER_SCRIPTURE_DIR:-}" ]] || "$root/scripts/fetch-scripture.sh"
fi
exe="$target_dir/$target/release/vesper.exe"
ui="$root/apps/control/dist"
# The bundled Bible translations (scripts/fetch-scripture.sh).
scripture="${VESPER_SCRIPTURE_DIR:-$target_dir/scripture}"
[[ -f "$exe" ]] || { echo "missing $exe (build first)" >&2; exit 1; }
[[ -f "$ui/index.html" ]] || { echo "missing $ui (npm run build in apps/control)" >&2; exit 1; }
compgen -G "$scripture/*.vbible" > /dev/null \
  || { echo "no Bible translations in $scripture (scripts/fetch-scripture.sh)" >&2; exit 1; }

out="$root/packaging/out"
stage="$out/windows-server"
rm -rf -- "$stage"
mkdir -p "$stage/licenses"
cp "$exe" "$stage/vesper.exe"
x86_64-w64-mingw32-strip "$stage/vesper.exe"
# FFmpeg's libraries. Not its ffmpeg and ffprobe tools: media preparation probes and converts
# in the server through IDN's FFmpeg adapter on these DLLs (IDN Beta14, docs/MEDIA-PREP-PLAN.md).
cp "$ffmpeg"/bin/av*.dll "$ffmpeg"/bin/sw*.dll "$stage/"
cp -r "$ui" "$stage/ui"
# Next to ui\, where vesper.exe looks by default (--scripture-dir).
mkdir -p "$stage/scripture"
cp "$scripture"/*.vbible "$stage/scripture/"
[[ -f "$scripture/SOURCES.txt" ]] && cp "$scripture/SOURCES.txt" "$stage/licenses/SCRIPTURE-SOURCES.txt"
cp "$root/apps/control/src-tauri/icons/icon.ico" "$stage/vesper.ico"
cp "$here/vesper-task.ps1" "$stage/"
mkdir -p "$stage/launcher.d"
cp "$here/launcher.d/vesper.json" "$stage/launcher.d/"
cp "$root/COPYRIGHT" "$stage/licenses/COPYRIGHT.txt"
cp "$root/LICENSE-MIT" "$stage/licenses/LICENSE-MIT.txt"
cp "$root/LICENSE-APACHE" "$stage/licenses/LICENSE-APACHE.txt"
# The ProPresenter 7 schemas compiled into the importer: their source and MIT licence.
cp "$root/crates/pro-import/PROTO.md" "$stage/licenses/PROPRESENTER-SCHEMAS.txt"
cp "$ffmpeg/LICENSE.txt" "$stage/licenses/FFMPEG-LICENSE.txt"
cp "$here/FFMPEG-NOTICE.txt" "$stage/licenses/FFMPEG-NOTICE.txt"
{
  cat "$root/COPYRIGHT"
  printf '\n\n'
  cat "$here/FFMPEG-NOTICE.txt"
} > "$stage/licenses/LICENSES.txt"

# Every DLL vesper.exe imports must be Windows' own or shipped next to it.
missing=0
while read -r dll; do
  lower="${dll,,}"
  if [[ -f "$stage/$dll" ]]; then continue; fi
  case "$lower" in
    api-ms-win-*|kernel32.dll|user32.dll|gdi32.dll|advapi32.dll|ws2_32.dll|ntdll.dll|\
    bcrypt.dll|bcryptprimitives.dll|crypt32.dll|secur32.dll|ole32.dll|oleaut32.dll|shell32.dll|\
    userenv.dll|iphlpapi.dll|msvcrt.dll|ucrtbase.dll|dxgi.dll|d3d12.dll|d3dcompiler_47.dll|\
    opengl32.dll|winmm.dll|avrt.dll|propsys.dll|mmdevapi.dll|dwmapi.dll|uxtheme.dll|\
    setupapi.dll|cfgmgr32.dll|powrprof.dll|version.dll|shlwapi.dll|netapi32.dll|dnsapi.dll|\
    mswsock.dll|ncrypt.dll|rpcrt4.dll|synchronization.dll|comctl32.dll|imm32.dll|winhttp.dll|\
    hid.dll|dbghelp.dll|psapi.dll|wtsapi32.dll|mfplat.dll|mf.dll|mfreadwrite.dll|ksuser.dll|\
    combase.dll|pdh.dll)
      ;;
    *) echo "vesper.exe imports $dll, which is neither shipped nor part of Windows" >&2; missing=1 ;;
  esac
done < <(x86_64-w64-mingw32-objdump -p "$stage/vesper.exe" | sed -n 's/^\s*DLL Name: //p' | sort -u)
[[ $missing == 0 ]] || exit 1

setup="$out/vesper-server-${version}-setup.exe"
makensis -V2 -DVERSION="$version" -DSTAGE="$stage" -DOUTFILE="$setup" "$here/vesper-server.nsi"
echo "$setup"
