"""Workflow cache policy contracts; warm-hit timings require real GitHub runners."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[1]
RUST_CACHE = 'Swatinem/rust-cache@779680da715d629ac1d338a641029a2f4372abb5'
RESTORE = 'actions/cache/restore@0057852bfaa89a56745cba8c7296529d2fc39830'
SAVE = 'actions/cache/save@0057852bfaa89a56745cba8c7296529d2fc39830'


def job(text, name):
    return re.split(r'\n  [a-zA-Z][\w-]*:\n', text.split(f'  {name}:\n', 1)[1], maxsplit=1)[0]


def step(text, name):
    return text.split(f'      - name: {name}\n', 1)[1].split('\n      - ', 1)[0]


class CachePolicyTests(unittest.TestCase):
    def setUp(self):
        self.ci = (ROOT / '.github/workflows/ci.yml').read_text()
        self.release = (ROOT / '.github/workflows/release.yml').read_text()

    def test_rust_cache_is_pinned_toolchain_keyed_and_dependency_only(self):
        for workflow, names in ((self.ci, ('engine', 'desktop')), (self.release, ('build', 'desktop-tests'))):
            for name in names:
                with self.subTest(job=name):
                    body = job(workflow, name)
                    cache = step(body, 'Cache Cargo dependencies')
                    self.assertIn(RUST_CACHE, cache)
                    self.assertIn('continue-on-error: true', cache)
                    self.assertIn('cache-bin: false', cache)
                    self.assertIn('cache-targets: true', cache)
                    self.assertIn('cache-workspace-crates: false', cache)
                    self.assertIn('cache-on-failure: false', cache)
                    self.assertIn('env-vars: VCToolsVersion WindowsSDKVersion ImageOS ImageVersion', cache)
                    self.assertIn("github.event_name != 'pull_request'", cache)
                    self.assertIn("github.ref == 'refs/heads/main'", cache)
                    self.assertNotIn('cache-directories:', cache)
                    self.assertLess(body.index('dtolnay/rust-toolchain'), body.index(RUST_CACHE))
                    if name != 'desktop-tests':
                        self.assertIn('${{ matrix.target }}', cache)
                    if name in ('build', 'desktop'):
                        self.assertLess(body.index('Verify native MSVC linker'), body.index(RUST_CACHE))

    def test_native_cache_has_exact_abi_recipe_identity_and_archive_scope(self):
        for workflow, name in ((self.ci, 'desktop'), (self.release, 'build')):
            with self.subTest(job=name):
                body = job(workflow, name)
                identity = step(body, 'Identify native dependency cache')
                for required in ('VCPKG_BASELINE', 'CARGO_BUILD_TARGET', 'RUNNER_ARCH',
                                 'VCToolsVersion', 'WindowsSDKVersion', 'ImageOS', 'ImageVersion',
                                 'Get-FileHash -LiteralPath $compiler', 'Get-FileHash -LiteralPath $linker',
                                 'scripts/build-torrent-worker.py', 'native/torrent-worker/CMakeLists.txt',
                                 "'.cache/ulos/vcpkg'", 'VCPKG_DEFAULT_BINARY_CACHE=',
                                 'VCPKG_BINARY_SOURCES=clear;files,$cache,readwrite'):
                    self.assertIn(required, identity)
                restore = step(body, 'Restore native dependency archives')
                self.assertIn(RESTORE, restore)
                self.assertNotIn('restore-keys:', restore)
                self.assertIn('path: ${{ steps.native-cache.outputs.path }}', restore)
                save = step(body, 'Save verified native dependency archives')
                self.assertIn(SAVE, save)
                self.assertIn("github.event_name != 'pull_request'", save)
                self.assertIn("github.ref == 'refs/heads/main'", save)
                for operation in (restore, save):
                    self.assertIn('continue-on-error: true', operation)
                    self.assertNotIn('target/native-deps', operation)
                    self.assertNotIn('target/torrent-worker-build', operation)
                    self.assertNotIn('dist/', operation)
                build = step(body, 'Build and test bundled native torrent worker')
                self.assertIn('--test --install-deps', build)
                self.assertNotIn('cache-hit', build)
                self.assertLess(body.index(RESTORE), body.index('Build and test bundled native torrent worker'))
                self.assertLess(body.index('Build and test bundled native torrent worker'), body.index(SAVE))
                self.assertIn('--ignored --test-threads=1', body)

    def test_all_platform_and_release_gates_remain(self):
        for name in ('engine', 'desktop'):
            body = job(self.ci, name)
            for target in ('x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu',
                           'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc', 'aarch64-apple-darwin'):
                self.assertIn(target, body)
        self.assertIn('cargo test --locked\n', job(self.ci, 'engine'))
        self.assertIn('cargo fmt --all -- --check', job(self.ci, 'engine'))
        self.assertIn('scripts/smoke-windows-installer.py', job(self.ci, 'engine'))
        self.assertIn('cargo test --locked --workspace -- --test-threads=1', job(self.ci, 'desktop'))
        self.assertIn('cargo clippy --locked --workspace --all-targets -- -D warnings', job(self.ci, 'desktop'))
        self.assertIn('x86_64-apple-darwin', job(self.release, 'build'))
        self.assertIn('cargo build --locked --release -p ulos-desktop', self.release)
        self.assertIn('scripts/release-package.py', self.release)
        publish = job(self.release, 'publish')
        self.assertIn('needs: [validate, build, desktop-tests]', publish)
        self.assertIn('sha256sum --check SHA256SUMS', publish)
        self.assertIn('--verify-tag', publish)
        self.assertNotIn('pull_request_target', self.ci + self.release)

    def test_cancel_stale_ci_but_not_release_or_tag_coverage(self):
        self.assertIn('group: ${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}', self.ci)
        self.assertIn('cancel-in-progress: true', self.ci)
        self.assertIn('cancel-in-progress: false', self.release)
        self.assertIn('  push:\n', self.ci)
        self.assertNotIn('tags-ignore:', self.ci)
        self.assertNotIn('Build Windows distribution payload', self.ci)
        self.assertNotIn('Ulos-Windows-${{ runner.arch }}-Setup', self.ci)


if __name__ == '__main__':
    unittest.main()
