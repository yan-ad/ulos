# Architecture

Updated: 2026-10-13. This is the planned architecture, not an implementation inventory.

## Workspace and dependency boundaries

| Crate | Initial responsibility | Dependencies/direction |
| --- | --- | --- |
| `crates/ulos-core` | Request validation; single-stream HTTP/HTTPS transfer and safe file lifecycle | Tokio; reqwest with rustls; no GPUI dependency |
| `crates/ulos-cli` | Parse URL/output arguments, invoke core, report outcome and exit status | Depends on core |
| `crates/ulos-desktop` | Minimal GPUI Kit starter window | GPUI Kit; future adapter to core, initially no transfers wired |

Configure core and CLI as workspace default members so ordinary checks do not require GUI platform dependencies. Networking and filesystem work must remain outside the UI render path. Future desktop integration should use commands/events and an owned async runtime rather than making core depend on GPUI entities.

## Initial transfer contract

1. Validate a parseable HTTP/HTTPS URL and usable output path. Reject unsupported schemes and invalid requests before creating a file. Recheck allowed schemes across redirects; preserve normal TLS certificate validation.
2. Require an acceptable full-response HTTP status. The initial downloader requests a full representation, not a range, and must not treat an unsolicited partial response as a complete file.
3. Create a unique `.part` staging file with exclusive creation. Never truncate a pre-existing final or staging file. A collision must produce a new candidate or a clear error, not an overwrite.
4. Stream response chunks to disk with bounded memory; do not accumulate the response in memory. Count bytes written and compare with the applicable expected body length when available. Unknown length is allowed but cannot provide a length-match guarantee. Content decoding must not cause comparison against a different representation's length.
5. Flush/close the successful transfer and publish it without replacing an existing destination. A check followed by an ordinary overwrite-capable rename is not sufficient: use a no-clobber publication strategy and handle a destination appearing during transfer.
6. On network, length, or filesystem failure, close and remove only staging files owned by this operation. The scaffold uses best-effort temporary-file cleanup; cleanup-error reporting is future work. Abrupt process termination can leave staging files; crash recovery is future work.

File creation and publication execute on blocking workers rather than the async task. Publication checks a cancellation flag before starting, but a filesystem publication already in progress can finish after cancellation. The CLI reports that possibility explicitly. A stronger cancellation/commit model is part of milestone 2, not a scaffold guarantee. Automatic redirect referers are disabled to avoid leaking signed query strings to another origin.

The scaffold has no checkpoint store or resumable partial-file contract. Do not retain failed partial files under a claim that they can be resumed. Filename sanitization and any automatic naming must prevent path traversal; initial explicit output avoids trusting server-supplied filenames.

## Persistence and resume: future design

SQLite-backed queue metadata and durable checkpoints are queued work, not implemented features. A proposed model stores transfer identity, destination/staging path, state, selected representation metadata, and verified byte offsets. Database updates and file durability need a documented recovery protocol; a row alone does not prove bytes survived a crash.

Resume must verify actual server behavior, not just `Accept-Ranges`. For a saved offset, send `Range` with a suitable `If-Range` validator. Prefer a stored strong ETag; a weak ETag is insufficient for strong representation identity. Validate `206`, `Content-Range` start/end/total, body length, and representation/encoding consistency before appending. A `200` response means restart safely, never append it to old bytes. Changed validators, inconsistent ranges, and `416` require explicit recovery rules rather than assuming completion. Without trustworthy representation identity, restart instead of claiming a safe resume.

Segmentation and rate limiting follow verified resume semantics. Bound segment count, connections, buffering, retries, and aggregate rate. Verify each response and final coverage; avoid overlapping/gapped writes and never advertise an unmeasured speed benefit.

## Browser bridge: future design

A small JS/TS extension will communicate through browser-native messaging to a Rust host, which validates input and hands explicit requests to the application/core. Chromium and Firefox have different host registration/allowlist rules. Use a versioned, bounded message schema and validate allowed extension identity, URLs, paths, and user intent. Keep browser secrets out of logs; authenticated URL/header handling remains an unresolved design requirement. Do not expose an unauthenticated network listener as a substitute for native messaging.

## Platform boundary

Use GPUI Kit's aligned dependency set rather than independently overriding GPUI snapshots. Linux needs a graphical X11/Wayland session and a working Vulkan driver. Platform packaging, runtime shutdown, UI accessibility, and filesystem publication semantics need platform-specific verification. See [Research](RESEARCH.md) and [Testing](TESTING.md).
