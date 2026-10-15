# Roadmap

Updated: 2026-10-14. See [Status](STATUS.md) for implementation and verification evidence. No dates or speed targets are promised.

## Milestone 1 — Workspace scaffold

**Initialized and locally verified on macOS; full native UI review pending.**

- Cargo workspace with core/CLI default members and a separate GPUI Kit desktop crate.
- Validated HTTP/HTTPS single-stream downloads through the core and CLI.
- Exclusive `.part` creation, streaming, applicable length checks, no-clobber publication, and best-effort owned-file cleanup.
- Minimal desktop starter at initialization; subsequent real HTTP integration is tracked below. No simulated downloads.
- Local regression tests, formatting/lints, desktop executable build, and observed on-screen window recorded in Status.

No Linux/Windows runtime or visual/accessibility results are inferred from macOS. UI review follows initialization, per the user's request; the initial code review does not replace a native visual review.

## Current continuation — HTTP manager foundation

**Implemented foundation; verification is tracked separately in [Status](STATUS.md).** Initialization was committed as `7bf99c6`; do not infer later commit or test outcomes from this roadmap.

- Shared UI-independent `HttpDownloadTask`, bounded coalescing state, caller-supplied Tokio handle, and cancellation on drop.
- Light IDM 6.28-inspired manager structure using GPUI Kit, native titlebar, macOS system menus/non-macOS Kit dropdowns, stock icon toolbar and grouped collapsible sidebar, viewport-virtualized borderless Kit DataTable with ten columns total: selection checkbox, Name, Size, Status, Transfer rate, Queue, Time left, Last try, Description, and Type, 36px rows and a 32px header, and StatusBar. This supersedes the earlier dark AB reference, without copied branding/source or an exact-pixel-match promise.
- Real 500-record session queue with one active HTTP or torrent transfer, explicit queue-order advancement, presentation-only search/filter/sort, stable selection, metadata-only removal/clear, and finished-file reveal.
- Independent native Add windows (new draft per invocation, maximum 16) with URL-path name suggestion, portable sanitization, native folder/save pickers, manual-name preservation, Start download / Download later.
- Native status windows (one reused/activated window per download, different IDs concurrent), real per-connection telemetry, Hide details, active cancellation, verified session Pause/Resume, functional Speed/Completion tabs, and persistent shared progress footer.
- Options copied-link detection: explicit off-by-default session opt-in, main-window-active one-second async checks suppressed by any Add/status/Scheduler window or Options dialog and rechecked after reads, bounded one-URL proposals and fingerprint deduplication, no autonomous downloads or non-URL retention/logging.
- Kit-only widgets, text and panels, with layout-only Kit `div`/`v_flex`/`h_flex`; Options remains a Kit modal. Child roots own their inputs/subscriptions and a strong shared manager handle, while manager registries keep weak child content. Main close leaves children usable; last-window close quits. Picker guards prevent Add dismissal/submission while outstanding. Record removal closes related status windows without file deletion.
- Change-gated 100ms watch observation and stale-rate refresh, unchanged-status snapshot suppression, and structural 500-record viewport/child-input render regressions. No measured latency or FPS claim.

Metadata-only SQLite destination/status/bytes history and session scheduling are implemented; durable source-bearing queues/checkpoints, browser bridge, and completion actions beyond confirmed shutdown remain future work. Restored source-less history cannot start/retry. Keyboard/focus, accessibility, complete runtime teardown, and native Linux/Windows validation remain pending; a test adapter is not native-platform evidence.

## Current continuation — controlled advanced transfers

**Source implemented; integration verification is partial and tracked separately.** Do not infer a full-workspace lint/test pass or native screenshot from this list.

- Desktop `start_with_options` controlled engine, default four bounded 1–8 connections, observed range/strong-ETag capability and response-by-response identity checks; truthful single-stream fallback and actual worker counts. CLI legacy single-stream/no-overwrite unchanged.
- Session Pause/Resume closes responses and resumes written offsets through validated ranges; paused active work retains its queue slot. No checkpoint or crash-resume claim.
- Live aggregate consumption limit (positive checked KiB/s input or unlimited), shared before-write grants across workers; not wire QoS or an unmeasured speedup. Connection changes apply only on queued/inactive-failed next starts.
- Reject default, finite numbered Rename with no-clobber publication, consent-gated staged Overwrite with original preservation on transfer failure and regular-file guards. Retry remains explicit.
- Real OS completion shutdown provider, confirmed per-download default-off intent, published-success/all-finished gate, visible cancellable 30-second countdown, and final live-status-window check. New work, retry, failure/cancellation, removal or status/all-window close cancel intent. No forced app closure/elevation; no host poweroff test.
- Native macOS full-content status chrome with fixed Download title, Kit icon-bearing segmented Status/Speed/Completion tabs, actual connection table, and percentage inside the shared 24px Kit progress footer on all tabs. Stock controls remain unmodified.

Outstanding: adversarial range/limiter/publication integration, latest full-workspace checks, native screenshot/focus/accessibility review and Linux/Windows execution/graphics validation. Test command-plan/gate logic without executing OS poweroff or changing settings. [Testing](TESTING.md) lists acceptance cases; [Status](STATUS.md) owns actual results.

