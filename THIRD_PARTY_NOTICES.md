# Third-party notices

Ulos remains licensed under the MIT License; retain the accompanying `LICENSE`.

## librqbit 9.0.1

- Upstream: <https://github.com/ikatson/rqbit>
- License: Apache License, Version 2.0
- Upstream license notice: <https://github.com/ikatson/rqbit/blob/v9.0.1/LICENSE>

Copyright 2021 Igor Katson

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

<https://www.apache.org/licenses/LICENSE-2.0>

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.

A copy of the Apache 2.0 license is included in `packaging/licenses/Apache-2.0.txt`
(and `Contents/Resources/licenses/Apache-2.0.txt` in the local macOS bundle).
This notice identifies the selected torrent engine; it is not a complete
transitive-dependency licensing or distribution-compliance audit. Distribution
still requires reviewing all bundled dependencies, notices, and assets.

## renamore 0.3.2 (Windows builds)

- Upstream: <https://github.com/indianakernick/renamore>
- License selected: MIT (upstream offers MIT OR Apache-2.0)
- Copyright (c) 2022 Indiana Kernick

Used for Windows exclusive directory publication (`MoveFileExW` without replacement
flags). The full MIT notice is bundled in `packaging/licenses/renamore-MIT.txt`.
No non-atomic rename fallback is used.

## Native BitTorrent worker (next build)

Ulos's protocol-1 worker links libtorrent-rasterbar 2.x (locally verified with
2.1.2), nlohmann-json, and platform runtime dependencies. The application remains
MIT; these dependencies keep their own licenses.

- libtorrent: Copyright (c) 2003–2020 Arvid Norberg. BSD-3-Clause plus the bundled
  component notices retained in `packaging/licenses/libtorrent-BSD-3-Clause.txt`
  and `packaging/licenses/libtorrent-2.1.2-NOTICES.txt`.
- nlohmann-json 3.12.0: Copyright (c) 2013–2025 Niels Lohmann. MIT notice retained
  in `packaging/licenses/nlohmann-json-MIT.txt`.
- Boost: Boost Software License 1.0, retained in `packaging/licenses/Boost-1.0.txt`.
- OpenSSL: Apache License 2.0, retained in
  `packaging/licenses/OpenSSL-Apache-2.0.txt`. The locally bundled runtime is 4.0.3;
  build environments may supply a different compatible OpenSSL runtime.

Windows native builds use a pinned vcpkg baseline; macOS/Linux packages include
the worker's non-system runtime libraries and preserve their notices. This is
not a complete transitive compliance audit or an endorsement by these authors.
