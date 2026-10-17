#!/usr/bin/env python3
"""Compile real installers around synthetic payloads; never install/run Ulos.

Windows CI supplies the pinned compiler in ULOS_ISCC. Both architecture branches
are compiled on each Windows runner; synthetic PE headers test staging only, not
native executable behavior. All generated files live in an owned temporary copy.
"""
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def main():
    if sys.platform != 'win32':
        sys.exit('This smoke requires Windows and the pinned Inno Setup compiler')
    version = subprocess.check_output(
        [sys.executable, str(ROOT / 'scripts/release-version.py')], cwd=ROOT, text=True).strip()
    with tempfile.TemporaryDirectory(prefix='ulos-installer-smoke-') as temp:
        root = Path(temp)
        for filename in ('Cargo.toml', 'Cargo.lock', 'LICENSE', 'THIRD_PARTY_NOTICES.md'):
            shutil.copy2(ROOT / filename, root / filename)
        # Cargo metadata --no-deps still reads every workspace member manifest.
        shutil.copytree(ROOT / 'crates', root / 'crates')
        shutil.copytree(ROOT / 'packaging', root / 'packaging')
        (root / 'scripts').mkdir()
        for filename in ('release-package.py', 'release-version.py'):
            shutil.copy2(ROOT / 'scripts' / filename, root / 'scripts' / filename)
        for arch, machine, target in (
            ('x86_64', 0x8664, 'x86_64-pc-windows-msvc'),
            ('arm64', 0xAA64, 'aarch64-pc-windows-msvc'),
        ):
            payload = root / 'target' / target / 'release'
            payload.mkdir(parents=True)
            header = bytearray(128)
            header[:2] = b'MZ'
            struct.pack_into('<I', header, 60, 64)
            header[64:68] = b'PE\0\0'
            struct.pack_into('<H', header, 68, machine)
            for name in ('ulos-desktop.exe', 'ulos-torrent-worker.exe'):
                (payload / name).write_bytes(header)
            env = dict(os.environ, CARGO_BUILD_TARGET=target, CARGO_TARGET_DIR=str(root / 'target'))
            subprocess.run([sys.executable, str(root / 'scripts/release-package.py'),
                            'windows', arch, version], cwd=root, env=env, check=True)
            output = root / 'dist' / f'Ulos-{version}-windows-{arch}-Setup.exe'
            if not output.is_file() or output.stat().st_size < 1024:
                raise RuntimeError(f'Missing/empty compiled installer: {output.name}')
            with output.open('rb') as binary:
                if binary.read(2) != b'MZ':
                    raise RuntimeError('Installer does not have an executable header')
            print(f'Real compiler smoke passed: {output.name} (synthetic payload; not executed)')


if __name__ == '__main__':
    main()
