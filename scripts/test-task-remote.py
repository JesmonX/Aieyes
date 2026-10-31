"""Official Cline/Roo layouts and real CLI metadata; no account or SSH required."""
import contextlib
import datetime
import importlib.util
import io
import json
import os
import pathlib
import runpy
import sqlite3
import tempfile
import time
import unittest
from unittest import mock

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('tasks', ROOT / 'scripts/task_remote.py')
tasks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tasks)
spec = importlib.util.spec_from_file_location('sessions', ROOT / 'scripts/session_remote.py')
sessions = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sessions)
CURRENT = json.loads((ROOT / 'crates/core/tests/fixtures/native-agents/cline-current.json').read_text())


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value))


def history(root, provider, cursors=None):
    output = io.StringIO()
    with contextlib.redirect_stdout(output):
        try:
            runpy.run_path(str(ROOT / 'scripts/remote_history.py'), init_globals={'REQUEST': dict(path=str(root), provider=provider, protocol=2, cursors=cursors or {}, preciseCounting=True)})
        except SystemExit as error:
            assert error.code == 0
    return [json.loads(line) for line in output.getvalue().splitlines()]


def events(frames):
    return [event for frame in frames for event in frame.get('events', [])]


def cursor(frames):
    return {frame['path']: frame['cursor'] for frame in frames if frame.get('cursor')}


