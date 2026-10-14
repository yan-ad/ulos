# Ulos desktop manager foundation

The latest user-supplied reference is the **light IDM 6.28 structure**, superseding the earlier dark AB Download Manager reference. Interpret the reference as a layout/workflow request, not inferred branding, source reuse, or a promise of pixel-identical rendering. Native platform chrome and GPUI Kit styles differ across operating systems.

Updated: 2026-10-14.

## Implemented direction

- Native titlebar. On macOS, Ulos / Tasks / File / Edit / Downloads / View / Help are native system-menu-bar menus via Kit `Menu` / `MenuItem`, not an in-window strip. Other platforms retain in-window Tasks / File / Downloads / View / Help Kit dropdowns; search is not in that menu line.
- One shared compact action Toolbar: stock icon-only Buttons except labeled Add URL, with accessible names and Kit `Button::tooltip`. The stock search Input occupies the same row in a shrinkable 240px wrapper (160px minimum), with no custom Input style. Category tree, table, and session status bar. The table has selection checkboxes plus four data columns: Name, Size, Status, Transfer rate. Name/size sorting and filename search affect presentation only.
- Stock Kit light appearance by default; View offers stock dark theme through `Theme::change`. The reference supplies structure, not a custom skin: the former 13px font, blue palette, 2px radii, and 32px table-row prescriptions are removed. Fonts, colors, radii, and controls follow Kit theme defaults rather than an asserted OS-independent font size. DataTable uses its public `Size` API for 48px rows so unmodified default 32px Buttons fit without clipping.
- Use GPUI Kit for every application widget, with no manual UI exceptions: Toolbar, StatusBar, Label, GroupBox, DescriptionList, Empty, DataTable, Form/Input, Button, Checkbox, Tabs, Progress, Spinner, Sidebar, menus, Dialog, and Icons as appropriate. `div`, `v_flex`, and `h_flex` reexported by Kit are allowed only for layout, not hand-built controls, text labels, or panels. No custom painting, browser/webview, bespoke GTK/Swift desktop UI, copied logos/artwork, or reference screenshot filenames as sample downloads.
- Empty tables are honest. The real session queue is capped at 500 records and runs one HTTP transfer at a time. FIFO advancement is independent of categories, filtering, search, and sort. Selection uses stable IDs.
- Add a download in an independent native window: each invocation opens a new draft (maximum 16 concurrently). Address, File name, Save as, Choose folder / Save as pickers, Start download / Download later. Preserve manually chosen names when the address changes; sanitize both suggestions and selected/edited filenames. No overwrite.
- Click a filename or choose Details for a native status window. One window per download is reused and activated on reopening; different download IDs can be viewed concurrently. Options remains a Kit modal; the separate-window requirement covers Add and status, not every dialog. Status shows real bytes and available rate/ETA; unknown values stay unknown. Hide details toggles the connection section, containing at most one actual HTTP-stream row. Pause is disabled; Cancel requests cancellation for active work. Speed limiter and Options on completion tabs are explanatory only.
- Stop Queue finishes current work without advancing. Stop All stops advancement and requests active cancellation; Stop selected only cancels a selected active item. Remove and Clear finished delete session records, never downloaded files. Show in folder requires a finished record.
- Options actively exposes copied-link detection, off by default and session-only. Check once per second only while the main window is active, all Add/status windows are closed, and Options is closed; offer a bounded HTTP/HTTPS URL for confirmation without autonomous network activity.
- Scheduler and Grabber are explicitly disabled. Persistent history, resume, limiting, scheduling, site grabbing, browser integration, and torrents are future work, not cosmetic controls implying support.

Buttons, Inputs, and Checkboxes keep their default styling in manager, Add, Options, and status views: no `.small()`, `.ghost()`, `.primary()`, or per-control size/padding/color/variant overrides. Layout sizing belongs to wrappers. Use `Toolbar::content`, not `Toolbar::child`, for stock controls: child preparation changes Button size and forces ghost/compact presentation. Icons are stock Lucide assets supplied by Kit’s full asset registration; tooltips require no separate `gpui-tooltip` dependency.

## Native lifecycle and rendering

Every window is opened through Kit and receives its own Root and native OS chrome. Child input, subscriptions, and native path prompts belong to that child window. A pending picker guards native close, dismissal, and submission until resolved. Closing the manager does not close children or discard their shared session owner; closing a passive status window does not cancel work. Remove/Clear finished close status windows for removed records, never delete files. The last-window close requests application quit.

The manager DataTable virtualizes viewport cells rather than eagerly building all record widgets. Transfer polling is change-gated, with stale-rate refreshes; identical status snapshots do not notify. Structural regression counters cover child-input isolation and viewport rendering, not measured opening latency or FPS. Add/status no longer use the modal entrance animation.

## Review boundaries

The main macOS window uses full-content native chrome: no separate visible "Ulos" title strip. Kit `TitleBar` contains the existing stock-control Toolbar, reserves native traffic-light space, and owns dragging/double-click behavior. Window metadata retains the app title. Native menus and child-window titles are unchanged; Windows/Linux keep their existing decorations. No replacement traffic lights or custom-painted titlebar controls.

On macOS, ⌘N opens Add Download from focused search or child inputs, ⌘, requests Options in the main window only, and ⌘Q quits; native Edit uses Kit input actions. Add URL has a keyboard shortcut; inputs, selection, and progress have accessible labels. The manager minimum is 880 × 540 logical pixels. Minimum-size overflow, keyboard/focus behavior, screen readers, clipboard/picker behavior, and native Linux/Windows rendering still need platform review. Native visual/accessibility capture is not authorized for this update; no first-paint timing or native AX outcome is claimed. The reference guides structure, not an exact-pixel-match or cross-platform visual guarantee.

Project name, supported Linux baseline, and full branding remain undecided. Ulos is licensed under MIT.
