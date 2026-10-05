#!/usr/bin/env python3
"""Embed Sparkle and sign the complete bundle; optionally notarize distribution artifacts."""
import argparse
import base64
import os
import pathlib
import plistlib
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent


def run(*args):
    subprocess.run([str(a) for a in args], check=True)


def bundle(app, mode):
    framework = ROOT / '.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64/Sparkle.framework'
    target = app / 'Contents/Frameworks/Sparkle.framework'
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists():
        shutil.rmtree(target)
    run('ditto', framework, target)
    plist_path = app / 'Contents/Info.plist'
    info = plistlib.loads(plist_path.read_bytes())
    key = os.environ.get('SPARKLE_PUBLIC_KEY', '').strip()
    if os.environ.get('AIEYES_REQUIRE_UPDATER') == '1' and not key:
        raise ValueError('SPARKLE_PUBLIC_KEY is required for release updates')
    if key:
        if len(base64.b64decode(key, validate=True)) != 32:
            raise ValueError('SPARKLE_PUBLIC_KEY must be a base64 Ed25519 public key')
        info['SUPublicEDKey'] = key
    import platform
    arch = 'arm64' if platform.machine() == 'arm64' else 'x64'
    info['SUFeedURL'] = f'https://github.com/JesmonX/Aieyes/releases/latest/download/appcast-macos-{arch}.xml'
    info['SUEnableAutomaticChecks'] = False
    info['SUAllowsAutomaticUpdates'] = False
    info['SUVerifyUpdateBeforeExtraction'] = True
    info['SURequireSignedFeed'] = True
    plist_path.write_bytes(plistlib.dumps(info))
    identity = os.environ.get('MACOS_SIGNING_IDENTITY') or '-'
    flags = ['--options', 'runtime', '--timestamp'] if identity != '-' else []
    # Sign nested code from the inside out; never use --deep to paper over missing signatures.
    nested = [target / 'Versions/B/Autoupdate', *sorted((target / 'Versions/B/XPCServices').glob('*.xpc')), target / 'Versions/B/Updater.app', target, app / 'Contents/Resources/aieyes-core', app]
    for path in nested:
        run('codesign', '--force', '--sign', identity, *flags, path)
    run('codesign', '--verify', '--deep', '--strict', app)
    if mode == 'release' and identity != '-':
        profile = os.environ.get('MACOS_NOTARY_PROFILE')
        if not profile:
            raise ValueError('A Developer ID release also requires MACOS_NOTARY_PROFILE')
        with tempfile.TemporaryDirectory(prefix='aieyes-notary-') as temporary:
            archive = pathlib.Path(temporary) / 'Aieyes.zip'
            run('ditto', '-c', '-k', '--keepParent', app, archive)
            run('xcrun', 'notarytool', 'submit', archive, '--keychain-profile', profile, '--wait')
        run('xcrun', 'stapler', 'staple', app)
        run('xcrun', 'stapler', 'validate', app)
        run('spctl', '--assess', '--type', 'execute', '--verbose=2', app)


def notarize_dmg(path):
    identity = os.environ.get('MACOS_SIGNING_IDENTITY')
    if not identity:
        return
    run('codesign', '--force', '--sign', identity, '--timestamp', path)
    run('xcrun', 'notarytool', 'submit', path, '--keychain-profile', os.environ['MACOS_NOTARY_PROFILE'], '--wait')
    run('xcrun', 'stapler', 'staple', path)
    run('xcrun', 'stapler', 'validate', path)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('command', choices=['bundle', 'dmg'])
    parser.add_argument('path', type=pathlib.Path)
    parser.add_argument('--mode', choices=['debug', 'release'], default='debug')
    args = parser.parse_args()
    try:
        if args.command == 'bundle':
            bundle(args.path, args.mode)
        else:
            notarize_dmg(args.path)
    except (ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'Distribution failed: {error}\n')
