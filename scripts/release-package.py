#!/usr/bin/env python3
"""Create absent release packages. Does not launch or install the application."""
import os
import re
from pathlib import Path
import shutil
import subprocess
import struct
import sys
import tarfile
import tempfile


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


INNO_SETUP_VERSION = '6.7.3'
REQUIRED_LICENSES = (
    'Apache-2.0.txt', 'Boost-1.0.txt', 'OpenSSL-Apache-2.0.txt',
    'libtorrent-2.1.2-NOTICES.txt', 'libtorrent-BSD-3-Clause.txt',
    'nlohmann-json-MIT.txt', 'renamore-MIT.txt', 'Inno-Setup-6.7.3.txt',
)


def build_windows_installer(root, stage, output, arch, version):
    """Invoke real ISCC; tests may mock the runner, never a ZIP fallback."""
    compiler = os.environ.get('ULOS_ISCC') or shutil.which('ISCC.exe') or shutil.which('iscc')
    if not compiler or not Path(compiler).is_file():
        raise RuntimeError('Inno Setup 6.7.3 ISCC.exe required; set ULOS_ISCC or run scripts/install-inno-setup.ps1')
    for filename in ('Ulos.exe', 'ulos-torrent-worker.exe', 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'INSTALL.md'):
        if not (stage / filename).is_file() or (stage / filename).stat().st_size == 0:
            raise ValueError(f'Missing/empty installer payload: {filename}')
    for filename in REQUIRED_LICENSES:
        if not (stage / 'licenses' / filename).is_file() or (stage / 'licenses' / filename).stat().st_size == 0:
            raise ValueError(f'Missing/empty required license: {filename}')
    # Plain text UTF-8 BOM: readable both in the stock wizard and Windows Notepad.
    sections = ['Ulos third-party software notices',
                'Ulos is MIT licensed. Dependencies retain their own licenses.\n'
                'These supplied notices are not a full transitive compliance audit.']
    for file in [stage / 'THIRD_PARTY_NOTICES.md', *sorted((stage / 'licenses').glob('*.txt'))]:
        sections.append(f'===== {file.name} =====\n{file.read_text(encoding="utf-8-sig")}')
    (stage / 'OSS-NOTICES.txt').write_text('\n\n'.join(sections), encoding='utf-8-sig')
    # Compile privately so failure cannot leave a release-shaped partial artifact.
    with tempfile.TemporaryDirectory(prefix='ulos-iscc-') as temp:
        built = Path(temp) / output.name
        subprocess.run([
            str(compiler), '/Qp', f'/DAppVersion={version}', f'/DPayloadArch={arch}',
            f'/DStageDir={stage.resolve()}', f'/DBuildOutputDir={temp}',
            f'/DOutputName={output.stem}', str(root / 'packaging/windows/ulos.iss'),
        ], check=True)
        if not built.is_file() or built.is_symlink() or built.stat().st_size < 64:
            raise RuntimeError('ISCC succeeded without producing the expected Setup.exe')
        with built.open('rb') as source:
            if source.read(2) != b'MZ':
                raise RuntimeError('ISCC output is not a Windows executable')
            source.seek(0)
            # Exclusive creation also refuses an output raced in after preflight.
            with output.open('xb') as destination:
                try:
                    shutil.copyfileobj(source, destination)
                except BaseException:
                    output.unlink()
                    raise


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if len(argv) != 3:
        sys.exit('Usage: release-package.py {macos|windows|linux} {arm64|x86_64} VERSION')
    platform, arch, version = argv
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?", version):
        sys.exit('Unsupported release version')
    root = Path(__file__).resolve().parent.parent
    os.chdir(root)
    actual = subprocess.check_output([sys.executable, 'scripts/release-version.py', version], text=True).strip()
    assert actual == version
    if (platform, arch) not in {(system, cpu) for system in ('macos', 'windows', 'linux') for cpu in ('arm64', 'x86_64')}:
        sys.exit('Unsupported platform/architecture')
    name = f'Ulos-{version}-{platform}-{arch}'
    dist = root / 'dist'
    dist.mkdir(exist_ok=True)
    extension = {'macos': '.dmg', 'windows': '-Setup.exe', 'linux': '.tar.gz'}[platform]
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
            build_windows_installer(root, stage, output, arch, version)
        else:
            # x:gz refuses existing files. This is a portable folder, not an AppImage.
            with tarfile.open(output, 'x:gz') as archive:
                archive.add(stage, arcname=name)
    print(output)


if __name__ == '__main__':
    main()
