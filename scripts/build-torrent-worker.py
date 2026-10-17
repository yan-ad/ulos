#!/usr/bin/env python3
"""Build the native BEP 52 worker and relocate its runtime dependencies.

No dependency installation unless --install-deps is explicitly selected. Windows
uses static libraries/CRT; Unix bundles non-OS libraries beside the executable.
"""
import argparse
import importlib.util
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
# Verified microsoft/vcpkg master commit: libtorrent 2.1.2. No WebRTC defaults.
VCPKG_BASELINE = '3cd21e48a75f67acfef289da52893ce79144aaa1'
spec = importlib.util.spec_from_file_location('release_package', ROOT / 'scripts/release-package.py')
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


def run(*args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def text(*args):
    return subprocess.check_output([str(arg) for arg in args], text=True)


def native_identity():
    system = {'Darwin': 'macos', 'Linux': 'linux', 'Windows': 'windows'}[platform.system()]
    arch = {'aarch64': 'arm64', 'arm64': 'arm64', 'amd64': 'x86_64', 'x86_64': 'x86_64'}.get(platform.machine().lower())
    if not arch:
        raise ValueError('Only native ARM64 and x86-64 builds are supported')
    target = os.environ.get('CARGO_BUILD_TARGET', '')
    expected = {'macos': 'apple-darwin', 'linux': 'unknown-linux-gnu', 'windows': 'pc-windows-msvc'}[system]
    cpu = 'aarch64' if arch == 'arm64' else 'x86_64'
    if target and target != f'{cpu}-{expected}':
        raise ValueError(f'CARGO_BUILD_TARGET={target} is not the native {system}/{arch} host')
    return system, arch


def mac_dependencies(binary):
    return [line.strip().split(' (compatibility version', 1)[0]
            for line in text('otool', '-L', binary).splitlines()[1:]]


def mac_system(path):
    return path.startswith(('/usr/lib/', '/System/Library/'))


def bundle_macos(worker, libdir, arch):
    """Walk original load commands before rewriting copies; fail on ambiguity."""
    pending = [(worker, worker)]
    seen = {}
    while pending:
        source, destination = pending.pop()
        for dependency in mac_dependencies(source):
            if mac_system(dependency):
                continue
            if dependency.startswith('@loader_path/'):
                resolved = source.parent / dependency[len('@loader_path/'):]
            elif dependency.startswith('/'):
                resolved = Path(dependency)
            else:
                raise ValueError(f'Unresolved Mach-O dependency {dependency} in {source}')
            resolved = resolved.resolve(strict=True)
            # A dylib's own install ID is also printed by otool -L.
            if resolved == source.resolve():
                continue
            name = Path(dependency).name
            if name in seen and seen[name] != resolved:
                raise ValueError(f'Conflicting runtime library basename: {name}')
            if name not in seen:
                seen[name] = resolved
                libdir.mkdir(exist_ok=True)
                copied = libdir / name
                shutil.copy2(resolved, copied)
                copied.chmod(copied.stat().st_mode | 0o200)
                package.verify_binary_architecture(copied, 'macos', arch)
                pending.append((resolved, copied))
            replacement = ('@loader_path/lib/' if destination == worker else '@loader_path/') + name
            run('install_name_tool', '-change', dependency, replacement, destination)
        if destination != worker:
            run('install_name_tool', '-id', '@loader_path/' + destination.name, destination)
    for binary in [*sorted(libdir.glob('*')), worker]:
        run('codesign', '--force', '--sign', '-', binary)
        run('codesign', '--verify', '--strict', binary)


# Keep the platform ABI runtime on the supported Linux host (Ubuntu 24.04+).
LINUX_SYSTEM = re.compile(r'^(?:linux-vdso|ld-linux|lib(?:c|m|pthread|dl|rt|resolv|util|stdc\+\+|gcc_s)\.so)')


def linux_dependencies(binary):
    result = []
    for line in text('ldd', binary).splitlines():
        if 'not found' in line:
            raise ValueError(f'Unresolved ELF dependency: {line.strip()}')
        match = re.match(r'\s*(\S+)\s+=>\s+(/\S+)', line)
        if match and not LINUX_SYSTEM.match(match[1]):
            result.append((match[1], Path(match[2])))
    return result


def bundle_linux(worker, libdir, arch):
    if not shutil.which('patchelf'):
        raise ValueError('patchelf is required to bundle Linux worker libraries (install patchelf)')
    pending = [worker]
    seen = {}
    while pending:
        source = pending.pop()
        for name, dependency in linux_dependencies(source):
            resolved = dependency.resolve(strict=True)
            if name in seen:
                if seen[name] != resolved:
                    raise ValueError(f'Conflicting runtime library: {name}')
                continue
            seen[name] = resolved
            libdir.mkdir(exist_ok=True)
            copied = libdir / name
            shutil.copy2(resolved, copied)
            copied.chmod(copied.stat().st_mode | 0o200)
            package.verify_binary_architecture(copied, 'linux', arch)
            pending.append(resolved)
    for binary in libdir.glob('*'):
        run('patchelf', '--set-rpath', '$ORIGIN', binary)
    run('patchelf', '--set-rpath', '$ORIGIN/lib', worker)


def verify_windows_static(worker):
    """Fail packaging if the static build accidentally acquired third-party DLLs."""
    listing = text('dumpbin', '/DEPENDENTS', worker)
    for name in re.findall(r'^\s+([^\s]+\.dll)\s*$', listing, re.MULTILINE | re.IGNORECASE):
        lower = name.lower()
        if lower.startswith(('api-ms-win-', 'ext-ms-win-')):
            continue
        if lower not in {'kernel32.dll', 'user32.dll', 'advapi32.dll', 'ws2_32.dll',
                         'mswsock.dll', 'iphlpapi.dll', 'bcrypt.dll', 'crypt32.dll',
                         'secur32.dll', 'shell32.dll', 'ole32.dll', 'ntdll.dll',
                         'shlwapi.dll', 'gdi32.dll', 'normaliz.dll', 'userenv.dll',
                         # Windows Debug Help is an OS component used by
                         # libtorrent's Debug assertion/backtrace support.
                         'dbghelp.dll'}:
            raise ValueError(f'Unexpected DLL dependency in static worker: {name}')


def install_dependencies(system, arch, target):
    if system == 'macos':
        run('brew', 'install', 'cmake', 'libtorrent-rasterbar', 'nlohmann-json')
    elif system == 'linux':
        run('sudo', 'apt-get', 'update')
        run('sudo', 'apt-get', 'install', '-y', 'cmake', 'g++', 'pkg-config', 'libtorrent-rasterbar-dev', 'nlohmann-json3-dev', 'patchelf')
    else:
        root = target / 'native-deps' / VCPKG_BASELINE
        if not root.exists():
            root.parent.mkdir(parents=True, exist_ok=True)
            run('git', 'clone', '--no-checkout', 'https://github.com/microsoft/vcpkg.git', root)
            run('git', '-C', root, 'checkout', '--detach', VCPKG_BASELINE)
        if text('git', '-C', root, 'rev-parse', 'HEAD').strip() != VCPKG_BASELINE:
            raise ValueError('vcpkg checkout does not match pinned baseline')
        run('cmd', '/c', str(root / 'bootstrap-vcpkg.bat'), '-disableMetrics')
        triplet = ('arm64' if arch == 'arm64' else 'x64') + '-windows-static'
        env = dict(os.environ, VCPKG_FEATURE_FLAGS='versions')
        # deprfun preserves ABI 2 across libtorrent 2.0/2.1; core disables
        # default WebTorrent/libdatachannel dependencies (not BitTorrent v2).
        run(root / 'vcpkg.exe', 'install', f'libtorrent[core,deprfun]:{triplet}', f'nlohmann-json:{triplet}', env=env)
        return root / 'scripts/buildsystems/vcpkg.cmake'
    return None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--profile', choices=('debug', 'release'), default='release')
    parser.add_argument('--test', action='store_true', help='Run CTest and relocated worker loopback tests')
    parser.add_argument('--output', type=Path, help='Directory containing the desktop executable')
    parser.add_argument('--cmake-prefix', help='CMake package prefix path')
    parser.add_argument('--toolchain', type=Path)
    parser.add_argument('--install-deps', action='store_true')
    args = parser.parse_args(argv)
    system, arch = native_identity()
    target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT / 'target')).resolve()
    toolchain = args.toolchain
    if args.install_deps:
        toolchain = install_dependencies(system, arch, target) or toolchain
    output = args.output or target / os.environ.get('CARGO_BUILD_TARGET', '') / args.profile
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    config = args.profile.capitalize()
    build = target / 'torrent-worker-build' / f'{system}-{arch}-{args.profile}'
    command = ['cmake', '-S', str(ROOT / 'native/torrent-worker'), '-B', str(build),
               f'-DCMAKE_BUILD_TYPE={config}', '-DBUILD_TESTING=ON',
               f'-DPython3_EXECUTABLE={sys.executable}']
    if system == 'macos':
        command += [f'-DCMAKE_OSX_ARCHITECTURES={"arm64" if arch == "arm64" else "x86_64"}']
        if not args.cmake_prefix and shutil.which('brew'):
            command += ['-DCMAKE_PREFIX_PATH=' + text('brew', '--prefix').strip()]
    if system == 'windows':
        command += ['-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded$<$<CONFIG:Debug>:Debug>',
                    '-DVCPKG_TARGET_TRIPLET=' + ('arm64' if arch == 'arm64' else 'x64') + '-windows-static']
    if toolchain:
        command += [f'-DCMAKE_TOOLCHAIN_FILE={toolchain.resolve()}']
    if args.cmake_prefix:
        command += [f'-DCMAKE_PREFIX_PATH={args.cmake_prefix}']
    run(*command)
    run('cmake', '--build', build, '--config', config, '--parallel', str(min(os.cpu_count() or 2, 8)))
    if args.test:
        run('ctest', '--test-dir', build, '-C', config, '--output-on-failure', '--no-tests=error')
    suffix = '.exe' if system == 'windows' else ''
    source = build / ('ulos-torrent-worker' + suffix)
    if not source.exists():
        source = build / config / source.name
    package.verify_binary_architecture(source, system, arch)
    # Build in a fresh staging directory to avoid mixing stale dependency sets.
    with tempfile.TemporaryDirectory(prefix='ulos-worker-') as temp:
        stage = Path(temp)
        worker = stage / source.name
        shutil.copy2(source, worker)
        libdir = stage / 'lib'
        if system == 'macos':
            bundle_macos(worker, libdir, arch)
        elif system == 'linux':
            bundle_linux(worker, libdir, arch)
        else:
            verify_windows_static(worker)
        if args.test:
            fixture = source.with_name('ulos-torrent-fixture' + suffix)
            run(sys.executable, ROOT / 'native/torrent-worker/tests/loopback.py', worker, fixture, timeout=180)
        shutil.copy2(worker, output / worker.name)
        # lib/ is reserved for this sidecar's managed dependency closure.
        if (output / 'lib').exists():
            shutil.rmtree(output / 'lib')
        if libdir.exists():
            shutil.copytree(libdir, output / 'lib')
    print(f'Built native {system}/{arch} worker: {output / source.name}')
    if args.test and os.environ.get('GITHUB_ENV'):
        fixture = source.with_name('ulos-torrent-fixture' + suffix)
        with open(os.environ['GITHUB_ENV'], 'a', encoding='utf-8') as env:
            env.write('ULOS_TORRENT_WORKER=' + str((output / source.name).resolve()) + '\n')
            env.write('ULOS_TORRENT_FIXTURE=' + str(fixture.resolve()) + '\n')


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f'Worker build failed: {error}')
