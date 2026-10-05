#!/usr/bin/env python3
"""Exercise real update signatures with disposable keys, never the release signing identity."""
import base64
import importlib.util
import os
import pathlib
import platform
import shutil
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLI = os.environ.get('AIEYES_TAURI_CLI') or shutil.which('tauri')
if not CLI:
    raise SystemExit('Install the pinned Tauri CLI before testing release signatures')


def run(args, *, env=None, input=None, succeeds=True):
    result = subprocess.run([str(a) for a in args], cwd=ROOT, env=env, input=input, capture_output=True, text=True)
    if (result.returncode == 0) != succeeds:
        raise AssertionError(f'Unexpected signature check result from {pathlib.Path(str(args[0])).name}')
    return result.stdout


subprocess.run(['cargo', 'build', '--locked', '--manifest-path', str(ROOT/'apps/desktop/src-tauri/Cargo.toml'), '--features', 'release-tools', '--bin', 'verify-update'], cwd=ROOT, check=True)
verifier = ROOT / ('apps/desktop/src-tauri/target/debug/verify-update.exe' if os.name == 'nt' else 'apps/desktop/src-tauri/target/debug/verify-update')
with tempfile.TemporaryDirectory(prefix='aieyes-signature-test-') as temporary:
    directory = pathlib.Path(temporary)
    private = directory/'tauri.key'
    run([CLI, 'signer', 'generate', '--ci', '--password', '', '--write-keys', private])
    env = dict(os.environ, TAURI_SIGNING_PRIVATE_KEY=private.read_text().strip(), TAURI_SIGNING_PRIVATE_KEY_PASSWORD='', AIEYES_UPDATER_PUBLIC_KEY=private.with_suffix('.key.pub').read_text().strip())
    package = directory/'fixture.AppImage'
    package.write_bytes(b'fixture update installer')
    run([CLI, 'signer', 'sign', '--app-version', '1.2.3', package], env=env)
    run([verifier, package, '1.2.3'], env=env)
    run([verifier, package, '1.2.4'], env=env, succeeds=False)
    package.write_bytes(b'tampered installer')
    run([verifier, package, '1.2.3'], env=env, succeeds=False)
    package.write_bytes(b'fixture update installer')
    other = directory/'other.key'
    run([CLI, 'signer', 'generate', '--ci', '--password', '', '--write-keys', other])
    run([verifier, package, '1.2.3'], env=dict(env, AIEYES_UPDATER_PUBLIC_KEY=other.with_suffix('.key.pub').read_text().strip()), succeeds=False)
    print('Tauri: valid signature accepted; tampering, wrong version and wrong key rejected')
    if platform.system() == 'Darwin':
        sign = ROOT/'.build/sparkle-tools/bin/sign_update'
        seed = base64.b64encode(os.urandom(32)).decode()
        swift = 'import Foundation; import CryptoKit; let key = try Curve25519.Signing.PrivateKey(rawRepresentation: Data(base64Encoded: readLine()!)!); print(key.publicKey.rawRepresentation.base64EncodedString())'
        public_key = run(['swift', '-e', swift], input=seed).strip()
        run(['swift', ROOT/'scripts/verify-sparkle-key.swift'], input=seed+'\n'+public_key+'\n')
        spec = importlib.util.spec_from_file_location('signmac', ROOT/'scripts/sign-macos-update.py')
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        dmg = directory/'Aieyes-1.2.3-macos-x64.dmg'; dmg.write_bytes(b'fixture mac installer')
        module.create_feed(directory, '1.2.3', 'x64', '99', sign, seed, public_key, 'Test release')
        feed = directory/'appcast-macos-x64.xml'
        signature = run([sign, '--ed-key-file', '-', '-p', dmg], input=seed).strip()
        run([sign, '--ed-key-file', '-', '--verify', dmg, signature], input=seed)
        dmg.write_bytes(b'tampered mac installer')
        run([sign, '--ed-key-file', '-', '--verify', dmg, signature], input=seed, succeeds=False)
        feed.write_text(feed.read_text().replace('Test release', 'Injected release'))
        run([sign, '--ed-key-file', '-', '--verify', feed], input=seed, succeeds=False)
        print('Sparkle: valid signed package/feed accepted; altered package and feed rejected')
