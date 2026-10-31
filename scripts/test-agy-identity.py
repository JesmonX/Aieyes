"""Hermetic identity/entitlement tests: never read a real credential store."""
import base64
import importlib.util
import json
import os
import pathlib
import sqlite3
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('agy_identity', pathlib.Path(__file__).with_name('agy_identity.py'))
agy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agy)


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
        with patch.object(agy, 'request_json', return_value={'email': 'old@example.test'}):
            with self.assertRaises(ValueError):
                agy.resolve_identity(current)

    def test_quota_expiry_never_launches_cli_or_calls_refresh(self):
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


if __name__ == '__main__':
    unittest.main()
