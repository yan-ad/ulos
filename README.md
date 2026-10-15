# Ulos Download Manager

Ulos Download Manager is a Rust desktop application for HTTP and HTTPS downloads, built with GPUI Kit. It includes a command-line downloader and a shared transfer engine that runs independently of the interface.

The project is in early development. The CLI retains single-stream downloads. The desktop now has a controlled transfer engine with verified parallel ranges, session pause/resume, an aggregate speed limit, explicit file-conflict choices, and confirmed completion shutdown. Durable recovery, browser integration, and torrent support remain planned. Linux is a first-class target alongside macOS and Windows, but current local verification is on macOS. Cross-platform builds do not establish native UI or accessibility support.

## Current features

- HTTP/HTTPS downloads streamed to disk without buffering the entire file in memory.
- Unique temporary files beside the destination and applicable response-length checks. Reject existing files by default, optionally publish a numbered filename, or explicitly authorize replacement only after a successful staged transfer.
- Cancellation through the CLI or desktop interface, with best-effort temporary-file cleanup.
- A desktop manager with native macOS system menus (Ulos, Tasks, File, Edit, Downloads, View, Help); non-macOS retains in-window Kit dropdown menus. One shared action toolbar has stock icon-only Buttons except the labeled Add URL button, accessible names, built-in tooltips, and a stock search Input on the same row. A collapsible stock Kit sidebar and a viewport-virtualized Kit DataTable show filename, size, status, and transfer rate, with stable-ID selection checkboxes. The default follows system appearance; the toolbar theme selector and View menu offer Follow system, Light, and Dark using stock Kit themes.
- A session queue capped at 500 records, with one active download (possibly multiple HTTP connections) and FIFO queue advancement. A paused download retains its active slot. Search, filtering, sorting, and selection do not change queue order.
- Independent native Add Download windows (new draft per invocation, up to 16) with editable URL, filename, and destination, native folder/save pickers, and options to start now or queue for later.
- Filename suggestions from the URL path, sanitized into a portable single filename. Naming itself makes no network request. Address edits automatically check advisory size with a debounced HEAD request; sizes use KB/MB/GB/TB with a filename-category icon. Clipboard-prefilled drafts remain network-free until edited or explicitly checked.
- Stock Kit radio groups for connections and existing-file conflicts. Conflict choices appear only for an existing regular target; replacement still requires destination-specific consent.
- Native download status windows showing actual progress, transfer rate, and estimated time when available. Each download has one reused/activated window; different download IDs can be viewed concurrently. Removing records does not delete downloaded files; completed downloads can be revealed in their folder.
- Optional copied-link detection that opens an Add Download form for confirmation. It is off by default, session-only, and gated by the active main manager window, suppressed by any Add/status window or Options dialog, and rechecked after async reads. Copying a URL never starts a download or network request.

Buttons, Inputs, and Checkboxes retain stock Kit appearance, including in child windows; no per-control size, padding, color, or variant overrides. All application widgets, panels, and text use GPUI Kit components; Kit-reexported `div`, `v_flex`, and `h_flex` are layout-only, with no custom-painted controls. Options remains a Kit modal. Native OS chrome and path pickers do not require bespoke GTK/Swift UI. Outstanding pickers guard Add close/dismiss/submit. Closing the manager leaves child windows usable; the app quits when its last window closes. Closing status alone never cancels a transfer, but does cancel its armed shutdown or countdown; removing/clearing records closes related status windows while leaving files untouched.

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

The sidebar starts expanded each session. Its Downloads group contains All downloads, Finished, Unfinished, and Queue; File types contains seven flat filters: Image, Music, Video, Apps, Document, Compressed, and Other. Expanded items show real session counts; collapsed items use Kit’s icon rail and tooltips, with counts retained in accessible names. Use the stock icon button immediately before Add URL, **⌘B / Ctrl+B** in the manager, or View’s sidebar command. Collapse does not reset search, filtering, selection, or FIFO order, and is not persisted.

The main macOS window extends the manager into the titlebar area, with native window controls beside the action toolbar and no separate visible Ulos title strip. Add windows keep native window controls without a visible title. The macOS status window uses transparent full-content chrome and a short fixed Download title, with Kit segmented tabs in its TitleBar.

Click a filename or use **Details** to inspect the transfer. Closing its details window does not cancel the download. Queue records and clipboard preferences are not saved between sessions.

### Desktop transfer controls

Add Download defaults to **4 connections**, with choices **1, 2, 4, 8**. The engine supports 1–8; actual connections may be fewer for small files or a server without verified range support. A `bytes=0-0` probe must return a valid identity-encoded `206` and strong ETag before parallel downloading or pause is enabled. Unsupported servers use one full stream. No measured speed improvement is promised.

