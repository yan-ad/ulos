"""Build/bundler policy unit tests; real wire fixtures run via --test."""
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('build_worker', Path(__file__).with_name('build-torrent-worker.py'))
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class BuildTests(unittest.TestCase):
    def test_native_targets_and_cross_compile_rejection(self):
        for system, cpu, target, expected in (
            ('Darwin', 'arm64', 'aarch64-apple-darwin', ('macos', 'arm64')),
            ('Windows', 'AMD64', 'x86_64-pc-windows-msvc', ('windows', 'x86_64')),
            ('Windows', 'ARM64', 'aarch64-pc-windows-msvc', ('windows', 'arm64')),
            ('Linux', 'aarch64', 'aarch64-unknown-linux-gnu', ('linux', 'arm64')),
        ):
            with self.subTest(target=target), mock.patch.object(build.platform, 'system', return_value=system), mock.patch.object(build.platform, 'machine', return_value=cpu), mock.patch.dict(os.environ, {'CARGO_BUILD_TARGET': target}):
                self.assertEqual(build.native_identity(), expected)
                with mock.patch.dict(os.environ, {'CARGO_BUILD_TARGET': 'x86_64-apple-darwin' if cpu != 'AMD64' else 'aarch64-pc-windows-msvc'}):
                    with self.assertRaises(ValueError):
                        build.native_identity()

    def test_linux_dependency_filter_and_missing(self):
        listing = '''linux-vdso.so.1 (0x123)
 libtorrent-rasterbar.so.2.0 => /lib/libtorrent-rasterbar.so.2.0 (0x123)
 libssl.so.3 => /lib/libssl.so.3 (0x123)
 libcrypto.so.3 => /lib/libcrypto.so.3 (0x123)
 libstdc++.so.6 => /lib/libstdc++.so.6 (0x123)
 libm.so.6 => /lib/libm.so.6 (0x123)
 libc.so.6 => /lib/libc.so.6 (0x123)
 libgcc_s.so.1 => /lib/libgcc_s.so.1 (0x123)
 /lib64/ld-linux-x86-64.so.2 (0x123)
'''
        with mock.patch.object(build, 'text', return_value=listing):
            self.assertEqual([name for name, _ in build.linux_dependencies('worker')], ['libtorrent-rasterbar.so.2.0', 'libssl.so.3', 'libcrypto.so.3'])
        with mock.patch.object(build, 'text', return_value='libtorrent-rasterbar.so => not found'):
            with self.assertRaises(ValueError):
                build.linux_dependencies('worker')

    def test_macos_recursive_relocation_and_sign_order(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            worker = root / 'worker'
            worker.write_text('exe')
            original = root / 'original'
            original.mkdir()
            torrent = original / 'libtorrent.dylib'
            crypto = original / 'libcrypto.dylib'
            torrent.write_text('torrent')
            crypto.write_text('crypto')
            # Mach-O load commands use Unix syntax even when this synthetic
            # policy test runs on Windows. Resolve loader-relative paths using
            # the host filesystem, not fake C:\\... Mach-O absolute paths.
            deps = {worker: ['@loader_path/original/libtorrent.dylib', '/usr/lib/libc++.1.dylib'], torrent: ['@loader_path/libtorrent.dylib', '@loader_path/libcrypto.dylib'], crypto: ['@loader_path/libcrypto.dylib', '/usr/lib/libSystem.B.dylib']}
            with mock.patch.object(build, 'mac_dependencies', side_effect=lambda p: deps[p]), mock.patch.object(build.package, 'verify_binary_architecture'), mock.patch.object(build, 'run') as run:
                build.bundle_macos(worker, root / 'lib', 'arm64')
                self.assertEqual({p.name for p in (root / 'lib').iterdir()}, {'libtorrent.dylib', 'libcrypto.dylib'})
                run.assert_any_call('install_name_tool', '-change', '@loader_path/original/libtorrent.dylib', '@loader_path/lib/libtorrent.dylib', worker)
                run.assert_any_call('install_name_tool', '-change', '@loader_path/libcrypto.dylib', '@loader_path/libcrypto.dylib', root / 'lib/libtorrent.dylib')
                calls = run.call_args_list
                first_sign = next(i for i, call in enumerate(calls) if call.args[0] == 'codesign')
                self.assertTrue(all(call.args[0] == 'codesign' for call in calls[first_sign:]))

    def test_macos_unresolved_rpath_fails_closed(self):
        with mock.patch.object(build, 'mac_dependencies', return_value=['@rpath/unknown.dylib']):
            with self.assertRaisesRegex(ValueError, 'Unresolved'):
                build.bundle_macos(Path('/worker'), Path('/lib'), 'arm64')

    def test_windows_rejects_runtime_dll_dependency(self):
        with mock.patch.object(build, 'text', return_value='    KERNEL32.dll\n    WS2_32.dll\n    dbghelp.dll\n'):
            build.verify_windows_static('worker.exe')
        for dependency in ('VCRUNTIME140.dll', 'torrent-rasterbar.dll', 'libcrypto-3.dll'):
            with self.subTest(dependency=dependency), mock.patch.object(build, 'text', return_value=f'    {dependency}\n'):
                with self.assertRaisesRegex(ValueError, 'Unexpected DLL'):
                    build.verify_windows_static('worker.exe')

    def test_pinned_vcpkg_commit(self):
        self.assertRegex(build.VCPKG_BASELINE, r'^[0-9a-f]{40}$')


if __name__ == '__main__':
    unittest.main()
