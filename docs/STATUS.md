# Implementation status

Updated: 2026-10-14. Local host: macOS 27.0.1, Apple Silicon, Rust/Cargo 1.98.1. Initialization is committed as `7bf99c6`; subsequent HTTP/manager changes remain uncommitted. No push performed.

## Implemented foundation

- HTTP/HTTPS single-stream CLI and desktop transfers, using the shared UI-independent Rust engine and owned Tokio task service.
- Light, reference-structured manager using Kit menus, standard icon/label Buttons in Toolbar, Sidebar, virtualized DataTable, Empty, and StatusBar. All panels, text and controls use Kit components; Kit-reexported layout helpers are layout-only. Native chrome remains platform-owned, not a hand-drawn Windows skin. No copied IDM code, logo, or sample rows.
- Real bounded session queue (500 records maximum), FIFO advancement, one active transfer, filename search, category/status/queued filters, sort, stable-ID selection, cancellation, clear-finished, and metadata-only removal. Stop queue allows current work to finish without advancing; Stop all also requests cancellation. Download now starts the new item if idle; while busy it queues the item and enables progression.
- Each Add action opens an independent native Kit-rooted window, with up to 16 simultaneous drafts. Each window owns its inputs, subscriptions, and native folder/save picker tasks. Closing is blocked while its picker is outstanding. URL-derived filenames and manual path/name synchronization remain unchanged, with no naming network request and no overwrite.
- Portable single-component filename sanitizer: controls/bidi characters, path separators and Windows punctuation, leading/trailing spaces/dots, reserved Windows device names, and a 180-byte UTF-8 filename limit with bounded extension preservation. File contents/type are not inferred from extensions; categories are presentation hints.
- Off-by-default, session-only clipboard detection enabled through the Kit Options modal. Polls once per second only when the main window is active, no native Add/status window is open, and Options is closed. Async completion rechecks consent, main-window focus and child-window eligibility. Confirmation-only; no automatic download, network request, content logging or disk history. Accepted candidates are bounded at 16 KiB; OS read allocation occurs before validation.
- Download status opens in a separate native window from the list/Details action or after Start download. One window per download ID is reused and activated; different downloads can have concurrent status windows. Kit GroupBox/DescriptionList/Tabs/Progress/Table/Button present actual telemetry. Closing status does not cancel; removing its record closes the corresponding status window without deleting files.
- Child roots strongly retain the shared manager; reverse registries retain weak content. Downloads and child-window submission/cancellation remain functional after the main window closes. Registry cleanup is deferred to avoid reentrant entity borrowing, and the app quits when its last native window closes.
- Speed limiter and completion tabs explain unavailable features; Pause, Scheduler, and Grabber are disabled and labeled not implemented. No fabricated connection rows or resume capability.
- Progress/state observation is bounded and latest-state/coalesced. Unchanged polling no longer invalidates the manager; rate expiry triggers presentation refresh only when the displayed rate changes. Unchanged status snapshots do not notify their view. DataTable builds viewport cells rather than all 500 records. No network/filesystem work occurs in rendering.
- macOS Tasks/File/Edit/Downloads/View/Help menus now use the system menu bar through Kit's native Menu API. Linux/Windows retain Kit dropdown menus. Search shares the compact command toolbar; only Add URL has a visible action label. Other toolbar actions use stock icon-only Kit Buttons with managed Kit tooltips and accessible names.
- Buttons, Inputs and Checkboxes retain stock Kit appearance. Removed per-control variants/sizes and custom palette overrides. Toolbar content slots preserve default control styling rather than applying the toolbar's automatic compact/ghost preparation. Table row allocation accommodates stock filename Buttons without changing those Buttons.
- Sidebar uses stock Kit groups: Downloads (All, Finished, Unfinished, Queue) and File types (seven category filters). A default Kit icon Button toggles the stock Sidebar between expanded labels/counts and an icon rail with built-in hover labels. Collapse is session-only and does not alter queue/filter/search/selection state.

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
ulos-desktop/download_table.rs  snapshot-backed virtualized Kit table
ulos-desktop/native_windows.rs  native roots and child-context subscriptions
ulos-desktop/native_menu.rs     guarded native commands and cached menu state
ulos-desktop/clipboard.rs       bounded URL proposal/deduplication
ulos-desktop/palette.rs         System-default preference and stock Light/Dark themes
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

### License update verification (2026-10-14)

- Selected MIT licensing with a single `LICENSE` file. `cargo metadata --locked --no-deps --format-version 1` confirms MIT for all three workspace crates. The license text, README links, and absence of stale licensing claims were checked.
- `cargo fmt --all -- --check`, `cargo test --locked` (44 tests), `cargo clippy --locked --all-targets -- -D warnings`, and `git diff --check` passed.
- The current working tree did not pass full-workspace verification: `cargo test --locked --workspace` had four desktop failures (`native_add_hands_manager_ownership_to_status_when_starting_after_main_close`, `selected_details_redact_signed_source_close_reopen_and_refresh_progress`, `removing_and_clearing_finished_records_closes_only_matching_native_status_roots`, and `viewport_table_and_native_add_do_not_rebuild_the_entire_queue`). `cargo clippy --locked --workspace --all-targets -- -D warnings` failed on `collapsible_if` in `manager.rs:447`. These results supersede the earlier passing snapshot for this working tree; the license update did not change desktop source.

