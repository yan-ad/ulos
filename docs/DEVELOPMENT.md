# Development and technical details

This document retains technical build and feature details from the development README. For downloads and installation, see the [README](../README.md).

# Ulos Download Manager

Ulos Download Manager is a Rust desktop application for HTTP, HTTPS, and local `.torrent` downloads, built with GPUI Kit. It includes a command-line downloader and a shared transfer engine that runs independently of the interface.

The project is in early development. The CLI retains single-stream downloads. The desktop now has a controlled transfer engine with verified parallel ranges, session pause/resume, an aggregate speed limit, explicit file-conflict choices, and confirmed completion shutdown. Local v1 `.torrent` downloads are implemented; durable recovery and browser integration remain planned. Linux is a first-class target alongside macOS and Windows, but current local verification is on macOS. Cross-platform builds do not establish native UI or accessibility support.

## Current features

- HTTP/HTTPS downloads streamed to disk without buffering the entire file in memory.
- Unique temporary files beside the destination and applicable response-length checks. Reject existing files by default, optionally publish a numbered filename, or explicitly authorize replacement only after a successful staged transfer.
- Cancellation through the CLI or desktop interface, with best-effort temporary-file cleanup.
- A desktop manager with native macOS system menus (Ulos, Tasks, File, Edit, Downloads, View, Help); non-macOS retains in-window Kit dropdown menus. One shared action toolbar has stock icon-only Buttons except the labeled Add New button, accessible names, built-in tooltips, and a stock search Input on the same row. A collapsible stock Kit sidebar and a viewport-virtualized Kit DataTable show ten columns total: selection checkbox, Name, Size, Status, Transfer rate, Queue, Time left, Last try, Description, and Type, with stable-ID checkbox selection independent of row focus. The default follows system appearance; the toolbar theme selector and View menu offer Follow system, Light, and Dark using stock Kit themes.
- A session capped at 500 records and 32 named queues (default Main queue), with one global active HTTP or torrent transfer (possibly multiple connections). A paused download retains its active slot. Scheduler supports explicit queue ordering and local once/weekly schedules while Ulos stays open; table search, filtering, sorting, and selection do not change dispatch order.
- Independent native Add Download windows (new draft per invocation, up to 16) with editable URL, filename, and destination, native folder/save pickers, and options to start now or queue for later.
- Filename suggestions from the URL path, sanitized into a portable single filename. Naming itself makes no network request. Address edits automatically check advisory size with a debounced HEAD request; sizes use KB/MB/GB/TB with a filename-category icon. Clipboard-prefilled drafts remain network-free until edited or explicitly checked.
- Stock Kit radio groups for connections and existing-file conflicts. Conflict choices appear only for an existing regular target; replacement still requires destination-specific consent.
- Native download status windows showing actual progress, transfer rate, and estimated time when available. Each download has one reused/activated window; different download IDs can be viewed concurrently. Removing records does not delete downloaded files; completed downloads can be revealed in their folder.
- Optional copied-link detection that opens an Add Download form for confirmation. It is off by default, session-only, and gated by the active main manager window, suppressed by any Add/status/Scheduler window or Options dialog, and rechecked after async reads. Copying a URL never starts a download or network request.

Buttons, Inputs, and Checkboxes retain stock Kit appearance, including in child windows; no per-control size, padding, color, or variant overrides. All application widgets, panels, and text use GPUI Kit components; Kit-reexported `div`, `v_flex`, and `h_flex` are layout-only, with no custom-painted controls. Options remains a Kit modal. Native OS chrome and path pickers do not require bespoke GTK/Swift UI. Outstanding pickers guard Add close/dismiss/submit. Closing the manager leaves child windows usable; the app quits when its last window closes. Closing status alone never cancels a transfer, but does cancel its armed shutdown or countdown; removing/clearing records closes related status windows while leaving files untouched.

The interface follows a traditional download-manager layout. It does not reuse IDM or AB Download Manager source or branding, and it is not an IDM replacement yet.

## Run from source

### Requirements

