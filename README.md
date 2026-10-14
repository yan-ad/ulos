# Ulos Download Manager

Ulos Download Manager is a Rust desktop application for HTTP and HTTPS downloads, built with GPUI Kit. It includes a command-line downloader and a shared transfer engine that runs independently of the interface.

The project is in early development. Single-stream downloads and a session-based desktop queue work today; pause/resume, browser integration, and torrent support are planned. Linux is a first-class target alongside macOS and Windows, but current local verification is on macOS. Cross-platform builds do not establish native UI or accessibility support.

## Current features

- HTTP/HTTPS downloads streamed to disk without buffering the entire file in memory.
- Unique temporary files beside the destination, response-length checks when available, and publication without overwriting existing files.
- Cancellation through the CLI or desktop interface, with best-effort temporary-file cleanup.
- A desktop manager with native macOS system menus (Ulos, Tasks, File, Edit, Downloads, View, Help); non-macOS retains in-window Kit dropdown menus. One shared action toolbar has stock icon-only Buttons except the labeled Add URL button, accessible names, built-in tooltips, and a stock search Input on the same row. Categories and a viewport-virtualized Kit DataTable show filename, size, status, and transfer rate, with stable-ID selection checkboxes. Stock Kit light theme is the default, with optional stock dark theme.
- A session queue capped at 500 records, with one active transfer and FIFO queue advancement. Search, filtering, sorting, and selection do not change queue order.
- Independent native Add Download windows (new draft per invocation, up to 16) with editable URL, filename, and destination, native folder/save pickers, and options to start now or queue for later.
- Filename suggestions from the URL path, sanitized into a portable single filename. Naming does not make a network request.
- Native download status windows showing actual progress, transfer rate, and estimated time when available. Each download has one reused/activated window; different download IDs can be viewed concurrently. Removing records does not delete downloaded files; completed downloads can be revealed in their folder.
- Optional copied-link detection that opens an Add Download form for confirmation. It is off by default, session-only, and gated by the active main manager window, suppressed by any Add/status window or Options dialog, and rechecked after async reads. Copying a URL never starts a download or network request.

Buttons, Inputs, and Checkboxes retain stock Kit appearance, including in child windows; no per-control size, padding, color, or variant overrides. All application widgets, panels, and text use GPUI Kit components; Kit-reexported `div`, `v_flex`, and `h_flex` are layout-only, with no custom-painted controls. Options remains a Kit modal. Native OS chrome and path pickers do not require bespoke GTK/Swift UI. Outstanding pickers guard Add close/dismiss/submit. Closing the manager leaves child windows usable; the app quits when its last window closes. Closing status alone never cancels a transfer; removing/clearing records closes related status windows while leaving files untouched.

The interface follows a traditional download-manager layout. It does not reuse IDM or AB Download Manager source or branding, and it is not an IDM replacement yet.

## Run from source

### Requirements

