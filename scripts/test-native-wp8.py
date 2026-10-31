"""Installed ZCode 3.15.3 credential protocol; no real login files or requests."""
import base64
import importlib.util
import json
import os
import pathlib
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    'account_scan', pathlib.Path(__file__).with_name('remote_account_scan.py'))
scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scan)

# Node crypto AES-256-GCM: SHA256("test-secret"), 12 bytes of 0x05, "zai".
CIPHER = 'enc:v1:BQUFBQUFBQUFBQUF.N6rUk4dZmqwly8-sZzrbtA.ffbf'


class ZCodeTransport(unittest.TestCase):
    def test_native_cipher_and_allowlist(self):
        raw = json.dumps({'oauth:active_provider': CIPHER,
                          'oauth:zai:user_info': '{"user_id":"fixture"}',
                          'oauth:zai:refresh_token': 'PRIVATE-REFRESH'}).encode()
        with patch.dict(os.environ, {'ZCODE_CREDENTIAL_SECRET': 'test-secret'}):
            projected = json.loads(scan.zcode_projection(raw))
        self.assertEqual(projected, {'oauth:active_provider': 'zai',
                                    'oauth:zai:user_info': '{"user_id":"fixture"}'})
        with patch.dict(os.environ, {'ZCODE_CREDENTIAL_SECRET': 'wrong'}):
            with self.assertRaises(ValueError):
                scan.zcode_projection(raw)

    def test_plaintext_needs_no_node_and_missing_node_is_reported(self):
        with patch.object(scan.subprocess, 'run', side_effect=FileNotFoundError('node')):
            raw = b'{"oauth:active_provider":"zai","oauth:zai:refresh_token":"PRIVATE"}'
            self.assertEqual(json.loads(scan.zcode_projection(raw)), {'oauth:active_provider': 'zai'})
            with self.assertRaises(ValueError):
                scan.zcode_projection(json.dumps({'oauth:active_provider': CIPHER}).encode())

    def test_remote_data_base_and_reloads_do_not_write_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            native = home / '.zcode/v2'
            native.mkdir(parents=True)
            path = native / 'credentials.json'
            raw = json.dumps({'oauth:active_provider': CIPHER}).encode()
            path.write_bytes(raw)
            request = {'provider': 'zcode', 'roots': [], 'patterns': [], 'env': '',
                       'primary': str(home / '.zcode')}
            with patch.dict(os.environ, {'HOME': str(home), 'USERPROFILE': str(home),
                                       'ZCODE_DATA_BASE_DIR': str(home),
                                       'ZCODE_CREDENTIAL_SECRET': 'test-secret'}):
                result = scan.collect(request)
                self.assertFalse(result['errors'])
                self.assertEqual(len(result['documents']), 1)
                self.assertEqual(json.loads(base64.b64decode(result['documents'][0]['data'])),
                                 {'oauth:active_provider': 'zai'})
                request['pathsOnly'] = [str(path)]
                again = scan.collect(request)
                self.assertEqual(result['documents'], again['documents'])
                self.assertEqual(path.read_bytes(), raw)


if __name__ == '__main__':
    unittest.main()
