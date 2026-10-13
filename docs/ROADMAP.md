# Roadmap

Updated: 2026-10-13. See [Status](STATUS.md) for implementation and verification evidence. No dates or speed targets are promised.

## Milestone 1 — Workspace scaffold

**Initialized and locally verified on macOS; full native UI review pending.**

- Cargo workspace with core/CLI default members and a separate GPUI Kit desktop crate.
- Validated HTTP/HTTPS single-stream downloads through the core and CLI.
- Exclusive `.part` creation, streaming, applicable length checks, no-clobber publication, and best-effort owned-file cleanup.
- Minimal desktop starter with no transfer integration or simulated downloads.
- Local regression tests, formatting/lints, desktop executable build, and observed on-screen window recorded in Status.

No Linux/Windows runtime or visual/accessibility results are inferred from macOS. UI review follows initialization, per the user's request; the initial code review does not replace a native visual review.

## Milestone 2 — Reliable CLI transfers and persistence

**Planned.**

- Harden failures, cancellation/publication semantics, cleanup-error reporting, retries, and slow-filesystem behavior with local fixtures.
- SQLite queue storage, checkpoint durability, startup recovery, and migrations.
- Safe resume with validators and verified ranges; restart on full responses or changed representations.
- Rate controls and bounded segmentation only after server-support and integrity tests.
- Run configured Linux/Windows CI and perform native Linux graphical validation; agree on distro/version targets.

Exit evidence: reproducible interruption/restart and data-integrity tests, explicit crash-recovery behavior, and a recorded platform matrix. A database row must never be treated as proof that file bytes survived a crash.

## Milestone 3 — Desktop integration, then browser integration

**Planned.**

### Phase A: desktop

Wire core commands and events into GPUI Kit. Show real queue/progress/error state, explicit URL/output entry, cancellation, and only controls supported by the engine. Verify runtime ownership/shutdown, keyboard operation, accessibility, and Linux display/scale/GPU combinations. Review the UI with the user.

### Phase B: browser

Implement a small JS/TS extension and native messaging hosts for agreed Chromium/Firefox targets. Verify registration, extension allowlists, message validation, explicit import consent, and disconnect/error handling. No automatic clipboard polling or protected-content bypass.

Exit evidence: end-to-end desktop/browser scenarios on named platforms/versions, including failures and clean-install packaging checks. A development window is not a distributable release.

## Decisions before release

- Linux distribution/version scope and package formats.
- Browser families/channels and extension distribution method.
- Final product name and license; dependency licenses do not license Ulos.
- OS baselines, accessibility acceptance criteria, and authenticated-download policy.
