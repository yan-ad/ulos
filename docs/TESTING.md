# Testing and validation

Updated: 2026-10-14. This is a test plan; [Status](STATUS.md) records actual verification results and known gaps.

## Workspace validation commands

Run from the workspace root after scaffold initialization. Core and CLI should be default members, so default lint/test commands avoid GUI system dependencies.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Explicit core/CLI selection is useful when workspace defaults change:

```sh
cargo clippy -p ulos-core -p ulos-cli --all-targets -- -D warnings
cargo test -p ulos-core -p ulos-cli
```

Full workspace validation includes the desktop dependency graph and requires the native toolchain and platform libraries described in [Research](RESEARCH.md):

```sh
cargo check --workspace
cargo test --workspace
cargo run -p ulos-desktop
```

A successful workspace check does not prove a native window renders correctly. A successful CLI test does not validate the GUI. Record command, commit, toolchain, OS, and actual outcome; distinguish unavailable infrastructure from failure.

## Initial core/CLI acceptance cases

Use deterministic local HTTP fixtures and temporary directories, not public download sites as primary test dependencies.

- URL validation: malformed URL, unsupported scheme, invalid output, and redirects to unsupported schemes.
- Complete response: known/unknown length, empty body, incremental chunks, and exact output bytes.
- Errors: rejected HTTP status, unsolicited partial response, dropped connection, truncated body, applicable length mismatch, write/permission failure, and unavailable destination directory.
- Safety: existing final file remains byte-identical; existing staging files are untouched; concurrent staging collisions are safe; a final destination appearing during transfer is never replaced.
- Resource lifecycle: bounded-memory streaming and owned `.part` cleanup on ordinary failure. Cleanup-error reporting is not implemented; cleanup is best effort. Process-kill leftovers must not be described as resumable until recovery exists.
- CLI: URL/output argument errors, meaningful exit status, honest completion/failure output, and no secrets leaked into diagnostics.

The scaffold tests dropping an active transfer future and cleaning up without publication. Test filesystem publication on each target platform rather than assuming rename behavior is portable.

## Future durable persistence/resume acceptance cases

Session resume exists in the controlled engine; durable restart recovery does not. Add durable tests with that future feature: SQLite migrations and recovery; interruption between file/database updates; durable verified offsets; valid `206` and `Content-Range`; shifted/inconsistent ranges; changed/weak validators; `200` restart without append; `416`; encoding changes; and stale or missing partial files. Verify final bytes, not merely transfer state labels. Segmentation tests must cover gaps/overlaps, actual server support, bounded concurrency, and aggregate rate limits.

## Desktop and platform matrix

The current entry point is the manager, not the older single-transfer starter. Source-level tests include pure queue/filter/sort/selection/telemetry invariants, URL-derived naming and sanitization, bounded clipboard URL acceptance/fingerprint deduplication, and Details percentages/stream state. Do not infer passing results or test counts from their presence; record actual commands and outcomes separately.

`manager_tests.rs` uses GPUI test adapters and real local HTTP fixtures for queue advancement even with every row hidden, Stop Queue without next-request dispatch, Stop All cancellation/temporary-file cleanup with the next item still queued, metadata-only removal, retained Add inputs/save prompts, unsupported-source failure, and stale-ID isolation. Further cases cover opt-in/dismissed clipboard proposals, safe prefill/manual-name preservation, folder-picker submission sanitation, redacted Details snapshots/close-reopen/progress refresh, and Details cancellation against a real partial transfer. Test adapters do not prove native clipboard, focus, picker, or rendering behavior.

### Native-menu and stock-control regression references

The source contains the following adapter expectations, not native visual/AX evidence or a passing-run claim. This docs-only update does not run the suite or count these expectations as passes.

