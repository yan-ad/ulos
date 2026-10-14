# Architecture

Updated: 2026-10-13. Current HTTP structure and future design are distinguished below; [Status](STATUS.md) records verification.

## Workspace and dependency boundaries

| Crate | Responsibility | Dependencies/direction |
| --- | --- | --- |
| `crates/ulos-core` | Request validation; single-stream HTTP/HTTPS transfer and safe file lifecycle | Tokio; reqwest with rustls; no GPUI dependency |
| `crates/ulos-cli` | Parse URL/output arguments, invoke core, report outcome and exit status | Depends on core |
| `crates/ulos-desktop` | Session manager and single-active-transfer HTTP orchestration | GPUI Kit; adapter to shared core task |

Configure core and CLI as workspace default members so ordinary checks do not require GUI platform dependencies. Networking and filesystem work must remain outside the UI render path. Desktop integration uses an owned async runtime and task state rather than making core depend on GPUI entities.

## HTTP task orchestration

`ulos-core/src/http.rs` owns the HTTP implementation; `lib.rs` re-exports its public API and declares modules. `tasks.rs` supplies `HttpDownloadTask::start` on a caller-supplied Tokio runtime handle, orchestrating preparation and download. A bounded, coalescing watch channel exposes `TransferState`: `Preparing`, `Downloading`, `Completed`, `Failed`, and `Cancelled`. It is latest-state delivery, not a lossless progress-event log. A terminal guard reports a generic failure on worker unwind without publishing panic text. Dropping the task requests cancellation without awaiting termination; it is not proof that publication or cleanup has finished. CLI and desktop share this task.

Desktop `main.rs` initializes the Kit palette in light mode, opens a native-titlebar manager window, binds Add URL, and owns the Tokio runtime with a two-second shutdown timeout. A timeout does not establish successful cleanup. `manager.rs` owns the active `HttpDownloadTask`, retained input/dialog entities, subscriptions, and GPUI observer tasks. Observations are guarded by active download ID so stale transfers cannot mutate a newer item. Terminal observations defer FIFO advancement until the current observer returns; the observer polls coalesced state at 100ms intervals. Network/filesystem work stays off the render path.

## Desktop manager modules and contracts

- `queue.rs`: pure presentation/session model with stable monotonic IDs, a 500-record cap, private source URLs, destination-derived categories, selection, search, and sorting. `next_queued` uses insertion IDs, never visible order. Lexical duplicate destinations are rejected for queued/active items; this is not filesystem-alias detection. Core no-clobber publication is the final safety boundary. No persistence or history store.
- `manager.rs`: one active transfer, queue start/stop, cancellation, record removal, finished-file reveal, clipboard proposals, and live Details snapshots. Stop Queue leaves current work running; Stop All also requests cancellation. Start download starts the chosen item when idle (not necessarily the oldest queued item), or enables queue advancement when busy. Subsequent queue advancement is FIFO. Download later does not enable a stopped queue; existing running advancement still applies.
- `manager_render.rs` / `palette.rs`: light reference-inspired native-titlebar structure, in-window Kit dropdown menus, icon-over-label toolbar, category tree, selection plus four data columns, and status bar. Sorting/filtering/search do not change scheduling. Scheduler/Grabber are disabled. Dark theme remains optional.
- `add_download.rs`: retained Address / File name / Save as inputs, native folder and save prompts, manual-name preservation, and sanitized destination submission. Prompts disable editing/dismissal while outstanding. They are platform pickers; Add, Details, and Options themselves are Kit modals in the same manager window, not separate native status windows.
- Core `naming.rs`: pure URL-path suggestion and portable component sanitization, with no HEAD or Content-Disposition request. Percent decoding is UTF-8-lossy; query/fragment never contribute to a name. Replace separators, controls, bidi formatting, and unsafe Windows punctuation; trim leading/trailing spaces/dots; protect reserved device bases including COM1–9/LPT1–9 and superscript ¹/²/³ variants. Bound names to 180 UTF-8 bytes on character boundaries, preserving a short extension when possible. Selected/edited filenames are also sanitized. Case/Unicode spelling is preserved, but normalization, filesystem equivalence, collision avoidance, and successful publication are not implied.
- `download_details.rs`: passive snapshot view with persistent tab/disclosure state. It never starts I/O; Cancel emits a parent-owned cancellation request. Unknown/zero totals do not invent percentages. At most one stream row is shown when actual progress or completion exists, not invented segments for queued items. Resume is labeled Not implemented, Pause disabled, and limiter/completion tabs explanatory. Rates use real byte/time observations and expire when stale; ETA requires a known total and usable rate.
- `clipboard.rs`: detector retains only a text fingerprint, not non-URL content. Options opt-in is session-only and off by default. A one-second GPUI timer checks active-window/dialog state before `read_from_clipboard_async`; disabled/inactive/dialog-open states perform no reads, and proposal delivery rechecks eligibility. Only one trimmed HTTP/HTTPS URL, no embedded credentials or internal whitespace, within 16 KiB is accepted. Repeated fingerprints suppress reopened proposals. GPUI may read clipboard contents before the application-level size rejection; the limit bounds accepted candidates, not an OS clipboard allocation guarantee. Wayland behavior depends on the compositor. There is no proposal-triggered network request or automatic download.

