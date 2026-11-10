#!/bin/sh
# A portable x86_64 Linux download. Linux has no in-app installer/updater yet.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary=${1:?Supply the built Linux executable}
if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
    printf '%s\n' "Build this package on x86_64 Linux." >&2
    exit 1
fi
if [ ! -x "$binary" ]; then
    printf 'No executable at %s\n' "$binary" >&2
    exit 1
fi
out="$root/target/release-package"
mkdir -p "$out"
stage=$(mktemp -d "$out/.shina-linux.XXXXXX")
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/Shina"
cp "$binary" "$stage/Shina/shina"
cp "$root/assets/project-icon.png" "$stage/Shina/shina.png"
cp "$root/assets/linux/README.txt" "$stage/Shina/README.txt"
cp "$root/LICENSE" "$stage/Shina/LICENSE"
tar -czf "$out/Shina-linux-x86_64.tar.gz" -C "$stage" Shina
printf 'Packaged %s\n' "$out/Shina-linux-x86_64.tar.gz"
