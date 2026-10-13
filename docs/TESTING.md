# Testing and validation

Updated: 2026-10-13. This is a test plan; [Status](STATUS.md) records actual verification results and known gaps.

## Planned workspace commands

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

The initial desktop check is an observed starter window with legible text and working input, not an active transfer UI. After integration verify real state updates, cancellation, errors, keyboard navigation, focus, accessibility, and shutdown without orphaned work. Review the UI with the user after initialization.

Only the current macOS host is available at initialization. Until recorded CI/native runs exist, Linux and Windows remain unvalidated. Linux must receive first-class testing:

| Dimension | Planned coverage |
| --- | --- |
| Distribution | Ubuntu 24.04 upstream baseline; additional distributions/versions to agree |
| Desktop/session | GNOME and KDE; Wayland and X11 |
| Scale | 100%, 150%, 200%; mixed-scale monitors where available |
| Graphics | Intel/AMD Mesa and NVIDIA configurations with working Vulkan drivers |
| Window lifecycle | Launch, resize, focus, text/input, close, error reporting |

Record exact distro, desktop, session, scale, GPU/driver, Rust version, and dependency lockfile. Build-only/headless CI is not a replacement for graphical validation. Windows needs MSVC/CMake/SDK validation; macOS needs the upstream OS baseline and CLT. No matrix row is a support guarantee before testing.

## Future browser and distribution tests

For selected Chromium/Firefox versions verify host registration and allowlists, bounded malformed/unsupported messages, unauthorized sender rejection, explicit user import, host disconnect/restart, and protocol-safe diagnostics. Confirm there is no automatic clipboard polling. Test clean installs and packaged native host discovery on each release platform. Choose the Ulos license and audit dependencies/notices before distribution.
