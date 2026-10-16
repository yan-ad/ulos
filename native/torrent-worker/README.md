# Native torrent worker (protocol 1)

Real libtorrent-rasterbar BEP 52 engine; no simulated progress or capability fallback.
Uses the BSD-3-Clause libtorrent 2 API (baseline 2.0.15; built and tested against
Homebrew 2.1.2). The application must bundle the worker and its dynamic libraries,
launch by absolute path without a shell, and include their license notices. This
source does not download or install libraries. `BUILD_TESTING=OFF` excludes the
fixture utility from release builds. Only the worker is installed.

## Build / acceptance

macOS dependencies: `brew install cmake libtorrent-rasterbar nlohmann-json`.
Linux: provide libtorrent 2 with BEP52 enabled, Boost, OpenSSL, nlohmann-json,
a C++17 compiler and CMake (Debian packages include libtorrent-rasterbar-dev,
nlohmann-json3-dev, libboost-dev, libssl-dev).

Windows (MSVC): provide CMake package configurations for libtorrent 2 and
nlohmann-json, for example using vcpkg's `x64-windows-static` triplet. Configure
with the vcpkg toolchain and `-DVCPKG_TARGET_TRIPLET=x64-windows-static`, then build
with `cmake --build <build> --config Release` and test with
`ctest --test-dir <build> -C Release --output-on-failure`. Set
`CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded$<$<CONFIG:Debug>:Debug>` if required by
the static dependency build. The executable is written to the build root even
with multi-config generators. No application resource or GUI subsystem is needed:
the parent launches this console helper with redirected pipes and Windows
`CREATE_NO_WINDOW`. Bundle any remaining nonstatic dependencies explicitly.
Windows code is implemented here; the acceptance run documented below must still
be repeated on a Windows host before distributing a Windows build.

```
cmake -S native/torrent-worker -B /tmp/ulos-worker -DCMAKE_BUILD_TYPE=Release
cmake --build /tmp/ulos-worker --parallel
ctest --test-dir /tmp/ulos-worker --output-on-failure
```

Debug worker builds retain debug symbols and application assertions. When an
installed libtorrent CMake package only supplies Release libraries, its
consumer-Debug-only `TORRENT_USE_ASSERTS` definition is removed: this flag changes
public header layouts and must match the linked library, not the worker's build
type. Packages providing a real Debug library keep that definition unchanged.

The Python standard-library test creates genuine v2-only, hybrid and v1 torrents,
seeds them through libtorrent on localhost, downloads through the worker, and
compares every file using SHA256 after process exit. Also tests manifest/start
barrier, pause/resume, limits, EOF, cancellation, unknown commands and no-upload
rejection. No external trackers, DHT, Python libtorrent bindings or Internet
services are needed. Fixture executable is test-only:

```
ulos-torrent-fixture --create-fixture /absolute/data/payload --torrent-out /tmp/test.torrent --v2
# alternatives: --hybrid, --v1
# optional final pair for parent integration tests with a localhost tracker:
# --tracker http://127.0.0.1:12345/announce
ulos-torrent-fixture --seed-test /tmp/test.torrent --data /absolute/data --listen 127.0.0.1:0
```

Seeder emits `{"event":"seed_ready","port":12345}` after checking files; stdin
newline or EOF stops it. `--data` is the parent of the created torrent root.
The seed-ready event waits for an actual nonzero listening port. The optional
fixture tracker is written into metainfo and must be supplied by the test; it
does not change production worker configuration. Test event-reader failures
include child exit status, stderr and prior JSON events; production errors remain
generic JSON only, with no raw diagnostics on stderr.

## Launch and trust barrier

### Explicit magnet metadata resolution (separate mode)

```
ulos-torrent-worker --resolve-magnet 'magnet:?xt=urn:btih:…' \
  --output /private/empty/absolute/directory --dht 0 --timeout 30
ulos-torrent-fixture --magnet-info /absolute/test.torrent
```

Resolution requires exactly those four argument pairs. `--timeout` is an integer
1–120 seconds; `--dht` is 0 or 1. The URI is bounded to 16 KiB. Output must be an
existing empty real absolute directory, owned exclusively by the parent. The
parent must validate the URI (including tracker credential/endpoint policy) and
launch this mode only on explicit user action, never automatically on paste.
The fixture helper returns `{"magnet":"magnet:?xt=…"}` and is test-only.

