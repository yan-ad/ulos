<p align="center">
  <img src="assets/branding/ulos.svg" width="104" alt="Ulos logo">
</p>

<h1 align="center">Ulos</h1>
<p align="center">A desktop download manager for links and torrent files.</p>
<p align="center">
  <a href="https://github.com/yan-ad/ulos/releases/latest"><img src="https://img.shields.io/github/v/release/yan-ad/ulos?label=Download" alt="Latest release"></a>
  <a href="https://github.com/yan-ad/ulos/stargazers"><img src="https://img.shields.io/github/stars/yan-ad/ulos?style=social" alt="Star Ulos on GitHub"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT license"></a>
</p>

![Ulos download manager on macOS](assets/screenshots/ulos-main.png)

## Download and install

**[Open the Releases page →](https://github.com/yan-ad/ulos/releases/latest)**

Choose the file for your computer. No terminal or build tools are needed.

| Computer | Download | Install |
| --- | --- | --- |
| Mac with Apple silicon (M1 or newer) | `macos-arm64.dmg` | Open the disk image and drag **Ulos** to **Applications**. |
| Mac with an Intel processor | `macos-x86_64.dmg` | Open the disk image and drag **Ulos** to **Applications**. |
| Windows, 64-bit | `windows-x86_64.zip` | Extract the entire ZIP, then open **Ulos.exe**. |
| Linux, 64-bit (Ubuntu 24.04 baseline) | `linux-x86_64.tar.gz` | Extract the archive in your file manager, allow **Ulos** to run as a program, then open it. |

These first builds are not Developer ID–signed/notarized on macOS or publisher-signed on Windows. Your system may show a security warning. Review the source and release before deciding whether to allow it; do not disable system security. On macOS, an approved exception can be made in **System Settings → Privacy & Security**. Linux packages depend on the system libraries listed in the release notes.

## What you can do

- Add an HTTP/HTTPS link or open/drop a `.torrent` file with **Add New**.
- Organize downloads into named queues and set one-time or weekly schedules.
- Pause supported transfers, track progress and speed, and choose how file conflicts are handled.
- Use light, dark, or system appearance.
- Check for a new version from **Help → Check for Updates**. **What's New** opens its GitHub release notes; updates are installed manually from Releases.

## Before you start

Ulos is early software. Keep it open for schedules to run. Queue sources and schedules are not restored after quitting; saved history cannot restart a transfer.

Torrent support currently uses trackers only, with uploads and seeding disabled. Trackerless torrents may not find peers. Magnet links and pure v2 torrents are not supported yet. Only download content you have permission to use.

## Help and feedback

[Report a problem](https://github.com/yan-ad/ulos/issues) · [Release notes](https://github.com/yan-ad/ulos/releases) · [Developer guide](docs/DEVELOPMENT.md)

MIT licensed. See [third-party notices](THIRD_PARTY_NOTICES.md) for included software.
