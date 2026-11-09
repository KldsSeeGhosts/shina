#!/bin/sh
# Builds a complete Shina.app without installing it, for a GitHub release's macOS asset.
#
#   sh tools/package-macos.sh [prebuilt-mac-binary]
#
# Writes target/release-package/Shina-macos-<arch>.zip, holding Shina.app. The release
# publishes that zip and a SHA256SUMS file beside shina.exe.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ "$#" -gt 1 ]; then
    printf '%s\n' "Usage: $0 [prebuilt-mac-binary]" >&2
    exit 2
fi
if [ "$#" -eq 0 ]; then
    (cd "$root" && cargo build --release)
    binary="$root/target/release/shina"
else
    binary=$1
fi
if [ ! -x "$binary" ]; then
    printf 'No executable at %s\n' "$binary" >&2
    exit 1
fi
version=$(awk -F '"' '/^version = / { print $2; exit }' "$root/Cargo.toml")
revision=${SHINA_SOURCE_REVISION:-$(git -C "$root" rev-parse HEAD)}
case $(uname -m) in
    arm64) arch=aarch64 ;;
    x86_64) arch=x86_64 ;;
    *)
        printf 'Unsupported Mac architecture: %s\n' "$(uname -m)" >&2
        exit 1
        ;;
esac
out="$root/target/release-package"
mkdir -p "$out"
# Work on a private sibling folder, then move the finished zip into place, so a failed run
# never leaves a half-built asset behind.
work=$(mktemp -d "$out/.shina-package.XXXXXX")
trap 'rm -rf "$work"' EXIT
stage="$work/stage"
mkdir -p "$stage"
# The bundle construction and its ad-hoc signing live in the installer script.
sh "$root/tools/install-macos.sh" --bundle-only "$stage" "$binary" "$version" "$revision"
asset="$out/Shina-macos-$arch.zip"
rm -f "$asset"
(cd "$stage" && /usr/bin/ditto -c -k --keepParent Shina.app "$asset")
/usr/bin/codesign --verify --deep --strict "$stage/Shina.app"
(cd "$out" && shasum -a 256 "$(basename "$asset")" > SHA256SUMS)
printf 'Packaged Shina %s for %s: %s\n' "$version" "$arch" "$asset"
printf 'Add this line to the release SHA256SUMS, then upload both files.\n'
