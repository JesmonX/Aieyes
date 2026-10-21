"""Read-only SSH config transport, using temporary fixtures only."""
import importlib.util
import os
import pathlib
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('account_scan', pathlib.Path(__file__).with_name('remote_account_scan.py'))
scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scan)


class NativeTransport(unittest.TestCase):
    def test_remote_environment_and_reference_only_reads(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory)
            root = home / 'custom'
            root.mkdir()
            (root / 'auth.json').write_text('{"token":"PRIVATE"}')
            (root / 'settings.json').write_text('{"key":"OTHER-PRIVATE"}')
            request = dict(provider='codex', roots=[], env='CODEX_HOME', patterns=['auth.json', 'settings.json'], primary='~/.codex')
            with patch.dict(os.environ, {'HOME': str(home), 'CODEX_HOME': str(root)}):
                result = scan.collect(request)
                self.assertEqual(len(result['documents']), 2)
                self.assertEqual(result['roots'], [str(root.resolve())])
                request.update(roots=[str(root)], env='', patterns=[], pathsOnly=[str(root / 'auth.json')])
                only = scan.collect(request)
                self.assertEqual([r['path'] for r in only['documents']], [str(root / 'auth.json')])
                self.assertEqual((root / 'auth.json').read_text(), '{"token":"PRIVATE"}')

    def test_relative_environment_and_large_or_invalid_inputs_are_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / 'auth.json').write_bytes(b'x' * (scan.LIMIT + 1))
            request = dict(provider='codex', roots=[directory], env='CODEX_HOME', patterns=['auth.json'])
            with patch.dict(os.environ, {'CODEX_HOME': 'relative-root'}):
                self.assertTrue(scan.collect(request)['errors'])
            request['env'] = ''
            result = scan.collect(request)
            self.assertFalse(result['documents'])
            self.assertTrue(result['errors'])


if __name__ == '__main__':
    unittest.main()
