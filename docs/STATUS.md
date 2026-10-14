# Implementation status

Updated: 2026-10-14. Local host: macOS 27.0.1, Apple Silicon, Rust/Cargo 1.98.1. Initialization is committed as `7bf99c6`; subsequent HTTP/manager changes remain uncommitted. No push performed.

## Implemented foundation

- HTTP/HTTPS single-stream CLI and desktop transfers, using the shared UI-independent Rust engine and owned Tokio task service.
- Light, reference-structured manager: native titlebar, Tasks/File/Downloads/View/Help menu strip, icon-over-label toolbar, category tree, compact filename/size/status/rate table, and session status bar. Uses GPUI Kit primitives and bundled Lucide assets; no copied IDM code, logo, or sample rows. The native platform still determines window chrome; this is structural similarity, not pixel-identical Windows styling.
- Real bounded session queue (500 records maximum), FIFO advancement, one active transfer, filename search, category/status/queued filters, sort, stable-ID selection, cancellation, clear-finished, and metadata-only removal. Stop queue allows current work to finish without advancing; Stop all also requests cancellation. Download now starts the new item if idle; while busy it queues the item and enables progression.
- Add Download dialog with URL, detected/editable filename, explicit save path, native Save As and folder pickers, Start download and Download later. URL-path detection makes no network request and ignores query/fragment data. Manual filename/path edits synchronize; explicit path edits take precedence over an earlier folder selection. No-overwrite publication remains enforced by the core.
- Portable single-component filename sanitizer: controls/bidi characters, path separators and Windows punctuation, leading/trailing spaces/dots, reserved Windows device names, and a 180-byte UTF-8 filename limit with bounded extension preservation. File contents/type are not inferred from extensions; categories are presentation hints.
- Off-by-default, session-only clipboard detection enabled through Options. Polls once per second only when the window is active and no Add/Details/Options dialog is open. Uses GPUI async clipboard access, rechecks focus and consent after completion, accepts one bounded HTTP/HTTPS URL, deduplicates proposals via a fingerprint, and opens confirmation only. No clipboard logging, disk history, automatic network request, or automatic download. Accepted candidate validation is bounded at 16 KiB; the OS may allocate its clipboard response before application validation.
- Download details opens from a filename/Details action and after Start download in the Add dialog. Live bytes, status, measured speed, estimated time, overall progress, disclosure toggle, actual single-stream connection table, and cancellation. Source display omits query, fragment, and credentials (not arbitrary secrets in URL paths). Closing the details view does not cancel a transfer. Reopening and terminal updates are tested.
- Speed limiter and completion tabs explain unavailable features; Pause, Scheduler, and Grabber are disabled and labeled not implemented. No fabricated connection rows or resume capability.
- Progress/state observation is bounded and latest-state/coalesced. Telemetry rates expire after two seconds without samples; unknown totals/rates/ETA are not invented. Networking/filesystem work is outside GPUI rendering.

## Module map

```text
ulos-core/http.rs               safe HTTP transfers
ulos-core/tasks.rs              owned worker, latest state, cancellation
ulos-core/naming.rs             pure filename detection/sanitization
ulos-desktop/queue.rs           bounded session model and measured telemetry
ulos-desktop/manager.rs         scheduling, dialog ownership, clipboard policy
ulos-desktop/manager_render.rs  Kit menus/sidebar/toolbar/table/status bar
ulos-desktop/add_download.rs    retained Add dialog and native path prompts
ulos-desktop/download_details.rs live status/detail presentation
ulos-desktop/clipboard.rs       bounded URL proposal/deduplication
ulos-desktop/palette.rs         light/default and optional dark theme
```

## Verification

Final integrated suite passed 76 tests: 19 core unit/naming, 14 HTTP/file lifecycle, 6 task lifecycle, 5 CLI, and 32 desktop/model/interaction tests. Tests use local HTTP fixtures and temporary files, not external download services. Desktop tests cover sequential execution, stop queue/all, metadata-only removal, search/filters, input retention, save/folder prompts, edit precedence, clipboard consent/deduplication, redacted details, reopening, and cancellation through the details view.

- `cargo test --locked --workspace`: 76 passed.
- `cargo clippy --locked --workspace --all-targets -- -D warnings`: passed.
- `cargo fmt --all -- --check`, `cargo build --locked -p ulos-desktop`, and `git diff --check`: passed. Local Markdown links also passed.
- Light palette contrast measured: body on white 15.65:1, muted text on white 7.82:1, white primary-button text on blue 5.93:1. This is not a full native accessibility audit.
- Native macOS manager window screenshots reviewed before and after fixes. Menu strip, toolbar, category tree, list header, empty state and footer are aligned. Review found omitted icons in the default Kit bundle and a separate primary-button token; both were corrected and confirmed using full bundled assets and proper theme tokens. Review process stopped afterward.
- Accessibility automation on this host was not authorized (`AXIsProcessTrusted() == false`); no permissions were changed. Native dialog automation, full screen-reader review, minimum-size/native keyboard traversal, and Windows/Linux screenshots remain unverified. GPUI test-window dialog/keyboard behavior is covered separately.
- CI is configured for macOS/Windows/Linux, but no remote CI run occurred. `block 0.1.6` still reports an upstream future-compatibility warning; current compilation succeeds.

## Known limits

- Session queue/history and clipboard preference are not persisted. No pause/resume, durable checkpoints, segmentation, retries/rate limiting, completion automation, browser capture, torrent engine, installer, or updater.
- Best-effort temporary-file cleanup, no crash-recovery scanner. Cancellation can race with final publication; clients disclose that a completed file may exist. Slow/network-volume behavior and directory crash durability remain unverified. Desktop runtime shutdown timeout is two seconds, not proof that all blocking filesystem work has finished.
- URL-path naming only; no HEAD or Content-Disposition detection. Manual paths do not expand `~`/environment variables. Non-UTF-8 path selection is rejected rather than altered. Existing final files are never replaced.
- Clipboard behavior depends on OS permissions/compositor support, especially Wayland. No global background-service guarantee. Signed URLs must remain in transfer memory to work; neither clipboard content nor source queries are written to logs/history.
- Details are a same-window Kit modal, not a separate native status window per transfer. One HTTP stream is displayed; resume capability is truthfully not implemented. Hidden speed/completion functionality remains roadmap.
- Project name/license, Linux baseline, browser targets, and distribution formats remain undecided. No Ulos open-source license is granted yet. No new commit was made during this continuation.

## Next

1. Native accessibility/keyboard and Linux compositor/GPU tests.
2. Persistent queue, checkpoint durability and recovery.
3. Safe validator-aware resume, then bounded segmentation.
4. Browser native-messaging handoff; later torrent engine research/license review.
