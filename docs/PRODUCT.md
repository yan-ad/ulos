# Ulos: product scope

Updated: 2026-10-14.

## Status and decisions

Initialization is approved. Ulos is a working name taken from the project directory, not a final product name. The selected direction is an IDM-style desktop download manager implemented in Rust with GPUI Kit. This describes the workflow inspiration, not compatibility with IDM or reuse of its implementation.

Linux is a first-class development and release priority alongside macOS and Windows. Cross-platform support is a goal, not a verified platform guarantee. The current development host is macOS; these documents do not establish any completed build, test, or platform validation.

This document records the implemented manager foundation and future scope. See [Status](STATUS.md) for implemented features and local verification; do not infer verification from this scope. UI review follows scaffold initialization.

## Current transfer scope

- `ulos-core`: a UI-independent library that validates download requests and provides both legacy single-stream HTTP and controlled verified-range desktop transfers to disk.
- `ulos-cli`: a URL/output command-line entry point using the shared core `HttpDownloadTask`.
- `ulos-desktop`: light manager foundation with native titlebar, native macOS system menus (Ulos / Tasks / File / Edit / Downloads / View / Help), and non-macOS in-window Kit dropdown menus. One shared toolbar has stock icon-only Buttons except labeled Add New, accessible labels/built-in tooltips, and a stock search Input on the same row, followed by grouped Downloads / File types navigation, viewport-virtualized Kit DataTable with ten columns total: selection checkbox, Name, Size, Status, Transfer rate, Queue, Time left, Last try, Description, and Type with stable-ID checkboxes independent of row focus, and status bar. The latest light IDM 6.28 layout reference supersedes the earlier dark AB reference; no source or branding is copied and native Kit rendering is not pixel-identical.
- Real bounded session queue: at most 500 records, one active transfer, explicit queue-order automatic advancement, independent presentation search/category filters/name-size sorting, stable selection, metadata-only removal/clear, and show-in-folder for finished records. SQLite stores metadata-only destination/status/bytes history, not source URLs or schedules. Restored source-less records cannot start or retry. Queue names, membership, ordering, schedules, descriptions, and Last try are session-only; this is not durable transfer recovery.
- Each Add action opens an independent native confirmation window (up to 16 live drafts), providing Address, File name, Save as, native folder/save pickers, Start download, and Download later. Start download starts the chosen item when idle; when busy it queues it and enables advancement. Download later does not enable advancement, but an already-running queue may process it.
- Filename suggestions use only the decoded final URL path segment (fallback `download.bin`), ignoring query/fragment. No HEAD or Content-Disposition lookup. Suggestions and selected/edited names are sanitized into a bounded portable component while preserving manual overrides.
- Filename Link / focused-row Enter / row double-click / Details opens a live native status window: one reused/activated window per download, with different IDs viewable concurrently. Closing status does not cancel a transfer, but withdraws shutdown intent/countdown. Closing the main window leaves child windows and their shared session owner alive; the last-window close quits. Removing/clearing records closes their related status windows, leaving downloaded files untouched. Options remains a Kit modal. Status, Speed, and Completion are functional Kit segmented tabs with stock icons. A shared footer shows actual progress with the percentage inside the Kit bar on all tabs. Hide details controls actual per-connection telemetry. HTTP Pause/Resume works only within verified range sessions; unsupported streams disable it. Speed controls aggregate limiting and next-start connections. Completion requires parent-confirmed power intent and a visible cancellable countdown.
- File safety: Reject is the default; Rename preserves the original using bounded numbered no-clobber publication. HTTP Overwrite is an explicit consent-gated exception for regular files after successful unique staging, never implicit replacement. Use bounded-memory streaming, applicable length validation, and best-effort owned staging cleanup on failure.

See [Architecture](ARCHITECTURE.md) for the transfer contract and [Roadmap](ROADMAP.md) for sequencing.

