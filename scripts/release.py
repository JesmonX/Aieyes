#!/usr/bin/env python3
"""Validate release versions and assemble the complete, named release asset set."""
import argparse
import hashlib
import json
import pathlib
import plistlib
import re
import shutil
import tomllib

ROOT = pathlib.Path(__file__).resolve().parent.parent
PACKAGES = {
    ("macos", "x64"): ("dmg",),
    ("macos", "arm64"): ("dmg",),
    ("windows", "x64"): ("exe",),
    ("linux", "x64"): ("deb", "AppImage"),
}


def version(root=ROOT, tag=None):
    core = tomllib.loads((root / "crates/core/Cargo.toml").read_text())["package"]["version"]
    versions = {
        "core": core,
        "desktop": tomllib.loads((root / "apps/desktop/src-tauri/Cargo.toml").read_text())["package"]["version"],
        "tauri": json.loads((root / "apps/desktop/src-tauri/tauri.conf.json").read_text())["version"],
        "macOS": plistlib.loads((root / "apps/macos/Info.plist").read_bytes())["CFBundleShortVersionString"],
    }
    for lock in ("Cargo.lock", "apps/desktop/src-tauri/Cargo.lock"):
        for package in tomllib.loads((root / lock).read_text())["package"]:
            if package["name"] in ("aieyes-core", "aieyes-desktop"):
                versions[f'{lock}:{package["name"]}'] = package["version"]
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", core):
        raise ValueError("Release requires a stable major.minor.patch version")
    if any(value != core for value in versions.values()):
        raise ValueError(f"Versions do not match: {versions}")
    if tag is not None and tag != f"v{core}":
        raise ValueError(f"Tag {tag!r} does not match v{core}")
    return core


def asset_names(release_version):
    return {f"Aieyes-{release_version}-{platform}-{arch}.{ext}"
            for (platform, arch), extensions in PACKAGES.items() for ext in extensions}


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
    (directory / "SHA256SUMS").write_text("".join(lines), encoding="utf-8")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("check", "collect", "checksums"))
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
        elif args.command == "checksums":
            checksum_assets(args.directory, release_version)
        print(release_version)
    except (ValueError, OSError) as error:
        parser.exit(1, f"Release validation failed: {error}\n")
