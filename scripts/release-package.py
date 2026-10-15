#!/usr/bin/env python3
"""Create absent release archives. Does not launch or install the application."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import zipfile

platform, arch, version = sys.argv[1:]
root = Path(__file__).resolve().parent.parent
os.chdir(root)
actual = subprocess.check_output([sys.executable, 'scripts/release-version.py', version], text=True).strip()
assert actual == version
if (platform, arch) not in {('macos', 'arm64'), ('macos', 'x86_64'), ('windows', 'x86_64'), ('linux', 'x86_64')}:
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
with tempfile.TemporaryDirectory(prefix='ulos-package-') as temp:
    stage = Path(temp) / name
    stage.mkdir()
    if platform == 'macos':
        subprocess.run(['sh', 'scripts/bundle-macos.sh', 'release', str(stage / 'Ulos.app')], check=True)
        (stage / 'Applications').symlink_to('/Applications')
    else:
        executable = 'Ulos.exe' if platform == 'windows' else 'Ulos'
        shutil.copy2(target / 'release' / ('ulos-desktop.exe' if platform == 'windows' else 'ulos-desktop'), stage / executable)
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