Details has stock Kit **Status**, **Speed**, and **Completion** segmented tabs with icons. The same progress footer stays visible on every tab, with a Kit Label inside the 24px Kit Progress bar; unknown percentages are labeled unknown. The connection table contains actual worker telemetry, not fabricated segments. **Pause / Resume** closes and reopens verified ranges within this running session; it is disabled for unsupported servers. There are no saved checkpoints or crash resume.

In **Speed**, enable the limiter, enter **1–1,048,576 KiB/s**, and apply. Disable it for unlimited output consumption. One budget covers all connections before writes; socket/TCP prefetch can still consume network bytes. This is not a wire-speed or network QoS guarantee. Live accepted changes interrupt limiter waits and appear in telemetry on subsequent updates. Connection-count changes apply only to queued or inactive failed downloads on their next start, never to active/paused workers.

Add Download offers **Ask / do not overwrite** (default), **Rename**, or **Overwrite**. Overwrite requires the explicit authorization checkbox for the selected destination; changing the destination resets consent. An already-exists failure exposes **Override existing** (Kit confirmation required) and **Add another file** (fresh numbered-name retry). No retry starts automatically. Rename keeps the original and publishes `stem (1).extension`, trying further bounded candidates on collision.

### Confirmed completion shutdown

**This feature can turn off your computer. Save all work in other applications first.** In an active download's **Completion** tab, check **Shut down after successful completion**, then accept the confirmation dialog. It is off by default and per download. Only successfully published completion with every queue record finished can start the visible **30-second countdown**; **Cancel shutdown** withdraws an undispatched request. New work, retry, failure, cancellation, record removal, closing that status window, or closing all windows cancels the intent. Closing status leaves downloading alone: no hidden countdown is allowed.

The production provider requests real OS shutdown: macOS System Events through `osascript` (Automation permission may prompt), Linux `systemctl poweroff` (requires an available provider and OS permission), or Windows `shutdown.exe /s /t 0`. There is no forced app closure, sudo, elevation, or permission bypass. Provider errors are displayed; successful command exit is not proof the computer powered off. After dispatch, Ulos cannot revoke the OS request. Tests inspect gates/command plans and suppress dispatch; no host poweroff or OS settings changes were tested. See [Testing](docs/TESTING.md).

### Command-line downloader

```sh
cargo run --locked -p ulos-cli -- --help
cargo run --locked -p ulos-cli -- https://example.org/file.zip --output ./file.zip
```

The URL above illustrates the command syntax; it is not a hosted test file. Replace it with a real download URL. `--output` (or `-o`) is required, and its parent directory must already exist. Existing destination files are never replaced.

Press `Ctrl+C` to request cancellation. The CLI reports success or failure through its exit status. URLs passed as arguments may appear in shell history or process listings, so avoid signed or private links during development.

## File safety and privacy

Transfers write into an exclusively created `.part` file beside the destination. The CLI and default desktop policy publish without clobbering. Desktop Rename uses no-clobber numbered publication; explicit Overwrite is the only exception, replacing an existing regular file only after staging succeeds. Overwrite refuses symlinks and directories, and preserves the old file on transfer failure. Failure and cancellation trigger best-effort cleanup of the transfer's own temporary file.

Cancellation can race with final publication, so a completed file may exist even after cancellation is requested. Abrupt termination can leave temporary files; crash recovery and resumable checkpoints are not implemented.

Copied-link detection requires explicit consent for each session. It accepts one bounded HTTP/HTTPS URL and offers confirmation rather than downloading automatically. Clipboard contents are not logged or persisted. Download details omit URL credentials, query strings, and fragments, but secrets in URL paths are not automatically redacted. Accepted source URLs, including signed queries, remain in memory while needed for the transfer.

Ulos does not provide DRM, captcha, or access-control bypass.

## Not implemented yet

- Durable checkpoints and pause/resume across application restarts (session pause/resume is implemented for verified range transfers).
- Persistent queues, download history, and startup recovery.
- Automatic retries (manual failed-download retry is available).
- Scheduling, site grabbing, and completion actions beyond confirmed shutdown.
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

Networking and filesystem work run outside the desktop render path. Both consumers use `HttpDownloadTask`, but the CLI uses legacy `start`/`Downloader` and the desktop uses `start_with_options`/`transfer.rs`. Their contracts are deliberately distinct. See [Architecture](docs/ARCHITECTURE.md) for the transfer contract and module boundaries.

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

### Appearance

Ulos starts in **Follow system** mode and responds to appearance changes while running. Use the monitor/sun/moon toolbar button or **View** to choose **Follow system**, **Light**, or **Dark**. Explicit Light/Dark choices remain fixed until changed. This session-only preference applies to manager, Add, and status windows; it resets to Follow system on the next launch. No custom control colors are applied.
