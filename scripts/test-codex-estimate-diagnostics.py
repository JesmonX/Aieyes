#!/usr/bin/env python3
import hashlib
import importlib.util
import json
import pathlib
import sqlite3
import tempfile
import time
import unittest

spec = importlib.util.spec_from_file_location('diagnostics', pathlib.Path(__file__).with_name('diagnose-codex-estimates.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class DiagnosticsTests(unittest.TestCase):
    def test_legacy_report_is_anonymous_read_only_and_allow_listed(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / 'aieyes.sqlite'
            db = sqlite3.connect(path)
            db.executescript('CREATE TABLE kv(key TEXT PRIMARY KEY,value TEXT);CREATE TABLE estimates(account_key TEXT,payload TEXT);')
            private = 'PRIVATE_EMAIL_TOKEN_PATH'
            settings = dict(accounts=[dict(id=private, name=private, provider='codex')], sources=[dict(id=private, path=private, hostId=private, provider='codex', codexHomeId=private)])
            db.execute('INSERT INTO kv VALUES(?,?)', ('settings', json.dumps(settings)))
            e = dict(accountKey='codex:' + private, checkpointAt=int(time.time()), calculationNote=private, reason=private, prices=[private], cost=2, automaticSegments=[dict(valid=True, attributionVersion=1, private=private)], rollingExcludedReason=private)
            db.execute('INSERT INTO estimates VALUES(?,?)', ('codex:' + private, json.dumps(e)))
            db.commit()
            db.close()
            before = hashlib.sha256(path.read_bytes()).hexdigest()
            result = module.diagnose(path)
            self.assertNotIn(private, json.dumps(result))
            self.assertEqual(result['estimates'][0]['account'], 'account-1')
            self.assertTrue(result['estimates'][0]['rollingExcluded'])
            self.assertEqual(before, hashlib.sha256(path.read_bytes()).hexdigest())


if __name__ == '__main__':
    unittest.main()
