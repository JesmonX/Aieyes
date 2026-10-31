"""Installed CodeBuddy IDE index shape: request totals and readonly SSH parity."""
import json
import pathlib
import runpy
import subprocess
import sys
import tempfile
import time
import unittest

HERE = pathlib.Path(__file__).resolve().parent
ide = runpy.run_path(str(HERE / 'codebuddy_ide_remote.py'))
sessions = runpy.run_path(str(HERE / 'session_remote.py'))


class CodeBuddyIde(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.temp.name) / 'CodeBuddyExtension/Data/u1/CodeBuddy/u1/history'
        self.path = self.root / 'workspace/conversation/index.json'
        self.path.parent.mkdir(parents=True)
        self.stamp = int(time.time() * 1000)
        self.value = dict(messages=[dict(message='PRIVATE')], requests=[dict(id='request', state='complete', startedAt=self.stamp,
            messages=['PRIVATE'], usage=dict(inputTokens=100, outputTokens=20, totalTokens=120, cacheTokens=30, cachedWriteTokens=10, lastTokens=9999, credit=.75))])
        self.path.write_text(json.dumps(self.value))

    def tearDown(self):
        self.temp.cleanup()

    def stream(self, cursors=None):
        request = dict(protocol=2, path=str(self.root), provider='codebuddy', cursors=cursors or {})
        result = subprocess.run([sys.executable, '-c', "import json,runpy,sys;runpy.run_path(sys.argv[1],init_globals={'REQUEST':json.loads(sys.argv[2])})", str(HERE / 'remote_history.py'), json.dumps(request)], capture_output=True, check=True, text=True)
        self.assertNotIn('PRIVATE', result.stdout)
        return [json.loads(line) for line in result.stdout.splitlines()]

    def test_transport_replays_only_changed_request_totals_and_preserves_source(self):
        before = self.path.read_bytes()
        frames = self.stream()
        self.assertEqual(frames[-1]['kind'], 'done')
        self.assertFalse(frames[-1]['partial'])
        rows = [row for frame in frames for row in frame.get('events', [])]
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]['tokens'], dict(input=60, output=20, cacheRead=30, cacheWrite=10))
        self.assertEqual(rows[0]['nativeCredits']['credits'], .75)
        self.assertEqual(rows[0]['model'], 'unknown')
        cursors = {frame['path']:frame['cursor'] for frame in frames if frame.get('cursor')}
        replay = self.stream(cursors)
        self.assertEqual(replay[-1]['readBytes'], 0)
        self.assertEqual(self.path.read_bytes(), before)
        self.value['requests'][0]['usage'].update(inputTokens=200, totalTokens=220)
        self.path.write_text(json.dumps(self.value))
        updated = [row for frame in self.stream(cursors) for row in frame.get('events', [])]
        self.assertEqual(updated[0]['id'], rows[0]['id'])
        self.assertEqual(updated[0]['tokens']['input'], 160)

    def test_lifecycle_uses_native_states_and_never_installs_cli_hooks_in_ide_storage(self):
        source = dict(id='ide', provider='codebuddy', path=str(self.root))
        result = sessions['main'](dict(sources=[source]))
        self.assertEqual(result['hooks'], [])
        events = result['polls'][0]['files'][0]['events']
        self.assertEqual([event['kind'] for event in events], ['start', 'complete'])
        self.assertFalse((self.root / 'settings.json').exists())
        self.assertFalse((self.root / '.aieyes').exists())
        self.value['requests'][0]['state'] = 'canceled'
        self.path.write_text(json.dumps(self.value))
        result = sessions['main'](dict(sources=[source]))
        self.assertEqual(result['polls'][0]['files'][0]['events'][-1]['kind'], 'interrupted')
        (self.path.parent / '.hidden').touch()
        self.assertEqual(sessions['main'](dict(sources=[source]))['polls'][0]['files'], [])

    def test_bad_request_or_torn_file_does_not_hide_healthy_history(self):
        self.value['requests'].append(dict(id='bad', startedAt=self.stamp, usage=dict(inputTokens=1, cacheTokens=2)))
        self.path.write_text(json.dumps(self.value))
        other = self.root / 'workspace/torn/index.json'
        other.parent.mkdir()
        other.write_text('{torn')
        frames = self.stream()
        self.assertTrue(frames[-1]['partial'])
        self.assertEqual(frames[-1]['failedFiles'], 1)
        self.assertEqual(frames[-1]['invalidRecords'], 1)
        self.assertEqual(len([row for frame in frames for row in frame.get('events', [])]), 1)
        self.assertFalse(any(frame.get('cursor') for frame in frames))
        source = dict(id='ide', provider='codebuddy', path=str(self.root))
        result = sessions['main'](dict(sources=[source]))['polls'][0]
        self.assertIsNotNone(result['error'])
        self.assertEqual(len(result['files']), 1)
        self.value['requests'][-1]['state'] = {'unknown':'state'}
        self.path.write_text(json.dumps(self.value))
        result = sessions['main'](dict(sources=[source]))['polls'][0]
        self.assertEqual(result['files'][0]['events'][-1]['kind'], 'unknown')


if __name__ == '__main__':
    unittest.main()