class TaskTests(unittest.TestCase):
    def test_cline_root_subagent_and_override_paths_import_incrementally(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            write(root / 'sessions/root/root.messages.json', CURRENT)
            child = dict(CURRENT, sessionId='child')
            child['messages'] = json.loads(json.dumps(CURRENT['messages']))
            child['messages'][1]['id'] = 'child-request'
            write(root / 'sessions/root/worker__job.messages.json', child)
            first = history(root, 'cline')
            self.assertEqual({row['sessionId'] for row in events(first)}, {'cline-live-fixture', 'child'})
            self.assertEqual({row['tokens']['input'] for row in events(first)}, {4689})
            self.assertNotIn('PRIVATE', json.dumps(first))
            self.assertEqual(events(history(root, 'cline', cursor(first))), [])
            self.assertEqual(history(root, 'cline', cursor(first))[-1]['readBytes'], 0)
            db = root / 'db/sessions.db'
            db.parent.mkdir()
            external = root / 'separate/root.messages.json'
            write(external, CURRENT)
            with sqlite3.connect(db) as connection:
                connection.execute('CREATE TABLE sessions(session_id TEXT,messages_path TEXT)')
                connection.execute('INSERT INTO sessions VALUES(?,?)', ('external', str(external)))
            before = db.read_bytes()
            self.assertEqual(len(events(history(db, 'cline'))), 1)
            self.assertEqual(before, db.read_bytes())

    def test_legacy_compaction_pairing_and_summary_precedence(self):
        def row(kind, count, stamp):
            return dict(type='say', say=kind, ts=stamp, modelInfo=dict(modelId='known-model'), text=json.dumps(dict(tokensIn=count, tokensOut=1, request='PRIVATE')))
        for provider in ('cline', 'roo', 'kilo'):
            with self.subTest(provider=provider), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                path = root / 'tasks/one/ui_messages.json'
                write(path, [row('api_req_started', 10, 1000), row('api_req_started', 20, 2000)])
                first = events(history(root, provider))[0]
                self.assertEqual(first['tokens']['input'], 30)
                self.assertEqual(first['model'], 'known-model')
                write(path, [row('deleted_api_reqs', 10, 1000), row('api_req_started', 20, 2000)])
                second = events(history(root, provider))[0]
                self.assertEqual(second['id'], first['id'])
                self.assertEqual(second['tokens'], first['tokens'])
                self.assertEqual(second['model'], 'unknown')
                write(path.with_name('history_item.json'), dict(id='one', ts=2000, parentTaskId='lead', tokensIn=30, tokensOut=2, apiConfigName='PRIVATE'))
                summary = history(root, provider)
                self.assertEqual(len(events(summary)), 1)
                self.assertEqual(events(summary)[0]['sessionId'], 'lead')
                self.assertNotIn('PRIVATE', json.dumps(summary))

    def test_legacy_summary_is_not_added_to_explicitly_related_child_records(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            path = root / 'tasks/lead/ui_messages.json'
            def row(kind, count):
                return dict(type='say', say=kind, ts=1000, modelInfo=dict(modelId='real-model'),
                            text=json.dumps(dict(source='subagents', tokensIn=count, tokensOut=0)))
            write(path, [row('api_req_started', 10), row('subagent_usage', 20)])
            first = history(root, 'cline')
            self.assertEqual(events(first)[0]['tokens']['input'], 30)
            child = json.loads(json.dumps(CURRENT))
            child['sessionId'] = 'child'
            child['origin']['parentThreadId'] = 'unrelated'
            child['messages'][1]['id'] = 'child-request'
            child['messages'][1]['metrics'] = dict(inputTokens=20, outputTokens=0)
            child_path = root / 'sessions/tree/worker__job.messages.json'
            write(child_path, child)
            unrelated = history(root, 'cline')
            self.assertFalse(unrelated[-1]['partial'])
            self.assertEqual(sum(row['tokens']['input'] for row in events(unrelated)), 50)
            child['origin']['parentThreadId'] = 'lead'
            write(child_path, child)
            mixed = history(root, 'cline', cursor(first))
            self.assertTrue(mixed[-1]['partial'])
            self.assertEqual(mixed[-1]['issues'][0]['code'], 'overlappingSubagentSummary')
            indexed = {row['sessionId']: row for row in events(mixed)}
            self.assertEqual(indexed['lead']['tokens']['input'], 10)
            self.assertEqual(indexed['lead']['clineTaskUsageVersion'], 3)
            self.assertEqual(indexed['child']['tokens']['input'], 20)
            replay = history(root, 'cline', cursor(mixed))
            self.assertEqual(events(replay), [])
            self.assertTrue(replay[-1]['partial'])

    def test_torn_and_invalid_task_files_leave_other_histories_readable(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            good = root / 'tasks/good/history_item.json'
            bad = root / 'tasks/bad/history_item.json'
            write(good, dict(id='good', ts=1000, tokensIn=1, tokensOut=2))
            write(bad, dict(id='bad', ts=1000, tokensIn=-1, tokensOut=2))
            first = history(root, 'roo')
            self.assertTrue(first[-1]['partial'])
            self.assertEqual(len(events(first)), 1)
            bad.write_text('{"PRIVATE":')
            torn = history(root, 'roo', cursor(first))
            self.assertTrue(torn[-1]['partial'])
            self.assertNotIn('PRIVATE', json.dumps(torn))
            write(bad, dict(id='bad', ts=1000, tokensIn=1, tokensOut=2))
            fixed = history(root, 'roo', cursor(first))
            self.assertFalse(fixed[-1]['partial'])
            self.assertEqual(events(fixed)[0]['id'], 'task:bad')

    def test_cline_database_session_lineage_and_pending_idle_states(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            db = root / 'db/sessions.db'
            db.parent.mkdir()
            now = int(time.time())
            stamp = datetime.datetime.fromtimestamp(now, datetime.timezone.utc).isoformat()
            with sqlite3.connect(db) as connection:
                connection.execute('CREATE TABLE sessions(session_id TEXT,status TEXT,updated_at TEXT,parent_session_id TEXT,interactive INTEGER,source TEXT)')
                for identity, parent, interactive, status in [('lead', '', 1, 'idle'), ('child', 'lead', 0, 'running'), ('grandchild', 'child', 0, 'pending'), ('run', '', 0, 'running'), ('run-child', 'run', 0, 'running'), ('unknown', '', 1, 'new-status'), ('orphan', 'missing', 0, 'running')]:
                    connection.execute('INSERT INTO sessions VALUES(?,?,?,?,?,?)', (identity, status, stamp, parent, interactive, 'cli'))
            result = sessions.collect_native(dict(id='cline', provider='cline', path=str(root)), {}, True)
            self.assertIsNone(result['error'])
            indexed = {row['identity']: row for row in result['files']}
            self.assertEqual(set(indexed), {'lead', 'child', 'grandchild', 'unknown', 'orphan'})
            self.assertEqual(indexed['lead']['events'][-1]['kind'], 'complete')
            self.assertEqual(indexed['grandchild']['events'][-1]['kind'], 'PermissionRequest')
            cached = {row['key']: row for row in result['files']}
            self.assertTrue(all(not row['events'] for row in sessions.collect_native(dict(id='cline', provider='cline', path=str(root)), cached, True)['files']))

    def test_cline_manifest_excludes_headless_when_only_session_directory_is_selected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory) / 'custom-sessions'
            now = int(time.time())
            stamp = datetime.datetime.fromtimestamp(now, datetime.timezone.utc).isoformat()
            for identity, interactive in [('run', False), ('interactive', True)]:
                write(root / identity / (identity + '.messages.json'), dict(CURRENT, sessionId=identity, updated_at=stamp))
                write(root / identity / (identity + '.json'), dict(version=1, session_id=identity, status='completed', source='cli', interactive=interactive, ended_at=stamp, prompt='PRIVATE'))
            result = sessions.collect_native(dict(id='cline', provider='cline', path=str(root)), {}, True)
            self.assertEqual([row['identity'] for row in result['files']], ['interactive'])
            self.assertNotIn('PRIVATE', json.dumps(result))

    def test_file_lineage_uses_old_and_overflow_manifests_without_reading_transcripts(self):
        for overflow in (False, True):
            with self.subTest(overflow=overflow), tempfile.TemporaryDirectory() as directory:
                root = pathlib.Path(directory)
                tree = root / 'sessions/tree'
                now = int(time.time())
                old = now - (100 if overflow else 90001)

                def record(name, identity, parent, interactive, at, transcript=True):
                    stamp = datetime.datetime.fromtimestamp(at, datetime.timezone.utc).isoformat()
                    path = tree / (name + '.json')
                    write(path, dict(session_id=identity, parent_session_id=parent, interactive=interactive,
                                     source='subagent' if parent else 'cli', status='running', updated_at=stamp, prompt='PRIVATE'))
                    os.utime(path, (at, at))
                    if transcript:
                        path = tree / (name + '.messages.json')
                        path.write_text('PRIVATE not JSON')
                        os.utime(path, (at, at))

                record('z-lead-record', 'lead', '', False, old, not overflow)
                record('a-worker__job', 'middle', 'lead', False, old)
                record('descendant', 'child', 'middle', False, now)
                record('orphan', 'orphan', 'missing', False, now)
                if overflow:
                    for index in range(254):
                        identity = 'filler-%03d' % index
                        record(identity, identity, '', True, now)
                with mock.patch.object(tasks, 'read', wraps=tasks.read) as reader:
                    rows, errors = tasks.sessions(root, 'cline', {}, now)
                self.assertEqual(errors, [])
                self.assertTrue(all(not call.args[0].name.endswith('.messages.json') for call in reader.call_args_list))
                self.assertIn('orphan', {row['identity'] for row in rows})
                self.assertTrue({'lead', 'middle', 'child'}.isdisjoint(row['identity'] for row in rows))
                self.assertNotIn('PRIVATE', json.dumps(rows))
                record('z-lead-record', 'lead', '', True, old, not overflow)
                rows, _ = tasks.sessions(root, 'cline', {row['key']: row for row in rows}, now)
                self.assertIn('child', {row['identity'] for row in rows})
                self.assertTrue({'lead', 'middle'}.isdisjoint(row['identity'] for row in rows))

    def test_legacy_sessions_cache_every_file_and_preserve_partial_updates(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            now = int(time.time())
            for identity in ('a', 'b'):
                write(root / ('tasks/' + identity + '/history_item.json'), dict(id=identity, ts=now * 1000, tokensIn=1, tokensOut=1, status='active', task='PRIVATE'))
            first, errors = tasks.sessions(root, 'roo', {}, now)
            self.assertEqual(errors, [])
            saved = {row['key']: row for row in first}
            path = root / 'tasks/a/history_item.json'
            write(path, dict(id='a', ts=now * 1000, tokensIn=1, tokensOut=1, status='completed', task='PRIVATE'))
            second, _ = tasks.sessions(root, 'roo', saved, now)
            self.assertEqual(next(row for row in second if row['identity'] == 'a')['events'][-1]['kind'], 'complete')
            path.write_text('{')
            kept, errors = tasks.sessions(root, 'roo', {row['key']: row for row in second}, now)
            self.assertTrue(errors)
            self.assertEqual(len(kept), 2)
            self.assertNotIn('PRIVATE', json.dumps(kept))


if __name__ == '__main__':
    unittest.main()
