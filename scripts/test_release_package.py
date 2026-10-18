"""Synthetic header and packaging safety tests, not native build/runtime tests."""
import contextlib
import importlib.util
import io
import os
import re
from pathlib import Path
import struct
import tempfile
import textwrap
import unittest
from unittest import mock

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
    def test_workflow_packaging_ignores_msvc_platform_environment(self):
        workflow = (Path(__file__).resolve().parents[1] / '.github/workflows/release.yml').read_text()
        step = workflow.split('      - name: Package desktop\n', 1)[1].split('\n      - ', 1)[0]
        self.assertIn('shell: python {0}', step)
        self.assertNotIn('\n          PLATFORM:', step)
        self.assertNotIn('\n          ARCH:', step)
        code = compile(textwrap.dedent(step.split('        run: |\n', 1)[1]), 'release-package-step', 'exec')
        for arch in ('x86_64', 'arm64'):
            for inherited in ('x64', 'ARM64', ''):
                with self.subTest(arch=arch, inherited=inherited), mock.patch.dict(os.environ, {
                    'Platform': inherited, 'PLATFORM': inherited, 'ARCH': 'wrong',
                    'ULOS_RELEASE_PLATFORM': 'windows', 'ULOS_RELEASE_ARCH': arch,
                    'RELEASE_VERSION': '0.0.1-pre.2',
                }), mock.patch('subprocess.run') as run:
                    exec(code, {})
                    run.assert_called_once_with([
                        package.sys.executable, 'scripts/release-package.py',
                        'windows', arch, '0.0.1-pre.2',
                    ], check=True)

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

    def test_usage_and_version_validation(self):
        for args in ([], ['windows', 'arm64'], ['windows', 'arm64', '../bad']):
            with self.assertRaises(SystemExit):
                package.main(args)


