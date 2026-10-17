# Local opener packaging

These are development packaging aids, not a publisher-signed/notarized or clean-host-verified cross-platform distribution.
Next-build Windows packages use a real per-user Inno Setup installer, described below. No magnet scheme is advertised. No custom
icon or unsupported minimum OS baseline is invented. Keep `LICENSE`,
`THIRD_PARTY_NOTICES.md`, and `licenses/Apache-2.0.txt` with redistributed binaries;
the supplied `licenses/libtorrent-BSD-3-Clause.txt`, `licenses/libtorrent-2.1.2-NOTICES.txt` (full component notices), `licenses/OpenSSL-Apache-2.0.txt`, `licenses/Boost-1.0.txt` and `licenses/nlohmann-json-MIT.txt` must also accompany the native helper/runtime closure. Review additional or changed dependency obligations before distribution; supplied notices are not a full transitive compliance audit.

## Native torrent worker: mandatory next-build payload

Build the desktop and matching native sidecar before packaging. The packager now **fails** if `ulos-torrent-worker(.exe)` is absent; a desktop-only archive is not a complete next-build distribution. The mandatory helper also supplies explicit metadata-only magnet resolution, including for v1. No OS magnet scheme is registered. Worker discovery is not proof that dependencies load. Keep its relocated `lib/` closure with it on macOS/Linux; Windows uses static dependencies/CRT and rejects unexpected non-system DLL imports. Do not distribute the test fixture.

For a development build with explicit dependency-install authorization:

```sh
python3 scripts/build-torrent-worker.py --profile debug --test --install-deps
```

Use `--profile release` for packaging. Dependency installation occurs **only** with `--install-deps`: Homebrew on macOS; apt through sudo on Linux; a pinned local vcpkg checkout on Windows. Without that flag, provide CMake, C++17, libtorrent 2, Boost/OpenSSL and nlohmann-json packages yourself, plus `patchelf` on Linux; use `--cmake-prefix` / `--toolchain` where needed. The flag does not authorize SDK/OS-permission bypass. The build respects `CARGO_TARGET_DIR` and requires `CARGO_BUILD_TARGET`, if set, to match native hardware. `--output` selects the directory containing the desktop executable; its `lib/` is reserved for this helper's managed dependency closure and replaced during staging.

The source uses libtorrent 2 API (2.0.15 baseline; local Homebrew installation 2.1.2), not an embedded v1-client feature switch. macOS/Homebrew and Linux resolve installed system libtorrent 2 packages (Homebrew 2.1.2 on the development host; Linux 2.0.x), not one exact pin across OSes. Windows uses vcpkg baseline `3cd21e48a75f67acfef289da52893ce79144aaa1` (libtorrent 2.1.2), `libtorrent[core,deprfun]` and `x64-windows-static` / `arm64-windows-static`. `deprfun` retains ABI 2; `core` avoids default WebTorrent/libdatachannel dependencies, not BitTorrent v2. Use a native MSVC C++ environment with Windows SDK/CMake already provided. Windows console-helper pipes, UTF-8 argv and `CREATE_NO_WINDOW` are source-implemented; no Windows runtime success is inferred here. No GPL engine is selected, but the BSD engine and transitive runtime licenses remain independent obligations; the provided full license/component-notice files must remain with redistributed packages, and binary copying alone does not establish compliance for changed dependency versions.

