#!/bin/sh
# Build locally, or pass a matching prebuilt Mac binary as the only argument.
#
# The packaging script and the updater also run this script in bundle-only mode, which builds
# and signs Shina.app in a folder without installing it:
#   sh tools/install-macos.sh --bundle-only <folder> <binary> <version> <revision>
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

# The complete bundle: executable and launcher, version and source revision, icon, ad-hoc
# signature. The updater validates exactly these before it replaces the installed app.
build_bundle() {
    app=$1
    binary=$2
    version=$3
    revision=$4
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$binary" "$app/Contents/MacOS/shina"
    cp "$root/assets/macos/launch-shina" "$app/Contents/MacOS/launch-shina"
    cp "$root/assets/macos/Info.plist" "$app/Contents/Info.plist"
    chmod 755 "$app/Contents/MacOS/shina" "$app/Contents/MacOS/launch-shina"
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :ShinaSourceRevision $revision" "$app/Contents/Info.plist"
    iconset=$(dirname "$app")/Shina.iconset
    mkdir "$iconset"
    for size in 16 32 128 256 512; do
        doubled=$((size * 2))
        /usr/bin/sips -z "$size" "$size" "$root/assets/project-icon.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
        /usr/bin/sips -z "$doubled" "$doubled" "$root/assets/project-icon.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
    done
    /usr/bin/iconutil -c icns "$iconset" -o "$app/Contents/Resources/Shina.icns"
    for size in 16 32 128 256 512; do
        /bin/rm "$iconset/icon_${size}x${size}.png" "$iconset/icon_${size}x${size}@2x.png"
    done
    rmdir "$iconset"
    /usr/bin/codesign --force --deep --sign - "$app"
    /usr/bin/codesign --verify --deep --strict "$app"
}

if [ "${1:-}" = "--bundle-only" ]; then
    if [ "$#" -ne 5 ]; then
        printf 'Usage: %s --bundle-only <folder> <binary> <version> <revision>\n' "$0" >&2
        exit 2
    fi
    folder=$2
    binary=$3
    version=$4
    revision=$5
    if [ ! -x "$binary" ]; then
        printf 'No executable at %s\n' "$binary" >&2
        exit 1
    fi
    if [ ! -d "$folder" ]; then
        printf 'No folder at %s\n' "$folder" >&2
        exit 1
    fi
    stage=$(mktemp -d "$folder/.Shina-bundle.XXXXXX")
    trap 'rmdir "$stage" 2>/dev/null || true' EXIT
    app="$stage/Shina.app"
    build_bundle "$app" "$binary" "$version" "$revision"
    rm -rf "$folder/Shina.app"
    mv "$app" "$folder/Shina.app"
    printf 'Built %s\n' "$folder/Shina.app"
    exit 0
fi

if [ "$#" -gt 1 ]; then
    printf '%s\n' "Usage: $0 [prebuilt-mac-binary]" >&2
    exit 2
fi

# The complete bundle: executable and launcher, version and source revision, icon, ad-hoc
# signature. The updater validates exactly these before it replaces the installed app.
build_bundle() {
    app=$1
    binary=$2
    version=$3
    revision=$4
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$binary" "$app/Contents/MacOS/shina"
    cp "$root/assets/macos/launch-shina" "$app/Contents/MacOS/launch-shina"
    cp "$root/assets/macos/Info.plist" "$app/Contents/Info.plist"
    chmod 755 "$app/Contents/MacOS/shina" "$app/Contents/MacOS/launch-shina"
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :ShinaSourceRevision $revision" "$app/Contents/Info.plist"
    iconset=$(dirname "$app")/Shina.iconset
    mkdir "$iconset"
    for size in 16 32 128 256 512; do
        doubled=$((size * 2))
        /usr/bin/sips -z "$size" "$size" "$root/assets/project-icon.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
        /usr/bin/sips -z "$doubled" "$doubled" "$root/assets/project-icon.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
    done
    /usr/bin/iconutil -c icns "$iconset" -o "$app/Contents/Resources/Shina.icns"
    for size in 16 32 128 256 512; do
        /bin/rm "$iconset/icon_${size}x${size}.png" "$iconset/icon_${size}x${size}@2x.png"
    done
    rmdir "$iconset"
    /usr/bin/codesign --force --deep --sign - "$app"
    /usr/bin/codesign --verify --deep --strict "$app"
}

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
destination=${SHINA_MACOS_APP_DIR:-/Applications}
if [ ! -d "$destination" ] || [ ! -w "$destination" ]; then
    printf 'Choose a writable applications folder with SHINA_MACOS_APP_DIR: %s\n' "$destination" >&2
    exit 1
fi
version=$(awk -F '"' '/^version = / { print $2; exit }' "$root/Cargo.toml")
revision=${SHINA_SOURCE_REVISION:-$(git -C "$root" rev-parse HEAD)}
stage=$(mktemp -d "$destination/.Shina-install.XXXXXX")
# Only remove our empty staging folder. On failure, retain partial artifacts for inspection.
trap 'rmdir "$stage" 2>/dev/null || true' EXIT
app="$stage/Shina.app"
build_bundle "$app" "$binary" "$version" "$revision"
target="$destination/Shina.app"
if [ -e "$target" ]; then
    # The updater already keeps the replaced build at Shina-app.previous. Otherwise retain a
    # dated backup for recovery.
    if [ ! -e "$destination/Shina-app.previous" ]; then
        backups="$HOME/Library/Application Support/Shina/App Backups"
        mkdir -p "$backups"
        backup="$backups/Shina-$(date +%Y%m%dT%H%M%S)-$$.app"
        mv "$target" "$backup"
        printf 'Previous app retained at %s\n' "$backup"
    else
        rm -rf "$target"
    fi
fi
mv "$app" "$target"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$target"
printf 'Installed Shina %s at %s, source %s\n' "$version" "$target" "$revision"
