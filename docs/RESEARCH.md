# Research notes and sources

Reference date: 2026-10-13. Sources describe upstream behavior, not proof that Ulos builds or supports a platform. Recheck requirements when locking dependencies or preparing releases.

## GPUI Kit

- [Installation](https://gpui-kit.com/docs/installation)
- [Getting Started](https://gpui-kit.com/docs/getting-started)

The consulted installation guide presents GPUI Kit 0.7.3 and Rust 1.92+ for its current locked dependency graph (including Linux's `oo7` dependency). It lists macOS 15+ with Xcode Command Line Tools, and Windows 10+ with Visual Studio 2022 C++ tooling/MSVC, a Windows SDK, and CMake. These are upstream setup baselines, not a tested Ulos support policy.

For Linux the guide lists verified Ubuntu 24.04 development packages, with equivalents needed on other distributions: compiler tooling, fontconfig, Wayland, WebKitGTK 4.1, XKB/X11 integration, OpenSSL/zstd, and Vulkan loader/validation libraries. ALSA development headers are needed only for the speech feature. A graphical X11 or Wayland session with a working Vulkan driver is required; a Vulkan loader alone is not a GPU driver. Check the selected features and dependency graph before deciding what Ulos actually needs.

The getting-started guide uses `gpui-kit` as the single aligned UI dependency, calls Kit initialization before opening windows, and registers assets. Kit enables macOS text rendering support. Follow the matching release API rather than mixing independently selected GPUI snapshots. GPUI Fast is optional upstream; selecting it is not a Ulos decision or a performance claim.

These notes are an original concise summary of linked GPUI Kit guidance, not copied examples. GPUI Kit documentation credits its prose under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) and its software/code examples under Apache-2.0. Those terms do not select a license for Ulos.

## Desktop reference and clipboard boundary

The latest explicit user reference is the light IDM 6.28 structure, superseding the earlier dark AB screenshot: native titlebar, menu strip, icon-over-label toolbar, category tree, compact table, and transfer details. It guides original GPUI Kit composition, not copied source/branding or a pixel-identical skin. Kit theme and native platform chrome necessarily differ by OS.

The current manager uses GPUI's normal asynchronous clipboard API, behind an off-by-default session Options toggle. Eligibility checks run once per second only for the active window with manager dialogs closed; disabled detection does not read the clipboard. Accept a single bounded HTTP/HTTPS URL and prefill confirmation without networking; retain fingerprints rather than non-URL text and do not log clipboard contents. Signed source/query values still exist in memory for an accepted download. Wayland access depends on compositor/provider behavior and requires native testing, as do native folder/save pickers. Naming is pure URL-path processing, not HEAD or Content-Disposition discovery.

## Browser integration references

- [Chrome native messaging](https://developer.chrome.com/docs/extensions/develop/concepts/native-messaging)
- [Firefox native messaging](https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/Native_messaging)

Use browser-native host registration, extension identity allowlists, and length-prefixed JSON messaging over standard input/output. Host diagnostic logging must not corrupt the protocol stream. Chromium uses `allowed_origins`; Firefox uses `allowed_extensions`. Registration, message limits, permissions, and installation paths differ by browser/OS and must be checked against the chosen targets. Native messaging alone does not provide automatic download interception or access to browser authentication state.

## HTTP and Rust references

- [MDN: HTTP range requests](https://developer.mozilla.org/en-US/docs/Web/HTTP/Guides/Range_requests)
- [MDN: If-Range](https://developer.mozilla.org/en-US/docs/Web/HTTP/Reference/Headers/If-Range)
- [reqwest documentation](https://docs.rs/reqwest/latest/reqwest/)
- [reqwest response API](https://docs.rs/reqwest/latest/reqwest/struct.Response.html)
- [Tokio documentation](https://docs.rs/tokio/latest/tokio/)

A range advertisement is not sufficient evidence of usable range support. Verify the actual response and representation before writing resumed or segmented bytes. Strong validators and `If-Range` protect against appending a different representation; `200` requires a safe restart. Keep content decoding and byte offsets consistent.

The selected networking direction is async reqwest with rustls and Tokio. Reqwest offers streaming response APIs; feature selection must enable the needed TLS/body behavior without buffering the full download. Its `latest` documentation is mutable: verify APIs and feature names against the version actually selected by the workspace.

## Torrent research: pending

Torrent support is a future design direction, not a researched or selected engine. No torrent dependency has been added. Evaluate candidate engines against maintenance, embedding/runtime ownership, bounded metadata parsing, piece integrity, multifile path safety, cancellation, tracker/DHT privacy, seeding/bandwidth/storage controls, and license compatibility before choosing one. No engine is endorsed here. See [Torrent scope](TORRENTS.md); the linked HTTP and GPUI sources above do not validate torrent behavior.

## Outstanding research and evidence

Choose Linux distributions, browser targets, final name, and Ulos license. Validate GNOME/KDE, X11/Wayland, scale factors, and GPU drivers through an explicit matrix. Only the current macOS development host is available at initialization; no completed Ulos platform tests are claimed here. Linux is a first-class priority, not a promise of compatibility with every distribution.
