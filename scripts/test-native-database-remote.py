"""Actual SQLite reads against disposable native schemas; no SSH or credentials."""
import contextlib
import importlib.util
import json
import os
import pathlib
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('native', ROOT / 'scripts/native_database_remote.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


@contextlib.contextmanager
def write_database(path):
    previous = path.stat().st_mtime_ns if path.exists() else 0
    with contextlib.closing(sqlite3.connect(path)) as db, db:
        yield db
    # These tests model writes between polls. Give each committed transaction a
    # distinct timestamp even when the filesystem clock coalesces fast writes.
    current = path.stat()
    os.utime(path, ns=(current.st_atime_ns, max(current.st_mtime_ns, previous + 1_000_000_000)))


def create(path, provider):
    with write_database(path) as db:
        if provider == 'opencode':
            db.executescript('CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);CREATE TABLE part(message_id TEXT,data TEXT);CREATE INDEX part_message ON part(message_id);')
        else:
            db.executescript('CREATE TABLE model_usage(id TEXT,session_id TEXT,turn_id TEXT,status TEXT,model_id TEXT,completed_at INTEGER,input_tokens INTEGER,output_tokens INTEGER,cache_read_input_tokens INTEGER,cache_creation_input_tokens INTEGER,reasoning_tokens INTEGER,provider_total_tokens INTEGER,computed_total_tokens INTEGER);CREATE TABLE turn_usage(session_id TEXT,turn_id TEXT,status TEXT,started_at INTEGER,completed_at INTEGER);')


def add(db, provider, number, complete=True, stamp=None, session='s'):
    stamp = stamp or int(time.time() * 1000) + number
    if provider == 'opencode':
        data = dict(role='assistant', time=dict(completed=stamp if complete else None), finish='stop' if complete else None,
                    modelID='fixture', providerID='provider', content='PRIVATE_TRANSCRIPT',
                    tokens=dict(input=10, output=2, reasoning=1, cache=dict(read=3, write=0), private='PRIVATE_COUNTER'))
        db.execute('INSERT INTO message VALUES(?,?,?,?,?)', (str(number), session, stamp, stamp, json.dumps(data)))
    else:
        db.execute('INSERT INTO model_usage VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)', (str(number), session, str(number), 'completed' if complete else 'running', 'fixture', stamp if complete else 0, 10, 3, 3, 0, 1, 16, 16))
        db.execute('INSERT INTO turn_usage VALUES(?,?,?,?,?)', (session, str(number), 'completed' if complete else 'running', stamp, stamp if complete else 0))


def settle(db, provider, number):
    if provider == 'opencode':
        db.execute("UPDATE message SET data=json_set(data,'$.time.completed',?,'$.finish','stop') WHERE id=?", (int(time.time() * 1000), str(number)))
    else:
        stamp = int(time.time() * 1000)
        db.execute("UPDATE model_usage SET completed_at=?,status='completed' WHERE id=?", (stamp, str(number)))
        db.execute("UPDATE turn_usage SET completed_at=?,status='completed' WHERE turn_id=?", (stamp, str(number)))


class Queue:
    """Only pending selection is modeled here; core tests validate Token parsing."""
    def __init__(self):
        self.cursor = {}
        self.pending = set()
        self.completed = set()
        self.frames = []

    def request(self):
        after = self.cursor.get('pendingAfter', 0)
        pending = sorted(n for n in self.pending if n > after) or sorted(self.pending)
        return dict(self.cursor, pending=pending[:256], pendingMore=len(pending) > 256)

    def emit(self, kind, **frame):
        self.frames.append(frame)
        if frame['nativeReset']:
            self.pending.clear()
        for rowid in frame['gone']:
            self.pending.discard(rowid)
        for row in frame['nativeRows']:
            data = row['data']
            complete = data.get('completed', 0) and (data.get('status') == 'completed' or data.get('role') == 'assistant')
            if complete:
                self.pending.discard(row['rowid'])
                self.completed.add(row['id'])
            else:
                self.pending.add(row['rowid'])
        if frame['cursor'] is not None:
            self.cursor = frame['cursor']

    def scan(self, path, provider):
        self.frames = []
        return m.history(path, provider, self.request(), self.emit)


class NativeDatabaseTests(unittest.TestCase):
    def test_v2_fork_boundaries_preserve_new_requests_and_delegation(self):
        for boundary in ('before', 'through'):
            with self.subTest(boundary=boundary), tempfile.TemporaryDirectory() as directory:
                path = pathlib.Path(directory) / 'opencode.db'
                stamp = int(time.time() * 1000)
                with write_database(path) as db:
                    db.executescript('CREATE TABLE session_v2(id TEXT,parent_id TEXT,fork_session_id TEXT,fork_boundary TEXT);CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,seq INTEGER,time_created INTEGER,data TEXT);')
                    db.execute('INSERT INTO session_v2 VALUES(?,?,?,?)', ('child', None, 'parent', json.dumps(dict(type=boundary, messageID='boundary'))))
                    db.execute("INSERT INTO session_v2 VALUES('worker','parent',NULL,NULL)")
                    for session, identity, seq in [('parent','original',1), ('parent','boundary',2), ('child','copy',1), ('child','boundary-copy',2), ('child','new',3), ('worker','independent',1)]:
                        data = dict(time=dict(created=stamp, completed=stamp), model=dict(id='fixture', providerID='api'), tokens=dict(input=100, output=0), content='PRIVATE_TRANSCRIPT')
                        db.execute('INSERT INTO session_message VALUES(?,?,?,?,?,?)', (identity, session, 'assistant', seq, stamp, json.dumps(data)))
                original = path.read_bytes()
                queue = Queue()
                queue.scan(path, 'opencode')
                rows = {r['id']: r['data'] for frame in queue.frames for r in frame['nativeRows']}
                self.assertEqual({key for key, row in rows.items() if row['inherited']}, {'copy', 'boundary-copy'} if boundary == 'through' else {'copy'})
                self.assertFalse(rows['new']['inherited'])
                self.assertFalse(rows['independent']['inherited'])
                self.assertNotIn('PRIVATE', json.dumps(queue.frames))
                self.assertEqual(path.read_bytes(), original)
                self.assertEqual(queue.scan(path, 'opencode')['readBytes'], 0)

    def test_mixed_tables_and_compaction_have_separate_cursors_and_no_private_payload(self):
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'native.db';create(path,'opencode');at=int(time.time()*1000)
            with write_database(path) as db:
                add(db,'opencode',1)
                db.execute('CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,time_created INTEGER,time_updated INTEGER,data TEXT)')
                data=dict(model=dict(id='fixture',providerID='test'),status='completed',time=dict(created=at),tokens=dict(input=10,output=5,reasoning=0,cache=dict(read=0,write=0)),summary='PRIVATE')
                db.execute('INSERT INTO session_message VALUES(?,?,?,?,?,?)',('compact','other','compaction',at,at,json.dumps(data)))
            frames=[];saved={}
            def emit(kind,**frame):
                frames.append(frame);saved[frame['path']]=frame['cursor']
            stats=m.history_all(path,'opencode',saved,emit)
            self.assertEqual(stats['scannedRecords'],2);self.assertEqual(len(saved),2)
            rows=[r for f in frames for r in f['nativeRows']]
            self.assertEqual({r['id'] for r in rows},{'1','compact'})
            self.assertEqual(next(r for r in rows if r['id']=='compact')['data']['completed'],at)
            self.assertNotIn('PRIVATE',json.dumps(frames))
            self.assertEqual(m.history_all(path,'opencode',saved,emit)['scannedRecords'],0)

    def test_v2_inline_tools_idle_and_old_event_time(self):
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'native.db';now=int(time.time());at=now*1000
            with write_database(path) as db:
                db.execute('CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,time_created INTEGER,time_updated INTEGER,data TEXT)')
                db.execute('CREATE TABLE session(id TEXT,parent_id TEXT)');db.execute("INSERT INTO session VALUES('child','parent')")
                data=dict(time=dict(created=at),content=[dict(type='tool',name='question',state=dict(status='running',input='PRIVATE'))])
                db.execute('INSERT INTO session_message VALUES(?,?,?,?,?,?)',('tool','child','assistant',at,at,json.dumps(data)))
                old=at-172800000
                db.execute('INSERT INTO session_message VALUES(?,?,?,?,?,?)',('old','old','idle',old,at,json.dumps(dict(time=dict(created=old),outcome='succeeded'))))
            files,_=m.sessions(path,'opencode',{},now)
            self.assertEqual(len(files),2);self.assertEqual(files[0]['events'][-1]['kind'],'PermissionRequest')
            self.assertEqual(files[0]['events'][-1]['parentSession'],'parent');self.assertNotIn('PRIVATE',json.dumps(files))
            with write_database(path) as db:
                db.execute('INSERT INTO session_message VALUES(?,?,?,?,?,?)',('idle','child','idle',at+1,at+1,json.dumps(dict(time=dict(created=at+1),outcome='succeeded'))))
            next_files,_=m.sessions(path,'opencode',files[-1],now)
            self.assertEqual(next_files[0]['events'][-1]['kind'],'complete')

    def test_history_append_and_old_pending_completion_read_two_rows_at_both_sizes(self):
        for provider in ('opencode', 'zcode'):
            for count in (10, 3000):
                with self.subTest(provider=provider, count=count), tempfile.TemporaryDirectory() as directory:
                    path = pathlib.Path(directory) / 'native.db'
                    create(path, provider)
                    with write_database(path) as db:
                        for number in range(count):
                            add(db, provider, number, complete=number != 0)
                    before = path.read_bytes()
                    queue = Queue()
                    self.assertEqual(queue.scan(path, provider)['scannedRecords'], count)
                    self.assertEqual(before, path.read_bytes())
                    self.assertNotIn('PRIVATE_', json.dumps(queue.frames))
                    with patch.object(m, 'connect', side_effect=AssertionError('unchanged file opened SQLite')):
                        self.assertEqual(queue.scan(path, provider), dict(readBytes=0, scannedRecords=0))
                    with write_database(path) as db:
                        settle(db, provider, 0)
                        add(db, provider, count)
                    self.assertEqual(queue.scan(path, provider)['scannedRecords'], 2)
                    self.assertEqual(len(queue.completed), count + 1)
                    self.assertFalse(queue.pending)
                    self.assertEqual(queue.scan(path, provider)['readBytes'], 0)

    def test_pending_sweep_survives_writes_stopping_and_disappearing_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'native.db'
            create(path, 'opencode')
            with write_database(path) as db:
                for number in range(700):
                    add(db, 'opencode', number, complete=False)
            queue = Queue()
            queue.scan(path, 'opencode')
            with write_database(path) as db:
                settle(db, 'opencode', 693)
                db.execute("DELETE FROM message WHERE id='10'")
            for _ in range(3):
                self.assertLessEqual(queue.scan(path, 'opencode')['scannedRecords'], 256)
            self.assertIn('693', queue.completed)
            self.assertNotIn(11, queue.pending)
            self.assertNotIn(694, queue.pending)
            self.assertEqual(queue.scan(path, 'opencode')['scannedRecords'], 0)

    def test_partial_transport_resumes_at_committed_row_without_skipping_later_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'native.db'
            create(path, 'opencode')
            with write_database(path) as db:
                for number in range(700):
                    add(db, 'opencode', number)
            queue = Queue()
            def interrupted(kind, **frame):
                if queue.frames:
                    raise ConnectionError('test interruption')
                queue.emit(kind, **frame)
            with self.assertRaises(ConnectionError):
                m.history(path, 'opencode', queue.request(), interrupted)
            self.assertEqual(queue.cursor['offset'], 256)
            self.assertEqual(queue.scan(path, 'opencode')['scannedRecords'], 444)
            self.assertEqual(len(queue.completed), 700)

    def test_schema_migration_and_file_replacement_reset_the_cursor(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'native.db'
            create(path, 'opencode')
            with write_database(path) as db:
                add(db, 'opencode', 0)
                db.execute('CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,time_created INTEGER,time_updated INTEGER,data TEXT)')
            queue = Queue()
            queue.scan(path, 'opencode')
            self.assertEqual(queue.cursor['table'], 'message')
            with write_database(path) as db:
                db.execute("INSERT INTO session_message SELECT id,session_id,'assistant',time_created,time_updated,json_set(data,'$.model',json_object('id','fixture','providerID','provider')) FROM message")
            queue.scan(path, 'opencode')
            self.assertEqual(queue.cursor['table'], 'session_message')
            self.assertTrue(queue.frames[0]['nativeReset'])
            replacement = path.with_name('replacement.db')
            create(replacement, 'opencode')
            with write_database(replacement) as db:
                add(db, 'opencode', 1)
            replacement.replace(path)
            queue.scan(path, 'opencode')
            self.assertTrue(queue.frames[0]['nativeReset'])
            self.assertEqual(queue.completed, {'0', '1'})

    def test_session_updates_are_bounded_and_cache_keeps_no_transcript(self):
        for provider in ('opencode', 'zcode'):
            for count in (10, 3000):
                with self.subTest(provider=provider, count=count), tempfile.TemporaryDirectory() as directory:
                    path = pathlib.Path(directory) / 'native.db'
                    create(path, provider)
                    now = int(time.time())
                    with write_database(path) as db:
                        for number in range(count):
                            add(db, provider, number, complete=number != count - 1, stamp=now * 1000 - count + number)
                    files, scanned = m.sessions(path, provider, {}, now)
                    self.assertEqual(scanned, count)
                    cursor = files[-1]
                    self.assertEqual(len(cursor['continuation']['rows']), 1)
                    self.assertNotIn('PRIVATE_', json.dumps(files))
                    with patch.object(m, 'connect', side_effect=AssertionError('unchanged file opened SQLite')):
                        cached, scanned = m.sessions(path, provider, cursor, now)
                        self.assertEqual(scanned, 0)
                        self.assertFalse(cached[0]['reset'])
                    with write_database(path) as db:
                        settle(db, provider, count - 1)
                        add(db, provider, count, complete=False)
                    files, scanned = m.sessions(path, provider, cursor, now)
                    self.assertEqual(scanned, 2)
                    self.assertEqual(files[0]['events'][-1]['turn'], str(count))
                    with write_database(path) as db:
                        settle(db, provider, count)
                    files, scanned = m.sessions(path, provider, files[-1], now)
                    self.assertEqual(scanned, 1)
                    self.assertEqual(files[0]['events'][-1]['kind'], 'complete')

    def test_kilo_run_sessions_are_headless_and_tui_sessions_count(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'kilo.db'
            create(path, 'opencode')
            run = json.dumps([{'permission': 'question', 'pattern': '*', 'action': 'deny'}])
            with write_database(path) as db:
                db.execute('CREATE TABLE session(id TEXT PRIMARY KEY,parent_id TEXT,permission TEXT)')
                db.execute('INSERT INTO session VALUES(?,?,?),(?,?,?)', ('run', None, run, 'tui', None, None))
                add(db, 'opencode', 1, session='run')
                add(db, 'opencode', 2, session='tui')
            files, _ = m.sessions(path, 'kilo', {}, int(time.time()))
            keys = [f['key'].rsplit('#', 1)[-1] for f in files if f['key'].count('#')]
            self.assertEqual(keys, ['tui'])

    def test_visible_session_cursor_never_exceeds_256_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'native.db'
            create(path, 'opencode')
            with write_database(path) as db:
                for number in range(700):
                    add(db, 'opencode', number, session=str(number))
            now = int(time.time())
            files, _ = m.sessions(path, 'opencode', {}, now)
            self.assertEqual(len(files[-1]['continuation']['rows']), 256)
            with write_database(path) as db:
                add(db, 'opencode', 700, session='new')
            files, scanned = m.sessions(path, 'opencode', files[-1], now)
            self.assertEqual(scanned, 257)
            self.assertEqual(len(files), 257)
            self.assertTrue(any(f['key'].endswith('#new') for f in files))

    def test_workbuddy_history_keeps_requests_and_rechecks_same_timestamp_updates(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'workbuddy.db'
            with write_database(path) as db:
                db.executescript((ROOT / 'crates/core/tests/fixtures/workbuddy/workbuddy.sql').read_text())
            original = path.read_bytes()
            frames = []
            emit = lambda kind, **values: frames.append(dict(kind=kind, **values))
            result = m.workbuddy_history(path, {}, emit)
            rows = [row for frame in frames for row in frame['events']]
            self.assertEqual(len(rows), 6)
            self.assertAlmostEqual(sum(row['nativeCredits']['credits'] for row in rows), 104.375)
            self.assertTrue(all(row['model'] == 'unknown' and row['tokens'] == {} for row in rows))
            self.assertNotIn('PRIVATE', json.dumps(frames))
            self.assertEqual(path.read_bytes(), original)
            self.assertEqual(result['scannedRecords'], 6)
            cursor = frames[-1]['cursor']
            with patch.object(m, 'connect', side_effect=AssertionError('unchanged WorkBuddy database reopened')):
                self.assertEqual(m.workbuddy_history(path, cursor, emit), dict(readBytes=0, scannedRecords=0))
            with write_database(path) as db:
                db.execute("UPDATE session_usage SET credit_json=? WHERE session_id='background'", ('{"req-4":3,"req-new":0.25}',))
                db.execute('INSERT INTO session_usage VALUES(?,?,?,?,?)', ('a-new', 0, 0, cursor['offset'], '{"req-same-time":0.5}'))
                db.execute("UPDATE sessions SET model='PRIVATE_CURRENT_MODEL'")
            frames.clear()
            m.workbuddy_history(path, cursor, emit)
            changed = {row['id']: row for frame in frames for row in frame['events']}
            self.assertEqual(set(changed), {'req-4', 'req-new', 'req-same-time'})
            previous = next(row for row in rows if row['id'] == 'req-4')
            self.assertEqual(changed['req-4']['inheritance'], previous['inheritance'])
            self.assertEqual(changed['req-4']['nativeCredits']['credits'], 3)
            self.assertNotIn('PRIVATE', json.dumps(frames))

    def test_workbuddy_history_pages_many_sessions_and_survives_database_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'workbuddy.db'
            with write_database(path) as db:
                db.execute('CREATE TABLE session_usage(session_id TEXT PRIMARY KEY,updated_at INTEGER,credit_json TEXT)')
                db.executemany('INSERT INTO session_usage VALUES(?,?,?)', [(f'session-{n:04}', 2000, json.dumps({f'request-{n}': .5})) for n in range(600)])
            frames = []
            emit = lambda kind, **values: frames.append(dict(kind=kind, **values))
            result = m.workbuddy_history(path, {}, emit)
            self.assertEqual(result['scannedRecords'], 600)
            self.assertEqual(sum(len(frame['events']) for frame in frames), 600)
            self.assertTrue(all(len(json.dumps(frame).encode()) < 1024 * 1024 for frame in frames))
            cursor = frames[-1]['cursor']
            path.rename(path.with_suffix('.previous'))
            with write_database(path) as db:
                db.execute('CREATE TABLE session_usage(session_id TEXT PRIMARY KEY,updated_at INTEGER,credit_json TEXT)')
                db.execute('INSERT INTO session_usage VALUES(?,?,?)', ('replacement', 1000, '{"replacement-request":1}'))
            frames.clear()
            m.workbuddy_history(path, cursor, emit)
            self.assertEqual([row['id'] for frame in frames for row in frame['events']], ['replacement-request'])

    def test_workbuddy_credits_travel_through_the_embedded_history_protocol(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'workbuddy.db'
            with write_database(path) as db:
                db.executescript((ROOT / 'crates/core/tests/fixtures/workbuddy/workbuddy.sql').read_text())
            request = json.dumps(dict(protocol=2, path=directory, provider='workbuddy', cursors={}))
            driver = "import json,pathlib,sys;p=pathlib.Path(sys.argv[1]);exec(compile((p/'remote_history.py').read_text(),'<embedded-history>','exec'),{'__name__':'__main__','REQUEST':json.loads(sys.argv[2]),'NATIVE_DATABASE_SCRIPT':(p/'native_database_remote.py').read_text()})"
            output = subprocess.run([sys.executable, '-c', driver, str(ROOT / 'scripts'), request], capture_output=True, text=True, check=True)
            self.assertNotIn('PRIVATE', output.stdout)
            frames = [json.loads(line) for line in output.stdout.splitlines()]
            self.assertEqual(frames[-1]['kind'], 'done')
            self.assertFalse(frames[-1]['partial'])
            self.assertEqual(sum(len(frame.get('events', [])) for frame in frames), 6)

    def test_workbuddy_sessions_skip_automation_and_cache_unchanged_files(self):
        fixture = ROOT / 'crates/core/tests/fixtures/workbuddy/workbuddy.sql'
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'workbuddy.db'
            with write_database(path) as db:
                db.executescript(fixture.read_text())
            now = int(time.time())
            files, scanned = m.sessions(path, 'workbuddy', {}, now)
            kinds = sorted((f['key'].rsplit('#', 1)[-1], f['events'][-1]['kind']) for f in files if '#' in f['key'])
            # Same rows and phases as Rust `workbuddy_lists_recent_user_sessions_and_skips_automation`.
            self.assertEqual(kinds, [('earlier-user', 'complete'), ('user-chat', 'complete'), ('user-live', 'start'), ('user-waiting', 'PermissionRequest')])
            self.assertEqual(scanned, 4)
            self.assertNotIn('PRIVATE', json.dumps(files))
            with patch.object(m, 'connect', side_effect=AssertionError('unchanged file opened SQLite')):
                cached, scanned = m.sessions(path, 'workbuddy', files[-1], now)
            self.assertEqual(scanned, 0)
            self.assertEqual(len(cached), 5)
            self.assertEqual(m.workbuddy_phase('Tool-Executing'), 'tool')


if __name__ == '__main__':
    unittest.main()
