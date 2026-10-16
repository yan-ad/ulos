#!/usr/bin/env python3
"""Create absent release archives. Does not launch or install the application."""
import os
from pathlib import Path
import shutil
import subprocess
import struct
import sys
import tarfile
import tempfile
import zipfile


def verify_binary_architecture(path, platform, expected):
    """Check a thin executable header; never infer architecture from its filename."""
    machines = {
        'windows': {0xAA64: 'arm64', 0x8664: 'x86_64'},
        'linux': {183: 'arm64', 62: 'x86_64'},
        'macos': {0x0100000C: 'arm64', 0x01000007: 'x86_64'},
    }
    with Path(path).open('rb') as binary:
        header = binary.read(64)
        machine = None
        if platform == 'windows' and len(header) == 64 and header[:2] == b'MZ':
            offset = struct.unpack_from('<I', header, 0x3C)[0]
            if offset >= 64:
                binary.seek(offset)
                pe = binary.read(24)
                if len(pe) == 24 and pe[:4] == b'PE\0\0':
                    machine = struct.unpack_from('<H', pe, 4)[0]
        elif platform == 'linux' and len(header) == 64 and header[:4] == b'\x7fELF':
            if header[4:7] == b'\x02\x01\x01':  # ELF64, little endian, version 1
                machine = struct.unpack_from('<H', header, 18)[0]
        elif platform == 'macos' and len(header) >= 32:
            endian = {b'\xcf\xfa\xed\xfe': '<', b'\xfe\xed\xfa\xcf': '>'}.get(header[:4])
            if endian:
                machine = struct.unpack_from(endian + 'I', header, 4)[0]
    actual = machines.get(platform, {}).get(machine)
    if actual != expected or expected not in ('arm64', 'x86_64'):
        raise ValueError(f'{path}: expected {platform}/{expected}, actual {actual or "unknown/invalid header"} (machine={machine})')
    return actual


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    platform, arch, version = argv
    root = Path(__file__).resolve().parent.parent
    os.chdir(root)
    actual = subprocess.check_output([sys.executable, 'scripts/release-version.py', version], text=True).strip()
    assert actual == version
    if (platform, arch) not in {(system, cpu) for system in ('macos', 'windows', 'linux') for cpu in ('arm64', 'x86_64')}:
        sys.exit('Unsupported platform/architecture')
    name = f'Ulos-{version}-{platform}-{arch}'
    dist = root / 'dist'
    dist.mkdir(exist_ok=True)
    extension = {'macos': '.dmg', 'windows': '.zip', 'linux': '.tar.gz'}[platform]
    output = dist / (name + extension)
    if output.exists() or output.is_symlink():
        sys.exit(f'Refusing existing output: {output}')
    target = Path(os.environ.get('CARGO_TARGET_DIR', root / 'target'))
    if os.environ.get('CARGO_BUILD_TARGET'):
        target /= os.environ['CARGO_BUILD_TARGET']
    binary = target / 'release' / ('ulos-desktop.exe' if platform == 'windows' else 'ulos-desktop')
    verify_binary_architecture(binary, platform, arch)
    worker = binary.with_name('ulos-torrent-worker.exe' if platform == 'windows' else 'ulos-torrent-worker')
    if not worker.is_file():
        sys.exit(f'Missing mandatory native torrent worker: {worker}; run scripts/build-torrent-worker.py first')
    verify_binary_architecture(worker, platform, arch)
    with tempfile.TemporaryDirectory(prefix='ulos-package-') as temp:
        stage = Path(temp) / name
        stage.mkdir()
        if platform == 'macos':
            subprocess.run(['sh', 'scripts/bundle-macos.sh', 'release', str(stage / 'Ulos.app')], check=True)
            verify_binary_architecture(stage / 'Ulos.app/Contents/MacOS/Ulos', platform, arch)
            (stage / 'Applications').symlink_to('/Applications')
        else:
            executable = 'Ulos.exe' if platform == 'windows' else 'Ulos'
            shutil.copy2(target / 'release' / ('ulos-desktop.exe' if platform == 'windows' else 'ulos-desktop'), stage / executable)
            shutil.copy2(worker, stage / worker.name)
            if (worker.parent / 'lib').is_dir():
                for dependency in (worker.parent / 'lib').iterdir():
                    verify_binary_architecture(dependency, platform, arch)
                shutil.copytree(worker.parent / 'lib', stage / 'lib')
            if platform == 'linux':
                shutil.copy2(root / 'packaging/linux/org.ulos.desktop.desktop', stage)
                shutil.copy2(root / 'assets/branding/ulos.png', stage / 'Ulos.png')
        for filename in ('LICENSE', 'THIRD_PARTY_NOTICES.md'):
            shutil.copy2(root / filename, stage)
        shutil.copytree(root / 'packaging/licenses', stage / 'licenses')
        shutil.copy2(root / 'packaging/INSTALL.md', stage / 'INSTALL.md')
        if platform == 'macos':
            subprocess.run(['hdiutil', 'create', '-volname', 'Ulos', '-srcfolder', str(stage), '-format', 'UDZO', str(output)], check=True)
            subprocess.run(['hdiutil', 'verify', str(output)], check=True)
        elif platform == 'windows':
            with zipfile.ZipFile(output, 'x', compression=zipfile.ZIP_DEFLATED) as archive:
                for file in sorted(stage.rglob('*')):
                    if file.is_file():
                        archive.write(file, file.relative_to(stage.parent))
        else:
            # x:gz refuses existing files. This is a portable folder, not an AppImage.
            with tarfile.open(output, 'x:gz') as archive:
                archive.add(stage, arcname=name)
    print(output)


if __name__ == '__main__':
    main()
