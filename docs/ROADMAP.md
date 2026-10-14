# Roadmap

Updated: 2026-10-13. See [Status](STATUS.md) for implementation and verification evidence. No dates or speed targets are promised.

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
- Light IDM 6.28-inspired manager structure using GPUI Kit, native titlebar, in-window menu strip, icon-over-label toolbar, category tree, compact four-column table, and status bar. This supersedes the earlier dark AB reference, without copied branding/source or an exact-pixel-match promise.
- Real 500-record session queue with one active HTTP transfer, FIFO advancement, presentation-only search/filter/sort, stable selection, metadata-only removal/clear, and finished-file reveal.
- Add form with URL-path name suggestion, portable sanitization, native folder/save pickers, manual-name preservation, Start download / Download later.
- Live same-window Details modal, real telemetry and one HTTP-stream row, Hide details, active cancellation, disabled Pause, and honest explanatory limiter/completion tabs.
- Options copied-link detection: explicit off-by-default session opt-in, active-window one-second async checks, bounded one-URL proposals and fingerprint deduplication, no autonomous downloads or non-URL retention/logging.

No persistent queue/history, resume, limiter, scheduler, site grabber, browser bridge, or completion actions are implemented. Scheduler/Grabber remain disabled. Keyboard/focus, accessibility, complete shutdown, and native Linux/Windows validation remain pending; a test adapter is not native-platform evidence.

## Milestone 2 — Reliable HTTP transfers and persistence

**Planned.**

- Harden failures, cancellation/publication semantics, cleanup-error reporting, retries, and slow-filesystem behavior with local fixtures.
- SQLite queue storage, checkpoint durability, startup recovery, and migrations.
- Safe resume with validators and verified ranges; restart on full responses or changed representations.
- Rate controls and bounded segmentation only after server-support and integrity tests.
- Run configured Linux/Windows CI and perform native Linux graphical validation; agree on distro/version targets.

Exit evidence: reproducible interruption/restart and data-integrity tests, explicit crash-recovery behavior, and a recorded platform matrix. A database row must never be treated as proof that file bytes survived a crash.

## Milestone 3 — Durable desktop workflows, then browser integration

**Planned.**

### Phase A: desktop

Extend the implemented session queue into durable workflows after persistence is ready. Keep only controls supported by the engine. Verify runtime ownership/shutdown, keyboard operation, accessibility, and Linux display/scale/GPU combinations. Review the UI with the user.

### Phase B: browser

Implement a small JS/TS extension and native messaging hosts for agreed Chromium/Firefox targets. Verify registration, extension allowlists, message validation, explicit import consent, and disconnect/error handling. Keep the implemented clipboard opt-in separate from browser import; no autonomous clipboard-triggered downloads or protected-content bypass.

Exit evidence: end-to-end desktop/browser scenarios on named platforms/versions, including failures and clean-install packaging checks. A development window is not a distributable release.

## Later milestone — Torrent support

**Planned; no torrent dependencies or UI controls today.**

Research engine suitability and licensing before selection; implement a separate core engine only after HTTP reliability and desktop integration. Validate magnet/`.torrent` metadata with bounded parsing, safe no-overwrite multifile destinations, and piece-hash integrity. Decide tracker/DHT privacy, seeding consent/defaults, bandwidth limits, and storage allocation/cleanup before exposing controls. Use deterministic fixtures and legal payloads. Introduce shared abstractions only when the second concrete backend needs them; persist protocol-tagged sources when SQLite is implemented. See [Torrent scope](TORRENTS.md).

## Decisions before release

- Linux distribution/version scope and package formats.
- Browser families/channels and extension distribution method.
- Final product name and license; dependency licenses do not license Ulos.
- OS baselines, accessibility acceptance criteria, and authenticated-download policy.
