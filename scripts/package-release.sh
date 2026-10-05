#!/bin/sh
# Package a built binary for a target into dist/ (used by the release workflow).
#   scripts/package-release.sh <version> <target> <path-to-binary>
set -eu
version="${1:?version}"
target="${2:?target}"
bin="${3:?binary}"
root="$(cd "$(dirname "$0")/.." && pwd)"
name="kara-v$version-$target"
stage="$root/dist/$name"
rm -rf "$stage"
mkdir -p "$stage"
cp "$bin" "$stage/"
cp "$root/README.md" "$root/LICENSE" "$root/NOTICE" "$root/CHANGELOG.md" "$stage/"
cd "$root/dist"
case "$target" in
  # Both formats contain a single top-level folder named like the archive.
  *windows*) 7z a -tzip "$name.zip" "$name" >/dev/null 2>&1 || powershell -NoProfile -Command "Compress-Archive -Path '$name' -DestinationPath '$name.zip'" ;;
  *) tar -czf "$name.tar.gz" "$name" ;;
esac
rm -rf "$stage"
ls -l "$root/dist"
