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
