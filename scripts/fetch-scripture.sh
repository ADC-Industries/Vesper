#!/usr/bin/env bash
# Downloads the bundled Bible translations from their public-domain sources, checks each file
# against its pinned SHA-256, and converts them into Vesper's translation format (.vbible).
#
#   scripts/fetch-scripture.sh [<out dir>]
#
# Downloads go to $SCRIPTURE_DOWNLOAD_DIR (default target/scripture-download/), the converted
# files to <out dir> (default $CARGO_TARGET_DIR/scripture, else target/scripture/), with
# SOURCES.txt recording where each text came from and its licence. Neither is committed.
# A source that can't be reached, or whose file no longer matches its pin, is skipped with a
# warning; the others are still converted (exit 0 while at least one translation is there).
# To pick up an updated source, check its new file and change its pin here.
#
# Needs: curl, unzip, sha256sum, cargo (builds vesper-scripture-convert).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
out="${1:-$target_dir/scripture}"
dl="${SCRIPTURE_DOWNLOAD_DIR:-$root/target/scripture-download}"
mkdir -p "$dl" "$out"

# id | url | sha256 | kind (vpl in a zip, or tsv) | name | licence
sources=(
  "KJV|https://ebible.org/Scriptures/eng-kjv_vpl.zip|0017af3b36d1d95b413f2f2092370562f0aea0b7b488585493bb8f94c78d06bc|vpl|King James Version|Public domain (in the United Kingdom, Crown patent)"
  "ASV|https://ebible.org/Scriptures/eng-asv_vpl.zip|6c41b8020cba9cf7041689f2356c55859e28023405537388be08bad93df2b050|vpl|American Standard Version (1901)|Public domain"
  "WEB|https://ebible.org/Scriptures/eng-web_vpl.zip|88308b1c50ec3a536175b29ca96e82602a1f2d7c8ce6aadeb9824a4464f4565c|vpl|World English Bible|Public domain"
  "BSB|https://bereanbible.com/bsb.txt|2ac3af1de52d4e68261cba91d85c320b7eadc6560e830d99e591767b8ff5ca96|tsv|Berean Standard Bible|Public domain (dedicated by its translators, 2023)"
)

(cd "$root" && cargo build -q -p vesper-scripture --bin vesper-scripture-convert)
convert="$target_dir/debug/vesper-scripture-convert"

missing=()
{
  echo "Bundled Bible translations: sources and licences (written by scripts/fetch-scripture.sh)"
  echo
} > "$out/SOURCES.txt.new"
for s in "${sources[@]}"; do
  IFS='|' read -r id url sha kind name license <<< "$s"
  file="$dl/$(basename "$url")"
  if [[ ! -f "$file" ]] || ! echo "$sha  $file" | sha256sum --quiet -c - 2>/dev/null; then
    rm -f -- "$file.part"
    if ! curl -fsSL --retry 2 --connect-timeout 20 -o "$file.part" "$url"; then
      echo "warning: $id: could not download $url; skipped" >&2
      missing+=("$id"); continue
    fi
    got="$(sha256sum "$file.part" | cut -d' ' -f1)"
    if [[ "$got" != "$sha" ]]; then
      echo "warning: $id: $url changed (SHA-256 $got, pinned $sha); skipped. Check it and update the pin." >&2
      rm -f -- "$file.part"
      missing+=("$id"); continue
    fi
    mv -- "$file.part" "$file"
  fi
  text="$dl/$id.txt"
  case "$kind" in
    vpl) unzip -p "$file" '*_vpl.txt' > "$text" ;;
    tsv) cp -- "$file" "$text" ;;
  esac
  "$convert" --kind "$kind" --input "$text" --out "$out" --id "$id" --name "$name" \
    --license "$license" --source "$url"
  printf '%s  %s\n    %s\n    SHA-256 %s\n    %s\n\n' "$id" "$name" "$url" "$sha" "$license" \
    >> "$out/SOURCES.txt.new"
done
mv -- "$out/SOURCES.txt.new" "$out/SOURCES.txt"

if (( ${#missing[@]} )); then
  echo "not bundled this time: ${missing[*]}" >&2
fi
compgen -G "$out/*.vbible" > /dev/null || { echo "no translation converted" >&2; exit 1; }
echo "translations in $out"