- Cargo and a toolchain compatible with the locked graph. The workspace declares Rust 1.92; the added torrent dependencies need a separate minimum-toolchain check before claiming that baseline works.
- Native platform prerequisites for the desktop application. Follow the [GPUI Kit installation guide](https://gpui-kit.com/docs/installation) for the version used by this project.

Desktop setup requires Xcode Command Line Tools on macOS, MSVC/C++ tooling with a Windows SDK and CMake on Windows, or the relevant development libraries on Linux. Linux also needs a graphical X11 or Wayland session and a working Vulkan driver. These are upstream requirements, not a verified Ulos support matrix. See [Research](RESEARCH.md) and [Testing](TESTING.md) for details.

Run the commands below from the repository root. The checked-in `Cargo.lock` pins the dependency graph.

### Desktop application

```sh
cargo run --locked -p ulos-desktop
```

Use **Add New → HTTP URL** to enter an HTTP/HTTPS address, review the suggested filename, and choose a destination. Choose **Start download** to start immediately when idle, or **Download later** to add the item without starting a stopped queue. If the queue is already running, it can still process items added for later.

On macOS, **⌘N** opens Add Download, **⌘,** opens Options when available, and **⌘Q** quits. Native Edit commands target the focused Kit input.

The sidebar starts expanded each session. Its Downloads group contains All downloads, Finished, Unfinished, and Queue; File types contains seven flat filters: Image, Music, Video, Apps, Document, Compressed, and Other. Expanded items show real session counts; collapsed items use Kit’s icon rail and tooltips, with counts retained in accessible names. Use the stock icon button beside the bottom download count, **⌘B / Ctrl+B** in the manager, or View’s sidebar command. Collapse does not reset search, filtering, selection, or dispatch order, and is not persisted.

The main macOS window extends the manager into the titlebar area, with native window controls above the independent table-aligned toolbar and no separate visible Ulos title strip. Add windows keep native window controls without a visible title. The macOS status window uses transparent full-content chrome and a short fixed Download title, with Kit segmented tabs in its TitleBar.

Click a filename Kit Link, press Enter on the focused row, double-click a row, or use **Details** to inspect the transfer. Closing its details window does not cancel the download. The borderless Kit table uses 36px rows and a 32px header through public APIs; filename Links have colored category icons, not Button variants. Time left uses actual transfer rate only; Last try is the local actual worker-start time; Description is user-entered in Scheduler. Unknown values stay blank. Clipboard preferences are session-only. SQLite stores metadata-only destination/status/bytes history, not source URLs or schedules. Restored source-less records cannot start or retry. Queue names, membership, ordering, schedules, descriptions, and Last try are session-only; this is not durable transfer recovery.

### Desktop transfer controls

HTTP Add Download defaults to **4 connections**, with choices **1, 2, 4, 8**. The engine supports 1–8; actual connections may be fewer for small files or a server without verified range support. A `bytes=0-0` probe must return a valid identity-encoded `206` and strong ETag before parallel downloading or pause is enabled. Unsupported servers use one full stream. No measured speed improvement is promised.

Details has stock Kit **Status**, **Speed**, and **Completion** segmented tabs with icons. The same progress footer stays visible on every tab, with a Kit Label inside the 24px Kit Progress bar; unknown percentages are labeled unknown. The connection table contains actual worker telemetry, not fabricated segments. **Pause / Resume** closes and reopens verified ranges within this running session; it is disabled for unsupported servers. There are no saved checkpoints or crash resume.

In **Speed**, enable the limiter, enter **1–1,048,576 KiB/s**, and apply. Disable it for unlimited output consumption. One budget covers all connections before writes; socket/TCP prefetch can still consume network bytes. This is not a wire-speed or network QoS guarantee. Live accepted changes interrupt limiter waits and appear in telemetry on subsequent updates. Connection-count changes apply only to queued or inactive failed downloads on their next start, never to active/paused workers.

HTTP Add Download offers **Ask / do not overwrite** (default), **Rename**, or **Overwrite**. Overwrite requires the explicit authorization checkbox for the selected destination; changing the destination resets consent. An already-exists failure exposes **Override existing** (Kit confirmation required) and **Add another file** (fresh numbered-name retry). No retry starts automatically. Rename keeps the original and publishes `stem (1).extension`, trying further bounded candidates on collision.

### Torrent files

Choose **Add New → Torrent file**, drop local `.torrent` files on the manager, or open one through startup arguments / the OS open-file callback. Every route opens an offline native review; nothing auto-submits. Review the name, total bytes, file manifest, tracker count, destination, and Reject/Rename policy, then choose **Start download** or **Download later**. Later does not start a stopped queue, but confirmation permits an already-running queue to dispatch it.

The separate core engine uses pinned `librqbit` 9.0.1 (Apache-2.0). It accepts bounded v1 metainfo and compatible v1 hybrids, not magnets or pure v2. Discovery is tracker-only after dispatch; payload upload, seeding, DHT, local discovery, inbound listeners, port forwarding, persistence and HTTP serving are off. Disabled upload can hinder reciprocal swarms. Actual session Pause/Resume is available; torrent live rate limits and connection settings are unavailable, not simulated HTTP controls.

The torrent **Status** summary keeps **Peers** (connected / connecting / queued), separate **Download speed** and **Upload speed** (payload upload disabled), and **Trackers** (configured count) visible without opening details. Download speed is the engine’s payload rate, not a verified-piece progress delta; partial pieces can transfer while verified bytes remain at 0 B. Stale active telemetry is unavailable, not a measured zero; stopped/terminal activity is zero. Labels distinguish **No peer discovery**, **Searching for peers**, **Connecting to peers**, **Waiting for peers**, and **Stalled**; see the [diagnostic contract](TORRENTS.md#live-diagnostics). Configured trackers are not successful announces or seed counts, and these observations cannot guarantee live peers or identify a tracker failure.

HTTP and torrents share one global active slot, named queues, schedules, status windows and post-publication completion gates. The existing table appends **Type**; there is no torrent sidebar group or dedicated torrent download folder. Single-file torrents use the payload's existing file-type category; multifile payloads use Other. Torrent completion requires engine teardown, full SHA-1 piece re-verification and file sync before no-clobber file/directory publication. Torrent Overwrite is never offered or accepted. History schema v2 retains protocol/directory flags but no metainfo, source or tracker; restored rows cannot restart/retry. Multifile directory removal is metadata-only, never recursive deletion.

Development opener metadata and a local unsigned `.app` bundling script are in [packaging](../packaging/README.md). macOS declares a torrent Viewer/imported UTI; Linux supplies a desktop Open With entry; Windows supplies an explicitly invoked per-user Open With registration script. These are not installed associations or automatic default changes, signed installers, or verified native launches. No Windows registration script was executed for this update. See [Torrent contract](TORRENTS.md) for limits and privacy.

### Scheduling & queues

Open **Scheduler** to create or rename queues (32 maximum including Main queue). Select downloads with the main-table checkboxes, then assign them to the selected queue in Scheduler. Delete a non-main queue to reassign its members to Main queue; active/paused work cannot be moved or reassigned and blocks deletion of its queue. Use **Move earlier / Move later** for waiting files, independently of table sort. Start/Stop Queue in the main toolbar targets Main queue; named queues are started/stopped in Scheduler. Across running queues, dispatch picks the smallest `(queue_order, id)`, not round-robin. Stopping a queue lets its active transfer finish without further advancement; **Stop All** stops every queue and requests active cancellation.

Schedules use local **Once** (future `YYYY-MM-DD`, `HH:MM`) or **Weekly** (at least one weekday) starts and an optional `HH:MM` stop. Stop must differ from start; an earlier stop is overnight on the next day and stops advancement, not the active transfer. Nonexistent local DST occurrences are skipped; ambiguous times use the earlier instant. After sleep, only the current unexpired window can start, never an ended window; without a stop, catch-up is limited to the start's local day, not an automatic end-of-day stop. Repeated ticks or a manual stop cannot restart the same occurrence. Recurrence never retries finished, failed, or cancelled rows automatically.

The reused native Scheduler child has its own Kit Root and stays usable after the main window closes. Ulos must remain open; there is no OS/background scheduler. Scheduler suppresses clipboard detection both before reads and when async proposals return. Enabling a schedule cancels and blocks completion shutdown. Queue and schedule configuration is session-only.

### Confirmed completion shutdown

**This feature can turn off your computer. Save all work in other applications first.** In an active download's **Completion** tab, check **Shut down after successful completion**, then accept the confirmation dialog. It is off by default and per download. Only successfully published completion with every queue record finished can start the visible **30-second countdown**; **Cancel shutdown** withdraws an undispatched request. New work, retry, failure, cancellation, record removal, enabling a schedule, closing that status window, or closing all windows cancels the intent. Enabled schedules also block arming/countdown/dispatch. Closing status leaves downloading alone: no hidden countdown is allowed.

The production provider requests real OS shutdown: macOS System Events through `osascript` (Automation permission may prompt), Linux `systemctl poweroff` (requires an available provider and OS permission), or Windows `shutdown.exe /s /t 0`. There is no forced app closure, sudo, elevation, or permission bypass. Provider errors are displayed; successful command exit is not proof the computer powered off. After dispatch, Ulos cannot revoke the OS request. Tests inspect gates/command plans and suppress dispatch; no host poweroff or OS settings changes were tested. See [Testing](TESTING.md).

### Command-line downloader

```sh
cargo run --locked -p ulos-cli -- --help
cargo run --locked -p ulos-cli -- https://example.org/file.zip --output ./file.zip
```

The URL above illustrates the command syntax; it is not a hosted test file. Replace it with a real download URL. `--output` (or `-o`) is required, and its parent directory must already exist. Existing destination files are never replaced.

Press `Ctrl+C` to request cancellation. The CLI reports success or failure through its exit status. URLs passed as arguments may appear in shell history or process listings, so avoid signed or private links during development.

## File safety and privacy

HTTP transfers write into an exclusively created `.part` file beside the destination. Torrents use a unique same-parent staging directory and Reject/Rename only; see the torrent contract above. The CLI and default desktop policy publish without clobbering. HTTP desktop Rename uses no-clobber numbered publication; explicit Overwrite is the only exception, replacing an existing regular file only after staging succeeds. Overwrite refuses symlinks and directories, and preserves the old file on transfer failure. Failure and cancellation trigger best-effort cleanup of the transfer's own temporary file.

Cancellation can race with final publication, so a completed file may exist even after cancellation is requested. Abrupt termination can leave temporary files; crash recovery and resumable checkpoints are not implemented.

Copied-link detection requires explicit consent for each session. It accepts one bounded HTTP/HTTPS URL and offers confirmation rather than downloading automatically. Clipboard contents and network secrets are not logged; clipboard contents are not persisted. Download details omit URL credentials, query strings, and fragments, but secrets in URL paths are not automatically redacted. Accepted source URLs, including signed queries, remain in memory while needed for the transfer.

Ulos does not provide DRM, captcha, or access-control bypass.

## Not implemented yet

- Durable checkpoints and pause/resume across application restarts (session pause/resume is implemented for verified range transfers).
- Durable source-bearing queues, checkpoints, and startup recovery (current SQLite history is metadata-only).
- Automatic retries (manual failed-download retry is available).
- Background/closed-app scheduling, and completion actions beyond confirmed shutdown.
- Browser extensions and native-messaging integration.
- Magnet links, pure v2 torrents, torrent live rate/connection controls, and durable torrent recovery.
- Installers and automatic updates.

Unsupported desktop controls are disabled or explain their limitations. Native accessibility, keyboard/focus behavior, and Linux/Windows graphical validation still need review. See [Status](STATUS.md) for recorded verification and known limits, and [Roadmap](ROADMAP.md) for planned work.

## Project structure

| Crate | Responsibility |
| --- | --- |
| [`ulos-core`](crates/ulos-core) | HTTP and torrent transfer logic, request validation, safe file lifecycle, filename sanitization, and task state. No GPUI dependency. |
| [`ulos-cli`](crates/ulos-cli) | Command-line downloader and diagnostic consumer of the shared engine. Builds the `ulos` binary. |
| [`ulos-desktop`](crates/ulos-desktop) | GPUI Kit interface, session queue, windows, clipboard consent, and transfer orchestration. |

Networking and filesystem work run outside the desktop render path. Both consumers use `HttpDownloadTask`, but the CLI uses legacy `start`/`Downloader` and the desktop uses `start_with_options`/`transfer.rs`. Their contracts are deliberately distinct. See [Architecture](ARCHITECTURE.md) for the transfer contract and module boundaries.

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

Tests use local HTTP/tracker/handcrafted-peer fixtures and temporary files, never public swarms. Changes to transfers should cover errors, cancellation, and no-overwrite behavior as well as successful downloads. Passing tests do not replace native rendering or accessibility checks. Read [AGENTS.md](AGENTS.md) before contributing.

## Documentation

- [Product scope](PRODUCT.md)
- [Architecture](ARCHITECTURE.md)
- [Desktop design direction](DESIGN.md)
- [Implementation and verification status](STATUS.md)
- [Testing and platform matrix](TESTING.md)
- [Roadmap](ROADMAP.md)
- [Research and dependency references](RESEARCH.md)
- [Torrent implementation and limits](TORRENTS.md)

## License

Copyright (c) 2026 Ulos Download Manager contributors.

Ulos Download Manager is licensed under the [MIT License](../LICENSE). Dependencies retain their respective licenses and notices.

### Appearance

Ulos starts in **Follow system** mode and responds to appearance changes while running. Use the monitor/sun/moon toolbar button or **View** to choose **Follow system**, **Light**, or **Dark**. Explicit Light/Dark choices remain fixed until changed. This session-only preference applies to manager, Add, status, and Scheduler windows; it resets to Follow system on the next launch. No custom control colors are applied; shared category icons provide file-type scanning cues.

## CI dependency caches and build scope

CI and release jobs use the SHA-pinned `Swatinem/rust-cache` v2.8.2 action for Cargo registry/git data and compiled dependency artifacts. Workspace crates and incremental artifacts are not retained. Cache identity includes the action's Rust compiler and Cargo manifest/lockfile fingerprinting, native OS/CPU, explicit target/job role, MSVC/SDK and runner-image environment. The engine/desktop/release-build roles use distinct keys; release desktop tests may reuse compatible main-branch desktop cache entries. No cross-target reuse is assumed. The action's dependency cleanup removes non-Cargo directories from the target archive; vcpkg checkout/install trees, CMake trees and shipped executables are not intentional cache inputs. Cargo metadata remains part of the cache action, not a build/test substitute.

Windows native jobs separately restore only vcpkg binary archives in `.cache/ulos/vcpkg`. Exact keys cover the pinned baseline, native architecture/target, MSVC and SDK versions, compiler/linker hashes, runner image and native build recipe/CMake configuration. No broad native restore prefix is used. `vcpkg install`, baseline validation, worker compilation, all CTests, architecture/static-DLL checks and relocated-worker loopback remain unconditional. Archive misses rebuild normally; cache service errors do not prevent cold builds. Native caches save only after the worker tests pass; Cargo caches save only on successful jobs. Main/version-tag jobs may save, while PRs only restore GitHub caches. Cache contents are executable build inputs, not a signing/provenance guarantee, and must contain no secrets.

New branch/PR commits cancel only older CI for that same ref. Release runs retain their separate non-cancelling concurrency guard. Both five-platform CI matrices and tag checks stay enabled. Ordinary CI validates Debug worker/desktop/tests plus the real Inno compiler synthetic installer smoke; it no longer spends time building optimized Windows application/Setup artifacts. Actual six-platform release packages, native integration tests, desktop tests, checksums and publication guards remain in `release.yml`.

The first build, cache eviction and compiler/SDK/runner-image changes can still be cold. Measure a successful trusted cache save followed by a warm restore on both Windows architectures before quoting a speedup. GitHub cache storage quotas can evict large dependency caches. To recover suspected stale/corrupt entries, delete the affected cache or increment the versioned `ulos-native-v1` / Rust cache key; do not disable validation or reuse an ABI-incompatible entry. Nothing here changes source tags or retroactively accelerates an already-running release.