Controls retain stock Kit theme defaults in the manager and child windows, with System-default appearance and explicit Light/Dark overrides and no per-control size, padding, color, radius, or Button variant overrides. Layout wrappers provide geometry; Borderless DataTable uses public Kit APIs for 36px rows and a 32px header; filename Links and colored category icons replace filename Buttons. Compact, undecorated rows prioritize real download metadata. macOS `⌘N`, `⌘,`, and `⌘Q` route to Add, available main-window Options, and Quit; native Edit targets focused Kit inputs. These presentation/menu changes add no transfer or scheduler behavior.

All application widgets, content text, and panels must use GPUI Kit, without manual UI exceptions: Toolbar, StatusBar, Label, GroupBox, DescriptionList, Empty, DataTable, Form/Input, Button, Checkbox, Tabs, Progress/Spinner and other Kit components. Kit-reexported `div`, `v_flex`, and `h_flex` are layout-only. Native OS chrome and path pickers are not a bespoke GTK/Swift widget implementation. Outstanding pickers block Add close/dismiss/submit until resolved.

## Torrent workflow and limits

Add New offers HTTP URL / Torrent file. Local picker, drag/drop, startup paths/file URLs and OS-open callbacks propose at most 16 offline review windows; they never automatically start networking. Review name, bytes, manifest, tracker count and destination, then explicitly Start or confirm Download later (an already-running queue may dispatch it). HTTP and torrents share the same table, existing category sidebar, named queues, single global slot and scheduler. Type is appended; single-file payload category is derived from its filename, multifile is Other. There is no torrent group or dedicated destination folder.

Torrent parsing is bounded and strict; tracker queries remain private in memory. Tracker-only discovery starts on dispatch, with DHT/LSD/listeners/UPnP/persistence/server/seeding off and payload upload disabled at compile/session levels. This can reduce swarm progress. Actual session Pause/Resume works; live rate/connection settings are unavailable. Joined engine teardown, full SHA-1 piece checks and file sync precede no-clobber file/directory publication; only Reject/Rename are supported. Completion actions require successful publication. Schema v2 history retains protocol/directory flags, no source/metainfo/tracker, and cannot restart/retry restored rows. Multifile removal is metadata-only, never recursive deletion.

Local opener packaging aids do not install or force default associations. Native packaging/launch, platform rendering and accessibility verification remain separate pending work. Engine selection is reviewed `librqbit` 9.0.1 under Apache-2.0; Ulos retains MIT and redistribution notices. Details: [Torrent contract](TORRENTS.md).

## Proposed product progression

HTTP/core CLI reliability, local torrent downloads and the shared session manager are implemented foundations. Persistent queues/history/checkpoints and browser integration follow; session queue visibility is implemented. Session pause/resume, aggregate rate limits, and bounded segmentation are source-implemented in the controlled engine, with verification tracked separately. Durable restart recovery remains future work; local once/weekly session schedules are implemented while Ulos stays open. No feature implies a measured speed improvement.

Browser integration is planned through Chromium/Firefox native messaging and a small JavaScript/TypeScript extension. Import should be an explicit user action, with permission and destination controls. Copied-link detection is implemented separately as an explicit Options opt-in, off by default and only for this session. Once-per-second checks use GPUI's asynchronous clipboard API only while the main Ulos window is active, no Add/status/Scheduler window is open, and Options is closed; eligibility is rechecked after each async read. Disabled detection performs no clipboard reads. Accept only one whitespace-trimmed HTTP/HTTPS URL within 16 KiB, rejecting embedded credentials and multi-link/non-URL input; a fingerprint suppresses repeated proposals while the form is closed. Non-URL text is not retained or logged. Accepted source/query data must remain in memory for signed downloads; Details strips query, fragment, and credentials. Detection only prefills a confirmation form and makes no autonomous download or naming network request. Wayland access depends on the compositor.

Scheduler supports up to 32 named queues including default Main queue; Controlled session resume, limiting, and confirmed real shutdown are implemented; persistent checkpoints and other completion actions are not.

