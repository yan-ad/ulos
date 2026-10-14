# Working on Ulos

Read README.md, docs/STATUS.md, and the relevant product/architecture documents before changing behavior. Keep implemented features distinct from roadmap items.

## Boundaries

- ulos-core owns transfer logic; never import GPUI into it.
- ulos-cli is the first runnable consumer and diagnostic tool.
- ulos-desktop owns GPUI state; never block rendering with network or filesystem work.
- Do not copy code from IDM or other projects without reviewing their licenses.
- Project license, final branding, and Linux distro baseline remain undecided.

## Transfer safety

Never overwrite user files implicitly. Stream into a unique temporary file beside the destination and publish without clobbering. Test errors and cancellation as well as successful transfers. Do not append a resumed response until ranges and resource validators are verified. Do not log URLs containing credentials, signed queries, cookies, or authorization headers. No DRM or access-control bypass.

## Verification

Run cargo fmt --all -- --check, cargo test, and cargo clippy --all-targets -- -D warnings. Run equivalent --workspace checks when desktop prerequisites are available. Do not report skipped checks as passes. Record platform limits in docs/STATUS.md. No automatic commits.

## UI

The user requested anti-slop review after implementation. No brand direction has been approved. Use a restrained, honest starter; no fake downloads, invented metrics, or controls without behavior. A full design brief and native accessibility/keyboard review are required before calling the GUI complete.

The user supplied IDM-style layout references; DESIGN.md records that working structure, not final branding. Keep GPUI Kit primitives. Clipboard detection is explicitly requested but must remain opt-in, session-only, active-window gated, and confirmation-only: never log clipboard contents or start network work just because a URL was copied. Filename detection must be single-component, portable and sanitized; do not overwrite destinations. Unsupported features must be visibly disabled or explained, never simulated.