- `native_menu_toolbar_uses_stock_sizes_and_one_row_at_minimum_width`: at 880px width, no macOS in-window menu strip, same-row search/actions, consistent stock Button heights, and all action controls before search. Also review the 880 × 540 minimum, 240px/160px search wrapper, and borderless 36px DataTable rows, 32px header, and unclipped stock filename Links/category icons on native platforms.
- `native_menu_add_bubbles_from_search_and_options_waits_for_children` and `native_menu_add_routes_to_surviving_child_after_main_closes`: global typed dispatch from focused search and surviving child roots; independent Add drafts and guarded main-window-only Options.
- `native_menu_start_guard_and_real_worker_stop_use_registered_actions`: disabled Start does nothing; registered Start/Stop actions drive a real local HTTP request and cancellation, not simulated transfer labels.
- `native_menu_filter_clear_and_disabled_remove_preserve_downloaded_files`: current selection/state guards reject active-record removal, filters/checkmarks track state, and Clear removes metadata without deleting completed file bytes. Pure `native_menu.rs` tests cover menu names, enabled flags, typed actions, and exclusive filter/theme checkmarks.
- `native_menu_theme_actions_apply_stock_theme_and_checked_state`: Light/Dark actions use stock theme values and updated menu state.
- `native_menu_new_download_shortcut_bubbles_from_focused_inputs`: secondary-N (⌘N on macOS) opens independent Add windows from search and child input without changing input text or queue records. Native review must also check ⌘, and ⌘Q.
- `native_menu_edit_actions_use_the_focused_search_input`: Kit SelectAll/Cut/Paste update focused input/search and clipboard without creating downloads. Review native Copy/Undo/Redo and child editing too.
- `native_menu_state_events_do_not_notify_the_manager_view`: child-open menu-state events and child-close availability keep manager notification count at zero; distinguish native focus redraw from explicit model notification. Theme metadata refresh is independently driven by global Theme observation and must not add manager notifications.

Native visual and accessibility capture is not authorized for this update. No first-paint, native menu rendering, tooltip focus, or screen-reader result is asserted.

### Collapsible-sidebar regression references

Four adapter regressions in `manager_tests.rs` describe the intended coverage, not passing results:

- `sidebar_mouse_toggle_restores_geometry_without_changing_queue_state`: mouse clicks on the toolbar toggle collapse/expand to Kit’s 48px/255px defaults while preserving search, selected IDs, filter, queued records, and dispatch order, with no active HTTP task.
- `sidebar_collapsed_filter_survives_expansion_without_resetting_selection`: invokes the real filter controller while collapsed, including Category(Image), then expands without resetting selection/filter. This is **not** a collapsed-item mouse-click test: generated item bounds are unavailable with the current Kit/Base observation-registry test feature configuration. No dependency change is needed for this controller coverage.
- `sidebar_keyboard_from_search_and_native_action_from_child_share_state`: manager-scoped secondary-B from focused search and the global native typed action from a child share collapse state and update View’s Collapse/Expand label without editing search text.
- `sidebar_empty_light_and_dark_layout_stays_in_one_row_at_minimum_size`: stock Light/Dark, expanded/collapsed, at 880 × 540; toggle immediately before Add URL, same-row toolbar/search geometry, and unchanged stock Button sizing.

The final-width fixture uses public `set_reduce_motion(true)` for deterministic layout. Kit’s transition uses the GPUI frame clock; advancing the executor clock is not proof of the actual 200ms animation. Animation, rail tooltip interaction, screen-reader names/counts, sidebar item focus and arrow navigation still require native review; no full native keyboard support is claimed.

### Native-window and rendering regression references

The current `crates/ulos-desktop/src/manager_tests.rs` contains these adapter tests; their presence is not a passing-run claim or native screenshot evidence:

- `native_add_windows_keep_independent_drafts_and_close_independently`: separate child contexts, independent draft input, independent close, and registry cleanup.
- `native_details_are_per_download_and_reopening_reuses_the_window`: per-ID status identity and reuse.
- `native_add_owns_manager_after_main_close_and_can_submit`: surviving child ownership and submission.
- `native_add_hands_manager_ownership_to_status_when_starting_after_main_close`: Add-to-status ownership handoff and real fixture transfer after main close.
- `native_picker_blocks_dismiss_and_submit_until_cancelled`: outstanding picker guards.
- `closing_other_details_does_not_cancel_transfer_and_surviving_child_cancels_after_main_close`: passive close versus explicit cancellation, using a real partial HTTP transfer.
- `removing_and_clearing_finished_records_closes_only_matching_native_status_roots`: record/root cleanup without file deletion.
- `viewport_table_and_native_add_do_not_rebuild_the_entire_queue`: render/cell counters at the 500-record cap; viewport cells are fewer than all rows times nine columns. Opening allows at most one manager focus redraw, while typing in the child must leave manager-render and table-cell counters unchanged.

Also review the empty manager, sorting/filtering and stable-ID checkboxes versus Kit positional keyboard highlight, multiple children up to the 16-Add cap, OS close during a picker, and actual last-window quit on native platforms. Counters test invalidation structure, not milliseconds, instantaneous opening, or FPS. Watch polling is 100ms but changed values and changed/stale rates gate notification; identical status snapshots must not notify.

Native review should cover:

