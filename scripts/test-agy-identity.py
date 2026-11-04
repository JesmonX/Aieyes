"""Hermetic identity/entitlement tests: never read a real credential store."""
import base64
import json
import os
import pathlib
import sqlite3
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from script_loader import load_script

agy = load_script('agy_identity', 'agy_identity.py')


def credential(sub='one', email='one@example.test'):
    claims = {'iss': 'https://accounts.google.com', 'email_verified': True,
              'sub': sub, 'email': email, 'exp': 1}
    encoded = base64.urlsafe_b64encode(json.dumps(claims).encode()).decode().rstrip('=')
    return {'token': {'access_token': 'fake-private-access', 'refresh_token': 'fake-private-refresh'},
            'auth_method': 'consumer', 'id_token': 'header.' + encoded + '.signature'}


def wire(number, value):
    def varint(value):
        data = bytearray()
        while value >= 128:
            data.append((value & 127) | 128)
            value >>= 7
        data.append(value)
        return bytes(data)
    if isinstance(value, int):
        return varint(number << 3) + varint(value)
    return varint(number << 3 | 2) + varint(len(value)) + value


def ide_topic(legacy=False, expiry=4070908800):
    if legacy:
        raw = json.dumps({'accessToken': 'private-ide-access', 'tokenType': 'Bearer',
                          'refreshToken': 'private-ide-refresh', 'expiryDateSeconds': expiry}).encode()
    else:
        # Independent native descriptor: OAuthTokenInfo fields 1 access, 2 type,
        # 3 refresh, 4 Timestamp expiry. Topic and Row each wrap field 1.
        raw = base64.b64encode(wire(1, b'private-ide-access') + wire(2, b'Bearer')
                              + wire(3, b'private-ide-refresh') + wire(4, wire(1, expiry)))
    entry = wire(1, b'oauthTokenInfoSentinelKey') + wire(2, wire(1, raw))
    return base64.b64encode(wire(1, entry)).decode()


def ide_store(root, value, name='Antigravity IDE'):
    path = pathlib.Path(root) / '.config' / name / 'User' / 'globalStorage' / 'state.vscdb'
    path.parent.mkdir(parents=True, exist_ok=True)
    db = sqlite3.connect(path)
    db.execute('CREATE TABLE IF NOT EXISTS ItemTable (key TEXT PRIMARY KEY, value TEXT)')
    if value is not None:
        db.execute('INSERT OR REPLACE INTO ItemTable VALUES (?, ?)',
                   ('antigravityUnifiedStateSync.oauthToken', value))
    db.commit()
    db.close()
    return path


