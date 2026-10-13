# Ulos: product scope

Updated: 2026-10-13.

## Status and decisions

Initialization is approved. Ulos is a working name taken from the project directory, not a final product name. The selected direction is an IDM-style desktop download manager implemented in Rust with GPUI Kit. This describes the workflow inspiration, not compatibility with IDM or reuse of its implementation.

Linux is a first-class development and release priority alongside macOS and Windows. Cross-platform support is a goal, not a verified platform guarantee. The current development host is macOS; these documents do not establish any completed build, test, or platform validation.

This document records the approved scaffold and future scope. See [Status](STATUS.md) for implemented features and local verification; do not infer verification from this scope. UI review follows scaffold initialization.

## Initial scaffold scope

- `ulos-core`: a UI-independent library that validates download requests and streams a single HTTP/HTTPS transfer to disk.
- `ulos-cli`: a URL/output command-line entry point using the core.
- `ulos-desktop`: a minimal GPUI Kit application, honestly labeled as a starter. Transfers are not wired into the initial desktop app.
- File safety: never overwrite an existing destination; use collision-safe staging names ending in `.part`, bounded-memory streaming, available length validation, and cleanup on failure.

See [Architecture](ARCHITECTURE.md) for the transfer contract and [Roadmap](ROADMAP.md) for sequencing.

## Proposed product progression

Reliable CLI transfers precede persistent queues/checkpoints, desktop integration, and browser integration. Future desktop workflows include explicit URL entry, queue visibility, progress, cancellation, and understandable failures. Pause/resume, scheduling, rate limits, and bounded segmentation require separate implementation and verification. None are implied by a starter window.

Browser integration is planned through Chromium/Firefox native messaging and a small JavaScript/TypeScript extension. Import should be an explicit user action, with permission and destination controls. There is no automatic clipboard polling.

## Boundaries

- HTTP/HTTPS first; other protocols are not in the scaffold scope.
- No DRM or captcha bypass, access-control circumvention, or promise to download every site/video.
- No claimed speed improvement. Segmentation is considered only when actual server range behavior supports it, with bounded concurrency and server-respecting limits.
- Do not reuse code from IDM or ABDM. Dependency licenses and notices must be reviewed independently.
- No Ulos license has been selected. Decide licensing before distribution; do not assume an open-source license from a dependency's license.

## Open requirements

Confirm target Linux distributions/versions, supported browsers and release channels, final name, and Ulos license. Packaging formats, supported OS baselines, accessibility requirements, authenticated-download handling, and release policy also need decisions. Record evidence before describing any platform as supported.