- Light IDM 6.28-inspired manager structure with the latest Kit/shadcn-style sidebar refinement: native titlebar, macOS system menus Ulos / Tasks / File / Edit / Downloads / View / Help (no in-window menu line), non-macOS in-window Kit dropdowns, and one shared action Toolbar with stock icon-only Buttons except Add URL, accessible names/built-in tooltips, and stock search Input on the same row. Check grouped collapsible sidebar, nine-column table (checkbox, Name, Size, Status, Transfer rate, Queue, Time left, Last try, Description), status bar, and filename-Link/focused-row Enter/double-click native status window. Confirm Kit controls, panels, text and semantic containers throughout (Toolbar, StatusBar, Label, GroupBox, DescriptionList, Empty, DataTable, Form/Input, Button, Checkbox, Tabs, Progress/Spinner); `div`/`v_flex`/`h_flex` are layout-only, with no manual painting or bespoke GTK/Swift UI. Options remains a Kit modal. Confirm no copied branding; do not demand an OS-independent pixel match.
- Empty state, 500-record cap, duplicate queued/active destinations, explicit queue order independent of table ordering, Start download versus Download later while idle/busy/running, Stop selected/Stop Queue/Stop All, and record removal/reveal with no file deletion.
- Address / File name / Save as, folder/save picker acceptance/cancellation and close/dismiss/submit guards while outstanding, manual overrides, unsafe selected/edited names, extension-preserving 180-byte limits, Unicode and reserved Windows device bases, no-overwrite collisions, and no naming-time HEAD/header request.
- Details live real bytes/rate/ETA, unknown/zero length, connecting/failure/cancel/completion, actual per-connection rows, Hide details, close/reopen, verified session Pause/Resume (disabled for unsupported streams), functional Speed/Completion tabs, and percentage inside the shared progress footer on every tab. No fake segmented progress. Reopen the same ID to activate its existing window, and open different IDs concurrently.
- Clipboard off-by-default session toggle, no reads when disabled/main-window-inactive/any-Add-or-status-open/Options-open, async active-window checks, bounded single-URL rejection, unchanged-fingerprint dedup after dismissal, prefill without overwriting an open form, no non-URL retention/logging, signed URL display redaction, and no network/download until explicit confirmation. Check Wayland compositor behavior; candidate bounds are not OS read-allocation bounds.
- Minimum-size overflow/scrolling, keyboard Add shortcut, accessible input/selection/progress names, focus restoration across independent native roots, screen readers, main-close survival, last-window quit, owned task teardown, and runtime shutdown including the two-second timeout. Do not assume cleanup completed solely because shutdown returned.

No new pass counts, native visual/accessibility outcomes, or Linux/Windows support claims are made here.

Only the current macOS host is available at initialization. Until recorded CI/native runs exist, Linux and Windows remain unvalidated. Linux must receive first-class testing:

| Dimension | Planned coverage |
| --- | --- |
| Distribution | Ubuntu 24.04 upstream baseline; additional distributions/versions to agree |
| Desktop/session | GNOME and KDE; Wayland and X11 |
| Scale | 100%, 150%, 200%; mixed-scale monitors where available |
| Graphics | Intel/AMD Mesa and NVIDIA configurations with working Vulkan drivers |
| Window lifecycle | Launch, resize, focus, text/input, close, error reporting |

Record exact distro, desktop, session, scale, GPU/driver, Rust version, and dependency lockfile. Build-only/headless CI is not a replacement for graphical validation. Windows needs MSVC/CMake/SDK validation; macOS needs the upstream OS baseline and CLT. No matrix row is a support guarantee before testing.

## Shared HTTP task acceptance cases

Test `HttpDownloadTask` independently of GPUI on a supplied Tokio runtime: preparation errors, actual output bytes, known/unknown lengths, and transitions through `Preparing`, `Downloading`, and terminal `Completed`/`Failed`/`Cancelled` states. Check bounded coalescing watch delivery with slow observers; intermediate progress need not be retained. Verify explicit cancellation and drop-requested cancellation, including publication races and best-effort cleanup. Test CLI and desktop adapters against the same task contract; drop is not an awaited join. These are acceptance requirements, not asserted completed results.

## Future torrent tests

Before torrent support ships, use deterministic local fixtures and legal payloads for valid/invalid magnet and `.torrent` sources, bounded/malformed metadata, bad piece hashes, interruption/cancellation, and safe multifile publication. Cover traversal/absolute paths, collisions, existing files, and no-overwrite behavior. Verify chosen tracker/DHT privacy policy, seeding consent/defaults, bandwidth limits, storage exhaustion/allocation, and teardown. Do not treat an infohash as an HTTP representation validator. Engine choice and fixture implementation are pending; no torrent tests or dependencies are claimed.

