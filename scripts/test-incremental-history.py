"""Protocol v2 regression fixtures; no SSH or personal histories are accessed."""
import contextlib
import io
import json
import os
import pathlib
import runpy
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = pathlib.Path(__file__).with_name('remote_history.py')


def collect(path, provider='custom', cursors=None, route_cursor=None):
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        try:
            runpy.run_path(str(SCRIPT), init_globals={'REQUEST': dict(path=str(path), provider=provider, protocol=2, cursors=cursors or {}, routeCursor=route_cursor or {})})
        except SystemExit as error:
            assert error.code == 0
    lines = output.getvalue().splitlines()
    assert all(len(line.encode()) + 1 <= 1024 * 1024 for line in lines)
    return [json.loads(line) for line in lines]


def cursors(frames):
    return {f['path']: f['cursor'] for f in frames if f['kind'] == 'batch' and f.get('cursor')}


def events(frames):
    return [e for f in frames if f['kind'] == 'batch' for e in f['events']]


def record(number, model='fixture'):
    return json.dumps(dict(id=str(number), sessionId='s', model=model, timestamp=1800000000, tokens=dict(input=number))) + '\n'


class IncrementalHistory(unittest.TestCase):
    def test_fork_resolves_archived_multilevel_parent_and_rechecks_changed_proof(self):
        for newline in ('\n', '\r\n'):
            with self.subTest(newline=newline), tempfile.TemporaryDirectory() as root:
                home=pathlib.Path(root);sessions=home/'sessions/2026/10/13';archive=home/'archived_sessions'
                sessions.mkdir(parents=True);archive.mkdir()
                def line(value): return (json.dumps(value,ensure_ascii=False)+newline).encode('utf-8')
                def meta(identity, base=None): return line(dict(type='session_meta',payload=dict(id=identity,history_base=base) if base else dict(id=identity)))
                def count(total): return line(dict(type='event_msg',timestamp=1800000000,payload=dict(type='token_count',info=dict(total_token_usage=dict(input_tokens=total)))))
                parent=meta('parent')+line(dict(type='response_item',payload=dict(text='PRIVATE_PARENT_TEXT 中文')))+count(900)
                middle=meta('middle',dict(thread_id='parent',end_byte_offset=len(parent)))
                child=meta('child',dict(thread_id='middle',end_byte_offset=len(middle)))+count(1000)
                (archive/'parent.jsonl').write_bytes(parent+count(9999));(archive/'middle.jsonl').write_bytes(middle)
                (sessions/'child.jsonl').write_bytes(child)
                with contextlib.closing(sqlite3.connect(home/'state_5.sqlite')) as db:
                    db.execute('CREATE TABLE threads(id TEXT PRIMARY KEY,rollout_path TEXT)')
                    db.execute('INSERT INTO threads VALUES(?,?)',('parent',str(archive/'parent.jsonl')))
                    db.commit()
                first=collect(sessions,'codex')
                inherited=events(first)[0]['inheritedHistory']
                self.assertEqual(inherited['tokens']['input'],900)
                self.assertNotIn('PRIVATE_PARENT_TEXT',json.dumps(first))
                self.assertEqual(collect(sessions,'codex',cursors(first))[-1]['readBytes'],0)
                (archive/'parent.jsonl').write_bytes(parent.replace(b'900',b'800')+count(9999))
                changed=collect(sessions,'codex',cursors(first))
                self.assertEqual(events(changed)[0]['inheritedHistory']['tokens']['input'],800)
                self.assertTrue(changed[0]['reset'])
                self.assertNotEqual(events(changed)[0]['inheritedHistory']['proof'],inherited['proof'])

    def test_missing_fork_parent_recovers_without_a_stale_missing_issue(self):
        for newline in ('\n', '\r\n'):
            with self.subTest(newline=newline), tempfile.TemporaryDirectory() as root:
                home=pathlib.Path(root)
                parent=(json.dumps(dict(type='session_meta',payload=dict(id='parent')))+newline).encode('utf-8')
                child=dict(type='session_meta',payload=dict(id='child',history_base=dict(thread_id='parent',end_byte_offset=len(parent))))
                (home/'child.jsonl').write_bytes((json.dumps(child)+newline).encode('utf-8'))
                first=collect(home,'codex');self.assertTrue(first[-1]['partial'])
                (home/'parent.jsonl').write_bytes(parent)
                restored=collect(home,'codex',cursors(first))
                self.assertFalse(restored[-1]['partial'])
                self.assertEqual(next(e for e in events(restored) if e.get('inheritedHistory'))['inheritedHistory']['tokens']['input'],0)

    def test_native_routes_are_incremental_and_never_export_private_request_fields(self):
        with tempfile.TemporaryDirectory() as root:
            home=pathlib.Path(root);sessions=home/'sessions/2026/10/13';sessions.mkdir(parents=True)
            with contextlib.closing(sqlite3.connect(home/'logs_2.sqlite')) as db:
                db.execute('CREATE TABLE logs(id INTEGER PRIMARY KEY,ts INTEGER,ts_nanos INTEGER,target TEXT,feedback_log_body TEXT,thread_id TEXT,process_uuid TEXT)')
                def insert(identity,body):
                    db.execute("INSERT INTO logs VALUES(?,1800000000,0,'codex_http_client::client',?,'session','process')",(identity,body));db.commit()
                insert(1,'url="https://secret:password@api.openai.com/v1/responses?api_key=PRIVATE" turn_id="turn" status=200 request_body="PROMPT"')
                insert(2,'url="https://api.openai.com/v1/models" status=200')
                first=collect(sessions,'codex');route=next(f for f in first if f['kind']=='routes')
                self.assertEqual(len(route['routes']),1)
                self.assertEqual(route['routes'][0]['category'],'api')
                self.assertEqual(route['routes'][0]['turnId'],'turn')
                for forbidden in ('secret','password','PRIVATE','PROMPT','api_key'): self.assertNotIn(forbidden,json.dumps(first))
                second=collect(sessions,'codex',route_cursor=route['cursor'])
                self.assertFalse(any(f['kind']=='routes' for f in second))
                insert(3,'url="https://chatgpt.com/backend-api/codex/responses" turn_id="next" status=200')
                third=collect(sessions,'codex',route_cursor=route['cursor'])
                new=next(f for f in third if f['kind']=='routes')
                self.assertEqual(len(new['routes']),1)
                self.assertEqual(new['routes'][0]['category'],'subscription')

    def test_append_partial_line_replacement_and_truncation(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / 'fixture.jsonl'
            path.write_text(record(1))
            first = collect(root)
            self.assertEqual(len(events(first)), 1)
            same = collect(root, cursors=cursors(first))
            self.assertEqual(events(same), [])
            self.assertEqual(same[-1]['readBytes'], 0)
            with path.open('a') as file:
                file.write(record(2) + record(3).rstrip('\n'))
            second = collect(root, cursors=cursors(first))
            self.assertEqual([e['id'] for e in events(second)], ['2'])
            self.assertFalse(next(f for f in second if f['kind'] == 'batch')['reset'])
            with path.open('a') as file:
                file.write('\n')
            third = collect(root, cursors=cursors(second))
            self.assertEqual([e['id'] for e in events(third)], ['3'])
            # Same path/size rewrite and inode replacement both restart parsing.
            path.write_text(record(4) + record(5) + record(6))
            rewrite = collect(root, cursors=cursors(third))
            self.assertTrue(rewrite[0]['reset'])
            self.assertEqual([e['id'] for e in events(rewrite)], ['4', '5', '6'])
            replacement = path.with_suffix('.replacement')
            replacement.write_text(record(7))
            os.replace(replacement, path)
            replaced = collect(root, cursors=cursors(rewrite))
            self.assertTrue(replaced[0]['reset'])
            path.write_text('')
            truncated = collect(root, cursors=cursors(replaced))
            self.assertEqual(truncated[0]['cursor']['offset'], 0)
            self.assertTrue(truncated[0]['reset'])

    def test_bounded_frames_and_restart_at_committed_checkpoint(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / 'fixture.jsonl'
            path.write_text(''.join(record(n, 'x' * 1800) for n in range(1700)))
            first = collect(root)
            batches = [f for f in first if f['kind'] == 'batch']
            self.assertGreater(len(batches), 3)
            # Simulate transport ending after one committed batch.
            resumed = collect(root, cursors=cursors(batches[:1]))
            combined = events(batches[:1]) + events(resumed)
            self.assertEqual([e['id'] for e in combined], [str(n) for n in range(1700)])
            self.assertEqual(collect(root, cursors=cursors(resumed))[-1]['readBytes'], 0)

    def test_nonstatistical_bytes_advance_and_private_text_never_leaves(self):
        with tempfile.TemporaryDirectory() as root:
            path = pathlib.Path(root) / 'fixture.jsonl'
            path.write_text((json.dumps({'type': 'response_item', 'payload': {'text': 'private-text' * 1000}}) + '\n') * 900)
            frames = collect(root, 'codex')
            self.assertEqual(events(frames), [])
            self.assertNotIn('private-text', json.dumps(frames))
            self.assertGreater(len(cursors(frames)), 0)
            self.assertGreater(len([f for f in frames if f['kind'] == 'batch']), 1)
            self.assertEqual(collect(root, 'codex', cursors(frames))[-1]['readBytes'], 0)

    def test_database_and_wal_fingerprints_and_failed_reads(self):
        with tempfile.TemporaryDirectory() as root:
            directory = pathlib.Path(root) / 'conversations'
            directory.mkdir()
            db = directory / 's.db'
            db.write_bytes(b'fixture')
            # Exercise incremental fingerprint/commit logic independently of the
            # protobuf parser, whose real SQLite fixtures live in core antigravity tests.
            with patch('sys.argv', ['collector', '/nonexistent', 'agy']), contextlib.redirect_stdout(io.StringIO()):
                module = runpy.run_path(str(SCRIPT))
            fn = module['_incremental']
            fn.__globals__['_agy_iter'] = lambda _: iter([{'event': json.loads(record(1))}])
            def scan(saved=None):
                output = io.StringIO()
                with contextlib.redirect_stdout(output):
                    fn(dict(path=root, provider='antigravity', cursors=saved or {}))
                return [json.loads(line) for line in output.getvalue().splitlines()]
            first = scan()
            self.assertEqual(len(events(first)), 1)
            self.assertEqual(events(scan(cursors(first))), [])
            wal = directory / 's.db-wal'
            wal.write_bytes(b'new transaction')
            second = scan(cursors(first))
            self.assertEqual(len(events(second)), 1)
            self.assertEqual(events(scan(cursors(second))), [])
            wal.unlink()
            self.assertEqual(len(events(scan(cursors(second)))), 1)
            fn.__globals__['_agy_iter'] = lambda _: iter([{'issue': dict(code='invalidRecord', message='fixture')}])
            failed = scan()
            self.assertTrue(failed[-1]['partial'])
            self.assertEqual(cursors(failed), {})


if __name__ == '__main__':
    unittest.main()
