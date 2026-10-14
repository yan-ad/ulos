# Ulos desktop manager foundation

The latest user-supplied reference is the **light IDM 6.28 structure**, superseding the earlier dark AB Download Manager reference. Interpret the reference as a layout/workflow request, not inferred branding, source reuse, or a promise of pixel-identical rendering. Native platform chrome and GPUI Kit styles differ across operating systems.

## Implemented direction

- Native titlebar; in-window Tasks / File / Downloads / View / Help menu strip using Kit dropdowns, not a claim of native OS menu integration.
- Icon-over-label action toolbar, category tree, compact table, and session status bar. The table has selection checkboxes plus four data columns: Name, Size, Status, Transfer rate. Name/size sorting and filename search affect presentation only.
- Light appearance by default, flat white/neutral surfaces, blue action/selection accent, structural borders, system font at 13px, small 2px radii, and compact 32px table rows. View offers an optional dark theme; it is not the governing reference.
- Compose GPUI Kit Sidebar, Table, Dialog, Input, Button, Checkbox, Progress, Tabs, and Icons. No browser/webview or custom-painted widget replacements, copied logos/artwork, or reference screenshot filenames as sample downloads.
- Empty tables are honest. The real session queue is capped at 500 records and runs one HTTP transfer at a time. FIFO advancement is independent of categories, filtering, search, and sort. Selection uses stable IDs.
- Add a download: Address, File name, Save as, Choose folder / Save as pickers, Start download / Download later. Preserve manually chosen names when the address changes; sanitize both suggestions and selected/edited filenames. No overwrite.
- Click a filename or choose Details for a same-window modal, not a detached status window. Status shows real bytes and available rate/ETA; unknown values stay unknown. Hide details toggles the connection section, containing at most one actual HTTP-stream row. Pause is disabled; Cancel requests cancellation for active work. Speed limiter and Options on completion tabs are explanatory only.
- Stop Queue finishes current work without advancing. Stop All stops advancement and requests active cancellation; Stop selected only cancels a selected active item. Remove and Clear finished delete session records, never downloaded files. Show in folder requires a finished record.
- Options actively exposes copied-link detection, off by default and session-only. Check once per second only while the window is active and Add, Details, and Options are closed; offer a bounded HTTP/HTTPS URL for confirmation without autonomous network activity.
- Scheduler and Grabber are explicitly disabled. Persistent history, resume, limiting, scheduling, site grabbing, browser integration, and torrents are future work, not cosmetic controls implying support.

## Review boundaries

Add URL has a keyboard shortcut; inputs, selection, and progress have accessible labels. Minimum-size overflow, keyboard/focus behavior, screen readers, clipboard/picker behavior, and native Linux/Windows rendering still need platform review. The reference guides structure, not an exact-pixel-match or cross-platform visual guarantee.

Project name, license, supported Linux baseline, and full branding remain undecided.