`--test` runs CTest plus acceptance against the relocated helper. The separate ignored Rust parent-task tests must also be explicitly run as described in [Testing](../docs/TESTING.md#explicit-native-acceptance-phase-not-results). Bundling uses `@loader_path` on macOS and relative ELF runtime paths on Linux, verifies dependency architecture and keeps platform ABI libraries on the system. Validate relocated binaries on clean native hosts before release.

## macOS

Build the desktop and matching worker/runtime closure, then run `./scripts/bundle-macos.sh [release|debug] [absent-output.app]`.
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

### Setup.exe (next build)

`python scripts/release-package.py windows x86_64 VERSION` (or `arm64`) produces
**`dist/Ulos-VERSION-windows-ARCH-Setup.exe`**, never a fallback ZIP. Existing
published ZIPs remain historical assets. The packager validates desktop/worker PE
headers, stages both binaries and **all** `packaging/licenses/` texts, and requires
the full supplied engine/runtime license set. It generates UTF-8 `OSS-NOTICES.txt`
from `THIRD_PARTY_NOTICES.md` plus all full license/component texts. Inno Setup's
stock `LicenseFile` page shows Ulos's MIT `LICENSE`; its separate scrollable
`InfoBeforeFile` page shows the complete combined OSS notices. The same documents
are installed offline. Supplied notices are not a completed transitive compliance
audit; changes to dependencies still require review.

`packaging/windows/ulos.iss` pins **Inno Setup 6.7.3**, the maintained Inno 6 release
listed on the [official downloads page](https://jrsoftware.org/isdl.php). Inno 7 is
not silently substituted. The preprocessor rejects any other compiler version.
The 32-bit compiler/Setup bootstrap can run under Windows ARM64's x86 emulation;
the **payload** remains native ARM64, using `ArchitecturesAllowed=arm64` **and**
`ArchitecturesInstallIn64BitMode=arm64`. x86-64 uses `x64os` for both, deliberately
requiring native x64 Windows rather than promising emulation compatibility. Do
not use `x64compatible` for the ARM64 payload. See Inno's [architecture identifiers](https://jrsoftware.org/ishelp/topic_archidentifiers.htm)
and [64-bit install mode](https://jrsoftware.org/ishelp/topic_setup_architecturesinstallin64bitmode.htm).

Developer/CI PowerShell commands (explicit tool-install opt-in; no Ulos launch):

```powershell
# From the repository root, with Python, native Rust/MSVC/SDK/CMake available:
$compiler = & ./scripts/install-inno-setup.ps1
if (-not $?) { throw 'Compiler installation failed' }
$env:ULOS_ISCC = $compiler
# CARGO_BUILD_TARGET must match the native host: x86_64-pc-windows-msvc
# or aarch64-pc-windows-msvc. CARGO_TARGET_DIR is respected by both scripts.
cargo build --locked --release -p ulos-desktop
python scripts/build-torrent-worker.py --profile release --test --install-deps
python scripts/release-package.py windows x86_64 0.0.1-pre.0
# ARM host: replace x86_64 with arm64; version must match desktop Cargo.toml.
```

The helper downloads only
`https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe`,
verifies SHA-256
`9c73c3bae7ed48d44112a0f48e66742c00090bdb5bef71d9d3c056c66e97b732`
([Microsoft winget 6.7.3 manifest](https://github.com/microsoft/winget-pkgs/blob/master/manifests/j/JRSoftware/InnoSetup/6.7.3/JRSoftware.InnoSetup.installer.yaml)),
and requires a valid **Pyrsys B.V.** Authenticode signature before execution. See
[official upstream verification guidance](https://jrsoftware.org/isdl-verify.php).
It installs the compiler under the current user's local app data, refuses an
existing destination, and never changes system PATH or requests elevation. An
already provisioned trusted exact-version compiler can instead be selected via
`ULOS_ISCC=C:\path\ISCC.exe` or PATH. The packager never downloads/installs tools;
missing ISCC, a compiler failure or missing output fails closed. Compile output is
private until success; final output creation is exclusive and refuses symlinks.
The installer-engine license is also bundled as `Inno-Setup-6.7.3.txt`.

Setup uses `PrivilegesRequired=lowest` with no privilege overrides, installs to
`{localappdata}\Programs\Ulos`, creates per-user Start menu links and an Installed
apps uninstall entry. No `[Run]`, `[Registry]`, services, default handlers,
magnet/file associations, automatic app launch, app termination or elevation.
`CloseApplications=no` and `RestartApplications=no` disable Restart Manager's
automatic closure/restart, including silent installs. Users must close Ulos before
updates/uninstall; downloads/history are not targeted for deletion.

**Validation limits:** Python tests use synthetic headers and a mocked compiler
runner to inspect staged payloads/notices, architecture defines, expected output,
missing-license/worker/compiler failures and exclusive publication. They do not
compile Inno Setup or validate native wizard rendering/extraction/runtime. Before
publishing, build with the real pinned ISCC on both native Windows targets;
manually inspect MIT/OSS pages, install as a standard user, verify worker launch
and installed license contents, then exercise update/uninstall with Ulos closed
and open (no forced termination), confirm no UAC/auto-launch/default changes and
no deletion of downloads/history. No such native result is claimed on macOS.

### Optional explicit Open With registration (not run by Setup)

After placing the binary in a stable local location, deliberately invoke
`packaging/windows/register-open-with.ps1 -ExecutablePath 'C:\\Apps\\Ulos\\Ulos.exe'`.
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
Cargo version matches. It is configured for six native packages: Apple
silicon/Intel macOS DMGs, Windows x86-64/ARM64 Setup.exe installers, and Ubuntu 24.04 x86-64/ARM64
portable tar archives. The ARM64 additions are pending a future release, not
retroactive assets for an existing release. Engine tests run on each build target;
full desktop adapter tests/lint also run in CI on Windows ARM64 and Linux ARM64
(and the existing platforms). The workflow explicitly runs native worker/loopback and ignored Rust parent-task tests as well as engine/CLI checks; this is configuration, not evidence of their completion or native visual validation.

The hosted runner labels `windows-11-arm` and `ubuntu-24.04-arm` are listed in
[GitHub's hosted runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
The Windows ARM64 [image inventory](https://github.com/actions/runner-images/blob/main/images/windows/Windows11-Arm64-Readme.md)
includes Visual Studio 2022 ARM64 C++ tools, Windows SDKs and CMake. Workflows
select the ARM64 MSVC environment and Visual Studio CMake generator/platform/host
toolset explicitly (including for native dependencies such as aws-lc-sys).
Each matrix entry requires the exact `rustc -vV` host to equal its Cargo target;
installing a cross target alone does not pass. The packager checks PE Machine,
ELF e_machine, or thin Mach-O cputype against the requested architecture before
creating a package. Header tests use synthetic fixtures and do not claim native
compilation or runtime testing. A sibling `ulos-torrent-worker(.exe)` is mandatory and architecture-checked, with relocated runtime libraries checked/copied when present. The workflow builds it first; the release packager itself does not manufacture a worker.

All builds must succeed before publication. SHA256SUMS is generated from the six artifacts; publishing uses
`--verify-tag --draft=false --prerelease=false --latest`, including `-pre` names.
The workflow never creates a source commit or tag, and refuses replacing an existing release. See INSTALL.md for GUI steps.
