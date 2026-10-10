"""Hermetic identity/entitlement tests: never read a real credential store."""
import base64
import importlib.util
import json
import os
import pathlib
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


class IdentityTests(unittest.TestCase):
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
        with tempfile.TemporaryDirectory() as root, patch.dict(os.environ, {'HOME': root, 'SSH_CONNECTION': 'fixture'}, clear=True), patch.object(agy.subprocess, 'run', side_effect=AssertionError('must not open keyring')):
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
