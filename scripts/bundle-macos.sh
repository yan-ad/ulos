#!/bin/sh
# Local unsigned bundle only. Never installs, registers, launches, or sets defaults.
set -eu
usage() {
    echo "Usage: $0 [release|debug] [absent-output.app]" >&2
    echo "First build: cargo build --locked -p ulos-desktop [--release]" >&2
    exit 2
}
[ "$#" -le 2 ] || usage
profile=${1:-release}
case "$profile" in release|debug) ;; *) usage ;; esac
[ "$(uname -s)" = Darwin ] || { echo 'macOS is required.' >&2; exit 1; }
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${2:-"$root/dist/Ulos.app"}
case "$output" in /*) ;; *) output="$PWD/$output" ;; esac
case "$output" in *.app) ;; *) usage ;; esac
# CARGO_TARGET_DIR may be absolute or relative to the repository root.
target=${CARGO_TARGET_DIR:-"$root/target"}
case "$target" in /*) ;; *) target="$root/$target" ;; esac
binary="$target/$profile/ulos-desktop"
[ -f "$binary" ] && [ -x "$binary" ] || { echo 'Build the selected desktop profile first.' >&2; exit 1; }
[ ! -e "$output" ] && [ ! -L "$output" ] || { echo 'Output exists; choose a new output path (no overwrite).' >&2; exit 1; }
plutil -lint "$root/packaging/macos/Info.plist" >/dev/null
mkdir -p -- "$(dirname -- "$output")"
# mkdir reserves the destination without replacing a concurrent or existing entry.
mkdir -- "$output"
mkdir -- "$output/Contents" "$output/Contents/MacOS" "$output/Contents/Resources"
cp -- "$binary" "$output/Contents/MacOS/ulos-desktop"
cp -- "$root/packaging/macos/Info.plist" "$output/Contents/Info.plist"
cp -- "$root/LICENSE" "$root/THIRD_PARTY_NOTICES.md" "$output/Contents/Resources/"
cp -R -- "$root/packaging/licenses" "$output/Contents/Resources/licenses"
printf 'APPL????' > "$output/Contents/PkgInfo"
printf 'Created local bundle: %s\nNo signing, notarization, installation, or association changes performed.\n' "$output"
