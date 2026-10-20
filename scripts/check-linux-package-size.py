#!/usr/bin/env python3
"""Inspect shipping ELF sections and report Linux download/installed sizes."""
import argparse
import io
import json
import pathlib
import struct
import tarfile


def elf_sections(data):
    if data[:6] != b'\x7fELF\x02\x01':
        raise ValueError('Expected a little-endian 64-bit ELF executable')
    offset = struct.unpack_from('<Q', data, 40)[0]
    entry_size, count, names_index = struct.unpack_from('<HHH', data, 58)
    headers = [struct.unpack_from('<IIQQQQIIQQ', data, offset + i * entry_size) for i in range(count)]
    names = headers[names_index]
    strings = data[names[4]:names[4] + names[5]]
    return {strings[h[0]:].split(b'\0', 1)[0].decode(): h[5] for h in headers}


def verify_binary(data):
    sections = elf_sections(data)
    unexpected = {name for name in sections if name in ('.symtab', '.strtab') or name.startswith('.debug_')}
    if unexpected:
        raise ValueError(f'Release executable retains symbols: {sorted(unexpected)}')
    if b'HarmonyOS_Sans_SC_' in data:
        raise ValueError('Linux executable embeds Windows-only font assets')
    return {'bytes': len(data), 'sections': sections}


def deb_files(path):
    data = path.read_bytes()
    if data[:8] != b'!<arch>\n':
        raise ValueError('Invalid Debian archive')
    offset = 8
    while offset + 60 <= len(data):
        header = data[offset:offset + 60]
        size = int(header[48:58])
        name = header[:16].decode().strip().rstrip('/')
        if name.startswith('data.tar'):
            with tarfile.open(fileobj=io.BytesIO(data[offset + 60:offset + 60 + size])) as archive:
                for member in archive:
                    if member.isfile():
                        yield member.name, archive.extractfile(member).read()
            return
        offset += 60 + size + size % 2
    raise ValueError('Debian archive has no file payload')


def inspect(bundle):
    report = {}
    for suffix in ('deb', 'rpm', 'AppImage'):
        packages = list(bundle.rglob('*.' + suffix))
        if len(packages) != 1:
            raise ValueError(f'Expected exactly one {suffix} installer, found {packages}')
        path = packages[0]
        item = {'name': path.name, 'downloadBytes': path.stat().st_size}
        if suffix == 'deb':
            files = list(deb_files(path))
            item['installedFileBytes'] = sum(len(data) for _, data in files)
            item['executables'] = {name: verify_binary(data) for name, data in files
                                   if name.rsplit('/', 1)[-1] in ('aieyes-desktop', 'aieyes-core')}
            if len(item['executables']) != 2:
                raise ValueError('Desktop executable or standalone core is missing')
        report[suffix] = item
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
    print(f'ELF symbols and Linux font exclusion verified; report: {args.report}')
