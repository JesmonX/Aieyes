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

    def test_gaps_keep_fingerprint_candidates_separate_from_verified_attribution(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary)/'aieyes.sqlite'
            db = sqlite3.connect(path)
            db.executescript("""CREATE TABLE kv(key TEXT,value TEXT);CREATE TABLE estimates(account_key TEXT,payload TEXT);
                CREATE TABLE events(id TEXT,provider TEXT,stamp INTEGER,payload TEXT,cost REAL);
                CREATE TABLE event_sources(event_id TEXT,source_id TEXT);
                CREATE TABLE codex_account_timeline(id INTEGER PRIMARY KEY,source_id TEXT,stamp INTEGER,identity_key TEXT,kind TEXT,uncertain_from INTEGER);
                CREATE TABLE codex_session_creators(source_id TEXT,session_id TEXT,identity_key TEXT,created_at INTEGER,conflict INTEGER);
                CREATE TABLE codex_request_limits(source_id TEXT,request_id TEXT,payload TEXT);
                CREATE TABLE quota_history(account_key TEXT,stamp INTEGER,payload TEXT);""")
            settings=dict(sources=[dict(id='s',provider='codex')],accounts=[dict(id='a',provider='codex',identityKey='A'),dict(id='b',provider='codex',identityKey='B')])
            db.execute('INSERT INTO kv VALUES(?,?)',('settings',json.dumps(settings)))
            db.execute("INSERT INTO codex_session_creators VALUES('s','thread','A',900,0)")
            for stamp in [1000,1020,5000]:db.execute("INSERT INTO codex_account_timeline(source_id,stamp,identity_key,kind) VALUES('s',?,'A','observed')",(stamp,))
            def quota(reset):return dict(origin='live',plan='plus',windows=[dict(windowMinutes=300,resetsAt=reset)])
            db.execute('INSERT INTO quota_history VALUES(?,?,?)',('codex:a',1010,json.dumps(quota(10000))))
            db.execute('INSERT INTO quota_history VALUES(?,?,?)',('codex:b',5000,json.dumps(quota(20000))))
            for i,(start,end,reset) in enumerate([(1005,1010,10000),(1800,1810,10000),(1810,1820,20000),(1820,1830,30000)]):
                event=dict(id=str(i),sessionId='thread',intervalStart=start,timestamp=end)
                db.execute('INSERT INTO events VALUES(?,?,?,?,?)',(str(i),'codex',end,json.dumps(event),1))
                db.execute('INSERT INTO event_sources VALUES(?,?)',(str(i),'s'))
                db.execute('INSERT INTO codex_request_limits VALUES(?,?,?)',('s',str(i),json.dumps(quota(reset))))
            db.commit();db.close()
            report=module.diagnose(path,session_id='thread')['session']
            self.assertEqual(report['eventCount'],4)
            self.assertEqual(report['provisionalFingerprintCandidates'],{'account-1':1,'account-2':1,'unmatched':1})
            self.assertFalse(report['provisionalCandidatesAreValuationEligible'])
            self.assertEqual(sum(g['events'] for g in report['observationGaps']),3)


if __name__ == '__main__':
    unittest.main()
