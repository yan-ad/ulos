# Ulos: product scope

Updated: 2026-10-14.

## Status and decisions

Initialization is approved. Ulos is a working name taken from the project directory, not a final product name. The selected direction is an IDM-style desktop download manager implemented in Rust with GPUI Kit. This describes the workflow inspiration, not compatibility with IDM or reuse of its implementation.

Linux is a first-class development and release priority alongside macOS and Windows. Cross-platform support is a goal, not a verified platform guarantee. The current development host is macOS; these documents do not establish any completed build, test, or platform validation.

This document records the implemented manager foundation and future scope. See [Status](STATUS.md) for implemented features and local verification; do not infer verification from this scope. UI review follows scaffold initialization.

## Current HTTP scope

- `ulos-core`: a UI-independent library that validates download requests and streams a single HTTP/HTTPS transfer to disk.
- `ulos-cli`: a URL/output command-line entry point using the shared core `HttpDownloadTask`.
- `ulos-desktop`: light manager foundation with native titlebar, native macOS system menus (Ulos / Tasks / File / Edit / Downloads / View / Help), and non-macOS in-window Kit dropdown menus. One shared toolbar has stock icon-only Buttons except labeled Add URL, accessible labels/built-in tooltips, and a stock search Input on the same row, followed by a category tree, viewport-virtualized Kit DataTable with four data columns and stable-ID checkboxes, and status bar. The latest light IDM 6.28 layout reference supersedes the earlier dark AB reference; no source or branding is copied and native Kit rendering is not pixel-identical.
- Real bounded session queue: at most 500 records, one active transfer, FIFO automatic advancement, independent presentation search/category filters/name-size sorting, stable selection, metadata-only removal/clear, and show-in-folder for finished records. No persisted queue or history.
- Each Add action opens an independent native confirmation window (up to 16 live drafts), providing Address, File name, Save as, native folder/save pickers, Start download, and Download later. Start download starts the chosen item when idle; when busy it queues it and enables advancement. Download later does not enable advancement, but an already-running queue may process it.
- Filename suggestions use only the decoded final URL path segment (fallback `download.bin`), ignoring query/fragment. No HEAD or Content-Disposition lookup. Suggestions and selected/edited names are sanitized into a bounded portable component while preserving manual overrides.
- Filename click / Details opens a live native status window: one reused/activated window per download, with different IDs viewable concurrently. Closing status does not cancel a transfer. Closing the main window leaves child windows and their shared session owner alive; the last-window close quits. Removing/clearing records closes their related status windows, leaving downloaded files untouched. Options remains a Kit modal. Status, Speed limiter, and Options on completion tabs do not imply new engine features: the latter two explain that they are unimplemented. Hide details controls one actual HTTP-stream progress section; Pause is disabled, Cancel is active only for active work.
- File safety: never overwrite an existing destination; use collision-safe staging names ending in `.part`, bounded-memory streaming, available length validation, and cleanup on failure.

See [Architecture](ARCHITECTURE.md) for the transfer contract and [Roadmap](ROADMAP.md) for sequencing.

Controls retain stock Kit theme defaults in the manager and child windows, with Light/Dark switching and no per-control size, padding, color, radius, or Button variant overrides. Layout wrappers provide geometry; 48px DataTable rows accommodate default Buttons. macOS `⌘N`, `⌘,`, and `⌘Q` route to Add, available main-window Options, and Quit; native Edit targets focused Kit inputs. These presentation/menu changes add no transfer or scheduler behavior.

All application widgets, content text, and panels must use GPUI Kit, without manual UI exceptions: Toolbar, StatusBar, Label, GroupBox, DescriptionList, Empty, DataTable, Form/Input, Button, Checkbox, Tabs, Progress/Spinner and other Kit components. Kit-reexported `div`, `v_flex`, and `h_flex` are layout-only. Native OS chrome and path pickers are not a bespoke GTK/Swift widget implementation. Outstanding pickers block Add close/dismiss/submit until resolved.

## Proposed product progression

HTTP core/CLI reliability and the session manager foundation are the current focus. Persistent queues/history/checkpoints and browser integration follow; session queue visibility is implemented. Pause/resume, scheduling, rate limits, and bounded segmentation require separate implementation and verification. None are implied by single-transfer integration.

Browser integration is planned through Chromium/Firefox native messaging and a small JavaScript/TypeScript extension. Import should be an explicit user action, with permission and destination controls. Copied-link detection is implemented separately as an explicit Options opt-in, off by default and only for this session. Once-per-second checks use GPUI's asynchronous clipboard API only while the main Ulos window is active, no Add/status window is open, and Options is closed; eligibility is rechecked after each async read. Disabled detection performs no clipboard reads. Accept only one whitespace-trimmed HTTP/HTTPS URL within 16 KiB, rejecting embedded credentials and multi-link/non-URL input; a fingerprint suppresses repeated proposals while the form is closed. Non-URL text is not retained or logged. Accepted source/query data must remain in memory for signed downloads; Details strips query, fragment, and credentials. Detection only prefills a confirmation form and makes no autonomous download or naming network request. Wayland access depends on the compositor.

Scheduler and Grabber are disabled placeholders. Resume, limiting, and completion actions are not implemented.

## Boundaries

- HTTP/HTTPS now; torrent support is explicitly planned for a later stage, with no current torrent dependencies, downloads, or UI controls. See [Torrent scope](TORRENTS.md).
- No DRM or captcha bypass, access-control circumvention, or promise to download every site/video.
- No claimed speed improvement. Segmentation is considered only when actual server range behavior supports it, with bounded concurrency and server-respecting limits.
- Do not reuse code from IDM or ABDM. Dependency licenses and notices must be reviewed independently.
- Ulos is licensed under MIT. Dependencies retain their own licenses and notices, which must be reviewed before distribution.

## Open requirements

Confirm target Linux distributions/versions, supported browsers and release channels, and final name. Packaging formats, supported OS baselines, accessibility requirements, authenticated-download handling, and release policy also need decisions. Record evidence before describing any platform as supported.
