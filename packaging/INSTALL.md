# Install Ulos

Download the archive for your operating system and CPU from the release assets.
SHA256SUMS records the archive digests; it is not a signature or publisher identity.

- **macOS Apple silicon:** use `macos-arm64.dmg`.
- **macOS Intel:** use `macos-x86_64.dmg`.
  Open the disk image and drag **Ulos** to **Applications**, then eject the image.
  These bundles are ad-hoc signed, not Developer ID signed or notarized.
  macOS may block them with Gatekeeper. Only if you trust the download, use the
  normal Finder Open / System Settings Privacy & Security approval flow. Do not
  disable Gatekeeper or remove quarantine globally. Runner build/test success does
  not certify compatibility with older macOS releases.
- **Windows x86-64:** extract the ZIP to a folder, then double-click **Ulos.exe**.
  The executable is not publisher-signed; Windows SmartScreen may warn. No registry
  changes or administrator access are required to start the application.
- **Linux x86-64:** extract the `.tar.gz` with your archive manager, then open
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