Unlike payload mode, this explicit invocation starts network resolution directly
after `{"event":"ready","protocol":1,"engine":"libtorrent","version":"2.…"}`.
No start command, manifest, or stats event is emitted. The only accepted command
is `{"command":"cancel"}`. EOF or cancellation shuts down the session, emits
`{"event":"cancelled"}`, and exits 0. Timeout, invalid input, missing required
piece layers, and engine errors emit the same static generic `error` event as
payload mode and exit 1, without URI, tracker, path, or engine diagnostics.
Success shuts down the session first and then emits
`{"event":"metadata","metainfo_hex":"<lowercase hex>"}`, exit 0. The bencoded
metainfo is at most 8 MiB (hex at most 16 MiB); parents must size their JSON-line
reader accordingly and independently validate these bytes and expected hashes
before queueing a payload download. Nothing is written to the output directory;
the parent owns cleanup even on error/cancellation.

Both v1 `btih` and v2 `btmh` hashes and hybrid magnets use libtorrent's magnet
parser and real BEP 9 peer exchange. Numeric `x.pe` IPv4/IPv6 endpoints are
supported, including percent-escaped colons/brackets; local tests append
`&x.pe=127.0.0.1%3A<port>`. URI web seeds are rejected and resume-data web seeds
are stripped. LSD/UPnP/NAT-PMP are off; DHT follows the explicit option and is
disabled (together with PEX on the handle) as soon as private metadata is observed.
Private status cannot be known before metadata arrives: DHT enabled during
resolution can disclose a private magnet hash. The UI must explain this tradeoff.

Payload downloads are prevented from add-time by non-auto-managed `upload_mode`
plus zero file priorities. **`stop_when_ready` is deliberately not used:**
libtorrent counts metadata exchange itself as downloading, so this flag would
stop before resolution. No code clears upload mode or enables payload requests.
Serialization uses `save_resume_data(save_info_dict)` and
`write_torrent_file_buf` with full piece-layer requirements, never the
`allow_missing_piece_layer` flag. BEP 9 transmits only the info dictionary, not
BEP 52 piece layers. Consequently v2/hybrid magnets whose files exceed one piece
currently fail closed; single-piece-per-file v2/hybrid and general v1 metadata
resolve. No fabricated empty layers or claim of full multi-piece v2 magnet
support is made. `worker-magnet` exercises all three modes and explicitly checks
this limitation, hash roundtrips, empty output, timeout, EOF and cancellation.

All seven pairs are required, order independent, no duplicate/unknown arguments:

```
ulos-torrent-worker --metainfo /private/source.torrent --output /owned/empty/root \
  --dht 0 --upload 1 --download-limit 0 --upload-limit 0 --peer-limit 100
```

Metainfo is bounded to 8 MiB, argument values to 4096 bytes. Output must already
be an empty real directory; neither input nor output may itself be a symlink.
Parent owns filesystem isolation and must not permit concurrent modifications.
Paths and symlinks in the parsed file inventory are checked; libtorrent's
sanitized inventory is NOT permission to trust unreviewed original metainfo.
The parent must independently compare the parsed source inventory and hashes.

Only libtorrent major 2 is accepted. Metadata is parsed **before constructing a
session**. stdout is flushed JSON-lines, starting with:

* `ready`: `protocol:1`, `engine:"libtorrent"`, `version:"2.…"`,
  `capabilities:{no_upload:false}` (explicit hard-no-upload capability).
* `manifest`: `files:[{path,length}]`, `name`, `private`, optional `infohash_v1`
  (40 lowercase hex) and `infohash_v2` (64 lowercase hex). Pad files are omitted;
  zero-byte regular files are retained; paths include the torrent root.

No session, sockets, DHT or trackers exist until stdin `{"command":"start"}`.
Parent must compare the manifest before sending start. EOF at this barrier emits
`cancelled` and exits 0. Malformed/oversized input emits a generic error and exits
1. No raw paths, URLs, engine exception text or logs are written to stderr.

## Commands after start

