# Future torrent support

**Planned, not implemented.** HTTP/HTTPS is the current focus. There are no torrent dependencies, transfers, or advertised UI controls. See [Roadmap](ROADMAP.md) and [Status](STATUS.md).

## Proposed boundary

- Add a separate UI-independent core torrent module/engine after the HTTP integration is reliable. Eventually separate concrete HTTP and torrent engines; do not add a generic root backend trait before a second concrete implementation establishes the need.
- Accept validated magnet or `.torrent` sources with bounded metadata parsing. Validate metadata paths and enforce safe multifile destinations without overwriting existing files; verify piece hashes.
- An infohash identifies torrent metadata, not a strong single-file HTTP validator. Keep HTTP resume identity separate. Future SQLite records should use protocol-tagged sources.
- Decide tracker/DHT privacy and network behavior, explicit seeding consent/defaults, bandwidth limits, and storage allocation/cleanup before exposing controls.

## Pending research and acceptance

No engine has been chosen. Evaluate embedding/runtime ownership, maintenance, safety, resource limits, cancellation, and licensing before selecting a dependency; Ulos licensing is also undecided. This document is a design note, not external engine research or a compatibility claim.

Use deterministic fixtures and legal payloads to test malformed metadata, unsafe paths, collisions, corrupt pieces, cancellation, teardown, and policy enforcement. Only download/share content with permission; torrent support does not authorize DRM or access-control circumvention. See [Testing](TESTING.md).
