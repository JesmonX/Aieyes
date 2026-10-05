#!/usr/bin/env python3
"""Sign final renamed installers, then verify against the embedded public key."""
import argparse
import os
import pathlib
import subprocess
import shutil
import release

parser = argparse.ArgumentParser()
parser.add_argument('--platform', choices=['windows', 'linux'], required=True)
parser.add_argument('--directory', type=pathlib.Path, default=release.ROOT/'dist/release')
args = parser.parse_args()
if not os.environ.get('AIEYES_UPDATER_PUBLIC_KEY') or not (os.environ.get('TAURI_SIGNING_PRIVATE_KEY') or os.environ.get('TAURI_SIGNING_PRIVATE_KEY_PATH')):
    parser.exit(1, 'Desktop release update signing keys are required\n')
cli = shutil.which('tauri')
if not cli:
    parser.exit(1, 'Install Tauri CLI 2.12.1 before signing\n')
version = release.version()
for ext in release.PACKAGES[(args.platform, 'x64')]:
    package = args.directory / f'Aieyes-{version}-{args.platform}-x64.{ext}'
    # Capture output so the private key can never be echoed by a failing command.
    result = subprocess.run([cli, 'signer', 'sign', '--app-version', version, str(package)], capture_output=True)
    if result.returncode:
        parser.exit(1, f'Could not sign {package.name}\n')
    subprocess.run(['cargo', 'run', '--locked', '--manifest-path', str(release.ROOT/'apps/desktop/src-tauri/Cargo.toml'), '--features', 'release-tools', '--bin', 'verify-update', '--', str(package), version], check=True)