class IdentityTests(unittest.TestCase):
    def setUp(self):
        # REST fixtures must never discover or launch a developer's real CLI.
        self.native = patch.object(agy, 'native_quota', return_value=None)
        self.native_mock = self.native.start()
        self.addCleanup(self.native.stop)

    def test_cli_groups_are_authoritative_even_when_rest_would_omit_gemini(self):
        current = credential()
        groups = [{'name': name, 'buckets': [{'id': name, 'name': 'Weekly', 'window': 'weekly',
                   'remaining_fraction': 1, 'reset_time': '2099-01-01T00:00:00Z'}]}
                  for name in ('Gemini Models', 'Claude and GPT models', 'Gemini 4 Argon')]
        self.native_mock.return_value = {'groups': groups, 'private': 'must-not-return'}
        with patch.object(agy, 'read_credential', return_value=current), \
                patch.object(agy, 'request_json', return_value={'paidTier': {'name': 'Google AI Pro'}}) as request:
            result = agy.quota()
            self.assertEqual([g['name'] for g in result['command']['data']['groups']], [g['name'] for g in groups])
            self.assertEqual(request.call_count, 1)
            self.assertEqual(request.call_args.args[1], agy.ASSIST_URL)
            self.assertNotIn('must-not-return', json.dumps(result))

    def test_expired_access_is_renewed_by_cli_and_owner_is_rechecked(self):
        old = credential()
        old['token']['expiry'] = '2001-01-01T00:00:00Z'
        new = credential()
        self.native_mock.return_value = {'groups': []}
        with patch.object(agy, 'read_credential', return_value=new):
            self.assertEqual(agy.identity_credential(old), new)
        with patch.object(agy, 'read_credential', return_value=credential('other')):
            with self.assertRaisesRegex(agy.PublicError, '账户已变化'):
                agy.identity_credential(old)

    def test_newer_same_owner_file_is_used_and_conflict_is_reported(self):
        with tempfile.TemporaryDirectory() as root, patch.dict(os.environ, {'HOME': root, 'USERPROFILE': root, 'SSH_CONNECTION': 'fixture'}, clear=True):
            home = pathlib.Path(root) / '.gemini'
            (home / 'antigravity-cli').mkdir(parents=True)
            old = credential(); old['token']['expiry'] = '2001-01-01T00:00:00Z'
            new = credential(); new['token']['expiry'] = '2099-01-01T00:00:00Z'
            (home / 'antigravity-oauth-token').write_text(json.dumps(old))
            target = home / 'antigravity-cli' / 'antigravity-oauth-token'
            target.write_text(json.dumps(new))
            self.assertEqual(agy.read_credential(), new)
            target.write_text(json.dumps(credential('other')))
            self.assertIn('多个不同', agy.observe()['metadataError'])

    def test_network_diagnostics_do_not_expose_response_or_stderr(self):
        for code, status, expected in [(22, b'401', agy.RELOGIN), (22, b'403', '拒绝访问'),
                                       (22, b'429', '过于频繁'), (28, b'000', '超时'), (7, b'000', '网络连接失败')]:
            result = subprocess.CompletedProcess([], code, b'private-response', status)
            with patch.object(agy, 'read_credential', return_value=credential()), \
                    patch.object(agy.subprocess, 'run', return_value=result):
                value = agy.quota()
                self.assertIn(expected, value['error'])
                self.assertNotIn('private', json.dumps(value))

    def test_ide_proto_and_legacy_json_are_read_without_writes_or_refresh_tokens(self):
        for legacy in (False, True):
            with tempfile.TemporaryDirectory() as root, \
                    patch.dict(os.environ, {'HOME': root, 'USERPROFILE': root}, clear=True), \
                    patch.object(agy.sys, 'platform', 'linux'):
                path = ide_store(root, ide_topic(legacy))
                before = path.read_bytes()
                value = agy.read_ide_credential()
                self.assertEqual(value['_origin'], 'ide')
                self.assertEqual(value['token']['access_token'], 'private-ide-access')
                self.assertEqual(value['token']['token_type'], 'Bearer')
                self.assertNotIn('refresh', json.dumps(value))
                self.assertEqual(path.read_bytes(), before)
                self.assertEqual(list(path.parent.iterdir()), [path])
                with self.assertRaises(ValueError):
                    agy.ide_token(base64.b64encode(wire(1, b'\x0a\xff')).decode())

    def test_cli_presence_never_falls_through_to_another_ide_account(self):
        with tempfile.TemporaryDirectory() as root, \
                patch.dict(os.environ, {'HOME': root, 'USERPROFILE': root, 'SSH_CONNECTION': 'fixture'}, clear=True), \
                patch.object(agy, 'read_ide_credential', side_effect=AssertionError('CLI owns this login')) as ide_reader, \
                patch.object(agy, 'request_json', side_effect=AssertionError('must not make an HTTP request')):
            folder = pathlib.Path(root) / '.gemini'
            folder.mkdir()
            path = folder / 'antigravity-oauth-token'
            for raw in (b'', b'not-json', b'{}'):
                path.write_bytes(raw)
                self.assertIsNone(agy.observe()['identity'])
            current = {'auth_method': 'consumer', 'id_token_email': 'cli@example.test',
                       'token': {'access_token': 'private-cli', 'expiry': '2001-01-01T00:00:00Z'}}
            path.write_text(json.dumps(current))
            self.assertEqual(agy.read_credential(), current)
            self.assertEqual(agy.quota(), {'error': agy.RELOGIN})
            ide_reader.assert_not_called()

    def test_ide_only_identity_requires_userinfo_and_expiry_does_not_refresh(self):
        with tempfile.TemporaryDirectory() as root, \
                patch.dict(os.environ, {'HOME': root, 'USERPROFILE': root}, clear=True), \
                patch.object(agy.sys, 'platform', 'linux'), \
                patch.object(agy.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1, b'')):
            ide_store(root, ide_topic())
            with patch.object(agy, 'request_json', side_effect=[
                    {'id': 'one', 'email': 'ide@example.test', 'verified_email': True},
                    {'licenses': [{'userTier': 'g1-pro-tier'}]}]) as request:
                result = agy.observe()
                self.assertEqual(result['identity']['key'], agy.identity(credential())['key'])
                self.assertEqual(result['identity']['email'], 'ide@example.test')
                self.assertNotIn('refresh', str(request.call_args_list))
                self.assertNotIn('private-ide', json.dumps(result))
            ide_store(root, ide_topic(expiry=1))
            with patch.object(agy, 'request_json', side_effect=AssertionError('expired IDE must not call HTTP')):
                self.assertEqual(agy.quota(), {'error': agy.RELOGIN_IDE})

    def test_signed_out_current_ide_does_not_reuse_legacy_ide_login(self):
        with tempfile.TemporaryDirectory() as root, \
                patch.dict(os.environ, {'HOME': root, 'USERPROFILE': root}, clear=True), \
                patch.object(agy.sys, 'platform', 'linux'):
            ide_store(root, None)
            ide_store(root, ide_topic(), 'Antigravity')
            self.assertIsNone(agy.read_ide_credential())

    def test_current_cli_email_hint_requires_verified_userinfo(self):
        current = {'token': {'access_token': 'private-current', 'refresh_token': 'never-use-this',
                            'expiry': '2099-01-01T00:00:00Z'},
                   'auth_method': 'consumer', 'id_token_email': 'old@example.test'}
        with patch.object(agy, 'request_json', return_value={
                'id': 'one', 'email': 'renamed@example.test', 'verified_email': True}) as request:
            resolved = agy.resolve_identity(current)
            self.assertEqual(resolved['key'], agy.identity(credential())['key'])
            self.assertEqual(request.call_args.args[1], agy.USERINFO_URL)
            self.assertTrue(agy.same_owner(current, current))
            self.assertFalse(agy.same_owner(current, {**current, 'token': {'access_token': 'different'}}))
            # A refreshed keychain entry and a stale file copy share only the grant.
            refreshed = {**current, 'token': {**current['token'], 'access_token': 'private-refreshed'}}
            self.assertTrue(agy.same_owner(current, refreshed))
            self.assertFalse(agy.same_owner(current, {**refreshed, 'token': {**refreshed['token'], 'refresh_token': 'other-grant'}}))
        with patch.object(agy, 'request_json', return_value={'email': 'old@example.test'}):
            with self.assertRaises(ValueError):
                agy.resolve_identity(current)

    def test_quota_expiry_without_cli_never_calls_token_endpoint(self):
        current = credential()
        current['token']['expiry'] = '2001-01-01T00:00:00Z'
        with patch.object(agy, 'read_credential', return_value=current), \
                patch.object(agy.subprocess, 'run', side_effect=AssertionError('expired token must not run a process')):
            self.assertEqual(agy.quota(), {'error': agy.RELOGIN})

    def test_native_quota_proto_projection_and_partial_credits(self):
        current = credential()
        native = {'groups': [{'displayName': 'Gemini Models', 'buckets': [
            {'bucketId': 'gemini-five-hour', 'window': '5h', 'remainingFraction': 0.4,
             'resetTime': '2099-01-01T00:00:00Z'}]}], 'private': 'never-return'}
        assist = {'paidTier': {'id': 'g1-pro-tier', 'name': 'Google AI Pro',
                  'availableCredits': [{'creditType': 'GOOGLE_ONE_AI', 'creditAmount': '1000'}]},
                  'cloudaicompanionProject': 'private-project', 'secret': 'never-return'}
        with patch.object(agy, 'read_credential', return_value=current), \
                patch.object(agy, 'request_json', side_effect=[assist, native]) as request:
            result = agy.quota()
            data = result['command']['data']
            self.assertEqual(data['groups'][0]['buckets'][0]['remaining_fraction'], 0.4)
            self.assertEqual(data['loadCodeAssist']['paidTier']['availableCredits'][0]['creditAmount'], '1000')
            self.assertEqual(request.call_args_list[1].args[2], {'project': 'private-project'})
            self.assertNotIn('private', json.dumps(result))
            self.assertNotIn('never-return', json.dumps(result))
        with patch.object(agy, 'read_credential', return_value=current), \
                patch.object(agy, 'request_json', side_effect=[ValueError('private'), native]):
            result = agy.quota()
            self.assertEqual(result['status'], 'SUCCESS')
            self.assertTrue(result['metadataError'])

    def test_credential_encodings_and_stable_subject(self):
        c = credential()
        for raw in [json.dumps(c).encode(), b'go-keyring-base64:' + base64.b64encode(json.dumps(c).encode())]:
            self.assertEqual(agy.identity(agy.decode(raw))['email'], 'one@example.test')
        self.assertEqual(agy.identity(c)['key'], agy.identity(credential(email='renamed@example.test'))['key'])
        self.assertNotEqual(agy.identity(c)['key'], agy.identity(credential('two'))['key'])
        for raw in [b'{}', b'[]', b'bad', b'go-keyring-base64:!', b'x' * (agy.LIMIT + 1)]:
            with self.assertRaises(Exception):
                agy.identity(agy.decode(raw))
        for field, value in [('auth_method', 'enterprise'), ('id_token', ''), ('id_token', 'bad.secret.value')]:
            with self.assertRaises(Exception):
                agy.identity({**c, field: value})

    def test_license_first_entry_and_unknown_is_not_free(self):
        for tier, expected in [('g1-pro-tier', 'Google AI Pro'), ('g1-ultra-tier', 'Google AI Ultra'), ('free-tier', 'Free')]:
            self.assertEqual(agy.subscription({'licenses': [{'userTier': tier}]}), expected)
        self.assertEqual(agy.subscription({'licenses': [{'tierDisplayName': 'Google AI Pro'}, {'tierDisplayName': 'Free'}]}), 'Google AI Pro')
        # A free tier keeps one label whatever its display name says.
        for tier in ['free-tier', 'g1-free-tier']:
            self.assertEqual(agy.subscription({'licenses': [{'userTier': tier, 'tierDisplayName': 'Gemini for individuals'}]}), 'Free')
        self.assertEqual(agy.subscription({'licenses': [{'userTier': 'g1-pro-tier', 'tierDisplayName': 'Google AI Pro (2 TB)'}]}), 'Google AI Pro (2 TB)')
        for value in [{}, {'licenses': []}, {'licenses': [{}]}, {'licenses': [{'userTier': 'future-tier'}]}]:
            self.assertIsNone(agy.subscription(value))

    def test_curl_credential_stays_on_stdin_and_errors_are_sanitized(self):
        c = credential()
        with patch.object(agy.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, b'{"licenses":[{"tierDisplayName":"Google AI Pro"}]}')) as run:
            self.assertEqual(agy.fetch_license(c), 'Google AI Pro')
            self.assertNotIn('fake-private-access', str(run.call_args.args))
            self.assertIn(b'fake-private-access', run.call_args.kwargs['input'])
            self.assertNotIn('fake-private-refresh', str(run.call_args))
        with patch.object(agy, 'read_credential', return_value=c), patch.object(agy, 'fetch_license', side_effect=ValueError('fake-private-access')):
            result = agy.observe()
            self.assertTrue(result['identity']['stale'])
            self.assertEqual(result['identity']['email'], 'one@example.test')
            self.assertNotIn('fake-private', json.dumps(result))
        with patch.object(agy, 'read_credential', side_effect=[c, credential('two')]), patch.object(agy, 'fetch_license', return_value='Google AI Pro'):
            self.assertIsNone(agy.observe()['identity'])

    def test_remote_files_follow_home_and_conflicting_stores_are_unknown(self):
        with tempfile.TemporaryDirectory() as root, patch.dict(os.environ, {'HOME': root, 'USERPROFILE': root, 'SSH_CONNECTION': 'fixture'}, clear=True), patch.object(agy.subprocess, 'run', side_effect=AssertionError('must not open keyring')):
            home = pathlib.Path(root) / '.gemini'
            home.mkdir()
            (home / 'antigravity-oauth-token').write_text(json.dumps(credential()))
            self.assertEqual(agy.identity(agy.read_credential())['email'], 'one@example.test')
            (home / 'antigravity-cli').mkdir()
            (home / 'antigravity-cli' / 'antigravity-oauth-token').write_text(json.dumps(credential('two')))
            self.assertIsNone(agy.observe()['identity'])
        with patch.dict(os.environ, {'GEMINI_API_KEY': 'fixture'}, clear=True):
            self.assertIsNone(agy.observe()['identity'])


