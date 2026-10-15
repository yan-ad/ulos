# Torrent files: implementation and limits

**Source-implemented, not a verification report.** The desktop uses the separate UI-independent `ulos-core/src/torrent.rs` backend with pinned `librqbit` 9.0.1. Ulos remains MIT; the selected engine is Apache-2.0. Retain `LICENSE`, `THIRD_PARTY_NOTICES.md` and the bundled Apache-2.0 text on redistribution; this is not a completed transitive license audit. See [Research](RESEARCH.md) and the [testing plan](TESTING.md).

## Intake and shared workflow

- **Add New → HTTP URL / Torrent file** offers separate native review windows. Picker, manager drag/drop, startup native paths/local `file://` URLs and OS-open callbacks all require confirmation. At most 16 raw file proposals per batch and 16 live Add drafts combined; paths/URL text are bounded to 16 KiB, argv to 18 tokens. Stable deduplication preserves native path encoding, including supported non-UTF8 local paths. Remote file hosts/UNC syntax, malformed URLs and unknown opener options fail closed. Lexical intake is not proof of file existence or local storage.
- Review name, total bytes, file count/manifest (first 20 displayed), tracker count, destination and collision policy offline. No session/tracker/peer is created by parsing, opening or dropping a file. **Start download** explicitly dispatches when idle; **Download later** does not enable a stopped queue, but confirmation allows an already-running queue to dispatch the item.
- HTTP/torrent records share the existing category sidebar and queue table, appending **Type** after Description, with no torrent sidebar group or dedicated torrent destination folder. Single-file torrents use their payload filename's existing category; multifile torrents use Other.
- One global active slot, named queues, explicit ordering, session scheduler and completion gates cover both protocols. Paused work holds the slot. Actual engine Pause/Resume is asynchronous: command acceptance is not observed Paused state. Torrent live rate/connection controls are unavailable; do not display simulated HTTP capabilities. Progress/phase/peer/upload counters come from the engine; initializing scan bytes are not reported as verified download progress.

## Offline metainfo profile

Accept v1 metainfo and compatible v1 hybrids containing a valid v1 manifest/piece list; **magnets and pure v2 are not implemented**. Strict parsing rejects some otherwise legal metainfo, including padding/symlink extensions and unsupported encodings; accepting hybrids does not imply all hybrid torrents are supported.

The regular-file worker read is capped at **8 MiB**. Before engine deserialization the strict bencode parser bounds depth to **32**, nodes to **65,536**, file count to **4,096**, and piece count to **262,144**. Piece length must be a power of two from 16 KiB through 64 MiB; piece count must match the nonzero checked total (at most `i64::MAX`). Dictionaries must be canonical/unique; negative integers and trailing data fail. Limits bound metadata, not required payload disk capacity.

Path validation is reject-only, not silent manifest rewriting: portable UTF-8 components up to 180 bytes, at most 32 components and 2,048 native path units, no traversal/absolute paths, separators, reserved/unsafe names, symlinks, padding or ambiguous alternate UTF-8 fields. NFC normalization plus full Unicode case folding rejects duplicate and file/directory-prefix conflicts across case-insensitive filesystems. The engine-decoded manifest must exactly match approved paths and lengths. Ancestor paths must not be modified by a hostile local process; this is not a filesystem sandbox.

## Networking and privacy

Discovery is **tracker-only on dispatch**, with HTTP/HTTPS/UDP trackers from approved metainfo (at most 64 entries, each at most 4,096 bytes). Embedded credentials/fragments and unsupported schemes fail; signed query values are preserved for operation, not displayed or logged. `TorrentSource` deliberately has no Debug representation, errors are redacted and owned engine threads suppress tracing diagnostics that could contain tracker secrets. Private tracker queries remain only in memory; tracker-only is not anonymous—trackers and contacted peers can observe the client's IP/infohash.

Engine default features are disabled; `rust-tls` and `disable-upload` are selected. Session payload upload is also disabled. DHT, local service discovery, inbound listeners, UPnP/port forwarding, persistence/fast resume, HTTP serving and seeding are off. Peer limit is 40. Payload-upload denial is not a claim of zero outbound protocol bytes: announces, handshakes and control messages still exist. Disabling reciprocal upload may hinder or prevent progress in some swarms. Public trackers/initial peers are not used by tests.

## Ownership, integrity and publication

Each dispatched worker owns unique `.ulos-torrent-*` staging beside the selected destination and a private Tokio runtime. The session is stopped and the runtime destroyed/joined before final verification/publication/cleanup, so engine tasks, blocking I/O and handles no longer mutate staged bytes. Final verification checks the exact approved tree, regular-file sizes and every SHA-1 piece in manifest order (including cross-file pieces), then syncs each file. An infohash identifies metainfo, not an HTTP strong validator or independent authenticity proof.

**Reject / Rename only; Overwrite is rejected before networking.** Single files publish by no-clobber hard link; multifile directories use no-replace rename on macOS/Linux and non-replacing directory rename on Windows. Rename tries the original and up to 1,000 bounded numbered candidates. Existing data, late collisions, symlinks and directories are never intentionally replaced. Native filesystem semantics still require platform testing. Completed is emitted only after successful publication and carries its actual path; completion shutdown cannot treat engine-finished alone as success.

Cancellation is checked between directory entries, files and bounded hash-read chunks; an individual blocking filesystem call cannot itself be interrupted. After teardown, staging cleanup is explicit. A cleanup failure before publication reports a redacted cleanup error; cleanup failure after publication preserves the successful receipt and can leave staging remnants. Publication already underway can win a cancellation race. Drop only requests cancellation, so the caller must retain the outer runtime until terminal observation. Abrupt termination can leave staging; no restart/checkpoint recovery is implemented.

## History, removal and packaging

SQLite history schema **v2** adds protocol/directory flags alongside metadata; it stores no raw torrent, source path/URL or tracker query. Restored source-less rows cannot start or retry. Named queues/schedules remain session-only. Multifile directory removal is metadata-only; Ulos does not recursively delete a completed payload directory.

Development packaging lives in [packaging](../packaging/README.md). `scripts/bundle-macos.sh` copies an already-built executable/notices into an absent `.app`, with imported `org.bittorrent.torrent` UTI and Viewer/Alternate document role; it does not launch/register/sign/notarize/install or change defaults. Linux's desktop entry advertises `application/x-bittorrent` / `%F` Open With. Windows's deliberately invoked per-user HKCU Open With registration script does not set UserChoice/defaults or machine keys. No Windows registration script was executed for this update. File associations are packaging declarations, not automatically installed or selected defaults, and do not prove native OS launches. Native association/open/drop, accessibility and clean-install validation remain pending.

Only transfer content you have permission to obtain. No DRM, captcha or access-control bypass is provided.
