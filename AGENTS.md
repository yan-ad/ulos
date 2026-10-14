# Working on Ulos

Read README.md, docs/STATUS.md, and the relevant product/architecture documents before changing behavior. Keep implemented features distinct from roadmap items.

## Boundaries

- ulos-core owns transfer logic; never import GPUI into it.
- ulos-cli is the first runnable consumer and diagnostic tool.
- ulos-desktop owns GPUI state; never block rendering with network or filesystem work.
- Do not copy code from IDM or other projects without reviewing their licenses.
- Project license is MIT; retain the LICENSE file. Final branding and Linux distro baseline remain undecided.

## Transfer safety

Never overwrite user files implicitly. Stream into a unique temporary file beside the destination and publish without clobbering. Test errors and cancellation as well as successful transfers. Do not append a resumed response until ranges and resource validators are verified. Do not log URLs containing credentials, signed queries, cookies, or authorization headers. No DRM or access-control bypass.

## Verification

Run cargo fmt --all -- --check, cargo test, and cargo clippy --all-targets -- -D warnings. Run equivalent --workspace checks when desktop prerequisites are available. Do not report skipped checks as passes. Record platform limits in docs/STATUS.md. No automatic commits.

## UI

The user requested anti-slop review after implementation. No brand direction has been approved. Use a restrained, honest starter; no fake downloads, invented metrics, or controls without behavior. A full design brief and native accessibility/keyboard review are required before calling the GUI complete.

The user supplied IDM-style layout references; DESIGN.md records that working structure, not final branding. All application widgets must use GPUI Kit, with no exceptions. No manually drawn controls/panels: use Kit controls, content labels, and semantic containers (Toolbar, StatusBar, Label, GroupBox, DescriptionList, Empty, DataTable, Form/Input, Button, Checkbox, Tabs, Progress, Spinner, Sidebar, menus, and Dialog as appropriate). Kit-reexported `div`, `v_flex`, and `h_flex` are layout-only; do not use them to hand-build widgets, panels, or text. No custom painting or bespoke GTK/Swift desktop UI. Use Kit native window roots and native picker APIs; keep child subscriptions/input in their own window context. Add and status use separate native windows; Options remains a Kit modal unless explicitly requested otherwise. Clipboard detection is explicitly requested but must remain opt-in, session-only, main-window-active gated (suppressed by any Add/status window or Options dialog, rechecked after async reads), and confirmation-only: never log clipboard contents or start network work just because a URL was copied. Filename detection must be single-component, portable and sanitized; do not overwrite destinations. Unsupported features must be visibly disabled or explained, never simulated.

### Permanent stock-control policy

As requested on 2026-10-14, leave Kit Button, Input, and Checkbox styling unmodified everywhere, including child/status windows. Do not add per-control size, padding, font/color/radius, or variant overrides (`.small()`, `.ghost()`, `.primary()`, custom `Input.style`, etc.). Derive behavior from content, icons, tooltips, accessible labels, and enabled/checked state, not styling. Wrapper/layout sizing is allowed; DataTable’s public `Size` API may provide row space for default controls. Use `Toolbar::content` for stock controls: `Toolbar::child` invokes library preparation that changes Button size/variant even without explicit modifiers. Use stock Kit/Lucide assets and built-in `Button::tooltip`; no separate tooltip dependency.

macOS menus belong to the system menu bar via Kit `Menu`/`MenuItem`, not an in-window imitation. Non-macOS Kit dropdowns remain in-window, with search in the shared action-toolbar row. Native menu callbacks hold weak owners, validate current state, and defer target-window updates to avoid reentrant active-window access; Options targets only the live main window. Refresh native metadata through `MenuStateChanged` and theme observation without manager notifications merely to update menu state. Preserve queue/HTTP behavior and disabled unsupported actions. Do not infer native rendering, accessibility, or first-paint results from adapter tests; do not claim a parent-running test suite as a pass.
