#!/usr/bin/env python3
"""Validate release versions and assemble the complete, named release asset set."""
import argparse
import base64
import hashlib
import json
import pathlib
import plistlib
import re
import shutil
import tomllib
import xml.etree.ElementTree as ET

ROOT = pathlib.Path(__file__).resolve().parent.parent
PACKAGES = {
    ("macos", "x64"): ("dmg",),
    ("macos", "arm64"): ("dmg",),
    ("windows", "x64"): ("exe",),
    ("linux", "x64"): ("deb", "AppImage"),
}


def version(root=ROOT, tag=None):
    core = tomllib.loads((root / "crates/core/Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    versions = {
        "core": core,
        "desktop": tomllib.loads((root / "apps/desktop/src-tauri/Cargo.toml").read_text(encoding="utf-8"))["package"]["version"],
        "tauri": json.loads((root / "apps/desktop/src-tauri/tauri.conf.json").read_text(encoding="utf-8"))["version"],
        "macOS": plistlib.loads((root / "apps/macos/Info.plist").read_bytes())["CFBundleShortVersionString"],
    }
    for lock in ("Cargo.lock", "apps/desktop/src-tauri/Cargo.lock"):
        for package in tomllib.loads((root / lock).read_text(encoding="utf-8"))["package"]:
            if package["name"] in ("aieyes-core", "aieyes-desktop"):
                versions[f'{lock}:{package["name"]}'] = package["version"]
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", core):
        raise ValueError("Release requires a stable major.minor.patch version")
    if any(value != core for value in versions.values()):
        raise ValueError(f"Versions do not match: {versions}")
    if tag is not None and tag != f"v{core}":
        raise ValueError(f"Tag {tag!r} does not match v{core}")
    return core


def installer_names(release_version):
    return {f"Aieyes-{release_version}-{platform}-{arch}.{ext}"
            for (platform, arch), extensions in PACKAGES.items() for ext in extensions}


def asset_names(release_version):
    packages = installer_names(release_version)
    return packages | {name + '.sig' for name in packages if '-macos-' not in name} | {'appcast-macos-x64.xml', 'appcast-macos-arm64.xml', 'latest.json'}


def update_manifest(directory, release_version):
    platforms = {}
    for target, suffix in [('windows-x86_64-nsis', 'windows-x64.exe'), ('linux-x86_64-appimage', 'linux-x64.AppImage'), ('linux-x86_64-deb', 'linux-x64.deb')]:
        name = f'Aieyes-{release_version}-{suffix}'
        signature = (directory / (name + '.sig')).read_text(encoding="utf-8").strip()
        decoded = base64.b64decode(signature, validate=True).decode('utf-8')
        if not decoded.startswith('untrusted comment:') or f'version:{release_version}' not in decoded:
            raise ValueError(f'Missing signed version in {name}')
        platforms[target] = {'url': f'https://github.com/JesmonX/Aieyes/releases/download/v{release_version}/{name}', 'signature': signature}
    notes_path = ROOT / 'dist/release-notes.txt'
    notes = notes_path.read_text(encoding="utf-8") if notes_path.exists() else f'Aieyes {release_version}：改进与修复。'
    manifest = {'version': release_version, 'notes': notes, 'platforms': platforms}
    (directory / 'latest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    validate_updates(directory, release_version)


def validate_updates(directory, release_version):
    manifest = json.loads((directory / 'latest.json').read_text(encoding="utf-8"))
    if manifest['version'] != release_version:
        raise ValueError('Updater manifest version mismatch')
    targets = {'windows-x86_64-nsis': 'windows-x64.exe', 'linux-x86_64-appimage': 'linux-x64.AppImage', 'linux-x86_64-deb': 'linux-x64.deb'}
    if set(manifest['platforms']) != set(targets):
        raise ValueError('Updater platform set is incomplete')
    for target, suffix in targets.items():
        entry = manifest['platforms'][target]
        name = f'Aieyes-{release_version}-{suffix}'
        if entry['url'] != f'https://github.com/JesmonX/Aieyes/releases/download/v{release_version}/{name}' or entry['signature'] != (directory / (name + '.sig')).read_text(encoding="utf-8").strip():
            raise ValueError('Updater URL/signature mismatch')
    sparkle = '{http://www.andymatuschak.org/xml-namespaces/sparkle}'
    builds = []
    for arch in ('x64', 'arm64'):
        feed = (directory / f'appcast-macos-{arch}.xml').read_text(encoding="utf-8")
        if '<!-- sparkle-signatures:' not in feed:
            raise ValueError('Sparkle feed is not signed')
        item = ET.fromstring(feed).find('./channel/item')
        name = f'Aieyes-{release_version}-macos-{arch}.dmg'
        if item is None or item.findtext(sparkle + 'shortVersionString') != release_version:
            raise ValueError('Sparkle version mismatch')
        builds.append(item.findtext(sparkle + 'version'))
        enclosure = item.find('enclosure')
        if enclosure is None or enclosure.get('url') != f'https://github.com/JesmonX/Aieyes/releases/download/v{release_version}/{name}' or int(enclosure.get('length', '0')) != (directory / name).stat().st_size:
            raise ValueError('Sparkle installer URL/length mismatch')
        if len(base64.b64decode(enclosure.get(sparkle + 'edSignature', ''), validate=True)) != 64:
            raise ValueError('Sparkle installer signature is missing')
    expected_build = plistlib.loads((ROOT / 'apps/macos/Info.plist').read_bytes())['CFBundleVersion']
    if builds != [expected_build, expected_build]:
        raise ValueError('Sparkle build versions do not match the application')


def collect(platform, arch, release_version, destination, root=ROOT):
    destination.mkdir(parents=True, exist_ok=True)
    patterns = {
        "dmg": "dist/*.dmg",
        "exe": "apps/desktop/src-tauri/target/release/bundle/nsis/*.exe",
        "deb": "apps/desktop/src-tauri/target/release/bundle/deb/*.deb",
        "AppImage": "apps/desktop/src-tauri/target/release/bundle/appimage/*.AppImage",
    }
    for ext in PACKAGES[(platform, arch)]:
        files = list(root.glob(patterns[ext]))
        if len(files) != 1 or files[0].stat().st_size == 0:
            raise ValueError(f"Expected one nonempty {ext} package, found {files}")
        shutil.copy2(files[0], destination / f"Aieyes-{release_version}-{platform}-{arch}.{ext}")


def checksum_assets(directory, release_version):
    expected = asset_names(release_version)
    found = {p.name for p in directory.iterdir() if p.name != "SHA256SUMS"}
    if found != expected:
        raise ValueError(f"Incomplete release: missing={expected - found}, unexpected={found - expected}")
    lines = []
    for name in sorted(expected):
        path = directory / name
        if not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"Empty or invalid asset: {name}")
        with path.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        lines.append(f"{digest}  {name}\n")
    validate_updates(directory, release_version)
    (directory / "SHA256SUMS").write_text("".join(lines), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "collect", "updates", "checksums"))
    parser.add_argument("--tag")
    parser.add_argument("--platform", choices=("macos", "windows", "linux"))
    parser.add_argument("--arch", choices=("x64", "arm64"))
    parser.add_argument("--directory", type=pathlib.Path, default=ROOT / "dist/release")
    args = parser.parse_args()
    try:
        release_version = version(tag=args.tag)
        if args.command == "collect":
            if (args.platform, args.arch) not in PACKAGES:
                parser.error("Unsupported platform/architecture")
            collect(args.platform, args.arch, release_version, args.directory)
        elif args.command == "updates":
            update_manifest(args.directory, release_version)
        elif args.command == "checksums":
            checksum_assets(args.directory, release_version)
        print(release_version)
    except (ValueError, OSError) as error:
        parser.exit(1, f"Release validation failed: {error}\n")
