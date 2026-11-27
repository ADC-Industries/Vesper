#!/usr/bin/env bash
# Downloads the pinned LGPL-only, shared FFmpeg build for the Windows server and unpacks it
# into packaging/downloads/ffmpeg-win64/ (gitignored, never committed).
#
# Source: BtbN's FFmpeg-Builds (https://github.com/BtbN/FFmpeg-Builds), "lgpl-shared" variant:
# no GPL components, DLLs plus import libraries and headers. The pin is a month-end
# autobuild, which that project keeps for about two years (daily builds are pruned after two
# weeks). To move the pin: pick a newer month-end autobuild's `win64-lgpl-shared-<major>.zip`,
# put its URL and SHA-256 (shown on the release page and by
# `gh api repos/BtbN/FFmpeg-Builds/releases/tags/<tag>`) below, keep the FFmpeg major that
# the server's `ffmpeg-sys-next` supports (the same major as Ubuntu's: libavcodec 62 = FFmpeg 8).
set -euo pipefail

URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-10-31-12-59/ffmpeg-n8.1.3-27-g8dca1586f4-win64-lgpl-shared-8.1.zip"
SHA256="5cef7bc48e513f3cee7c881e8b2999d835541edb1846c3e89443a67f8f8556ab"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
downloads="$root/packaging/downloads"
zip="$downloads/$(basename "$URL")"
dest="$downloads/ffmpeg-win64"

mkdir -p "$downloads"
if [[ ! -f "$zip" ]] || ! echo "$SHA256  $zip" | sha256sum -c --status; then
  echo "Downloading $(basename "$URL")"
  curl -fsSL --retry 3 -o "$zip.part" "$URL"
  echo "$SHA256  $zip.part" | sha256sum -c -
  mv "$zip.part" "$zip"
fi

if [[ ! -f "$dest/.pin" ]] || [[ "$(cat "$dest/.pin")" != "$SHA256" ]]; then
  tmp="$downloads/ffmpeg-win64.unpack"
  rm -rf -- "$tmp"
  mkdir -p "$tmp"
  (cd "$tmp" && python3 -I -m zipfile -e "$zip" .)
  rm -rf -- "$dest"
  # The zip holds one top folder (ffmpeg-n8.1…-lgpl-shared-8.1/) with bin/ include/ lib/.
  mv "$tmp"/*/ "$dest"
  rm -rf -- "$tmp"
  echo "$SHA256" > "$dest/.pin"
fi

# Refuse a build with GPL parts (the configure line is in every library's licence string).
if grep -a -q -- '--enable-gpl' "$dest"/bin/avcodec-*.dll; then
  echo "error: $dest is a GPL build" >&2
  exit 1
fi
echo "FFmpeg for Windows: $dest"
