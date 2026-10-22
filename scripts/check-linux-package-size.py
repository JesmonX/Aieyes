#!/usr/bin/env python3
"""Inspect shipping ELF sections and report Linux download/installed sizes."""
import argparse
import configparser
from email.parser import BytesParser
import hashlib
import io
import json
import pathlib
import re
import shutil
import struct
import subprocess
import tarfile
import tempfile


def elf_payloads(data):
    if data[:6] != b'\x7fELF\x02\x01':
        raise ValueError('Expected a little-endian 64-bit ELF executable')
    offset = struct.unpack_from('<Q', data, 40)[0]
    entry_size, count, names_index = struct.unpack_from('<HHH', data, 58)
    headers = [struct.unpack_from('<IIQQQQIIQQ', data, offset + i * entry_size) for i in range(count)]
    names = headers[names_index]
    strings = data[names[4]:names[4] + names[5]]
    return {strings[h[0]:].split(b'\0', 1)[0].decode(): data[h[4]:h[4]+h[5]]
            for h in headers if h[1] != 8}  # SHT_NOBITS has no file payload.


def verify_binary(data):
    payloads = elf_payloads(data)
    sections = {name:len(value) for name,value in payloads.items()}
    unexpected = {name for name in sections if name in ('.symtab', '.strtab') or name.startswith('.debug_')}
    if unexpected:
        raise ValueError(f'Release executable retains symbols: {sorted(unexpected)}')
    if struct.unpack_from('<H', data, 18)[0] != 62:
        raise ValueError('Expected an x86-64 Linux executable')
    versions = {tuple(map(int, match.split(b'.'))) for match in
                re.findall(rb'GLIBC_([0-9]+\.[0-9]+(?:\.[0-9]+)?)\x00', payloads.get('.dynstr', b''))}
    maximum = max(versions, default=(0, 0))
    if maximum > (2, 35):
        raise ValueError(f'Executable exceeds the Ubuntu 22.04 glibc baseline: {maximum}')
    # Tauri patches only its bundle marker between DEB/RPM/AppImage. linuxdeploy
    # may also change ELF paths/headers; compare program code and constant data.
    program = payloads['.text'] + re.sub(rb'__TAURI_BUNDLE_TYPE_VAR_(DEB|RPM|APP)',
                                        b'__TAURI_BUNDLE_TYPE_VAR_UNK', payloads['.rodata'])
    return {'bytes': len(data), 'sha256':hashlib.sha256(data).hexdigest(),
            'programSha256':hashlib.sha256(program).hexdigest(),
            'maxRequiredGlibc':'.'.join(map(str, maximum)), 'sections': sections}


def deb_files(path, member_prefix='data.tar'):
    data = path.read_bytes()
    if data[:8] != b'!<arch>\n':
        raise ValueError('Invalid Debian archive')
    offset = 8
    while offset + 60 <= len(data):
        header = data[offset:offset + 60]
        size = int(header[48:58])
        name = header[:16].decode().strip().rstrip('/')
        if name.startswith(member_prefix):
            with tarfile.open(fileobj=io.BytesIO(data[offset + 60:offset + 60 + size])) as archive:
                for member in archive:
                    if member.isfile():
                        yield member.name, archive.extractfile(member).read()
            return
        offset += 60 + size + size % 2
    raise ValueError(f'Debian archive has no {member_prefix} payload')


def rpm_files(path):
    data = subprocess.check_output(['rpm2cpio', str(path)])
    offset = 0
    while offset + 110 <= len(data):
        header = data[offset:offset+110]
        if header[:6] not in (b'070701', b'070702'):
            raise ValueError('Unsupported RPM cpio payload')
        fields = [int(header[i:i+8], 16) for i in range(6, 110, 8)]
        mode, size, namesize = fields[1], fields[6], fields[11]
        name = data[offset+110:offset+110+namesize-1].decode()
        start = (offset+110+namesize+3) & ~3
        if name == 'TRAILER!!!': return
        if mode & 0o170000 == 0o100000:
            yield name, data[start:start+size]
        offset = (start+size+3) & ~3
    raise ValueError('Truncated RPM cpio payload')


def appimage_files(path):
    with tempfile.TemporaryDirectory(prefix='aieyes-package-check-') as directory:
        root = pathlib.Path(directory)
        executable = root/'bundle.AppImage'
        shutil.copyfile(path, executable)
        executable.chmod(0o700)
        subprocess.run([str(executable), '--appimage-extract'], cwd=root,
                       stdout=subprocess.DEVNULL, check=True, timeout=120)
        payload = root/'squashfs-root'
        for file in payload.rglob('*'):
            # Desktop/icon symlinks at the AppDir root duplicate usr/share.
            if file.is_file() and not file.is_symlink():
                yield file.relative_to(payload).as_posix(), file.read_bytes()


