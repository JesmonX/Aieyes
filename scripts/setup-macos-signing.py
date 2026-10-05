#!/usr/bin/env python3
"""Optional CI-only Developer ID keychain. Never required for ad-hoc releases."""
import base64
import json
import os
import pathlib
import secrets
import shlex
import subprocess
import sys


def run(*args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout.strip()


if os.environ.get('GITHUB_ACTIONS') != 'true':
    sys.exit('This helper only configures ephemeral GitHub Actions runners')
state = pathlib.Path(os.environ['RUNNER_TEMP']) / 'aieyes-signing'
keychain = state.with_suffix('.keychain-db')
if '--cleanup' in sys.argv:
    if state.exists():
        previous = json.loads(state.read_text())
        run('security', 'default-keychain', '-s', previous['default'])
        run('security', 'list-keychains', '-d', 'user', '-s', *previous['search'])
        run('security', 'delete-keychain', str(keychain))
        state.unlink()
    sys.exit(0)
certificate = os.environ.get('MACOS_CERTIFICATE_P12')
identity = os.environ.get('MACOS_SIGNING_IDENTITY')
if not certificate and not identity:
    print('Using ad-hoc signing; Apple notarization is not configured')
    sys.exit(0)
if not all(os.environ.get(key) for key in ['MACOS_CERTIFICATE_P12', 'MACOS_SIGNING_IDENTITY', 'APPLE_ID', 'APPLE_TEAM_ID', 'APPLE_APP_PASSWORD']):
    sys.exit('Developer ID signing requires certificate, identity and notarization credentials')
password = secrets.token_urlsafe(32)
p12 = state.with_suffix('.p12')
try:
    previous = {
        'default': run('security', 'default-keychain').strip('"'),
        'search': shlex.split(run('security', 'list-keychains', '-d', 'user')),
    }
    run('security', 'create-keychain', '-p', password, str(keychain))
    state.write_text(json.dumps(previous))
    run('security', 'set-keychain-settings', '-lut', '21600', str(keychain))
    run('security', 'unlock-keychain', '-p', password, str(keychain))
    run('security', 'default-keychain', '-s', str(keychain))
    run('security', 'list-keychains', '-d', 'user', '-s', str(keychain), *previous['search'])
    p12.write_bytes(base64.b64decode(certificate, validate=True)); p12.chmod(0o600)
    run('security', 'import', str(p12), '-P', os.environ.get('MACOS_CERTIFICATE_PASSWORD', ''), '-k', str(keychain), '-T', '/usr/bin/codesign')
    run('security', 'set-key-partition-list', '-S', 'apple-tool:,apple:', '-s', '-k', password, str(keychain))
    run('xcrun', 'notarytool', 'store-credentials', 'aieyes-ci', '--apple-id', os.environ['APPLE_ID'], '--team-id', os.environ['APPLE_TEAM_ID'], '--password', os.environ['APPLE_APP_PASSWORD'])
    with open(os.environ['GITHUB_ENV'], 'a') as env:
        env.write(f'MACOS_SIGNING_IDENTITY={identity}\nMACOS_NOTARY_PROFILE=aieyes-ci\n')
except Exception:
    sys.exit('Developer ID setup failed; verify CI signing credentials')
finally:
    p12.unlink(missing_ok=True)
