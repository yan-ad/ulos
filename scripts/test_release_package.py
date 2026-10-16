"""Synthetic header and packaging safety tests, not native build/runtime tests."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock
import zipfile

spec = importlib.util.spec_from_file_location('release_package', Path(__file__).with_name('release-package.py'))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


def header(platform, arch):
    data = bytearray(128)
    if platform == 'windows':
        data[:2] = b'MZ'
        struct.pack_into('<I', data, 60, 64)
        data[64:68] = b'PE\0\0'
        struct.pack_into('<H', data, 68, 0xAA64 if arch == 'arm64' else 0x8664)
    elif platform == 'linux':
        data[:7] = b'\x7fELF\x02\x01\x01'
        struct.pack_into('<H', data, 18, 183 if arch == 'arm64' else 62)
    else:
        data[:4] = b'\xcf\xfa\xed\xfe'
        struct.pack_into('<I', data, 4, 0x0100000C if arch == 'arm64' else 0x01000007)
    return data


class ArchitectureTests(unittest.TestCase):
    def test_all_six_headers_and_mismatches(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'binary'
            for platform in ('windows', 'linux', 'macos'):
                for arch in ('arm64', 'x86_64'):
                    with self.subTest(platform=platform, arch=arch):
                        path.write_bytes(header(platform, arch))
                        self.assertEqual(package.verify_binary_architecture(path, platform, arch), arch)
                        with self.assertRaises(ValueError):
                            package.verify_binary_architecture(path, platform, 'x86_64' if arch == 'arm64' else 'arm64')

    def test_malformed_headers_fail_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'binary'
            for platform in ('windows', 'linux', 'macos'):
                valid = header(platform, 'arm64')
                wrong_machine = bytearray(valid)
                index = {'windows': 68, 'linux': 18, 'macos': 4}[platform]
                wrong_machine[index:index + 4] = b'\0' * 4
                for data in (b'', b'MZ', valid[:20], bytes(128), wrong_machine):
                    path.write_bytes(data)
                    with self.subTest(platform=platform, size=len(data)):
                        with self.assertRaises(ValueError):
                            package.verify_binary_architecture(path, platform, 'arm64')
            bad_offset = header('windows', 'arm64')
            struct.pack_into('<I', bad_offset, 60, 0xFFFFFFF0)
            path.write_bytes(bad_offset)
            with self.assertRaises(ValueError):
                package.verify_binary_architecture(path, 'windows', 'arm64')

    def test_windows_arm_archive_required_worker_and_absent_output(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'scripts').mkdir()
            (root / 'packaging/licenses').mkdir(parents=True)
            for filename in ('LICENSE', 'THIRD_PARTY_NOTICES.md', 'packaging/INSTALL.md', 'packaging/licenses/MIT.txt'):
                (root / filename).write_text('test')
            target = root / 'custom/aarch64-pc-windows-msvc/release'
            target.mkdir(parents=True)
            (target / 'ulos-desktop.exe').write_bytes(header('windows', 'arm64'))
            old_cwd = Path.cwd()
            try:
                with mock.patch.object(package, '__file__', str(root / 'scripts/release-package.py')), mock.patch.object(package.subprocess, 'check_output', return_value='1.0.0\n'), mock.patch.dict(os.environ, {'CARGO_TARGET_DIR': str(root / 'custom'), 'CARGO_BUILD_TARGET': 'aarch64-pc-windows-msvc'}):
                    for worker_present in (False, True):
                        if worker_present:
                            (target / 'ulos-torrent-worker.exe').write_bytes(header('windows', 'arm64'))
                        if not worker_present:
                            with self.assertRaises(SystemExit):
                                package.main(['windows', 'arm64', '1.0.0'])
                            self.assertFalse((root / 'dist/Ulos-1.0.0-windows-arm64.zip').exists())
                            continue
                        with contextlib.redirect_stdout(io.StringIO()) as stdout:
                            package.main(['windows', 'arm64', '1.0.0'])
                        output = root / 'dist/Ulos-1.0.0-windows-arm64.zip'
                        self.assertIn(output.name, stdout.getvalue())
                        with zipfile.ZipFile(output) as archive:
                            self.assertIn('Ulos-1.0.0-windows-arm64/Ulos.exe', archive.namelist())
                            self.assertEqual('Ulos-1.0.0-windows-arm64/ulos-torrent-worker.exe' in archive.namelist(), worker_present)
                        with self.assertRaises(SystemExit):
                            package.main(['windows', 'arm64', '1.0.0'])
                        output.unlink()
                    (target / 'ulos-torrent-worker.exe').write_bytes(header('windows', 'x86_64'))
                    with self.assertRaises(ValueError):
                        package.main(['windows', 'arm64', '1.0.0'])
                    self.assertFalse(output.exists())
            finally:
                os.chdir(old_cwd)


if __name__ == '__main__':
    unittest.main()
