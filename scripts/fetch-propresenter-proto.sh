#!/usr/bin/env bash
# Downloads the ProPresenter 7 protobuf schemas pro-import is built from (community
# reverse-engineered, MIT: github.com/greyshirtguy/ProPresenter7-Proto, folder autogen-proto,
# at a pinned commit), checks every file against its pinned SHA-256
# (scripts/propresenter-proto.sha256) and puts them where crates/pro-import/build.rs reads them.
# They are not in this repository (crates/pro-import/PROTO.md).
#
#   scripts/fetch-propresenter-proto.sh
#
# Into $PROPRESENTER_PROTO_DIR (default target/propresenter-proto/; build.rs reads the same
# variable). Nothing is downloaded when the folder already holds every pinned file. A file
# that doesn't match its pin stops the script (nothing is changed then).
# Needs: curl, tar, sha256sum.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo="https://github.com/greyshirtguy/ProPresenter7-Proto"
commit="1c019ab1897286c9bc611f0a10f9f7da8d01dcbb"
license_sha="2745fe0fbee80bb3fdeceb36390576ada0c3701337fe0ddc08c930b80089b0e5"
pins="$root/scripts/propresenter-proto.sha256"
out="${PROPRESENTER_PROTO_DIR:-$root/target/propresenter-proto}"

# The pinned files' lines (comments left out).
pinned() { grep -v '^#' "$pins"; }
# Whether folder $1 holds every pinned file, as pinned, and the licence.
complete() {
  [[ -d "$1" ]] && (cd "$1" && pinned | sha256sum --quiet --strict -c - > /dev/null 2>&1) \
    && echo "$license_sha  $1/LICENSE-ProPresenter7-Proto" | sha256sum --quiet -c - > /dev/null 2>&1
}

if complete "$out"; then
  echo "ProPresenter 7 schemas already in $out"
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
archive="$work/schemas.tar.gz"
curl -fsSL --retry 2 --connect-timeout 20 -o "$archive" "$repo/archive/$commit.tar.gz" || {
  echo "could not download $repo at $commit" >&2
  exit 1
}
mkdir "$work/x"
tar -xzf "$archive" -C "$work/x" --strip-components=1 --wildcards \
  "*/autogen-proto/*" "*/LICENSE"
src="$work/x/autogen-proto"
if ! (cd "$src" && pinned | sha256sum --quiet --strict -c -); then
  echo "$repo at $commit: the files above don't match their pins in $pins; nothing changed" >&2
  exit 1
fi
got="$(sha256sum "$work/x/LICENSE" | cut -d' ' -f1)"
[[ "$got" == "$license_sha" ]] || {
  echo "$repo at $commit: LICENSE doesn't match its pin (SHA-256 $got); nothing changed" >&2
  exit 1
}

# Only the pinned files, whole: assembled beside the folder, then put in its place.
new="$out.new"
rm -rf -- "$new"
mkdir -p "$new"
pinned | while read -r _ file; do
  mkdir -p "$new/$(dirname "$file")"
  cp -- "$src/$file" "$new/$file"
done
cp -- "$work/x/LICENSE" "$new/LICENSE-ProPresenter7-Proto"
printf '%s\n' "ProPresenter 7 schemas from $repo (autogen-proto) at commit $commit." \
  "MIT licensed: LICENSE-ProPresenter7-Proto. Written by scripts/fetch-propresenter-proto.sh." \
  > "$new/SOURCE.txt"
complete "$new" || { echo "the files copied to $new don't match their pins" >&2; exit 1; }
rm -rf -- "$out"
mv -- "$new" "$out"
echo "ProPresenter 7 schemas in $out ($(pinned | wc -l) files)"
