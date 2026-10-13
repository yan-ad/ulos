# Initialization status

Verified: 2026-10-13 on macOS 27.0.1, Apple Silicon (`aarch64-apple-darwin`), Rust 1.98.1 / Cargo 1.98.1. Cargo.lock captures the dependency graph. Git is initialized on `main`; no commit, remote, or push was created.

## Implemented

- Cargo workspace with core/CLI default members and a separate GPUI Kit 0.7.3 desktop crate.
- UI-independent HTTP/HTTPS streaming downloader, explicit output paths, unique `.part` staging, available response-length checks, and no-clobber publication.
- HTTP 200-only complete transfers; unsolicited partial or encoded bodies are rejected. Unknown-length and empty bodies are supported.
- Redirect validation, HTTPS downgrade rejection, disabled automatic referers, no embedded URL credentials, and network diagnostics stripped of URLs.
- Core progress events, CLI exit codes/help, and Ctrl+C cancellation. No URL or cookie logs.
- Minimal themed GPUI starter window with close-to-quit behavior. No download controls, fake queue, or GUI transfer integration.
- Persistent product, architecture, research, testing, and roadmap documents; GitHub Actions engine-test and desktop-check jobs for Linux, Windows, and macOS.

## Local verification

- `cargo test --locked`: **23 passed** (4 core validation, 14 HTTP/file-lifecycle integration, 5 CLI tests).
- `cargo clippy --locked --all-targets -- -D warnings`: passed for default members.
- `cargo fmt --all -- --check`: passed for all workspace sources.
- `cargo clippy --locked -p ulos-desktop -- -D warnings`: passed.
- `cargo test --locked --workspace`: passed; the same 23 tests ran with desktop dependencies included. The desktop has no interaction tests yet.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
- `cargo build --locked -p ulos-desktop`: passed using Xcode Command Line Tools; full Xcode was not needed on this host.
- Executable startup smoke: launched `target/debug/ulos-desktop`; macOS CoreGraphics reported a visible layer-0 window, 780 × 472 including window chrome. No startup errors appeared in its captured output. The smoke process was stopped afterward.

These are compiler, local-fixture, and startup checks. They do not establish visual rendering quality, accessibility, performance, live HTTPS interoperability, or Windows/Linux behavior. CI is configured but has not run remotely.

## Review after implementation

The initial UI contains only an honest status explanation and a CLI invocation example. It follows the framework theme without gradients, glass, invented metrics, sample downloads, or nonfunctional controls. Full brand/design direction is deliberately unresolved. Native visual, keyboard, screen-reader, contrast, and scaling review remains pending; do not treat the starter as a finished UI.

## Known limits

- No pause/resume, SQLite, persistent queues, segmentation, retries, rate controls, browser extension, installer, or auto-update.
- Temporary-file cleanup is best effort. Cleanup failures are not separately reported, and abrupt termination or pending filesystem work can leave `.part` files. No recovery scan exists yet.
- File setup and publication run on blocking workers. Cancellation can race with final publication already in progress; the CLI explicitly warns that a completed file may exist. Tokio runtime shutdown can wait for blocking filesystem calls. Validate slow/network-volume behavior before stronger cancellation promises.
- Explicit path validation currently checks directory status synchronously; future GUI integration must run request preparation off the render/UI thread.
- Files are synced before publication, but the destination directory is not synced. No crash-durability guarantee is made.
- The Rust 1.92 baseline comes from upstream GPUI requirements; only Rust 1.98.1 was tested locally.
- GPUI's macOS dependency graph emits a future-incompatibility warning for `block 0.1.6` (`static of uninhabited type`, Rust issue #74840). Current build passes. Review upstream updates before future toolchain upgrades; do not suppress or patch vendor code without investigation.
- Linux compositor/GPU/scaling matrix and Windows native checks remain pending. Cross-platform intent is not a support guarantee.
- Project name, license, Linux distro targets, browser targets, and distribution formats remain undecided. No Ulos open-source license is granted yet.

## Next work

1. Confirm Linux distro/desktop, licensing, and product naming.
2. Add persistent state and a tested cancellation/finalization model with crash recovery.
3. Implement validator-aware resume, then bounded segmentation where supported.
4. Wire real transfers into GPUI; review UI with the user.
5. Add explicit browser handoff through native messaging after the core/desktop flow is reliable.
