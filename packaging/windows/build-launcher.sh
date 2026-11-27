#!/usr/bin/env bash
# Cross-builds the Vesper Launcher for Windows (x86_64-pc-windows-gnu, MinGW-w64) on Linux,
# with its tray app and the bundled IDN Node and tools, and packs its installer:
# packaging/out/vesper-launcher-<version>-setup.exe.
#
#   packaging/windows/build-launcher.sh [--no-build]
#
# --no-build packs what is already built (the IDN programs in packaging/out/idn-node-windows,
# from packaging/idn-node/build.sh windows).
# Needs: rustup target x86_64-pc-windows-gnu, gcc-mingw-w64-x86-64, g++-mingw-w64-x86-64, nsis,
# and for the IDN programs what packaging/idn-node/build.sh lists.
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
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/crates/vesper-launcher/Cargo.toml" | head -n1)"

if [[ $build == 1 ]]; then
  # The IDN programs first, with IDN's own cross-compiler settings (its .cargo/config.toml);
  # the settings below are Vesper's.
  "$root/packaging/idn-node/build.sh" windows
  # C/C++ parts: MinGW-w64 with POSIX threads; the C and C++ runtimes linked in statically.
  export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc-posix
  export CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++-posix
  export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc-posix
  export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-C link-arg=-static-libgcc -C link-arg=-static-libstdc++ -C link-arg=-Wl,-Bstatic -C link-arg=-lwinpthread -C link-arg=-Wl,-Bdynamic"
  (cd "$root" && cargo build --release --locked --target "$target" -p vesper-launcher --bin vesper-launcher)
  # (The tray is its own workspace: built into the same target folder, where it is looked for.)
  (cd "$root/apps/launcher-tray/src-tauri" && cargo build --release --locked --target "$target" --target-dir "$target_dir")
fi
rel="$target_dir/$target/release"
idn="$root/packaging/out/idn-node-windows"
for f in "$rel/vesper-launcher.exe" "$rel/vesper-launcher-tray.exe" "$idn/idn-node.exe"; do
  [[ -f "$f" ]] || { echo "missing $f (build first)" >&2; exit 1; }
done
# The WebView2 loader the tray needs next to it (webview2-com-sys's copy for the GNU target).
webview2="$(find "$rel" -path '*webview2-com-sys*' -name WebView2Loader.dll -path '*x64*' | head -n1)"
[[ -n "$webview2" ]] || webview2="$(find "$rel" -name WebView2Loader.dll | head -n1)"
[[ -f "$webview2" ]] || { echo "no WebView2Loader.dll in $rel" >&2; exit 1; }

out="$root/packaging/out"
stage="$out/windows-launcher"
rm -rf -- "$stage"
mkdir -p "$stage/licenses" "$stage/idn-node"
cp "$rel/vesper-launcher.exe" "$rel/vesper-launcher-tray.exe" "$stage/"
x86_64-w64-mingw32-strip "$stage/vesper-launcher.exe" "$stage/vesper-launcher-tray.exe"
cp "$webview2" "$stage/WebView2Loader.dll"
cp "$root/apps/launcher-tray/src-tauri/icons/icon.ico" "$stage/vesper-launcher.ico"
cp "$here/launcher-task.ps1" "$here/launcher.json" "$stage/"
cp "$idn"/*.exe "$stage/idn-node/"
cp -r "$idn/licenses" "$stage/idn-node/"
cp "$idn/VERSION.txt" "$stage/idn-node/"
# Vesper updates this IDN Node (with the Launcher), not the Site Manager (IDN Beta14: the marker
# beside idn-node.exe, first line the product; the Node reports it, the Site Manager leaves the
# machine out of its updates). The uninstaller removes the folder with it.
printf 'Vesper\r\n' > "$stage/idn-node/idn-updates-managed-by"
cp "$root/COPYRIGHT" "$stage/licenses/COPYRIGHT.txt"
cp "$root/LICENSE-MIT" "$stage/licenses/LICENSE-MIT.txt"
cp "$root/LICENSE-APACHE" "$stage/licenses/LICENSE-APACHE.txt"
{
  cat "$root/COPYRIGHT"
  printf '\n\n'
  cat "$idn/licenses/IDN-NOTICE.txt"
} > "$stage/licenses/LICENSES.txt"

# Every DLL the programs import must be Windows' own or shipped next to them.
missing=0
for exe in "$stage"/*.exe "$stage"/idn-node/*.exe; do
  while read -r dll; do
    lower="${dll,,}"
    if [[ -f "$(dirname "$exe")/$dll" ]]; then continue; fi
    case "$lower" in
      api-ms-win-*|kernel32.dll|user32.dll|gdi32.dll|advapi32.dll|ws2_32.dll|ntdll.dll|\
      bcrypt.dll|bcryptprimitives.dll|crypt32.dll|secur32.dll|ole32.dll|oleaut32.dll|shell32.dll|\
      userenv.dll|iphlpapi.dll|msvcrt.dll|ucrtbase.dll|dxgi.dll|d3d12.dll|d3dcompiler_47.dll|\
      opengl32.dll|winmm.dll|avrt.dll|propsys.dll|mmdevapi.dll|dwmapi.dll|uxtheme.dll|\
      setupapi.dll|cfgmgr32.dll|powrprof.dll|version.dll|shlwapi.dll|netapi32.dll|dnsapi.dll|\
      mswsock.dll|ncrypt.dll|rpcrt4.dll|synchronization.dll|comctl32.dll|imm32.dll|winhttp.dll|\
      hid.dll|dbghelp.dll|psapi.dll|wtsapi32.dll|mfplat.dll|mf.dll|mfreadwrite.dll|ksuser.dll|\
      combase.dll|pdh.dll|newdev.dll|mfsensorgroup.dll|xinput1_4.dll)
        ;;
      *) echo "$(basename "$exe") imports $dll, which is neither shipped nor part of Windows" >&2; missing=1 ;;
    esac
  done < <(x86_64-w64-mingw32-objdump -p "$exe" | sed -n 's/^\s*DLL Name: //p' | sort -u)
done
[[ $missing == 0 ]] || exit 1

setup="$out/vesper-launcher-${version}-setup.exe"
makensis -V2 -DVERSION="$version" -DSTAGE="$stage" -DOUTFILE="$setup" "$here/vesper-launcher.nsi"
echo "$setup"
