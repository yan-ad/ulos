# Local opener packaging

These are development packaging aids, not signed/notarized installers or a
verified cross-platform distribution. No magnet scheme is advertised. No custom
icon or unsupported minimum OS baseline is invented. Keep `LICENSE`,
`THIRD_PARTY_NOTICES.md`, and `licenses/Apache-2.0.txt` with redistributed binaries;
the notices are not a full transitive compliance audit.

## macOS

Build the desktop, then run `./scripts/bundle-macos.sh [release|debug] [absent-output.app]`.
The default output is `dist/Ulos.app`. The bundle executable is named **Ulos**,
with the supplied icon, numeric Apple version, full `UlosVersion`, LICENSE and
third-party notices. `CARGO_TARGET_DIR` and `CARGO_BUILD_TARGET` are respected.
Existing outputs (including symlinks) are refused.

The script applies an ad-hoc integrity signature; it is **not Developer ID signed
or notarized**. The authored `.icon` document is retained, and the generated
`.icns` is the declared fallback supported without Icon Composer. No installer,
Launch Services registration or default-handler command runs. To try a local
file, choose Ulos explicitly in Open With; do not select Change All unless intended.
Direct `Ulos.app/Contents/MacOS/Ulos /absolute/file.torrent` uses startup argv.

The imported `org.bittorrent.torrent` UTI conforms to `public.data`, tags the
`torrent` extension and `application/x-bittorrent` MIME, and is declared as Viewer
with Alternate rank, not Owner. This is an existing public torrent identifier,
not a newly claimed Ulos-owned format. No minimum macOS version is set until the
project chooses and verifies its deployment baseline.

## Linux

`linux/org.ulos.desktop.desktop` declares `Exec=Ulos %F` for local files,
so each selected file arrives as one startup argument, including spaces. The
binary must be on the desktop session's PATH. This metadata uses the existing
`application/x-bittorrent` MIME; it does not invent a competing MIME database or
sidebar folder. Users may deliberately copy it to their per-user applications
directory; no script installs it, runs `xdg-mime default`, or updates defaults.
Linux desktop discovery and native launch remain unverified on this macOS host.

## Windows

After placing the binary in a stable local location, deliberately invoke
`packaging/windows/register-open-with.ps1 -ExecutablePath 'C:\\Apps\\Ulos\\ulos-desktop.exe'`.
`-WhatIf` previews the intended registration without registry writes. This
per-user HKCU-only script requires no administrator rights. It adds the
`Ulos.Torrent` ProgID, `.torrent\\OpenWithProgids`, and RegisteredApplications
capabilities. It never writes the extension's default value, UserChoice,
URL schemes, or machine-wide keys. Open With uses a quoted executable and quoted
`"%1"`; one file arrives as one startup argument. Deliberate reruns update only
these Ulos-owned values to the supplied executable. No registration script was
executed during development verification; PowerShell/Windows native behavior
needs Windows validation. To undo, manually remove only `Ulos.Torrent`, its
OpenWithProgids value, `HKCU\\Software\\Ulos\\Capabilities`, and the Ulos
RegisteredApplications value; do not remove shared extension keys or UserChoice.

## Intake integration contract

`intake::argument_paths(std::env::args_os().skip(1))` accepts startup paths,
local `file://` URLs, optional `--`, and ignores legacy macOS `-psn_` tokens.
No startup files returns an empty success; unknown flags fail closed.
`opened_urls(Vec<String>)` handles OS callbacks; `torrent_paths` handles drag/drop
and picker-native PathBufs. Empty picker/callback batches are rejected; callers
should treat picker cancellation separately. All batches cap at 16 raw file
entries before stable PathBuf deduplication; paths/URL text cap at 16 KiB encoded
bytes. argv additionally caps at 18 raw tokens. Native non-UTF8 paths are retained.
Only lexical extension validation occurs: existence, regular-file/symlink policy,
metadata size/parsing and destination consent belong to the worker/UI pipeline.
Remote file URL hosts and UNC/network syntax are rejected; local-looking mounted
paths cannot be classified as remote without filesystem access. Nothing here
persists input URLs, reads metainfo, starts networking or automatically submits a
transfer. Parent UI must present each accepted file for review.

## Association references

- [Apple CFBundleDocumentTypes / roles / handler rank](https://developer.apple.com/library/archive/documentation/General/Reference/InfoPlistKeyReference/Articles/CoreFoundationKeys.html)
- [Apple imported UTI declarations](https://developer.apple.com/documentation/bundleresources/information-property-list/utimportedtypedeclarations)
- [Transmission's existing torrent UTI declaration (reference only; not copied app metadata)](https://github.com/transmission/transmission/blob/main/macosx/Info.plist)
- [freedesktop Exec %F semantics](https://specifications.freedesktop.org/desktop-entry-spec/latest/exec-variables.html)
- [Microsoft OpenWithProgids](https://learn.microsoft.com/en-us/windows/win32/shell/how-to-include-an-application-on-the-open-with-dialog-box)
- [Microsoft registered application capabilities](https://learn.microsoft.com/en-us/windows/win32/shell/default-programs)

## Release workflow

`.github/workflows/release.yml` accepts an existing `v` tag and verifies that its
Cargo version matches. It builds native Apple silicon/Intel macOS DMGs, a Windows
x64 ZIP and an Ubuntu 24.04 x64 portable tar archive, with engine tests on each
platform and full desktop tests/lint on macOS. All builds must succeed before
publication. SHA256SUMS is generated from the four artifacts; publishing uses
`--verify-tag --draft=false --prerelease=false --latest`, including `-pre` names.
The workflow never creates a source commit or tag. See INSTALL.md for GUI steps.