## Session scheduling and metadata

Create/rename queues in Scheduler; assign main-table checkbox selections there. Delete only non-main queues, reassigning members to Main; active/paused work cannot move. Explicit Move earlier/later affects waiting-file order independently of table sort. One global transfer selects minimum `(queue_order, id)` across running queues. Main-toolbar Start/Stop Queue controls Main queue; named queues use Scheduler. Stop queue lets active work finish; Stop All stops all queues and cancels active work.

Once requires a future local `YYYY-MM-DD` / `HH:MM`; Weekly requires at least one weekday. Optional unequal stop uses local `HH:MM`, with earlier stop overnight. Skip nonexistent DST occurrences and choose the earlier ambiguous instant. Wake catches only an unexpired current window, never ended windows; no-stop catch-up is limited to the start day, not an automatic end-of-day stop. Repeated ticks/manual stop do not restart the same occurrence, and recurrence does not retry terminal records. The reused native Scheduler Root survives main closure and gates both clipboard phases; Ulos must remain open.

Time left uses actual rate only, Last try displays local actual worker-start time, and Description is user-entered in Scheduler. Unknown table metadata stays blank. Names, schedules, membership/order, descriptions, and attempt times are session-only, not source-bearing durable recovery. Network secrets must not be logged.

## Boundaries

- HTTP/HTTPS plus local v1 `.torrent` / compatible v1 hybrids; no magnets or pure v2. See [Torrent contract](TORRENTS.md).
- No DRM or captcha bypass, access-control circumvention, or promise to download every site/video.
- No claimed speed improvement. Segmentation is considered only when actual server range behavior supports it, with bounded concurrency and server-respecting limits.
- Do not reuse code from IDM or ABDM. Dependency licenses and notices must be reviewed independently.
- Ulos is licensed under MIT. Dependencies retain their own licenses and notices, which must be reviewed before distribution.

## Open requirements

Confirm target Linux distributions/versions, supported browsers and release channels, and final name. Packaging formats, supported OS baselines, accessibility requirements, authenticated-download handling, and release policy also need decisions. Record evidence before describing any platform as supported.


## Advanced workflow limits and safety

The HTTP desktop defaults to four connections (Add offers 1/2/4/8; engine and Details accept 1–8). A verified identity-encoded range probe and strong ETag are required for segmentation and pause; otherwise one full stream is used. Every resumed/segment response verifies identity and exact range; changed resources or unexpected If-Range full responses fail rather than silently restart or append. Actual connection count can be lower than configured. Paused work retains the single active queue slot. Checkpoints and crash resume are not implemented.

HTTP live speed input is positive whole-number 1–1,048,576 KiB/s, or unlimited when disabled. The shared budget limits chunk/output consumption before writes across all connections, not TCP-prefetched network bytes or wire QoS. Connection changes affect only queued or inactive failed next starts, not active/paused transfers. Accepted settings are parent-owned and telemetry updates subsequently; no instant server propagation is claimed.

An HTTP already-exists failure offers Override existing with confirmation or Add another file via a fresh Rename retry. Initial Add Overwrite requires a destination-specific authorization checkbox. Both routes revalidate parent state; consent is not inferred from a selected menu item alone. The CLI remains unchanged: single-stream with no overwrite.

Completion shutdown is per download and off by default. Users must confirm a save-all-work warning. Only published Completed with every record finished and no active/paused task permits the visible 30-second countdown. Cancel, new work/retry, failure/cancellation, record removal, enabling a schedule, status close, or all-window close withdraws intent. Enabled schedules block shutdown arming, countdown, and dispatch. The final check requires the status window still live, so closing it cannot leave a hidden countdown. Production requests real OS poweroff with operator permissions; no forced app closure/elevation/bypass. Provider exit success is not observed poweroff. Tests suppress dispatch and do not exercise real host shutdown. See [Architecture](ARCHITECTURE.md) and [Testing](TESTING.md).