## Future browser and distribution tests

For selected Chromium/Firefox versions verify host registration and allowlists, bounded malformed/unsupported messages, unauthorized sender rejection, explicit user import, host disconnect/restart, and protocol-safe diagnostics. Verify browser import remains explicitly consented and separate from the implemented session clipboard opt-in; neither should trigger autonomous downloads. Test clean installs and packaged native host discovery on each release platform. Include the Ulos LICENSE file and audit dependencies/notices before distribution.

## System-default appearance

Theme resolver tests cover light/dark and vibrant appearances, System callbacks, explicit overrides and returning to System. Manager tests cover preference-based native checkmarks, selector geometry/clicks at 880 × 540, global stock tokens across multiple native roots without losing drafts, and System selection from a child after main-window closure. Appearance transitions call the application handler because Kit’s external test API does not expose the platform appearance simulation; these tests are not a claim of actual OS settings changes or Linux compositor appearance delivery. Native observers are retained in each window. No test changes the host OS theme.


## Controlled-engine acceptance and safe power testing

This section is a source inventory and acceptance plan, not a new test-run report. Existing targeted areas include 11 core HTTP tests, 5 controlled-transfer tests, 7 task tests, 7 Details/parser tests, 17 queue tests and 7 completion gate/command-plan tests. Presence/counts are not evidence of a pass. Eight `advanced_*` manager adapter cases cover parallel pause/resume with slot retention, live rates/zero rejection, retained bounded next-start connections, numbered conflict retry without initial HTTP, confirmed overwrite retry, Add consent reset on destination edit, shared progress footer, and shutdown confirmation/countdown/new-work cancellation. Parent records the latest actual run and lint outcome in Status; this docs-only task does not claim all workspace checks passed.

Use local HTTP fixtures and temporary files to verify:

- Probe `206` identity, one byte/exact Content-Range and strong ETag; ignore Accept-Ranges as evidence. Probe `200` reused once, weak/missing ETag new full GET, malformed ranges, encoding mismatches and unsupported status. Actual worker count is bounded 1–8 and can shrink for tiny files/fallback.
- Every segment and resumed offset sends If-Range and validates exact Content-Range/total, effective URL, validator, identity and actual body length. Changed resource/If-Range `200` must fail without appended bytes or silent restart; verify final bytes, gaps/overlaps and cleanup, not just state labels.
- Pause closes range responses at written offsets; no new queued item starts while paused. Resume revalidates remaining ranges. Unsupported-stream Pause stays disabled; cancellation closes workers and cleans owned staging best effort. A killed process has no resumable checkpoint guarantee.
- Aggregate limiter across all connections, no initial burst, positive checked 1–1,048,576 KiB/s input, zero/overflow/non-number rejection, unlimited toggle, interrupted outstanding waits and subsequent accepted-options telemetry. Measure output-consumption timing, not strict wire-speed control: TCP/reqwest can prefetch network bytes.
- Default Reject before any HTTP when destination exists; old bytes retained on failure/cancel. Rename publication collisions retry bounded numbered candidates without clobbering, preserving extension and 180-byte sanitation. Overwrite requires Add checkbox or retry dialog and parent revalidation; edits reset consent. Successful staged replacement, destination symlink/directory refusal, publication races, permissions and Windows replacement semantics need platform fixtures. Ancestor symlink/path aliasing is not a security sandbox claim.
- Persistent footer geometry on all tabs at compact/minimum sizes, known/unknown percentages, actual connection rows and state icons, retained inputs, no mid-active connection reconfiguration, stock Button/Input/Checkbox style, native macOS status TitleBar/traffic-light behavior. Adapter layout checks are not native screenshots or accessibility evidence.

**Never invoke real shutdown during ordinary tests.** `completion.rs` tests manipulate synthetic instants and inspect platform command plans only; parent dispatch is suppressed under `cfg(test)`. Verify default off, explicit confirmation, no retroactive completed-item arming, exactly-once gate consumption, published completion/all-records-finished requirement, and visible 30-second countdown. Exercise new work/retry, failure/cancel, removal, status close and all-window close cancellation, plus final live-status-window/idleness checks. Closing status must leave the transfer running but cancel power intent. Queued/paused/failed/cancelled unfinished records must block shutdown. Test provider-failure presentation using suppression or safe fixtures, not actual system power actions.

