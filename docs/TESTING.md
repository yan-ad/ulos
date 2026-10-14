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

## Future persistence/resume acceptance cases

Add tests with the features, not claims that the scaffold already supports them: SQLite migrations and recovery; interruption between file/database updates; durable verified offsets; valid `206` and `Content-Range`; shifted/inconsistent ranges; changed/weak validators; `200` restart without append; `416`; encoding changes; and stale or missing partial files. Verify final bytes, not merely transfer state labels. Segmentation tests must cover gaps/overlaps, actual server support, bounded concurrency, and aggregate rate limits.

## Desktop and platform matrix

The current entry point is the manager, not the older single-transfer starter. Source-level tests include pure queue/filter/sort/selection/telemetry invariants, URL-derived naming and sanitization, bounded clipboard URL acceptance/fingerprint deduplication, and Details percentages/stream state. Do not infer passing results or test counts from their presence; record actual commands and outcomes separately.

`manager_tests.rs` uses GPUI test adapters and real local HTTP fixtures for FIFO advancement even with every row hidden, Stop Queue without next-request dispatch, Stop All cancellation/temporary-file cleanup with the next item still queued, metadata-only removal, retained Add inputs/save prompts, unsupported-source failure, and stale-ID isolation. Further cases cover opt-in/dismissed clipboard proposals, safe prefill/manual-name preservation, folder-picker submission sanitation, redacted Details snapshots/close-reopen/progress refresh, and Details cancellation against a real partial transfer. Test adapters do not prove native clipboard, focus, picker, or rendering behavior.

### Native-menu and stock-control regression references

The source contains the following adapter expectations, not native visual/AX evidence or a passing-run claim. Full tests are currently being run separately by the parent task; this docs-only update does not count or report them as passed.

- `native_menu_toolbar_uses_stock_sizes_and_one_row_at_minimum_width`: at 880px width, no macOS in-window menu strip, same-row search/actions, consistent stock Button heights, and all action controls before search. Also review the 880 × 540 minimum, 240px/160px search wrapper, and 48px DataTable rows with unclipped default Buttons on native platforms.
- `native_menu_add_bubbles_from_search_and_options_waits_for_children` and `native_menu_add_routes_to_surviving_child_after_main_closes`: global typed dispatch from focused search and surviving child roots; independent Add drafts and guarded main-window-only Options.
- `native_menu_start_guard_and_real_worker_stop_use_registered_actions`: disabled Start does nothing; registered Start/Stop actions drive a real local HTTP request and cancellation, not simulated transfer labels.
- `native_menu_filter_clear_and_disabled_remove_preserve_downloaded_files`: current selection/state guards reject active-record removal, filters/checkmarks track state, and Clear removes metadata without deleting completed file bytes. Pure `native_menu.rs` tests cover menu names, enabled flags, typed actions, and exclusive filter/theme checkmarks.
- `native_menu_theme_actions_apply_stock_theme_and_checked_state`: Light/Dark actions use stock theme values and updated menu state.
- `native_menu_new_download_shortcut_bubbles_from_focused_inputs`: secondary-N (⌘N on macOS) opens independent Add windows from search and child input without changing input text or queue records. Native review must also check ⌘, and ⌘Q.
- `native_menu_edit_actions_use_the_focused_search_input`: Kit SelectAll/Cut/Paste update focused input/search and clipboard without creating downloads. Review native Copy/Undo/Redo and child editing too.
- `native_menu_state_events_do_not_notify_the_manager_view`: child-open menu-state events and child-close availability keep manager notification count at zero; distinguish native focus redraw from explicit model notification. Theme metadata refresh is independently driven by global Theme observation and must not add manager notifications.

Native visual and accessibility capture is not authorized for this update. No first-paint, native menu rendering, tooltip focus, or screen-reader result is asserted.

### Native-window and rendering regression references

The current `crates/ulos-desktop/src/manager_tests.rs` contains these adapter tests; their presence is not a passing-run claim or native screenshot evidence:

- `native_add_windows_keep_independent_drafts_and_close_independently`: separate child contexts, independent draft input, independent close, and registry cleanup.
- `native_details_are_per_download_and_reopening_reuses_the_window`: per-ID status identity and reuse.
- `native_add_owns_manager_after_main_close_and_can_submit`: surviving child ownership and submission.
- `native_add_hands_manager_ownership_to_status_when_starting_after_main_close`: Add-to-status ownership handoff and real fixture transfer after main close.
- `native_picker_blocks_dismiss_and_submit_until_cancelled`: outstanding picker guards.
- `closing_other_details_does_not_cancel_transfer_and_surviving_child_cancels_after_main_close`: passive close versus explicit cancellation, using a real partial HTTP transfer.
- `removing_and_clearing_finished_records_closes_only_matching_native_status_roots`: record/root cleanup without file deletion.
- `viewport_table_and_native_add_do_not_rebuild_the_entire_queue`: render/cell counters at the 500-record cap; viewport cells are fewer than all rows times five columns. Opening allows at most one manager focus redraw, while typing in the child must leave manager-render and table-cell counters unchanged.

Also review the empty manager, sorting/filtering and stable-ID checkboxes versus Kit positional keyboard highlight, multiple children up to the 16-Add cap, OS close during a picker, and actual last-window quit on native platforms. Counters test invalidation structure, not milliseconds, instantaneous opening, or FPS. Watch polling is 100ms but changed values and changed/stale rates gate notification; identical status snapshots must not notify.

Native review should cover:

- Latest light IDM 6.28-inspired structure: native titlebar, macOS system menus Ulos / Tasks / File / Edit / Downloads / View / Help (no in-window menu line), non-macOS in-window Kit dropdowns, and one shared action Toolbar with stock icon-only Buttons except Add URL, accessible names/built-in tooltips, and stock search Input on the same row. Check category tree, selection-plus-four-column table, status bar, and filename-click native status window. Confirm Kit controls, panels, text and semantic containers throughout (Toolbar, StatusBar, Label, GroupBox, DescriptionList, Empty, DataTable, Form/Input, Button, Checkbox, Tabs, Progress/Spinner); `div`/`v_flex`/`h_flex` are layout-only, with no manual painting or bespoke GTK/Swift UI. Options remains a Kit modal. Confirm no copied branding; do not demand an OS-independent pixel match.
- Empty state, 500-record cap, duplicate queued/active destinations, FIFO independent of table ordering, Start download versus Download later while idle/busy/running, Stop selected/Stop Queue/Stop All, and record removal/reveal with no file deletion.
- Address / File name / Save as, folder/save picker acceptance/cancellation and close/dismiss/submit guards while outstanding, manual overrides, unsafe selected/edited names, extension-preserving 180-byte limits, Unicode and reserved Windows device bases, no-overwrite collisions, and no naming-time HEAD/header request.
- Details live real bytes/rate/ETA, unknown/zero length, connecting/failure/cancel/completion, one actual stream row, Hide details, close/reopen, disabled Pause, Resume Not implemented, and explanatory Speed limiter / Options on completion tabs. No fake segmented progress. Reopen the same ID to activate its existing window, and open different IDs concurrently.
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
