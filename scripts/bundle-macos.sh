#!/bin/sh
# Copy an already built executable; never installs, launches or changes defaults.
set -eu
usage() {
    echo "Usage: $0 [release|debug] [absent-output.app]" >&2
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
target=${CARGO_TARGET_DIR:-"$root/target"}
case "$target" in /*) ;; *) target="$root/$target" ;; esac
if [ -n "${CARGO_BUILD_TARGET:-}" ]; then target="$target/$CARGO_BUILD_TARGET"; fi
binary="$target/$profile/ulos-desktop"
[ -f "$binary" ] && [ -x "$binary" ] || { echo "Build the selected desktop profile first: $binary" >&2; exit 1; }
[ ! -e "$output" ] && [ ! -L "$output" ] || { echo 'Output exists (no overwrite).' >&2; exit 1; }
version=$(cd "$root" && python3 scripts/release-version.py)
[ -f "$root/assets/branding/ulos.icns" ] || { echo 'Missing assets/branding/ulos.icns' >&2; exit 1; }
plutil -lint "$root/packaging/macos/Info.plist" >/dev/null
mkdir -p -- "$(dirname -- "$output")"
# mkdir atomically reserves an absent destination, including against symlinks.
mkdir -- "$output"
mkdir -- "$output/Contents" "$output/Contents/MacOS" "$output/Contents/Resources"
cp -- "$binary" "$output/Contents/MacOS/Ulos"
cp -- "$root/packaging/macos/Info.plist" "$output/Contents/Info.plist"
cp -- "$root/assets/branding/ulos.icns" "$output/Contents/Resources/Ulos.icns"
cp -- "$root/LICENSE" "$root/THIRD_PARTY_NOTICES.md" "$output/Contents/Resources/"
cp -R -- "$root/packaging/licenses" "$output/Contents/Resources/licenses"
# Preserve the authored Icon Composer source even on Xcodes without its compiler.
if [ -d "$root/assets/branding/ulos-osx.icon" ]; then
    cp -R -- "$root/assets/branding/ulos-osx.icon" "$output/Contents/Resources/ulos-osx.icon"
    # The Icon Composer document is retained as a source asset. Ulos.icns is
    # always the declared icon; runner-specific actool behavior cannot silently
    # change the bundle or deployment baseline.

fi
python3 - "$output/Contents/Info.plist" "$version" <<'PY'
import plistlib, re, sys
path, version = sys.argv[1:]
with open(path, 'rb') as f:
    plist = plistlib.load(f)
plist['CFBundleShortVersionString'] = re.match(r'\d+\.\d+\.\d+', version).group()
plist['CFBundleVersion'] = '1'
plist['UlosVersion'] = version
with open(path, 'wb') as f:
    plistlib.dump(plist, f)
PY
plutil -lint "$output/Contents/Info.plist" >/dev/null
printf 'APPL????' > "$output/Contents/PkgInfo"
# Ad-hoc sealing is local integrity metadata, NOT Developer ID or notarization.
codesign --force --sign - "$output"
codesign --verify --deep --strict "$output"
printf 'Created %s (ad-hoc signed, not Developer ID signed or notarized).\n' "$output"