def inspect_payload(files):
    executables, desktops, icons = {}, {}, {}
    total = 0
    for name, data in files:
        total += len(data)
        basename = name.rsplit('/', 1)[-1]
        if basename in ('aieyes-desktop', 'aieyes-core'):
            if basename in executables: raise ValueError(f'Duplicate executable: {name}')
            executables[basename] = {'path': name, **verify_binary(data)}
            if basename == 'aieyes-desktop':
                # prepare-wakeup.cjs checks the original font hashes before the
                # build. Check their embedded asset keys here; programSha256
                # also compares the actual compressed resources across packages.
                assets = [f'/fonts/HarmonyOS_Sans_SC_{weight}.ttf'
                          for weight in ('Regular', 'Medium', 'Bold')]
                assets += ['/fonts/LICENSE.txt', '/fonts/sources.json']
                missing = [asset for asset in assets if asset.encode() not in data]
                if missing:
                    raise ValueError(f'Missing bundled HarmonyOS font assets: {missing}')
                executables[basename]['bundledFontAssets'] = assets
        elif '/share/applications/' in name and basename.endswith('.desktop'):
            config = configparser.ConfigParser(interpolation=None)
            config.optionxform = str
            config.read_string(data.decode())
            desktops[basename] = dict(config['Desktop Entry'])
        elif '/share/icons/hicolor/' in name and basename == 'aieyes-desktop.png':
            icons[name.split('/share/icons/', 1)[1]] = hashlib.sha256(data).hexdigest()
    if set(executables) != {'aieyes-desktop', 'aieyes-core'}:
        raise ValueError('Desktop executable or standalone core is missing')
    if len(desktops) != 1 or not icons:
        raise ValueError('Desktop launcher or application icons are missing')
    entry = next(iter(desktops.values()))
    for key, expected in {'Name':'Aieyes', 'Exec':'aieyes-desktop', 'Icon':'aieyes-desktop',
                          'StartupWMClass':'aieyes-desktop', 'Type':'Application', 'Terminal':'false'}.items():
        if entry.get(key) != expected:
            raise ValueError(f'Unexpected desktop identity: {key}={entry.get(key)!r}')
    return {'installedFileBytes':total, 'executables':executables, 'desktopEntries':desktops, 'icons':icons}


def inspect(bundle):
    report = {}
    for suffix in ('deb', 'rpm', 'AppImage'):
        packages = list(bundle.rglob('*.' + suffix))
        if len(packages) != 1:
            raise ValueError(f'Expected exactly one {suffix} installer, found {packages}')
        path = packages[0]
        item = {'name': path.name, 'downloadBytes': path.stat().st_size,
                'sha256':hashlib.sha256(path.read_bytes()).hexdigest()}
        if suffix == 'deb':
            control = next(data for name, data in deb_files(path, 'control.tar')
                           if name.rsplit('/', 1)[-1] == 'control')
            metadata = BytesParser().parsebytes(control)
            item['version'] = metadata['Version']
            item['architecture'] = metadata['Architecture']
            item['depends'] = str(metadata.get('Depends', ''))
            if 'appindicator' in item['depends'].lower():
                raise ValueError('Linux StatusNotifier tray must not require AppIndicator packages')
            item.update(inspect_payload(deb_files(path)))
        if suffix == 'rpm':
            item['requires'] = subprocess.check_output(['rpm', '-qp', '--requires', str(path)], text=True).splitlines()
            if any('appindicator' in dependency.lower() for dependency in item['requires']):
                raise ValueError('Linux StatusNotifier tray must not require AppIndicator libraries')
            item['version'], item['architecture'] = subprocess.check_output(
                ['rpm', '-qp', '--queryformat', '%{VERSION}\n%{ARCH}', str(path)], text=True).splitlines()
            item.update(inspect_payload(rpm_files(path)))
        if suffix == 'AppImage':
            item.update(inspect_payload(appimage_files(path)))
        report[suffix] = item
    if report['deb']['version'] != report['rpm']['version']:
        raise ValueError('DEB/RPM versions differ')
    if report['deb']['architecture'] != 'amd64' or report['rpm']['architecture'] != 'x86_64':
        raise ValueError('Installer architecture mismatch')
    for kind, item in report.items():
        for name, executable in item['executables'].items():
            if executable['programSha256'] != report['deb']['executables'][name]['programSha256']:
                raise ValueError(f'{kind} ships different {name} program code or resources')
        for key in ('desktopEntries', 'icons'):
            if item[key] != report['deb'][key]:
                raise ValueError(f'{kind} has inconsistent {key}')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=pathlib.Path, default=pathlib.Path('apps/desktop/src-tauri/target/release/bundle'))
    parser.add_argument('--report', type=pathlib.Path, default=pathlib.Path('.local/linux-packages/sizes.json'))
    args = parser.parse_args()
    report = inspect(args.bundle)
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    for kind, package in report.items():
        print(f'{kind}: {package["downloadBytes"] / 1024**2:.2f} MiB ({package["name"]})')
    print(f'All three payloads: ELF/glibc, bundled fonts/license, identical program/resources, launchers and icons verified; report: {args.report}')
