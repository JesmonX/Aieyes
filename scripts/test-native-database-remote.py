"""Actual SQLite reads against disposable native schemas; no SSH or credentials."""
import contextlib
import importlib.util
import json
import pathlib
import sqlite3
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('native', ROOT / 'scripts/native_database_remote.py')
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)


def create(path, provider):
    with contextlib.closing(sqlite3.connect(path)) as db, db:
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
    def test_history_append_and_old_pending_completion_read_two_rows_at_both_sizes(self):
        for provider in ('opencode', 'zcode'):
            for count in (10, 3000):
                with self.subTest(provider=provider, count=count), tempfile.TemporaryDirectory() as directory:
                    path = pathlib.Path(directory) / 'native.db'
                    create(path, provider)
                    with contextlib.closing(sqlite3.connect(path)) as db, db:
                        for number in range(count):
                            add(db, provider, number, complete=number != 0)
                    before = path.read_bytes()
                    queue = Queue()
                    self.assertEqual(queue.scan(path, provider)['scannedRecords'], count)
                    self.assertEqual(before, path.read_bytes())
                    self.assertNotIn('PRIVATE_', json.dumps(queue.frames))
                    with patch.object(m, 'connect', side_effect=AssertionError('unchanged file opened SQLite')):
                        self.assertEqual(queue.scan(path, provider), dict(readBytes=0, scannedRecords=0))
                    with contextlib.closing(sqlite3.connect(path)) as db, db:
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
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                for number in range(700):
                    add(db, 'opencode', number, complete=False)
            queue = Queue()
            queue.scan(path, 'opencode')
            with contextlib.closing(sqlite3.connect(path)) as db, db:
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
            with contextlib.closing(sqlite3.connect(path)) as db, db:
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
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                add(db, 'opencode', 0)
                db.execute('CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,time_created INTEGER,time_updated INTEGER,data TEXT)')
            queue = Queue()
            queue.scan(path, 'opencode')
            self.assertEqual(queue.cursor['table'], 'message')
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                db.execute("INSERT INTO session_message SELECT id,session_id,'assistant',time_created,time_updated,json_set(data,'$.model',json_object('id','fixture','providerID','provider')) FROM message")
            queue.scan(path, 'opencode')
            self.assertEqual(queue.cursor['table'], 'session_message')
            self.assertTrue(queue.frames[0]['nativeReset'])
            replacement = path.with_name('replacement.db')
            create(replacement, 'opencode')
            with contextlib.closing(sqlite3.connect(replacement)) as db, db:
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
                    with contextlib.closing(sqlite3.connect(path)) as db, db:
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
                    with contextlib.closing(sqlite3.connect(path)) as db, db:
                        settle(db, provider, count - 1)
                        add(db, provider, count, complete=False)
                    files, scanned = m.sessions(path, provider, cursor, now)
                    self.assertEqual(scanned, 2)
                    self.assertEqual(files[0]['events'][-1]['turn'], str(count))
                    with contextlib.closing(sqlite3.connect(path)) as db, db:
                        settle(db, provider, count)
                    files, scanned = m.sessions(path, provider, files[-1], now)
                    self.assertEqual(scanned, 1)
                    self.assertEqual(files[0]['events'][-1]['kind'], 'complete')

    def test_visible_session_cursor_never_exceeds_256_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / 'native.db'
            create(path, 'opencode')
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                for number in range(700):
                    add(db, 'opencode', number, session=str(number))
            now = int(time.time())
            files, _ = m.sessions(path, 'opencode', {}, now)
            self.assertEqual(len(files[-1]['continuation']['rows']), 256)
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                add(db, 'opencode', 700, session='new')
            files, scanned = m.sessions(path, 'opencode', files[-1], now)
            self.assertEqual(scanned, 257)
            self.assertEqual(len(files), 257)
            self.assertTrue(any(f['key'].endswith('#new') for f in files))


if __name__ == '__main__':
    unittest.main()