class NativeUsageTests(unittest.TestCase):
    def test_only_builtin_usage_runs_in_empty_workspace_and_raw_output_is_not_returned(self):
        def run(args, **kwargs):
            self.assertEqual(args[1:], ['--log-file', os.devnull, '--print', '/usage', '--output-format', 'json'])
            self.assertEqual(list(pathlib.Path(kwargs['cwd']).iterdir()), [])
            self.assertEqual(kwargs['stdin'], subprocess.DEVNULL)
            self.assertEqual(kwargs['env']['AIEYES_INTERNAL'], '1')
            self.assertNotIn('fake-private', str(args))
            return subprocess.CompletedProcess(args, 0, b'{"status":"SUCCESS","command":{"name":"usage","data":{"groups":[]}},"private":"discard"}')
        with patch.dict(os.environ, {'AIEYES_AGY_BINARY': '/fixture/agy'}), patch.object(agy.subprocess, 'run', side_effect=run):
            self.assertEqual(agy.native_quota(credential()), {'groups': []})
        with patch.object(agy.subprocess, 'run', side_effect=AssertionError('IDE must not run CLI')):
            self.assertIsNone(agy.native_quota({'_origin': 'ide'}))
        with patch.dict(os.environ, {'AIEYES_AGY_BINARY': '/fixture/agy'}), \
                patch.object(agy.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, b'{"status":"SUCCESS","response":"private"}')):
            with self.assertRaisesRegex(agy.PublicError, '未返回模型组'):
                agy.native_quota(credential())


if __name__ == '__main__':
    unittest.main()
