<p align="center">
  <img src="assets/branding/ulos.svg" width="104" alt="Ulos logo">
</p>

# Ulos

A desktop download manager for macOS, Windows, and Linux.

[Download Ulos](https://github.com/yan-ad/ulos/releases/latest) · [Installation](packaging/INSTALL.md) · [Report a problem](https://github.com/yan-ad/ulos/issues)

![Ulos download manager on macOS](assets/screenshots/ulos-main.png)

## Features

- [x] HTTP / HTTPS downloads
- [x] Torrent v1, v2, and hybrid files
- [x] Magnet links with explicit metadata resolution
- [x] Pause / resume supported transfers
- [x] Download queues and schedules
- [x] Import / export HTTP links as IDM EF2 or text
- [x] Light, dark, and system appearance

Early software: keep Ulos open for transfers and schedules. V2/hybrid and advanced torrent transfers require explicit peer-upload consent. Public DHT is opt-in. Some v2/hybrid magnets need a complete `.torrent` file. See [torrent support and limits](docs/TORRENTS.md).

Windows uses per-user Setup installers; macOS uses DMGs; Linux uses tar archives. Download only assets listed on the release. Builds are not publisher-signed/notarized; do not disable OS security to install them.

[Developer guide](docs/DEVELOPMENT.md) · [Implementation status](docs/STATUS.md) · [Release notes](RELEASE_NOTES.md)

MIT licensed. Dependencies retain their [own licenses and notices](THIRD_PARTY_NOTICES.md).