Every command is a JSON object ending in newline, at most 1024 bytes excluding
newline. Extra fields, unknown commands, malformed JSON, negative/out-of-range
values and partial input at EOF are fatal. Commands may be pipelined. Each main
loop read consumes at most 16 KiB and accepts at most 64 commands; excessive
command batches fail closed instead of building an unbounded command queue.

* `{"command":"pause"}` / `{"command":"resume"}`: actual handle operations.
* `{"command":"cancel"}`: shuts down then emits `cancelled`, exit 0.
* `{"command":"limits","download_bytes_per_second":0,"upload_bytes_per_second":0}`:
  both required, integer 0 (unlimited) through 1073741824 bytes/second. Initial
  CLI limits have the same bounds. Peer limit must be 1–10000.
* `{"command":"peers","limit":100}`: changes the real torrent connection cap;
  integer 1–10000, with no other fields accepted.
* `{"command":"connect_peer","address":"127.0.0.1","port":12345}`:
  direct loopback-only peer connection for local acceptance testing; numeric
  IPv4/IPv6 loopback addresses and ports 1–65535 only. No hostname resolution.

EOF cancels even if a session is running. The worker polls stdin on its main
thread, with no detached command reader or blocked join on stdin.
POSIX uses `poll`/`read`; Windows uses `PeekNamedPipe` and reads only available
bytes with `ReadFile`, checking a waiting pipe every 10 ms for up to the nominal
100 ms frame. Broken/invalid stdin pipes are EOF. Use a redirected pipe, not an
interactive Windows console. Windows entry points convert wide argv to UTF-8;
filesystem paths use `u8path`, and manifest/libtorrent paths use UTF-8 strings.

## Stats and shutdown

While running, `stats` events are sampled on a nominal 100 ms poll:
`phase` (`checking`, `downloading`, `paused`, `publishing`), `downloaded_bytes`,
`total_bytes`, `download_bytes_per_second`, `upload_bytes_per_second`,
`uploaded_bytes`, `peers`, `connecting_peers`, `queued_peers`, `seen_peers`,
`stalled_for_secs`, `tracker_count`.

Payload rates and upload totals come from libtorrent. `downloaded_bytes` is
`total_wanted_done`, not raw bytes transferred; final integrity is the parent's
responsibility. `peers` excludes half-open connections; `connecting_peers` is
`num_connections - num_peers`; `queued_peers` means known eligible connection
candidates (`connect_candidates`), not a fabricated scheduler queue;
`seen_peers` is the currently known peer list, not cumulative unique peers.
Stall time resets on payload-download or completed-byte changes and phase changes;
paused/checking/publishing time never accumulates as download inactivity. Tracker count
is the input tracker count, never a list of URLs. Paused is observed from the
actual handle flag and reported payload rates are zero in that phase.

Completion requires libtorrent `is_seeding` (all files, no selections).
`publishing` denotes engine completion only, not parent publication. The worker
pauses, synchronously destroys the session/network/disk threads and closes files,
then emits `complete` and exits 0. No background seeding persists. Errors unwind
and destroy the session before the generic error event. Parent should impose a
bounded process shutdown deadline because tracker stop announces can take time.
The libtorrent `session.hpp` destruction contract explicitly synchronizes teardown
unless a `session_proxy` is retained. This worker never creates such a proxy;
the torrent handle is destroyed before the session by reverse declaration order.

## Privacy and limitations

LSD, UPnP and NAT-PMP are always off. Private torrents additionally disable DHT,
PEX and LSD per torrent, and never enable the global DHT session. Public DHT is
controlled by `--dht`. Uploads may occur during downloads only with explicit
`--upload 1`. **Hard no-upload is unsupported:** `--upload 0` still provides the
offline handshake/manifest but fails closed on start, before session creation,
with `This engine cannot guarantee no-upload mode`, exit 1. Zero upload limits
are unlimited; torrent `upload_mode` disables downloads, so neither is misused
as a false privacy guarantee. Parent must visibly reject this combination or
route a compatible engine, never silently override consent.

Native macOS/Linux and Windows pipe implementations. No resume-data persistence, magnet metadata
fetching, selection, HTTP API, payload logs or long-lived seeding mode. The test
fixture is not a production arbitrary-peer interface. Review 2.x ABI/dependencies
and exact distribution license notices when packaging an alternate version.