Production plans use macOS System Events/osascript, Linux systemctl poweroff and Windows shutdown.exe `/s /t 0` without force or elevation. Plan tests do not prove provider availability, operator permission, Automation prompts, open-application handling, or observed poweroff. No host shutdown or OS settings modification was performed for this update. Any future deliberate native power test requires an explicitly authorized operator, saved work and an isolated/test machine; record provider exit separately from actual observed power state.

### Add inspection and radios (2026-10-15)

Loopback/adapter coverage includes debounced Address changes, no automatic request from clipboard/programmatic prefill, HEAD-only results and cancellation on edit/close, stale-result rejection, decimal units through TB, sanitized target-path checks, missing/file/directory/symlink classification, consent reset on path edits, and stock radio mouse interaction. Native visual/AX review of title-free Add chrome, minimum-size scrolling, picker interaction and focus remains required; adapter checks do not establish native rendering or screen-reader behavior.

Add submission follow-up: exercise real Enter from Address/Save As with incomplete and complete drafts; missing fields produce guidance with no AddRequest, and a completed form emits Start exactly once. Invalid/credential-bearing URLs and pending/stale destination checks must not submit. Existing-file Reject disables Start and Download later while Cancel still emits dismissal and Rename can resolve the conflict. Native options tests check Add's minimizable/resizable flags are false while status capabilities remain unchanged; they do not prove traffic-light visibility.

Icon-color checks verify all seven category colors are distinct from one another and from navigation orange in stock Light/Dark themes, and meet 3:1 non-text contrast on the relevant stock surfaces. Existing sidebar collapse/filter, theme-switch, table and Add tests exercise shared icon integration. Overwrite-copy coverage verifies the redundant paragraph is suppressed even after an attempted submission while missing consent still blocks submission and explicit authorization enables it. Native visual/keyboard/AX verification remains separate.


## Named queues, local schedules, and compact table acceptance plan

These are verification requirements, not executed commands, passing results, or native-platform evidence. Record actual runs separately; never dispatch real shutdown.

- Validate the 32-queue cap including default Main queue, trimmed unique names, create/rename, protected Main deletion, non-main deletion with reassignment to Main, stale child requests, and active/paused reassignment/deletion rejection. Select main-table checkboxes then assign in Scheduler; focus/highlight alone must not change selected IDs. Reorder waiting files earlier/later at both boundaries; table sorting/filtering/search must not change `(queue_order, id)` dispatch.
- With loopback fixtures, start multiple queues and verify exactly one global active HTTP transfer, minimum `(queue_order, id)` arbitration, paused-slot retention, main-toolbar Start/Stop targeting Main only, and Scheduler controls targeting named queues. Stop queue must let active work finish without advancing that queue; Stop All must stop every queue and request active cancellation. Recurrence must never retry finished/failed/cancelled rows.
- Cover strict `YYYY-MM-DD` / `HH:MM`, invalid dates/times, past/equal-now Once, nonempty Weekly weekdays, optional stop, equal start/stop rejection and overnight earlier stop. Use controlled local clocks/timezones for nonexistent DST start/stop skipping and earlier ambiguous start/stop selection. Exercise repeated ticks, clock rollback, manual stop within an occurrence, skipped ended windows, wake inside a current unexpired window, and no-stop catch-up limited to its local start day without automatic midnight stop.
- Verify one reused/activated Scheduler native child with its own Kit Root, retained inputs, close/reopen, main-close survival and last-window quit. Suppress clipboard reads while Scheduler is open and reject a pending async proposal when it opens. Enabling a schedule must cancel an armed shutdown/countdown and block confirmation/final dispatch; tests must suppress OS power actions.
- Assert public Kit `bordered(false)`, 36px rows and 32px header without decorative rounded outer framing; nine headers in order; Name/Size sorting; stock Kit filename Link with colored category icon and no Button variant override. Check Link, focused-row Enter, double-click and Details routes independently from checkbox selection, including after sort/filter and at minimum size with horizontal scrolling. Native keyboard/AX review remains separate from adapters.
- Check actual-rate-only Time left (blank without usable rate/total), local Last try only on actual worker start, explicit Scheduler Description edits, and blank unknown values. No fake attempts, dates, descriptions or rates. Keep 500-row viewport rendering bounded rather than instantiating all nine cells per record.
- Restart against metadata-only SQLite history: destination/status/bytes may restore, but sources/schedules must not; restored source-less rows cannot start/retry. Queue names, membership/order, schedules, descriptions and attempt times must not be presented as durable recovery. Check diagnostics do not log source/network secrets or clipboard contents.