### MIT-only follow-up verification (2026-10-14)

- Consolidated licensing into a single MIT `LICENSE` file. All three crates report `MIT`; README links and documentation consistency checks passed.
- `cargo test --locked --quiet` (44 tests), `cargo test --locked --workspace --quiet` (85 tests), and default Clippy passed. Workspace test results now supersede the four failures recorded above.
- `cargo fmt --all -- --check` found unrelated formatting changes in `manager_tests.rs`. Workspace Clippy still fails on `collapsible_if` in `manager.rs:447`. Neither issue was changed by the licensing update.

### Kit-only native-window verification (2026-10-14, current)

These results supersede the earlier desktop failures, formatting differences and Clippy failure recorded above; those entries are retained as history.

- `cargo test --locked --workspace`: **85 passed** (19 core unit/naming, 14 HTTP/file lifecycle, 6 task lifecycle, 5 CLI, 41 desktop/model/window tests).
- `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `cargo build --locked -p ulos-desktop`, and `git diff --check`: passed. Local documentation links passed.
- Multi-window tests verify independent Add drafts and input, status-window reuse and concurrent IDs, picker close guards, deferred registry cleanup, metadata removal closing related status windows, and child submission/cancellation after main-window closure. Real loopback transfers still verify exact output and cancellation cleanup.
- A 500-record regression verifies viewport-bounded Kit DataTable cells and zero manager-model notifications from opening/typing in Add. GPUI's forced test frames and native focus changes are not interpreted as production queue notifications. Identical status snapshots do not notify; idle worker polls no longer continuously redraw the manager.
- Source evidence identified the previous modal's default 250 ms entrance animation. Add/status now bypass that modal path. This removes a configured delay and eager rendering work; it is not a universal first-paint latency guarantee.
- An isolated, temporary macOS harness opened the actual Kit content in four distinct, visible native windows: manager, two Add windows, and status. Individual Add/status screenshots were reviewed. Sample synchronous Add construction calls were approximately **28–44 ms**; these measure window-construction calls only, not compositor presentation or user-perceived latency. The first external window listing ran before the children appeared; a later native probe and exact window captures confirmed their visibility. No production window-tabbing change was needed.
- The smoke harness used a clearly labeled queued fixture, started no download, and was removed afterward along with its executable. The production tree contains no native probe, demo fixture, platform-specific UI code, or custom-painted widgets. All visual content uses Kit components; layout helpers provide geometry only.
- Native accessibility automation remains unauthorized on this host; no permission was changed. Windows/Linux native behavior and full accessibility/keyboard review remain unverified. The existing upstream `block 0.1.6` future-compatibility warning remains.

## Known limits

- Session queue/history and clipboard preference are not persisted. No pause/resume, durable checkpoints, segmentation, retries/rate limiting, completion automation, browser capture, torrent engine, installer, or updater.
- Best-effort temporary-file cleanup, no crash-recovery scanner. Cancellation can race with final publication; clients disclose that a completed file may exist. Slow/network-volume behavior and directory crash durability remain unverified. Desktop runtime shutdown timeout is two seconds, not proof that all blocking filesystem work has finished.
- URL-path naming only; no HEAD or Content-Disposition detection. Manual paths do not expand `~`/environment variables. Non-UTF-8 path selection is rejected rather than altered. Existing final files are never replaced.
- Clipboard behavior depends on OS permissions/compositor support, especially Wayland. No global background-service guarantee. Signed URLs must remain in transfer memory to work; neither clipboard content nor source queries are written to logs/history.
- Native child windows are implemented. Native macOS Add/status screenshots were reviewed using an isolated harness; complete native accessibility, keyboard, minimum-size and Windows/Linux review remains pending. One HTTP stream is displayed; resume, speed-limiter and completion automation remain unimplemented.
- Ulos is licensed under MIT; see `LICENSE`. Dependencies retain their own licenses and notices. Project name, Linux baseline, browser targets, and distribution formats remain undecided. No new commit was made during this continuation.

### Native macOS menus and stock toolbar verification (2026-10-14, latest)

- `cargo test --locked --workspace`: **99 passed** (44 core/CLI and 55 desktop/menu/model/window tests).
- Workspace Clippy with `-D warnings`, formatting check, desktop build, and `git diff --check`: passed. Documentation links passed.
- Tests dispatch registered menu commands from search and child inputs, including after main-window closure. Native callbacks defer target-window updates to avoid reentrant window borrowing and recheck current guards. Start/stop exercise a real local HTTP worker; filter/clear/remove checks preserve downloaded files.
- Command-N works from focused main/child inputs; native Edit Select All/Cut/Paste remains with the focused Kit input. Options uses the surviving main handle only; opening child windows disables it. A separate menu-state event refreshes native enablement without notifying the manager view. Stock Theme changes refresh cached menu checkmarks.
- Toolbar geometry is tested at 880px width: stock-height Buttons and search share one row, with no duplicate macOS menu strip. Icon-only controls have Kit `.tooltip(...)` text and accessible names. No separate custom tooltip implementation or styling override was introduced; native hover-popup appearance was not automated.
- The production macOS app stayed open in the final native smoke; a named-window-only screenshot confirmed the single toolbar row, icon-only actions, inline search and stock light controls. Earlier short-lived smoke launches produced no diagnostics and were not treated as visual evidence. The final review process was stopped afterward. The screenshot excludes the system menu bar; native menu inventory/action routing are separately covered by tests and the public Kit API.
- Linux/Windows runtime menu fallback and native screen-reader/hover interaction remain unverified. The upstream `block 0.1.6` future-compatibility warning remains. No new commit or push was performed.

### Full-content macOS manager chrome (2026-10-14, latest)

- Removed the separate visible Ulos native title strip on the main macOS window using transparent full-content window options and Kit `TitleBar`. Native traffic lights and window metadata remain; Kit owns drag/double-click behavior. Add/status native titles, system menus, non-macOS chrome and stock control styling are unchanged.
- `cargo test --locked --workspace`: **100 passed** (44 core/CLI, 56 desktop/menu/model/window tests). Window-option and 880px toolbar regressions verify native-control spacing and no extra titlebar row.
- Workspace Clippy with `-D warnings`, formatting check, and desktop build passed. Named-window-only native screenshot review confirmed the integrated toolbar/traffic lights and absence of the separate dark title strip. The window's captured native height is 760px, matching content bounds rather than the prior content-plus-titlebar height. Review process stopped afterward.
- This is macOS rendering evidence, not a Windows/Linux or native screen-reader pass. No commit/push performed.

### Stock grouped sidebar and toggle (2026-10-14, latest)

- Added a stock icon-only toggle before Add URL, with a changing accessible name and Kit tooltip. View offers the same command; Cmd/Ctrl+B toggles from the manager context. macOS native View command also routes from a surviving child window.
- Stock SidebarGroup components replace the nested category tree with Downloads and File types groups. All eleven filters remain present in the collapsed icon rail; expanded suffix counts are real session counts, and accessibility labels include those counts. No Sidebar/Button/Input/Checkbox custom styling was added. The specialized SidebarToggleButton was not used because it internally forces compact/ghost styling.
- `cargo test --locked --workspace`: **104 passed** (44 core/CLI, 60 desktop/model/menu/window tests). Workspace Clippy with `-D warnings`, formatting check, desktop build and diff whitespace checks passed.
- Four added regressions cover actual toggle-button clicks, 255px expanded/48px collapsed geometry, search/selection/FIFO preservation, controller filtering while collapsed, manager shortcut and native child menu dispatch, and both stock themes at 880 × 540. Geometry fixtures use GPUI's public reduced-motion setting; production retains Kit's standard collapse transition. Collapsed filter invocation is controller-tested, not a native item-click or screen-reader navigation claim.
- Production app startup produced no captured errors. Two scoped native screenshot attempts failed with `could not create image from window`; no native visual pass is claimed for this update, and no permissions were changed. The review process was stopped. Existing native screen-reader/arrow navigation and Windows/Linux review remain pending.
- No commit or push performed.

### System-default appearance and theme choices (2026-10-14, latest)

- Startup defaults to **Follow system**. A stock toolbar theme-selector Button/DropdownMenu and native/fallback View menus offer Follow system, Light, and Dark. Monitor/Sun/Moon icons and checked choices represent the preference, not merely the effective colors. All components still use stock Kit colors and control styling.
- System clears an application appearance override and follows retained appearance observers in the manager, Add, and status windows. Manual Light/Dark choices set the application override and ignore OS appearance callbacks until System is chosen again. Preference applies across windows and survives main-window closure while children remain; it is session-only and resets to System on restart.
- `cargo test --locked --workspace`: **110 passed** (44 core/CLI, 66 desktop/menu/theme/model/window tests). Workspace Clippy with `-D warnings`, formatting check, build, and diff whitespace checks passed.
- Added resolver/handler and integration coverage for System-default startup, light/dark/vibrant appearance resolution, manual override stability, returning to System, exclusively checked native choices, theme selector at 880 × 540, shared stock colors across child roots, preserved drafts, and System command after main close. Tests call the application appearance handler: the external Kit test API does not expose platform appearance simulation. No host OS appearance settings were changed.
- Native startup produced no captured errors; the named-window screenshot attempt failed (`could not create image from window`). No new native visual or real OS-switch pass is claimed. Review process stopped and permissions left unchanged. OS/compositor appearance-event delivery, screen-reader behavior and Windows/Linux native rendering remain unverified.
- No commit or push performed; existing upstream `block 0.1.6` future-compatibility warning remains.

## Next work

1. Native accessibility/keyboard and Linux compositor/GPU tests.
2. Persistent queue, checkpoint durability and recovery.
3. Safe validator-aware resume, then bounded segmentation.
4. Browser native-messaging handoff; later torrent engine research/license review.