- Rust 1.92 or newer and Cargo.
- Native platform prerequisites for the desktop application. Follow the [GPUI Kit installation guide](https://gpui-kit.com/docs/installation) for the version used by this project.

Desktop setup requires Xcode Command Line Tools on macOS, MSVC/C++ tooling with a Windows SDK and CMake on Windows, or the relevant development libraries on Linux. Linux also needs a graphical X11 or Wayland session and a working Vulkan driver. These are upstream requirements, not a verified Ulos support matrix. See [Research](docs/RESEARCH.md) and [Testing](docs/TESTING.md) for details.

Run the commands below from the repository root. The checked-in `Cargo.lock` pins the dependency graph.

### Desktop application

```sh
cargo run --locked -p ulos-desktop
```

Use **Add URL** to enter an HTTP/HTTPS address, review the suggested filename, and choose a destination. Choose **Start download** to start immediately when idle, or **Download later** to add the item without starting a stopped queue. If the queue is already running, it can still process items added for later.

On macOS, **⌘N** opens Add Download, **⌘,** opens Options when available, and **⌘Q** quits. Native Edit commands target the focused Kit input.

The main macOS window extends the manager into the titlebar area, with native window controls beside the action toolbar and no separate visible Ulos title strip. Add/status windows retain their native titles.

Click a filename or use **Details** to inspect the transfer. Closing its details window does not cancel the download. Queue records and clipboard preferences are not saved between sessions.

### Command-line downloader

```sh
cargo run --locked -p ulos-cli -- --help
cargo run --locked -p ulos-cli -- https://example.org/file.zip --output ./file.zip
```

The URL above illustrates the command syntax; it is not a hosted test file. Replace it with a real download URL. `--output` (or `-o`) is required, and its parent directory must already exist. Existing destination files are never replaced.

Press `Ctrl+C` to request cancellation. The CLI reports success or failure through its exit status. URLs passed as arguments may appear in shell history or process listings, so avoid signed or private links during development.

## File safety and privacy

Transfers write into an exclusively created `.part` file beside the destination, then publish the completed file without clobbering an existing destination. Failure and cancellation trigger best-effort cleanup of the transfer's own temporary file.

Cancellation can race with final publication, so a completed file may exist even after cancellation is requested. Abrupt termination can leave temporary files; crash recovery and resumable checkpoints are not implemented.

Copied-link detection requires explicit consent for each session. It accepts one bounded HTTP/HTTPS URL and offers confirmation rather than downloading automatically. Clipboard contents are not logged or persisted. Download details omit URL credentials, query strings, and fragments, but secrets in URL paths are not automatically redacted. Accepted source URLs, including signed queries, remain in memory while needed for the transfer.

Ulos does not provide DRM, captcha, or access-control bypass.

## Not implemented yet

- Pause/resume and durable transfer checkpoints.
- Persistent queues, download history, and startup recovery.
- Segmented downloads, automatic retries, and speed limiting.
- Scheduling, site grabbing, and completion automation.
- Browser extensions and native-messaging integration.
- Torrent downloads, including magnet links and `.torrent` files.
- Installers and automatic updates.

Unsupported desktop controls are disabled or explain their limitations. Native accessibility, keyboard/focus behavior, and Linux/Windows graphical validation still need review. See [Status](docs/STATUS.md) for recorded verification and known limits, and [Roadmap](docs/ROADMAP.md) for planned work.

## Project structure

| Crate | Responsibility |
| --- | --- |
| [`ulos-core`](crates/ulos-core) | HTTP transfer logic, request validation, safe file lifecycle, filename sanitization, and task state. No GPUI dependency. |
| [`ulos-cli`](crates/ulos-cli) | Command-line downloader and diagnostic consumer of the shared engine. Builds the `ulos` binary. |
| [`ulos-desktop`](crates/ulos-desktop) | GPUI Kit interface, session queue, windows, clipboard consent, and transfer orchestration. |

Networking and filesystem work run outside the desktop render path. Both consumers use the shared `HttpDownloadTask` service. See [Architecture](docs/ARCHITECTURE.md) for the transfer contract and module boundaries.

## Development and verification

Default workspace commands cover core and CLI without requiring desktop system libraries:

```sh
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

With desktop prerequisites installed, check the full workspace:

```sh
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked -p ulos-desktop
```

Tests use local HTTP fixtures and temporary files. Changes to transfers should cover errors, cancellation, and no-overwrite behavior as well as successful downloads. Passing tests do not replace native rendering or accessibility checks. Read [AGENTS.md](AGENTS.md) before contributing.

## Documentation

- [Product scope](docs/PRODUCT.md)
- [Architecture](docs/ARCHITECTURE.md)
- [Desktop design direction](DESIGN.md)
- [Implementation and verification status](docs/STATUS.md)
- [Testing and platform matrix](docs/TESTING.md)
- [Roadmap](docs/ROADMAP.md)
- [Research and dependency references](docs/RESEARCH.md)
- [Future torrent scope](docs/TORRENTS.md)

## License

Copyright (c) 2026 Ulos Download Manager contributors.

Ulos Download Manager is licensed under the [MIT License](LICENSE). Dependencies retain their respective licenses and notices.
