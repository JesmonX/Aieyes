"""Qoder setup must preserve existing user startup files on read failures."""
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from script_loader import load_script

env = load_script('qoder_env', 'qoder_env_remote.py')


class QoderEnvironmentTests(unittest.TestCase):
    def test_existing_bytes_and_mode_survive_idempotent_updates(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / '.profile'
            original = b'export KEEP=\xff\n'
            path.write_bytes(original)
            path.chmod(0o600)
            self.assertTrue(env._merge(str(path)))
            self.assertFalse(env._merge(str(path)))
            self.assertTrue(path.read_bytes().startswith(original))
            self.assertEqual(path.read_bytes().count(b'export QODER_EXPOSE_TOKEN_USAGE=true'), 1)
            self.assertEqual(path.read_bytes().count(b'export QODERCN_EXPOSE_TOKEN_USAGE=true'), 1)
            if os.name != 'nt':
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_read_failure_does_not_replace_existing_file(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / '.profile'
            original = b'export KEEP=1\n'
            path.write_bytes(original)
            with patch('builtins.open', side_effect=PermissionError('fixture read denied')):
                with self.assertRaises(PermissionError):
                    env._merge(str(path))
            self.assertEqual(path.read_bytes(), original)
            with self.assertRaises(OSError):
                env._merge(directory)

    def test_missing_file_is_created(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / '.profile'
            self.assertTrue(env._merge(str(path)))
            self.assertEqual(path.read_bytes(), env._block())


if __name__ == '__main__':
    unittest.main()
