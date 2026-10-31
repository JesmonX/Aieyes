#!/usr/bin/env python3
"""Offline SSH substitute for Rust native quota integration tests only.

Executes the real bundled payload and routes HTTP solely to the fixture server.
There is deliberately no corresponding switch in the production protocol.
"""
import base64
import json
import os
import pathlib
import shlex
import sys
import urllib.parse

if not os.environ.get('AIEYES_NATIVE_QUOTA_FIXTURE') or 'quota-fixture.invalid' not in sys.argv:
    raise SystemExit('This program is only an offline native quota fixture.')
if os.environ.get('AIEYES_NATIVE_QUOTA_ARGV_LOG'):
    with pathlib.Path(os.environ['AIEYES_NATIVE_QUOTA_ARGV_LOG']).open('a') as stream:
        stream.write(json.dumps(sys.argv) + '\n')
if os.environ.get('AIEYES_NATIVE_QUOTA_REMOTE_ENV'):
    environment = json.loads(pathlib.Path(os.environ['AIEYES_NATIVE_QUOTA_REMOTE_ENV']).read_text())
    for name in ('COPILOT_GITHUB_TOKEN', 'GH_TOKEN', 'GITHUB_TOKEN'):
        os.environ.pop(name, None)
        if name in environment:
            os.environ[name] = environment[name]
raw = sys.stdin.read()
if not raw.lstrip().startswith('{'):
    # Native discovery transports the bounded scanner as stdin and its
    # request as base64 argv. Exercise this real path without opening SSH.
    shell_script = shlex.split(sys.argv[-1])[-1]
    request = json.loads(base64.b64decode(shlex.split(shell_script)[-1]))
    providers = [target.get('provider') for target in request.get('targets', [])] if request.get('action') == 'fingerprints' else [request.get('provider')]
    if not providers or any(provider not in ('copilot', 'pi', 'omp') for provider in providers):
        raise SystemExit('Only Copilot and Pi/OMP discovery belong to this fixture.')
    scope = {'__name__': 'aieyes_copilot_scan_fixture'}
    exec(compile(raw, '<aieyes-copilot-scan-fixture>', 'exec'), scope)
    print(json.dumps(scope['run_request'](request), separators=(',', ':')))
    raise SystemExit(0)
base = os.environ['AIEYES_NATIVE_QUOTA_HTTP']
if not base.startswith('http://127.0.0.1:'):
    raise SystemExit('A loopback fixture is required.')
payload = json.loads(raw)
if 'proxy' in payload['request']:
    raise SystemExit('Local proxy settings must not enter an SSH quota payload.')
scope = {'__name__': 'aieyes_native_quota_fixture', 'SCRIPTS': payload['scripts'], 'CLIENT_VERSION': payload['version']}
exec(compile(payload['script'], '<aieyes-native-quota-fixture>', 'exec'), scope)

def send(spec, token, proxy):
    url = urllib.parse.urlsplit(spec['url'])
    if not scope['allowed_url'](spec['url']):
        raise scope['QuotaError']('endpoint')
    spec = dict(spec, url=base + url.path + ('?' + url.query if url.query else ''))
    return scope['http'](spec, token, {'mode': 'direct'}, url_guard=lambda url: url.startswith(base + '/'))

try:
    result = {'version': 1, 'result': scope['run'](payload['request'], send=send)}
except scope['QuotaError'] as error:
    result = {'version': 1, 'errorCode': str(error)}
except Exception:
    result = {'version': 1, 'errorCode': 'fixture'}
print(json.dumps(result, separators=(',', ':')))
