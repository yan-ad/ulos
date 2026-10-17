# Ulos 0.0.1-pre.1

This testing release includes the following source and packaging changes. Old
0.0.1-pre.0 assets remain unchanged; download only assets actually listed for this tag.

- Automatic v1/v2/hybrid `.torrent` detection and backend selection. Genuine BEP 52
  v2/hybrid uses a bundled BSD libtorrent 2 native process, not a v2 toggle on rqbit.
- V1 defaults remain tracker-only/no payload upload. Native v2/hybrid requires
  explicit peer-upload consent; advanced v1 can opt in too. Optional public DHT,
  initial/live download/upload limits and peer caps use the native engine. Private
  payload DHT stays off; local discovery, UPnP/port forwarding, post-completion
  seeding and durable resume remain unavailable. This is not unlimited/every-torrent support.
- Explicit **Resolve magnet** in the Torrent file draft and public Rust API validate
  bounded btih hex/base32 and btmh SHA-256/hybrid identities. Native metadata-only
  discovery uses trackers/direct numeric peers; a separate public-DHT checkbox is
  off by default and warns that private status is unknown before resolution. No
  paste-triggered networking, automatic payload dispatch or OS magnet association.
  Exact requested hashes/core validation precede review; required v2/hybrid piece
  layers unavailable through BEP 9 fail closed with `.torrent` fallback. Resolution
  uses private temporary storage, zero file priorities/`upload_mode` and bounded
  120-second engine/125-second host deadlines; see the [contract](docs/TORRENTS.md).
- Offline manifest approval precedes native session Start; bounded shell-free IPC,
  independent v2 SHA-256 Merkle and hybrid SHA-1 verification, resource/path limits,
  teardown-before-verification and no-clobber publication retain safety boundaries.
- This app requires the matching native helper/runtime closure. Packaging
  fails on a missing worker, checks binary architecture and must retain engine and
  runtime notices/licenses, including supplied libtorrent BSD/full component,
  OpenSSL Apache-2.0, Boost and nlohmann-json MIT files. macOS/Linux use system
  libtorrent 2 packages; Windows pins 2.1.2 through vcpkg, not an all-OS version pin.
  See [packaging](packaging/README.md).
- The release workflow targets six packages: macOS ARM64/x86-64 DMGs, Windows ARM64/x86-64
  Setup.exe installers and Ubuntu 24.04 ARM64/x86-64 tar archives. Windows ARM64 uses `windows-11-arm`,
  native MSVC/SDK and the activated NMake toolchain; Linux ARM64 uses
  `ubuntu-24.04-arm`. Exact Rust host/target and executable-header checks protect
  architecture identity.
- Windows packages now use Inno Setup 6.7.3: `Ulos-{version}-windows-{arch}-Setup.exe`,
  not portable ZIPs. Setup presents the Ulos MIT License and a separate readable
  third-party notice page, installs the matching mandatory worker/full supplied
  licenses per user, and provides Start menu shortcuts and uninstall. No elevation,
  app auto-launch/forced close, torrent/magnet association or default takeover.
  Compiler acquisition verifies a pinned upstream download hash and Authenticode
  publisher; native Windows build/install/uninstall review remains a release gate.
  Existing 0.0.1-pre.0 ZIP assets are unchanged.
- CI configures real worker loopback and explicitly invoked ignored parent-task
  integration tests, plus desktop adapter checks. Configuration and synthetic
  packaging-header tests are not successful six-platform compile/runtime results.
  Native ARM runtime, clean-install and visual/accessibility validation remain
  separate gates; no current CI result is claimed here.

---

## Migration and immersive manager update

- Added Tasks → Import/Export migration actions for IDM `.ef2` and plain text URL files. Imports are reviewed and queued without automatic starts; unsupported credentials/cookies/POST fields are rejected.
- Added native Tasks access to Torrent or Magnet intake.
- The command/search area is aligned with the table column and the sidebar now occupies the full content height. Details remains available per row instead of the bulk toolbar.
- Windows packages are per-user Setup.exe installers with MIT and third-party notices.

# Ulos 0.0.1-pre.0

First public Ulos build. This version is published as a normal GitHub release, not marked as a GitHub prerelease, despite the early-version tag.

## Included

- HTTP/HTTPS downloads and local v1 `.torrent` files in one download list.
- Add New menu, torrent file picker, file opening and drag-and-drop confirmation.
- Named queues, explicit ordering and one-time/weekly schedules while Ulos is open.
- Transfer progress, torrent peer/download/upload observations, light/dark appearance.
- Ulos artwork, About Ulos, and a notification when a newer normal GitHub release is available.

## Downloads

- Apple silicon Mac: `Ulos-0.0.1-pre.0-macos-arm64.dmg`
- Intel Mac: `Ulos-0.0.1-pre.0-macos-x86_64.dmg`
- Windows x64: `Ulos-0.0.1-pre.0-windows-x86_64.zip`
- Linux x64: `Ulos-0.0.1-pre.0-linux-x86_64.tar.gz`

Open the DMG and drag Ulos to Applications, or extract the ZIP/tar archive and open Ulos. No CLI installation is required. `SHA256SUMS` lists download checksums.

## Current limits

- Early software: queue sources, schedules, and resumable transfers do not survive quitting. Metadata-only history is saved.
- Torrents: tracker-only, no payload upload/seeding, magnets or pure v2. Peer availability is not guaranteed.
- macOS bundles are ad-hoc signed for integrity, not Developer ID signed or notarized. Windows binaries are not publisher-signed. OS security warnings may appear; never disable security protections globally.
- Linux baseline: Ubuntu 24.04 x64; other distributions are not certified. Runtime libraries include fontconfig, Wayland/X11/xkbcommon, ALSA, Vulkan, OpenSSL, and zstd as required by the build.
- Native screen-reader and complete Windows/Linux visual validation remain incomplete.

## CI fixes

- Open owned staged files with read/write access before Windows final synchronization; do not create or truncate them.
- Set accepted test sockets to blocking mode explicitly for Windows fixtures.
- Scope platform-only imports and menu subscriptions correctly, and update checkout's Node runtime.