## Current continuation — session queues, schedules, and compact table

**Source behavior, not a new verification result.** Scheduler has up to 32 named queues (default Main queue), create/rename/non-main deletion with reassignment to Main, membership from main-table checkboxes, explicit waiting-file Move earlier/later, and user descriptions. Active/paused work cannot move. One global HTTP transfer selects minimum `(queue_order, id)` across running queues; main-toolbar queue controls target Main, named queues use Scheduler. Stop queue lets active work finish; Stop All stops every queue and cancels active work.

Once/weekly local schedules require Ulos open: future Once `YYYY-MM-DD` / `HH:MM`, nonempty weekly weekdays, optional unequal stop with overnight handling. Skip nonexistent DST occurrences/use the earlier fold instant; wake only catches an unexpired window, never replays ended windows (no-stop catch-up only on its local start day). Repeated ticks/manual stop cannot restart an occurrence and recurrence cannot retry terminal rows. Reused Scheduler native Root survives main close, gates both clipboard phases, and schedule enablement cancels/blocks completion shutdown.

The ten-column table uses checkbox selection independent of row focus, Kit filename Links with colored category icons, focused-row Enter/double-click Details, actual-rate ETA, local actual-worker Last try, and explicit Description; unknown metadata stays blank. No Button variant or decorative rounded border. Queue names, schedules, membership/order, descriptions, and attempt times remain session-only; metadata history contains neither sources nor schedules and is not durable recovery. Keep network secrets out of logs. See Testing for planned verification; no new run/result is claimed here.

## Milestone 2 — Reliable HTTP transfers and persistence

**Planned.**

- Harden failures, cancellation/publication semantics, cleanup-error reporting, retries, and slow-filesystem behavior with local fixtures.
- Extend metadata-only SQLite history into source-bearing queue storage, checkpoint durability, startup recovery, and migrations, with explicit privacy and recovery contracts.
- Durable recovery with validators, verified offsets and ranges, and explicit safe restart decisions. Current session resume fails on changed representations or unexpected If-Range full responses; it must not silently restart or append.
- Extend source-implemented aggregate rate controls and bounded session segmentation with broader integration/platform verification and durable recovery tests.
- Run configured Linux/Windows CI and perform native Linux graphical validation; agree on distro/version targets.

Exit evidence: reproducible interruption/restart and data-integrity tests, explicit crash-recovery behavior, and a recorded platform matrix. A database row must never be treated as proof that file bytes survived a crash.

## Milestone 3 — Durable desktop workflows, then browser integration

**Planned.**

### Phase A: desktop

Extend the implemented session queue into durable workflows after persistence is ready. Keep only controls supported by the engine. Verify runtime ownership/shutdown, keyboard operation, accessibility, and Linux display/scale/GPU combinations. Review the UI with the user.

### Phase B: browser

Implement a small JS/TS extension and native messaging hosts for agreed Chromium/Firefox targets. Verify registration, extension allowlists, message validation, explicit import consent, and disconnect/error handling. Keep the implemented clipboard opt-in separate from browser import; no autonomous clipboard-triggered downloads or protected-content bypass.

Exit evidence: end-to-end desktop/browser scenarios on named platforms/versions, including failures and clean-install packaging checks. A development window is not a distributable release.

## Current continuation — Torrent files

**Source-implemented; verification and native packaging acceptance remain separate.**

- Separate UI-independent `torrent.rs` backend using reviewed Apache-2.0 `librqbit` 9.0.1; Ulos remains MIT and retains third-party notices.
- Offline bounded v1 / compatible v1-hybrid metainfo review, strict portable paths, tracker-only explicitly dispatched downloads, payload upload disabled, joined engine teardown and full piece re-verification before no-clobber Reject/Rename publication.
- Add New → HTTP URL / Torrent file; `.torrent` picker/drop/startup/OS-open confirmation; same categories, shared queue/table with appended Type, no torrent sidebar group or special download folder.
- Shared global slot, named queues, session scheduler and post-publication completion; honest unavailable torrent live rate/connection controls, actual session pause, metadata-only v2 history and no recursive multifile directory removal.
- Development macOS bundle/imported UTI, Linux desktop entry and explicit per-user Windows Open With registration aid; none forces defaults or establishes an installed/verified association.

Pending: execute the local handcrafted-peer/integrity/cancel/pause and mixed-queue test plan, native open/drop/association checks, package notice audit and locked-graph MSRV checks. Magnets, pure v2, durable source/checkpoint recovery and torrent rate/connection controls remain unimplemented. See [Torrent contract](TORRENTS.md) and [Testing](TESTING.md). No public trackers/initial peers are used in tests; no native distribution or cross-compilation result follows from source implementation.

## Decisions before release

- Linux distribution/version scope and package formats.
- Browser families/channels and extension distribution method.
- Final product name and dependency license/notice audit; Ulos is licensed under MIT.
- OS baselines, accessibility acceptance criteria, and authenticated-download policy.