Accepted source URLs, including signed query values, are retained in actual task/session memory for downloads. Source-bearing models deliberately avoid Debug; Details snapshots remove query/fragment/credentials. This is not a claim that application memory contains no secrets. No clipboard contents are logged or persisted.

## HTTP transfer contract

1. Validate a parseable HTTP/HTTPS URL and usable output path. Reject unsupported schemes and invalid requests before creating a file. Recheck allowed schemes across redirects; preserve normal TLS certificate validation.
2. Require an acceptable full-response HTTP status. The initial downloader requests a full representation, not a range, and must not treat an unsolicited partial response as a complete file.
3. Create a unique `.part` staging file with exclusive creation. Never truncate a pre-existing final or staging file. A collision must produce a new candidate or a clear error, not an overwrite.
4. Stream response chunks to disk with bounded memory; do not accumulate the response in memory. Count bytes written and compare with the applicable expected body length when available. Unknown length is allowed but cannot provide a length-match guarantee. Content decoding must not cause comparison against a different representation's length.
5. Flush/close the successful transfer and publish it without replacing an existing destination. A check followed by an ordinary overwrite-capable rename is not sufficient: use a no-clobber publication strategy and handle a destination appearing during transfer.
6. On network, length, or filesystem failure, close and remove only staging files owned by this operation. The scaffold uses best-effort temporary-file cleanup; cleanup-error reporting is future work. Abrupt process termination can leave staging files; crash recovery is future work.

File creation and publication execute on blocking workers rather than the async task. Publication checks a cancellation flag before starting, but a filesystem publication already in progress can finish after cancellation. The CLI reports that possibility explicitly. A stronger cancellation/commit model is part of milestone 2, not a scaffold guarantee. Automatic redirect referers are disabled to avoid leaking signed query strings to another origin.

The scaffold has no checkpoint store or resumable partial-file contract. Do not retain failed partial files under a claim that they can be resumed. Desktop filename sanitization prevents suggestions/edited names from becoming path components; the core/CLI explicit output contract does not trust server-supplied names.

## Persistence and resume: future design

SQLite-backed queue metadata and durable checkpoints are queued work, not implemented features. A proposed model stores transfer identity, destination/staging path, state, selected representation metadata, and verified byte offsets. Database updates and file durability need a documented recovery protocol; a row alone does not prove bytes survived a crash.

Resume must verify actual server behavior, not just `Accept-Ranges`. For a saved offset, send `Range` with a suitable `If-Range` validator. Prefer a stored strong ETag; a weak ETag is insufficient for strong representation identity. Validate `206`, `Content-Range` start/end/total, body length, and representation/encoding consistency before appending. A `200` response means restart safely, never append it to old bytes. Changed validators, inconsistent ranges, and `416` require explicit recovery rules rather than assuming completion. Without trustworthy representation identity, restart instead of claiming a safe resume.

Segmentation and rate limiting follow verified resume semantics. Bound segment count, connections, buffering, retries, and aggregate rate. Verify each response and final coverage; avoid overlapping/gapped writes and never advertise an unmeasured speed benefit.

## Torrent boundary: future design

Torrent support is planned, not implemented. HTTP already has a concrete core module; a future torrent engine will have a separate module while keeping orchestration independent of GPUI. Do not introduce a root generic backend trait before a concrete second backend establishes the shared contract.

A separate core torrent module/engine would validate magnet or `.torrent` sources, bound metadata parsing, enforce safe multifile destinations with no overwrites, and verify piece hashes. An infohash identifies torrent metadata; it is not a strong single-file HTTP representation validator and must not be substituted for an ETag/`If-Range` contract. When SQLite persistence arrives, store protocol-tagged source data rather than treating every source as an HTTP URL. Tracker/DHT privacy, seeding, bandwidth, and storage policy remain decisions. See [Torrent scope](TORRENTS.md) for research and acceptance boundaries.

## Browser bridge: future design

A small JS/TS extension will communicate through browser-native messaging to a Rust host, which validates input and hands explicit requests to the application/core. Chromium and Firefox have different host registration/allowlist rules. Use a versioned, bounded message schema and validate allowed extension identity, URLs, paths, and user intent. Keep browser secrets out of logs; authenticated URL/header handling remains an unresolved design requirement. Do not expose an unauthenticated network listener as a substitute for native messaging.

## Platform boundary

Use GPUI Kit's aligned dependency set rather than independently overriding GPUI snapshots. Linux needs a graphical X11/Wayland session and a working Vulkan driver. Platform packaging, runtime shutdown, UI accessibility, and filesystem publication semantics need platform-specific verification. See [Research](RESEARCH.md) and [Testing](TESTING.md).
