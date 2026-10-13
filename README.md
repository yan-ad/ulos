# Ulos

A Rust desktop download manager using GPUI Kit, with Linux treated as a first-class target. Ulos is a working name. This is an initial scaffold, not an IDM replacement yet.

## Available now

- HTTP/HTTPS single-stream CLI downloads with bounded-memory streaming.
- Explicit output paths, no-overwrite publication, and best-effort temporary-file cleanup.
- Progress events in the core API, response-length checks, and Ctrl+C cancellation in the CLI.
- A minimal GPUI desktop window. Transfers are **not connected to the GUI** yet.

Pause/resume, persistent queues, segmented transfers, browser integration, and installers are planned, not implemented.

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
- [Current initialization status](docs/STATUS.md)

## License

The project license is undecided. No open-source license is granted for Ulos yet. Choose a license before sharing releases; dependencies retain their respective licenses. This project does not copy IDM or AB Download Manager source or branding.
