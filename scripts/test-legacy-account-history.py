import importlib.util
import json
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('repair', Path(__file__).with_name('repair-legacy-account-history.py'))
repair = importlib.util.module_from_spec(spec)
spec.loader.exec_module(repair)


class LegacyHistoryTests(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(':memory:')
        self.db.executescript('''
            CREATE TABLE kv(key TEXT PRIMARY KEY,value TEXT);
            CREATE TABLE events(id TEXT PRIMARY KEY,provider TEXT,account_id TEXT,model TEXT,stamp INTEGER,payload TEXT,cost REAL,priced_tokens INTEGER,price TEXT);
            CREATE TABLE event_sources(event_id TEXT REFERENCES events(id),source_id TEXT,PRIMARY KEY(event_id,source_id));
            CREATE TABLE source_identities(source_id TEXT,provider TEXT,account_id TEXT,PRIMARY KEY(source_id,provider,account_id));
        ''')
        self.db.execute('PRAGMA foreign_keys=ON')
        self.db.execute('INSERT INTO kv VALUES(?,?)', ('settings', json.dumps({'sources': [{'id': 'current', 'provider': 'codex'}], 'accounts': []})))
        self.db.execute('INSERT INTO kv VALUES(?,?)', ('deleted-account:codex:old', json.dumps({'id': 'old', 'provider': 'codex'})))
        self.db.execute('INSERT INTO source_identities VALUES(?,?,?)', ('removed', 'codex', 'old'))

    def tearDown(self):
        self.db.close()

    def event(self, source, account, identity='session:0:1:cumulative', stable=True, cost=2):
        key = repair.event_id('codex', account or 'source:' + source, identity)
        payload = dict(id=key, sourceId=source, accountId=account, provider='codex', sessionId='session',
                       model='model', timestamp=1, tokens=dict(input=100, output=1), attribution='event')
        if stable:
            payload['importIdentity'] = identity
        self.db.execute('INSERT INTO events VALUES(?,?,?,?,?,?,?,?,?)',
                        (key, 'codex', account, 'model', 1, json.dumps(payload), cost, 101, '{"frozen":true}'))
        self.db.execute('INSERT INTO event_sources VALUES(?,?)', (key, source))
        return key

    def test_proven_duplicate_preserves_price_sources_and_replay_scopes(self):
        old = self.event('removed', 'old', stable=False, cost=7)
        current = self.event('current', '', cost=9)
        aliases = {'removed': 'current'}
        groups, rows, memberships = repair.plan(self.db, aliases)
        self.assertEqual(groups, [(current, [old])])
        repair.repair(self.db, aliases, groups, rows, memberships)
        self.assertEqual(self.db.execute('SELECT id,cost FROM events').fetchall(), [(current, 7)])
        self.assertEqual(set(self.db.execute('SELECT source_id FROM event_sources').fetchall()), {('current',), ('removed',)})
        self.assertIn(('current', 'codex', 'old'), self.db.execute('SELECT * FROM source_identities').fetchall())
        self.assertEqual(repair.plan(self.db, aliases)[0], [])
        self.assertEqual(self.db.execute('PRAGMA foreign_key_check').fetchall(), [])

    def test_same_usage_without_matching_hash_is_retained(self):
        self.event('removed', 'old', identity='different-identity', stable=False)
        self.event('current', '')
        self.assertEqual(repair.plan(self.db, {'removed': 'current'})[0], [])

    def test_active_account_is_never_merged(self):
        self.event('removed', 'active', stable=False)
        self.event('current', '')
        self.assertEqual(repair.plan(self.db, {'removed': 'current'})[0], [])

    def test_unrelated_source_without_alias_is_retained(self):
        self.event('removed', 'old', stable=False)
        self.event('current', '')
        self.assertEqual(repair.plan(self.db, {})[0], [])

    def test_token_mismatch_is_retained(self):
        old = self.event('removed', 'old', stable=False)
        self.event('current', '')
        raw = json.loads(self.db.execute('SELECT payload FROM events WHERE id=?', (old,)).fetchone()[0])
        raw['tokens']['input'] = 200
        self.db.execute('UPDATE events SET payload=? WHERE id=?', (json.dumps(raw), old))
        self.assertEqual(repair.plan(self.db, {'removed': 'current'})[0], [])

    def test_explicit_copied_source_requires_matching_hash_and_usage(self):
        settings = {'sources': [{'id': name, 'provider': 'codex'} for name in ['current', 'copy']]}
        self.db.execute("UPDATE kv SET value=? WHERE key='settings'", (json.dumps(settings),))
        old = self.event('removed', '', stable=False)
        copied = self.event('copy', '')
        aliases = {'removed': 'current'}
        self.assertEqual(repair.plan(self.db, aliases)[0], [])
        self.assertEqual(repair.plan(self.db, aliases, [('removed', 'copy')])[0], [(copied, [old])])

    def test_cli_backup_is_standalone_and_second_apply_is_idempotent(self):
        self.event('removed', 'old', stable=False)
        self.event('current', '')
        self.db.commit()
        with tempfile.TemporaryDirectory() as root:
            database = Path(root) / 'history.sqlite'
            destination = sqlite3.connect(database)
            self.db.backup(destination)
            destination.close()
            command = [sys.executable, str(Path(__file__).with_name('repair-legacy-account-history.py')),
                       '--database', str(database), '--source-alias', 'removed=current', '--apply']
            result = json.loads(subprocess.check_output(command, text=True))
            self.assertEqual(result['removedRows'], 1)
            backup = sqlite3.connect('file:' + result['backup'] + '?mode=ro', uri=True)
            self.assertEqual(backup.execute('SELECT count(*) FROM events').fetchone()[0], 2)
            backup.close()
            self.assertEqual(Path(result['backup']).stat().st_mode & 0o777, 0o600)
            self.assertEqual(json.loads(subprocess.check_output(command, text=True))['removedRows'], 0)


if __name__ == '__main__':
    unittest.main()