class WindowsInstallerTests(unittest.TestCase):
    """Mock compiler orchestration only; no native installer validation claimed."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / 'scripts').mkdir()
        (self.root / 'packaging/licenses').mkdir(parents=True)
        (self.root / 'packaging/windows').mkdir()
        (self.root / 'packaging/windows/ulos.iss').write_text('synthetic script')
        for filename in ('LICENSE', 'THIRD_PARTY_NOTICES.md', 'packaging/INSTALL.md'):
            (self.root / filename).write_text('test notices: © contributors', encoding='utf-8')
        for filename in package.REQUIRED_LICENSES:
            (self.root / 'packaging/licenses' / filename).write_text('license ' + filename)
        self.compiler = self.root / 'ISCC.exe'
        self.compiler.write_bytes(b'compiler fixture, never executed')
        self.target = self.root / 'custom/aarch64-pc-windows-msvc/release'
        self.target.mkdir(parents=True)
        self.output = self.root / 'dist/Ulos-1.0.0-windows-arm64-Setup.exe'
        self.old_cwd = Path.cwd()
        self.addCleanup(os.chdir, self.old_cwd)
        for patcher in (
            mock.patch.object(package, '__file__', str(self.root / 'scripts/release-package.py')),
            mock.patch.object(package.subprocess, 'check_output', return_value='1.0.0\n'),
            mock.patch.dict(os.environ, {'CARGO_TARGET_DIR': str(self.root / 'custom'),
                                        'CARGO_BUILD_TARGET': 'aarch64-pc-windows-msvc',
                                        'ULOS_ISCC': str(self.compiler)}),
        ):
            patcher.start()
            self.addCleanup(patcher.stop)
        self.set_binaries('arm64')

    def set_binaries(self, arch):
        for filename in ('ulos-desktop.exe', 'ulos-torrent-worker.exe'):
            (self.target / filename).write_bytes(header('windows', arch))

    def compile_fixture(self, command, check):
        self.assertTrue(check)
        self.assertEqual(command[0], str(self.compiler))
        self.assertEqual(command[-1], str(self.root / 'packaging/windows/ulos.iss'))
        defines = dict(arg[2:].split('=', 1) for arg in command if arg.startswith('/D'))
        self.assertEqual(defines['AppVersion'], '1.0.0')
        self.assertEqual(defines['PayloadArch'], self.arch)
        stage = Path(defines['StageDir'])
        self.assertEqual((stage / 'Ulos.exe').read_bytes(), header('windows', self.arch))
        self.assertEqual((stage / 'ulos-torrent-worker.exe').read_bytes(), header('windows', self.arch))
        for filename in package.REQUIRED_LICENSES:
            self.assertTrue((stage / 'licenses' / filename).is_file())
        self.assertTrue((stage / 'INSTALL.md').is_file())
        self.assertTrue((stage / 'LICENSE').is_file())
        notices = (stage / 'OSS-NOTICES.txt').read_text(encoding='utf-8-sig')
        self.assertIn('© contributors', notices)
        for filename in package.REQUIRED_LICENSES:
            self.assertIn('===== ' + filename + ' =====', notices)
        # This MZ fixture only exercises artifact publication, not a real installer.
        (Path(defines['BuildOutputDir']) / (defines['OutputName'] + '.exe')).write_bytes(b'MZ' + bytes(126))

    def run_package(self, arch='arm64'):
        self.arch = arch
        with contextlib.redirect_stdout(io.StringIO()):
            package.main(['windows', arch, '1.0.0'])

    def test_both_architectures_compile_and_no_zip_fallback(self):
        for arch in ('arm64', 'x86_64'):
            with self.subTest(arch=arch):
                self.set_binaries(arch)
                with mock.patch.object(package.subprocess, 'run', side_effect=self.compile_fixture) as runner:
                    self.run_package(arch)
                    runner.assert_called_once()
                output = self.root / f'dist/Ulos-1.0.0-windows-{arch}-Setup.exe'
                self.assertTrue(output.is_file())
                self.assertFalse(list((self.root / 'dist').glob('*.zip')))
                with self.assertRaises(SystemExit):
                    self.run_package(arch)
                output.unlink()

    def test_missing_worker_and_mismatched_worker_fail_before_compilation(self):
        worker = self.target / 'ulos-torrent-worker.exe'
        with mock.patch.object(package.subprocess, 'run') as runner:
            worker.unlink()
            with self.assertRaises(SystemExit):
                self.run_package()
            worker.write_bytes(header('windows', 'x86_64'))
            with self.assertRaises(ValueError):
                self.run_package()
            runner.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_missing_compiler_fails_closed(self):
        self.compiler.unlink()
        with self.assertRaisesRegex(RuntimeError, 'ISCC.exe required'):
            self.run_package()
        self.assertFalse(self.output.exists())
        self.assertFalse(list((self.root / 'dist').glob('*.zip')))

    def test_missing_license_fails_closed(self):
        (self.root / 'packaging/licenses/Boost-1.0.txt').unlink()
        with mock.patch.object(package.subprocess, 'run') as runner:
            with self.assertRaisesRegex(ValueError, 'required license'):
                self.run_package()
            runner.assert_not_called()
        self.assertFalse(self.output.exists())

    def test_compiler_failure_missing_output_and_invalid_output_not_published(self):
        import subprocess
        def invalid_output(command, check):
            defines = dict(arg[2:].split('=', 1) for arg in command if arg.startswith('/D'))
            (Path(defines['BuildOutputDir']) / (defines['OutputName'] + '.exe')).write_bytes(bytes(128))
        for effect, exception in ((subprocess.CalledProcessError(1, 'ISCC'), subprocess.CalledProcessError),
                                  (None, RuntimeError), (invalid_output, RuntimeError)):
            with self.subTest(effect=effect):
                with mock.patch.object(package.subprocess, 'run', side_effect=effect):
                    with self.assertRaises(exception):
                        self.run_package()
                self.assertFalse(self.output.exists())
                self.assertFalse(list((self.root / 'dist').glob('*.zip')))

    def test_existing_symlink_output_refused(self):
        self.output.parent.mkdir()
        self.output.symlink_to(self.root / 'absent')
        with self.assertRaises(SystemExit):
            self.run_package()

    def test_workflows_pin_windows_native_linker_and_powershell_cargo(self):
        root = Path(__file__).resolve().parent.parent
        guard = '      - name: Verify native MSVC linker and Rust build-script linking\n'
        for name, jobs in (('ci.yml', ('engine', 'desktop')), ('release.yml', ('build',))):
            text = (root / '.github/workflows' / name).read_text()
            self.assertEqual(text.count(guard), len(jobs))
            for job in jobs:
                with self.subTest(workflow=name, job=job):
                    # Job contents contain deeper indentation; stop only at a sibling job.
                    body = re.split(r'\n  [a-zA-Z][\w-]*:\n', text.split(f'  {job}:\n', 1)[1], maxsplit=1)[0]
                    self.assertEqual(body.count(guard), 1)
                    smoke = body.split(guard, 1)[1].split('\n      - ', 1)[0]
                    for required in (
                        "if: runner.os == 'Windows'", 'shell: pwsh',
                        'Get-Command cl.exe', "Join-Path $tools 'link.exe'",
                        "'CARGO_TARGET_'", "'_LINKER'", '>> $env:GITHUB_ENV',
                        '>> $env:GITHUB_PATH', "Join-Path $smoke 'build.rs'",
                        'cargo build --offline', 'if ($LASTEXITCODE -ne 0)',
                        'finally {', 'Remove-Item -LiteralPath $smoke',
                    ):
                        self.assertIn(required, smoke)
                    self.assertLess(body.index(guard), body.index('      - name: Build and test bundled native torrent worker')
                                    if job != 'engine' else body.index('      - run: cargo test --locked'))
        release = (root / '.github/workflows/release.yml').read_text()
        for name in ('Test engine and CLI (Windows native shell)',
                     'Verify real v2 and hybrid task publication (Windows)',
                     'Check desktop compilation (Windows)', 'Check strict lint (Windows)',
                     'Build desktop (Windows)'):
            step = release.split(f'      - name: {name}\n', 1)[1].split('\n      - ', 1)[0]
            self.assertIn("if: runner.os == 'Windows'", step)
            self.assertIn('shell: pwsh', step)

    def test_iss_policy_and_exact_compiler_pin(self):
        source = Path(__file__).resolve().parent.parent / 'packaging/windows/ulos.iss'
        script = source.read_text()
        for setting in ('PrivilegesRequired=lowest', 'CloseApplications=no', 'RestartApplications=no',
                        'ChangesAssociations=no', 'LicenseFile={#StageDir}\\LICENSE',
                        'InfoBeforeFile={#StageDir}\\OSS-NOTICES.txt',
                        '#if VER != EncodeVer(6, 7, 3)', '#define InstallArchitecture "arm64"',
                        '#define InstallArchitecture "x64os"',
                        'ArchitecturesInstallIn64BitMode={#InstallArchitecture}'):
            self.assertIn(setting, script)
        for forbidden in ('[Run]', '[UninstallRun]', '[Registry]', 'PrivilegesRequiredOverridesAllowed'):
            self.assertNotIn(forbidden, script)


if __name__ == '__main__':
    unittest.main()
