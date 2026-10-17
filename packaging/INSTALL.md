# Install Ulos

Download the package for your operating system and CPU from the release assets.
Windows ARM64 and Linux ARM64 are configured for future releases; use only assets
actually listed on the selected release. Older releases may not include them.
SHA256SUMS records the package digests; it is not a signature or publisher identity.

- **macOS Apple silicon:** use `macos-arm64.dmg`.
- **macOS Intel:** use `macos-x86_64.dmg`.
  Open the disk image and drag **Ulos** to **Applications**, then eject the image.
  These bundles are ad-hoc signed, not Developer ID signed or notarized.
  macOS may block them with Gatekeeper. Only if you trust the download, use the
  normal Finder Open / System Settings Privacy & Security approval flow. Do not
  disable Gatekeeper or remove quarantine globally. Runner build/test success does
  not certify compatibility with older macOS releases.
- **Windows Intel/AMD x86-64** (`windows-x86_64-Setup.exe`) or **Windows ARM64**
  (`windows-arm64-Setup.exe`, for ARM-based PCs), **next build only**: open Setup,
  read/accept the Ulos MIT license, then read the scrollable third-party OSS notice
  page. Install for your Windows account (default: `%LOCALAPPDATA%\Programs\Ulos`).
  No administrator access/elevation is requested. Setup adds a Start menu shortcut
  and an entry in Windows Installed apps for uninstalling. It does not launch Ulos,
  close running applications, register torrent/magnet handlers, or change defaults.
  Close Ulos yourself before updating/uninstalling; do not interrupt active downloads.
  Use the Start menu to open Ulos after installation. The installed `LICENSE`,
  `THIRD_PARTY_NOTICES.md`, `OSS-NOTICES.txt` and `licenses/` remain readable offline;
  the Start menu also links to the combined notices. Uninstall does not purge your
  downloads or saved history. The installer is not publisher-signed; SmartScreen
  may warn. Review the source/download; never disable Windows security globally.
  **Existing 0.0.1-pre.0 Windows assets remain ZIPs:** extract the entire ZIP to a
  folder, then open **Ulos.exe**. This change does not rewrite published releases.
- **Linux x86-64** (`linux-x86_64.tar.gz`) or **Linux ARM64**
  (`linux-arm64.tar.gz`): extract the `.tar.gz` with your archive manager, then open
  **Ulos** in the extracted folder. Some file managers require confirming that it
  is executable. This is a portable folder, **not** an AppImage or fully static
  application: it is built on Ubuntu 24.04 and needs a compatible desktop runtime
  (including system graphics, font, Wayland/X11 and other linked libraries).
  Older distributions are not supported by this build baseline. The desktop file
  is optional metadata, not an automatically installed launcher; adjust its Exec
  and Icon to absolute extracted paths before deliberately installing it locally.

No installer changes default torrent handlers. Choose Ulos explicitly in Open With
when desired, or use the app's file picker. Keep LICENSE, THIRD_PARTY_NOTICES.md
and the licenses folder with redistributed copies.

## Next-build torrent files

Install/extract/copy the complete application, not just its main executable: the native
`ulos-torrent-worker` helper and included `lib/` runtime libraries are required
for v2/hybrid torrents, explicit magnet metadata resolution (including v1), peer uploads, public DHT and torrent rate/peer controls. Next-build application bundles require this helper and its complete runtime closure.
Keep them in their supplied locations; no separate end-user engine installation
is intended. The old 0.0.1-pre.0 assets are not retroactively changed.

Ulos detects v1/v2/hybrid automatically, but v2/hybrid requires explicit **Allow
peer uploads while downloading** consent. Default v1 remains tracker-only with
no payload upload. Known-private payload DHT is always off. In the Torrent file draft, paste a magnet and explicitly choose **Resolve magnet** for metadata-only discovery; nothing resolves on paste or downloads payload automatically. A separate public-DHT resolution checkbox is off by default: private status is unknown until metadata arrives, so enabling it can expose a private hash. Review the verified result before Start/Download later. V2/hybrid magnets with files larger than one piece cannot supply required piece layers through BEP 9 and fail closed; choose a `.torrent` file instead. There is no OS magnet-handler registration. Local discovery, port forwarding, post-completion seeding and durable resume remain unavailable; safety/resource limits still apply. A missing/unlaunchable helper is an error, not permission to silently
change engines or upload policy. See the [torrent guide](../docs/TORRENTS.md).
