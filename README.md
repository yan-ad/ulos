# Ulos

A Rust desktop download manager using GPUI Kit, with Linux treated as a first-class target. Ulos is a working name. HTTP/HTTPS is the current implementation focus; torrent support is planned for a later stage. This is not an IDM replacement yet.

## Current HTTP scope

- HTTP/HTTPS single-stream CLI downloads with bounded-memory streaming.
- Explicit output paths, no-overwrite publication, and best-effort temporary-file cleanup.
- Response-length checks and cancellation, with a shared UI-independent `HttpDownloadTask` for CLI and desktop orchestration.
- Light desktop manager foundation: native titlebar, Tasks / File / Downloads / View / Help menu strip, icon-over-label toolbar, category tree, and compact Name / Size / Status / Transfer rate table with selection checkboxes.
- Real sequential session queue (up to 500 records), search/filter/sort, cancellation, metadata-only removal, and show-in-folder for finished files. Queue advancement is FIFO; table ordering never changes scheduling.
- Add-download confirmation with Address, File name, Save as, native folder/save pickers, Start download, and Download later. URL-path filename suggestions and edited names are sanitized; no naming network request is made.
- Click a filename or use Details for a live same-window modal with actual progress, rate/ETA when available, and one HTTP-stream row. Pause is disabled; limiter/completion tabs explain unimplemented features.
- Optional copied-link detection in Options: off by default, session-only, active-window checks once per second. A bounded single HTTP/HTTPS URL opens the Add form, never an automatic download. No simulated transfers or torrent controls.

The initialization snapshot was committed as `7bf99c6`; subsequent implementation and verification are tracked separately. See [Status](docs/STATUS.md) for verified checks and remaining native UI/accessibility/platform gaps.

Pause/resume, persistent queues/history, speed limiting, scheduling, site grabbing, segmented transfers, browser integration, torrents, and installers are planned, not implemented.

## Development

Rust 1.92 or newer. The desktop crate needs GPUI's platform prerequisites; see [Research](docs/RESEARCH.md) and the [upstream installation guide](https://gpui-kit.com/docs/installation).

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run -p ulos-cli -- https://example.org/file.zip --output ./file.zip
cargo run -p ulos-desktop
```

The example URL is illustrative, not a hosted test file. The destination parent must exist. CLI URLs may appear in shell history or process listings; avoid signed/private links during development.

Default Cargo commands build core and CLI only; use `--workspace` for desktop checks. The GUI is not required for engine tests.

## Project documents

- [Product scope](docs/PRODUCT.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Roadmap](docs/ROADMAP.md)
- [Research and dependency references](docs/RESEARCH.md)
- [Testing and platform matrix](docs/TESTING.md)
- [Future torrent scope](docs/TORRENTS.md)
- [Current implementation and verification status](docs/STATUS.md)

## License

The project license is undecided. No open-source license is granted for Ulos yet. Choose a license before sharing releases; dependencies retain their respective licenses. This project does not copy IDM or AB Download Manager source or branding.
