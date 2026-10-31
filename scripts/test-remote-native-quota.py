#!/usr/bin/env python3
"""Offline native SSH quota fixtures. No real credential, login or quota calls."""
import contextlib
import http.server
import importlib.util
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('remote_native_quota', HERE / 'remote_native_quota.py')
remote = importlib.util.module_from_spec(spec)
spec.loader.exec_module(remote)
TOKEN = 'native-quota-fixture-access-DO-NOT-EXPORT'
REFRESH = 'native-quota-fixture-refresh-DO-NOT-EXPORT'


@contextlib.contextmanager
def fixture_server(status=200, value=None, raw=None, extra=None, change=None):
    seen = []
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
            seen.append({'method': self.command, 'path': self.path, 'headers': dict(self.headers), 'body': body})
            if change:
                change()
            output = raw if raw is not None else json.dumps(value if value is not None else {'balance': 12.5}).encode()
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(output)))
            for name, content in (extra or {}).items():
                self.send_header(name, content)
            self.end_headers()
            with contextlib.suppress(BrokenPipeError, ConnectionResetError):
                self.wfile.write(output)
        do_POST = do_GET
        def log_message(self, *_):
            pass
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield 'http://127.0.0.1:%s' % server.server_port, seen
    finally:
        server.shutdown()
        thread.join()
        server.server_close()


class NativeQuotaTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='aieyes-remote-native-quota-')
        self.root = pathlib.Path(self.temporary.name)
    def tearDown(self):
        self.temporary.cleanup()
    def ref(self, provider='opencode', selector='/opencode-go', kind='api', name='auth.json', value=None):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        if value is not None:
            path.write_text(json.dumps(value) if not isinstance(value, str) else value)
        return {'provider': provider, 'selector': selector, 'kind': kind, 'root': str(self.root), 'configPath': str(path)}
    def go(self):
        return self.ref(value={'opencode-go': {'type': 'api', 'key': TOKEN, 'unknownSecret': REFRESH}})
    def request(self, ref, url, headers=None, method='GET', body=None):
        inspected = remote.run({'reference': ref})
        return {'reference': ref, 'action': 'http', 'pin': inspected['pin'], 'proxy': {'mode': 'direct'},
                'request': {'url': url, 'method': method, 'body': body,
                            'headers': headers or {'Authorization': 'Bearer ' + remote.MARKER}}}
    def send(self, request):
        # The loopback exception lives only in this test callable, never a wire flag.
        return remote.run(request, send=lambda spec, token, _: remote.http(spec, token,
            {'mode': 'direct'}, url_guard=lambda url: url.startswith('http://127.0.0.1:')))
    def assert_private(self, value):
        raw = json.dumps(value)
        self.assertNotIn(TOKEN, raw)
        self.assertNotIn(REFRESH, raw)

    def test_gemini_projects_expiry_and_project_in_auth_and_fingerprint(self):
        for provider in ('pi', 'omp'):
            for expiry in (4102444800, 4102444800000):
                entry = {'type': 'oauth', 'access': TOKEN, 'refresh': REFRESH,
                         'expires': expiry, 'projectId': 'project-one', 'unknownSecret': REFRESH}
                ref = self.ref(provider, '/google-gemini-cli', 'login', 'auth.json', {'google-gemini-cli': entry})
                value = remote.resolve(ref)
                expected = {'access': TOKEN, 'expires': expiry, 'projectId': 'project-one'}
                self.assertEqual(value['auth'], expected)
                self.assertEqual(value['public']['fingerprint'], remote.digest(remote.compact(expected)))
                projected = value['public']['value']['google-gemini-cli']
                self.assertEqual(projected['projectId'], 'project-one')
                self.assertEqual(projected['expires'], expiry)
                self.assert_private(value['public'])
                entry['projectId'] = 'project-two'
                pathlib.Path(ref['configPath']).write_text(json.dumps({'google-gemini-cli': entry}))
                self.assertNotEqual(value['public']['pin'], remote.resolve(ref)['public']['pin'])

    def test_gemini_malformed_routing_metadata_never_becomes_null(self):
        for field, values in (('projectId', ({}, [], True, 7, '', 'two projects', 'x\ny', 'x' * 257)),
                              ('expires', ({}, [], True, '4102444800', 1.5, 2**63))):
            for value in values:
                entry = {'type': 'oauth', 'access': TOKEN, 'expires': 4102444800000, 'projectId': 'project-one', field: value}
                ref = self.ref('omp', '/google-gemini-cli', 'login', 'auth.json', {'google-gemini-cli': entry})
                result = remote.main({'reference': ref})
                self.assertEqual(result.get('errorCode'), 'credential', (field, value))
                self.assert_private(result)

    def test_omp_database_pin_tracks_disabled_and_replaced_pool_rows(self):
        path = self.root / 'agent.db'
        db = sqlite3.connect(path)
        try:
            db.executescript('PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;')
            db.executescript((HERE.parent / 'crates/core/tests/fixtures/omp-auth.sql').read_text())
            ref = self.ref('omp', '/google-gemini-cli#1', 'login', 'agent.db')
            first = remote.resolve(ref)
            self.assertEqual(first['auth'], {'access': 'PRIVATE-GEMINI-ONE', 'expires': 4102444800000, 'projectId': 'project-one'})
            self.assertNotIn('PRIVATE-GEMINI', remote.compact(first['public']))
            request = {'reference': ref, 'action': 'http', 'pin': first['public']['pin'],
                       'request': {'url': 'https://www.googleapis.com/oauth2/v2/userinfo'}}
            db.execute('UPDATE auth_credentials SET data=? WHERE id=1', (json.dumps({'access': TOKEN, 'projectId': 'project-two'}),))
            db.commit()
            self.assertEqual(remote.main(request)['errorCode'], 'changed')
            second = remote.resolve(ref)
            self.assertNotEqual(first['public']['fingerprint'], second['public']['fingerprint'])
            db.execute("UPDATE auth_credentials SET disabled_cause='signed out' WHERE id=1")
            db.commit()
            self.assertEqual(remote.main({'reference': ref})['errorCode'], 'removed')
        finally:
            db.close()

    def test_gemini_endpoint_allowlist_excludes_refresh_onboarding_and_models(self):
        for url in ('https://www.googleapis.com/oauth2/v2/userinfo',
                    'https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist',
                    'https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota'):
            self.assertTrue(remote.allowed_url(url))
            self.assertFalse(remote.allowed_url(url + '?access_token=ignored'))
        for url in ('https://oauth2.googleapis.com/token', 'https://www.googleapis.com/oauth2/v4/token',
                    'https://cloudcode-pa.googleapis.com/v1internal:onboardUser',
                    'https://cloudcode-pa.googleapis.com/v1internal:generateContent',
                    'https://cloudcode-pa.googleapis.com.evil.invalid/v1internal:retrieveUserQuota'):
            self.assertFalse(remote.allowed_url(url))

    def test_inspect_does_not_export_tokens_or_unknown_fields(self):
        public = remote.main({'reference': self.go()})
        self.assert_private(public)
        self.assertEqual(public['result']['fingerprint'], remote.digest(TOKEN))
        self.assertEqual(set(public['result']['value']), {'nativeApi'})
    def test_http_headers_and_post_body_use_remote_token(self):
        with fixture_server() as (url, seen):
            request = self.request(self.go(), url, method='POST', body='{}', headers={
                'Authorization': remote.MARKER, 'Connect-Protocol-Version': '1', 'Content-Type': 'application/json'})
            result = self.send(request)
        self.assertEqual(result, {'status': 200, 'data': {'balance': 12.5}})
        self.assertEqual(seen[0]['headers']['Authorization'], TOKEN)
        self.assertEqual(seen[0]['body'], b'{}')
        self.assert_private(result)
        self.assert_private(request)
    def test_pin_rejects_changed_entry_before_any_http(self):
        ref = self.go()
        request = self.request(ref, 'https://opencode.ai/zen/go/v1/usage')
        pathlib.Path(ref['configPath']).write_text(json.dumps({'opencode-go': {'type': 'api', 'key': TOKEN + '-other'}}))
        self.assertEqual(remote.main(request)['errorCode'], 'changed')
    def test_pin_discards_response_after_login_switch(self):
        ref = self.go()
        def change():
            pathlib.Path(ref['configPath']).write_text(json.dumps({'opencode-go': {'type': 'api', 'key': TOKEN + '-other'}}))
        with fixture_server(change=change) as (url, _):
            with self.assertRaisesRegex(remote.QuotaError, '^changed$'):
                self.send(self.request(ref, url))
    def test_redirect_never_forwards_credential_or_returns_error_body(self):
        with fixture_server() as (target, target_seen):
            with fixture_server(status=302, value={'message': TOKEN}, extra={'Location': target}) as (url, seen):
                result = self.send(self.request(self.go(), url))
        self.assertEqual(result, {'status': 302})
        self.assertEqual(len(seen), 1)
        self.assertEqual(target_seen, [])
        self.assert_private(result)
    def test_http_error_is_only_status(self):
        for status in (401, 403, 429, 500):
            with fixture_server(status=status, value={'error': TOKEN, 'refresh_token': REFRESH}) as (url, _):
                result = self.send(self.request(self.go(), url))
            self.assertEqual(result, {'status': status})
            self.assert_private(result)
    def test_response_bounds_and_private_echo(self):
        for raw, code in ((b'x' * (remote.RESPONSE_LIMIT + 1), 'response_large'),
                          (b'not json', 'response_format'),
                          (json.dumps({'message': TOKEN}).encode(), 'response_private')):
            with fixture_server(raw=raw) as (url, _):
                with self.assertRaisesRegex(remote.QuotaError, '^' + code + '$'):
                    self.send(self.request(self.go(), url))
        with fixture_server(value={'access_token': TOKEN, 'nested': {'refreshToken': REFRESH}, 'balance': 7}) as (url, _):
            result = self.send(self.request(self.go(), url))
        self.assertEqual(result['data'], {'nested': {}, 'balance': 7})
    def test_endpoint_allowlist_and_wire_cannot_enable_loopback(self):
        for url in ('http://api.github.com/user', 'https://api.github.com.evil.invalid/user',
                    'https://api.github.com/user?token=x', 'https://evil.invalid/',
                    'https://api.github.com@evil.invalid/user', 'https://api.github.com/user#fragment'):
            self.assertFalse(remote.allowed_url(url), url)
        request = self.request(self.go(), 'http://127.0.0.1:1')
        request['fixture'] = True
        self.assertEqual(remote.main(request)['errorCode'], 'endpoint')
        self.assertTrue(remote.allowed_url('https://api.cline.bot/api/v1/users/usr_1/balance'))
        self.assertFalse(remote.allowed_url('https://api.cline.bot/api/v1/users/../balance'))
    def test_dsh_yaml_and_env_select_exact_record(self):
        ref = self.ref('deepseek-harness', '/records/deepseek-account-platform~1default', 'login', '.credentials.yaml',
                       'version: 1\nrecords:\n  deepseek-account-platform/default:\n    kind: grant\n    payload:\n      version: 1\n      issuer: https://platform.deepseek.com\n      token: ' + TOKEN + '\n      accountId: uid-1\n      refresh_token: ' + REFRESH + '\nrefs:\n  DEEPSEEK_API_KEY: another-fixture-key\n')
        credential = remote.resolve(ref)
        self.assertEqual(credential['token'], TOKEN)
        self.assertEqual(credential['public']['fingerprint'], remote.digest(TOKEN))
        self.assert_private(credential['public'])
        ref = self.ref('deepseek-harness', '/DEEPSEEK_API_KEY', 'api', '.env',
                       'export DEEPSEEK_API_KEY="' + TOKEN + '"\nDEEPSEEK_BASE_URL=https://api.deepseek.com\n')
        credential = remote.resolve(ref)
        self.assertEqual(credential['token'], TOKEN)
        self.assertEqual(credential['public']['baseUrl'], 'https://api.deepseek.com')
        for line in ('export DEEPSEEK_API_KEY="' + TOKEN + '" # native comment',
                     'DEEPSEEK_API_KEY=' + TOKEN + ' # operator note',
                     "DEEPSEEK_API_KEY='" + TOKEN + "' # quoted", 'DEEPSEEK_API_KEY=`' + TOKEN + '` # backtick'):
            ref = self.ref('deepseek-harness', '/DEEPSEEK_API_KEY', 'api', '.env', line + '\n')
            self.assertEqual(remote.resolve(ref)['token'], TOKEN)
            self.assert_private(remote.main({'reference': ref}))
        self.assertEqual(remote.dotenv('A="one\\ntwo" # x\nB=\'three\nfour\'\nBAD-NAME=ignored\n'), {'A': 'one\ntwo', 'B': 'three\nfour'})
        self.assertEqual(remote.yaml_plain('refs:\n  DEEPSEEK_API_KEY: fixture #operator note\n  HASH: fixture#literal\n  QUOTED: "fixture #literal"\n'),
                         {'refs': {'DEEPSEEK_API_KEY': 'fixture', 'HASH': 'fixture#literal', 'QUOTED': 'fixture #literal'}})
    def test_toml_api_and_kimi_managed_region(self):
        ref = self.ref('kimi', '/providers/moonshot', 'api', 'config.toml',
                       '[providers.moonshot]\nbase_url = "https://api.moonshot.ai/v1"\napi_key = "' + TOKEN + '"\n')
        self.assertEqual(remote.resolve(ref)['public']['baseUrl'], 'https://api.moonshot.ai/v1')
        login = self.ref('kimi', '', 'login', 'credentials.json', {'access_token': TOKEN, 'refresh_token': REFRESH})
        (self.root / 'region').write_text('global\n')
        self.assertEqual(remote.resolve(login)['public']['baseUrl'], 'https://api.kimi.ai/coding/v1')
        (self.root / 'config.toml').write_text('[providers."managed:kimi-code"]\nbase_url = "https://api.kimi.com/coding/v1"\n')
        self.assertEqual(remote.resolve(login)['public']['baseUrl'], 'https://api.kimi.com/coding/v1')
        (self.root / 'config.toml').write_text('[providers."managed:kimi-code".oauth]\noauthHost = "https://auth.kimi.ai"\n')
        self.assertEqual(remote.resolve(login)['public']['baseUrl'], 'https://api.kimi.ai/coding/v1')
    def test_cline_zcode_and_trae_projections(self):
        ref = self.ref('cline', '/providers/cline/settings/auth', 'login', 'settings/providers.json', {
            'providers': {'cline': {'settings': {'auth': {'accessToken': TOKEN, 'accountId': 'a', 'refreshToken': REFRESH}}}}})
        self.assert_private(remote.resolve(ref)['public'])
        ref = self.ref('zcode', '/zcodeJwtToken', 'login', 'v2/credentials.json', {
            'zcodeJwtToken': TOKEN, 'zaiUserInfo': {'userId': 123, 'email': 'fixture@example.invalid'}, 'provider': 'zai', 'refresh': REFRESH})
        self.assertEqual(remote.resolve(ref)['public']['value']['zaiUserInfo']['userId'], 123)
        self.assert_private(remote.resolve(ref)['public'])
        ref = self.ref('trae', '/iCubeAuthInfo:~1~1icube.cloudide', 'login', 'storage.json', {
            remote.TRAE_LOGIN: json.dumps({'token': TOKEN, 'userId': 99, 'host': 'grow-normal.trae.ai', 'refreshToken': REFRESH})})
        result = remote.resolve(ref)
        self.assertEqual(result['public']['fingerprint'], remote.digest(remote.compact([ref['selector'], '99'])))
        self.assert_private(result['public'])
    def test_cline_native_key_locations_and_provider_aliases(self):
        for name, settings, suffix in (
            ('cline', {'apiKey': TOKEN}, ''),
            ('cline', {'auth': {'apiKey': TOKEN, 'accountId': 'fixture-user'}}, ''),
            ('named-login', {'provider': 'cline', 'apiKey': TOKEN, 'auth': {'accountId': 'fixture-user'}}, ''),
            ('named-login', {'provider': 'cline', 'auth': {'accessToken': TOKEN, 'accountId': 'fixture-user'}}, '/auth'),
            ('cline', {'apiKey': TOKEN, 'auth': {'accessToken': ''}}, '/auth'),
        ):
            with self.subTest(name=name, settings=list(settings)):
                ref = self.ref('cline', '/providers/' + name + '/settings' + suffix, 'login', 'settings/providers.json',
                               {'providers': {name: {'settings': settings}}})
                result = remote.resolve(ref)
                self.assertEqual(result['token'], TOKEN)
                self.assertEqual(result['public']['fingerprint'], remote.digest(TOKEN))
                if name == 'named-login':
                    self.assertEqual(result['public']['value']['providers'][name]['settings']['provider'], 'cline')
                self.assert_private(result['public'])
        for name, settings in (
            ('openrouter', {'auth': {'apiKey': TOKEN}}),
            ('named-router', {'provider': 'openrouter', 'auth': {'apiKey': TOKEN}}),
            ('named-router', {'provider': 'openrouter', 'apiKey': TOKEN}),
        ):
            ref = self.ref('cline', '/providers/' + name + '/settings', 'api', 'settings/providers.json',
                           {'providers': {name: {'settings': settings}}})
            result = remote.resolve(ref)
            self.assertEqual(result['token'], TOKEN)
            self.assertEqual(result['public']['baseUrl'], 'https://openrouter.ai/api/v1')
            self.assert_private(result['public'])
    def test_kilo_native_openrouter_route_defaults_follow_the_selected_section(self):
        for section in ('providers', 'provider', 'model', 'model_providers'):
            for route in ({'apiKey': TOKEN}, {'options': {'apiKey': TOKEN}}):
                ref = self.ref('kilo', '/' + section + '/openrouter', 'api', 'config.json',
                               {section: {'openrouter': route}})
                result = remote.resolve(ref)
                self.assertEqual(result['token'], TOKEN)
                self.assertEqual(result['public']['baseUrl'], 'https://openrouter.ai/api/v1')
                self.assert_private(result['public'])
    def test_grok_timing_projection_pins_all_selection_evidence(self):
        def token(subject, issued):
            claims = {'sub': subject, 'email': subject + '@fixture.invalid', 'iat': issued, 'exp': 4102444800}
            return 'fixture.' + remote.base64.urlsafe_b64encode(remote.compact(claims).encode()).decode().rstrip('=') + '.signature'
        first, other = token('selected-user', 300), token('other-private-user', 100)
        value = {'https://auth.x.ai::a': {'key': first, 'expires_at': 4102444800},
                 'https://auth.x.ai::b': {'key': other, 'expires_at': 4102444800}}
        ref = self.ref('grok', '/https:~1~1auth.x.ai::a', 'login', 'auth.json', value)
        public = remote.run({'reference': ref})
        selected = public['value']['https://auth.x.ai::a']['key']
        unselected = public['value']['https://auth.x.ai::b']['key']
        self.assertEqual(remote.claims(selected)['iat'], 300)
        self.assertEqual(remote.claims(unselected), {'iat': 100, 'exp': 4102444800})
        self.assertNotIn('other-private-user', remote.compact(public))
        self.assertNotIn(first, remote.compact(public))
        self.assertNotIn(other, remote.compact(public))
        request = self.request(ref, 'https://cli-chat-proxy.grok.com/v1/billing?format=credits')
        def change():
            changed = dict(value)
            changed['https://auth.x.ai::b'] = dict(value['https://auth.x.ai::b'], key=token('other-private-user', 400))
            pathlib.Path(ref['configPath']).write_text(json.dumps(changed))
        change()
        self.assertNotEqual(public['pin'], remote.run({'reference': ref})['pin'])
        with self.assertRaisesRegex(remote.QuotaError, '^changed$'):
            remote.run(request, send=lambda *_: self.fail('changed selection must fail before HTTP'))
        pathlib.Path(ref['configPath']).write_text(json.dumps(value))
        with fixture_server(change=change) as (url, _):
            with self.assertRaisesRegex(remote.QuotaError, '^changed$'):
                self.send(self.request(ref, url))
    def test_sqlite_cursor_and_opencode_projection_do_not_write_database(self):
        path = self.root / 'state.vscdb'
        with contextlib.closing(sqlite3.connect(path)) as db, db:
            db.execute('CREATE TABLE ItemTable(key TEXT, value TEXT)')
            db.executemany('INSERT INTO ItemTable VALUES(?,?)', [('cursorAuth/accessToken', TOKEN), ('cursorAuth/stripeMembershipType', 'pro')])
        before = path.read_bytes()
        ref = self.ref('cursor', '/cursorAuth', 'login', 'state.vscdb')
        self.assert_private(remote.resolve(ref)['public'])
        self.assertEqual(path.read_bytes(), before)
        path = self.root / 'opencode.db'
        with contextlib.closing(sqlite3.connect(path)) as db, db:
            db.execute('CREATE TABLE credential(id TEXT, integration_id TEXT, value TEXT, active INTEGER, time_created INTEGER)')
            db.execute('INSERT INTO credential VALUES(?,?,?,?,?)', ('1', 'opencode-go', json.dumps({'type': 'api', 'key': TOKEN}), 1, 9))
        before = path.read_bytes()
        ref = self.ref(name='opencode.db')
        self.assertEqual(remote.resolve(ref)['token'], TOKEN)
        self.assertEqual(path.read_bytes(), before)
    def test_copilot_github_token_preference_and_enterprise(self):
        github = 'gho_' + TOKEN
        ref = self.ref('pi', '/github-copilot', 'login', value={'github-copilot': {
            'type': 'oauth', 'access': 'short-lived-copilot-token', 'refresh': github, 'expires': 1}})
        result = remote.resolve(ref)
        self.assertEqual(result['token'], github)
        self.assertEqual(result['public']['fingerprint'], remote.digest(remote.compact({'access': github})))
        self.assertNotIn(github, json.dumps(result['public']))
        pathlib.Path(ref['configPath']).write_text(json.dumps({'github-copilot': {
            'type': 'oauth', 'access': github, 'enterpriseUrl': 'https://enterprise.invalid'}}))
        self.assertEqual(remote.main({'reference': ref})['errorCode'], 'unsupported')
    def test_copilot_last_logged_in_user_keeps_the_full_file_selection(self):
        ref = self.ref('copilot', '/copilot_tokens/https:~1~1github.com:alice', 'login', 'config.json', {
            'copilot_tokens': {'https://github.com:alice': 'gho_' + TOKEN, 'https://github.com:bob': 'gho_other-native-fixture'},
            'last_logged_in_user': {'login': 'Bob'}})
        self.assertEqual(remote.main({'reference': ref})['errorCode'], 'changed')
        ref['selector'] = '/copilot_tokens/https:~1~1github.com:bob'
        self.assertEqual(remote.resolve(ref)['token'], 'gho_other-native-fixture')
    def test_copilot_environment_uses_owning_host_precedence_and_never_exports_token(self):
        for selected, environment in (
            ('COPILOT_GITHUB_TOKEN', {'COPILOT_GITHUB_TOKEN': 'gho_' + TOKEN, 'GH_TOKEN': 'gho_lower', 'GITHUB_TOKEN': 'gho_lowest'}),
            ('GH_TOKEN', {'COPILOT_GITHUB_TOKEN': ' ', 'GH_TOKEN': 'gho_' + TOKEN, 'GITHUB_TOKEN': 'gho_lower'}),
            ('GITHUB_TOKEN', {'GITHUB_TOKEN': 'gho_' + TOKEN}),
        ):
            with self.subTest(selected=selected), mock.patch.dict(os.environ, environment, clear=True):
                ref = dict(provider='copilot', selector='', kind='login', root=str(self.root), configPath='env:' + selected)
                public = remote.run({'reference': ref})
                self.assert_private(public)
                self.assertEqual(public['fingerprint'], remote.digest('gho_' + TOKEN))
                with fixture_server(value={'id': 7, 'login': 'fixture'}) as (url, seen):
                    request = self.request(ref, url, headers={'Authorization': 'token ' + remote.MARKER})
                    self.assert_private(request)
                    self.assertEqual(self.send(request)['data']['id'], 7)
                self.assertEqual(seen[0]['headers']['Authorization'], 'token gho_' + TOKEN)
    def test_copilot_environment_pin_rejects_rotation_and_new_higher_priority_token(self):
        ref = dict(provider='copilot', selector='', kind='login', root=str(self.root), configPath='env:GH_TOKEN')
        with mock.patch.dict(os.environ, {'GH_TOKEN': 'gho_' + TOKEN}, clear=True):
            request = self.request(ref, 'https://api.github.com/user')
            with mock.patch.dict(os.environ, {'GH_TOKEN': 'gho_rotated'}):
                self.assertEqual(remote.main(request)['errorCode'], 'changed')
            with mock.patch.dict(os.environ, {'COPILOT_GITHUB_TOKEN': 'gho_override'}):
                self.assertEqual(remote.main(request)['errorCode'], 'changed')
            with mock.patch.dict(os.environ, {'GH_TOKEN': ''}):
                self.assertEqual(remote.main(request)['errorCode'], 'removed')
            # A rotation during an HTTP read also discards the response.
            with fixture_server(change=lambda: os.environ.update(GH_TOKEN='gho_rotated')) as (url, seen):
                with self.assertRaisesRegex(remote.QuotaError, '^changed$'):
                    self.send(self.request(ref, url))
            self.assertEqual(len(seen), 1)
    def test_copilot_environment_references_are_whitelisted_and_require_a_native_root(self):
        ref = dict(provider='copilot', selector='', kind='login', root=str(self.root / 'missing'), configPath='env:GH_TOKEN')
        with mock.patch.dict(os.environ, {'GH_TOKEN': 'gho_' + TOKEN}, clear=True):
            self.assertEqual(remote.main({'reference': ref})['errorCode'], 'removed')
        with mock.patch.dict(os.environ, {'COPILOT_GITHUB_TOKEN': 'gho_' + TOKEN}, clear=True):
            ref['configPath'] = 'env:COPILOT_GITHUB_TOKEN'
            self.assert_private(remote.run({'reference': ref}))
            for change in ({'configPath': 'env:UNRELATED_SECRET'}, {'selector': '/token'},
                           {'provider': 'pi'}, {'root': 'relative'}, {'kind': 'api'}):
                self.assertIn('errorCode', remote.main({'reference': dict(ref, **change)}))
    def test_kiro_active_selector_and_stable_profile_fingerprint(self):
        path = self.root / 'data.sqlite3'
        future = int(time.time()) + 3600
        with contextlib.closing(sqlite3.connect(path)) as db, db:
            db.execute('CREATE TABLE auth_kv(key TEXT, value TEXT)')
            db.execute('CREATE TABLE state(key TEXT, value TEXT)')
            db.execute('INSERT INTO state VALUES(?,?)', ('api.codewhisperer.profile', json.dumps({'arn': 'arn:aws:codewhisperer:us-east-1:1:profile/a'})))
            db.execute('INSERT INTO auth_kv VALUES(?,?)', ('kirocli:social:token', json.dumps({'access_token': TOKEN, 'refresh_token': REFRESH, 'provider': 'Google', 'expires_at': future})))
            db.execute('INSERT INTO auth_kv VALUES(?,?)', ('kirocli:odic:token', json.dumps({'access_token': 'old-token', 'expires_at': 1})))
        ref = self.ref('kiro', '/auth_kv/social', 'login', 'data.sqlite3')
        result = remote.resolve(ref)
        self.assertEqual(result['token'], TOKEN)
        self.assert_private(result['public'])
        ref['selector'] = '/auth_kv/odic'
        self.assertEqual(remote.main({'reference': ref})['errorCode'], 'changed')
        with contextlib.closing(sqlite3.connect(path)) as db, db:
            db.execute('UPDATE auth_kv SET value=? WHERE key=?',
                       (json.dumps({'access_token': 'oidc-native-fixture', 'expires_at': future}), 'kirocli:odic:token'))
        # The equal maximum is social, matching Rust's max_by_key traversal.
        ref['selector'] = '/auth_kv/social'
        self.assertEqual(remote.resolve(ref)['token'], TOKEN)
        ref['selector'] = '/auth_kv/odic'
        self.assertEqual(remote.main({'reference': ref})['errorCode'], 'changed')
    def test_serialized_process_protocol_uses_stdin_and_private_projection(self):
        request = {'reference': self.go()}
        result = subprocess.run([sys.executable, str(HERE / 'remote_native_quota.py')],
                                input=json.dumps(request), text=True, capture_output=True, check=True)
        self.assertEqual(result.stderr, '')
        self.assert_private(result.stdout)
        self.assertEqual(json.loads(result.stdout)['version'], 1)
    def test_zcode_flat_store_selects_one_profile_and_manual_key(self):
        ref = self.ref('zcode', '/zcodejwttoken', 'login', 'v2/credentials.json', {
            'zcodejwttoken': TOKEN, 'oauth:active_provider': 'bigmodel',
            'oauth:bigmodel:user_info': json.dumps({'id': 'organization-1', 'user_id': 'user-1', 'rawProfile': {'email': 'fixture@example.invalid'}}),
            'oauth:zai:user_info': json.dumps({'id': 'other-user'}), 'oauth:bigmodel:refresh_token': REFRESH})
        public = remote.resolve(ref)['public']
        self.assert_private(public)
        self.assertNotIn('other-user', json.dumps(public))
        key = 'account-provider:coding-plan:account:zai-individual-coding-plan:account:key-fixture:api-key'
        value = {'account-provider:account:zai-individual-coding-plan:identity': 'key-fixture',
                 'account-provider:account:zai-individual-coding-plan:auth-source': 'manual', key: TOKEN}
        ref = self.ref('zcode', '/' + key, 'api', 'v2/credentials.json', value)
        result = remote.resolve(ref)
        self.assertEqual(result['token'], TOKEN)
        self.assertEqual(result['public']['baseUrl'], 'https://api.z.ai/api/coding/paas/v4')
        value['account-provider:account:zai-individual-coding-plan:identity'] = 'key-other'
        pathlib.Path(ref['configPath']).write_text(json.dumps(value))
        self.assertEqual(remote.main({'reference': ref})['errorCode'], 'changed')
    def test_zcode_native_encryption_is_opened_only_on_source_host(self):
        import shutil
        if not shutil.which('node'):
            self.skipTest('ZCode native codec requires Node.js')
        node = r"""const c=require('node:crypto'),k=c.createHash('sha256').update('fixture-secret').digest();
        const out={};for(const [n,v] of Object.entries(JSON.parse(require('node:fs').readFileSync(0,'utf8')))){
        const iv=Buffer.alloc(12,5),d=c.createCipheriv('aes-256-gcm',k,iv),b=Buffer.concat([d.update(v),d.final()]);
        out[n]='enc:v1:'+iv.toString('base64url')+'.'+d.getAuthTag().toString('base64url')+'.'+b.toString('base64url');}
        process.stdout.write(JSON.stringify(out));"""
        raw = subprocess.run(['node', '-e', node], input=json.dumps({'zcodejwttoken': TOKEN,
            'oauth:active_provider': 'zai', 'oauth:zai:user_info': '{"user_id":"user-1"}'}),
            text=True, capture_output=True, check=True)
        ref = self.ref('zcode', '/zcodejwttoken', 'login', 'v2/credentials.json', json.loads(raw.stdout))
        with mock.patch.dict(os.environ, {'ZCODE_CREDENTIAL_SECRET': 'fixture-secret'}):
            self.assertEqual(remote.resolve(ref)['token'], TOKEN)
            self.assert_private(remote.main({'reference': ref}))
        with mock.patch.dict(os.environ, {'ZCODE_CREDENTIAL_SECRET': 'wrong-fixture-secret'}):
            self.assertEqual(remote.main({'reference': ref})['errorCode'], 'format')
    @unittest.skipIf(os.name == 'nt', 'SSH Codex fixture requires POSIX executable permissions')
    def test_codex_uses_ephemeral_rpc_and_defers_refresh(self):
        claims = {'https://api.openai.com/auth': {'chatgpt_user_id': 'user-1', 'chatgpt_account_id': 'default-team'}}
        token = 'fixture.' + remote.base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip('=') + '.signature'
        ref = self.ref('pi', '/openai', 'login', value={'openai': {'type': 'oauth', 'access': token,
                        'refresh': REFRESH, 'accountId': 'selected-team'}})
        program = self.root / 'codex-fixture'
        program.write_text('''#!/usr/bin/env python3
import json,os,pathlib,sys
if '--version' in sys.argv:
 print('codex-cli 0.125.0');raise SystemExit()
assert 'cli_auth_credentials_store="ephemeral"' in sys.argv
assert os.environ.get('AIEYES_INTERNAL')=='1'
root=pathlib.Path(os.environ['CODEX_HOME'])
assert (root.stat().st_mode & 0o777)==0o700 and not (root/'auth.json').exists()
waiting=None
for line in sys.stdin:
 v=json.loads(line);method=v.get('method')
 if v.get('id')=='refresh-fixture':
  assert 'error' in v
  print(json.dumps({'id':waiting,'result':{'rateLimits':{'primary':{'usedPercent':25,'windowDurationMins':300}}}}),flush=True);continue
 if method=='initialized':continue
 if method=='account/login/start':
  assert v['params']['type']=='chatgptAuthTokens' and v['params']['chatgptAccountId']=='selected-team'
  assert 'refresh' not in json.dumps(v['params']).lower();result={}
 elif method=='account/read':result={'account':{'type':'chatgpt'},'workspaceRouting':{'chatgptAccountId':'selected-team'}}
 elif method=='account/rateLimits/read':
  waiting=v['id'];print(json.dumps({'id':'refresh-fixture','method':'account/chatgptAuthTokens/refresh','params':{}}),flush=True);continue
 else:
  assert method=='initialize';result={}
 print(json.dumps({'id':v['id'],'result':result}),flush=True)
''')
        program.chmod(0o700)
        public = remote.run({'reference': ref})
        result = remote.run({'reference': ref, 'pin': public['pin'], 'action': 'codex',
                             'binary': str(program), 'proxy': {'mode': 'direct'}})
        self.assertEqual(result['data']['rateLimits']['primary']['usedPercent'], 25)
        self.assertNotIn(token, json.dumps(result))
        self.assert_private(result)
    def test_remote_kiro_has_no_cli_refresh_action(self):
        ref = self.ref('kiro', '/kiro-auth-token', 'login', 'kiro-auth-token.json',
                       {'accessToken': TOKEN, 'expiresAt': 1, 'profileArn': 'arn:aws:codewhisperer:us-east-1:1:profile/a'})
        public = remote.run({'reference': ref})
        self.assertEqual(remote.main({'reference': ref, 'pin': public['pin'], 'action': 'kiroCli'})['errorCode'], 'request')


if __name__ == '__main__':
    unittest.main()
