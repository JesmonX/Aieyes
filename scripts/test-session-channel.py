"""Offline channel protocol tests. No real Feishu account, agent, TTY, or SSH host."""
import base64
import hashlib
import json
import os
import pathlib
import socket
import struct
import subprocess
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
from script_loader import load_script

m = load_script('session_channel', 'session_channel.py')
ROOT = pathlib.Path(__file__).resolve().parents[1]
NATIVE = '11111111-2222-4333-8444-555555555555'
CHILD = 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee'
unix_socket = unittest.skipUnless(os.name != 'nt' and hasattr(socket, 'AF_UNIX'), 'Native control socket requires Unix')


def target(phase='complete'):
    return dict(key='sfixture', stateKey='complete:turn-one:1', observedAt=int(time.time()), path='', logPaths=[],
                session=dict(id='session', sourceId='source', provider='codex', machineId='host', nativeSession=NATIVE,
                             turnId='turn-one', phase=phase, turnStartedAt=1000, workspace='/fixture'))


def question():
    return dict(id=7, method='item/tool/requestUserInput', params=dict(threadId=NATIVE, turnId='turn-one', itemId='item',
                questions=[dict(id='choice', question='Choose', options=[dict(label='One'), dict(label='Two')])]))


class Server:
    def __init__(self, directory, changed=False, pending=False, lost=False, wrong_cwd=False):
        root = pathlib.Path(directory) / 'app-server-control'
        root.mkdir(exist_ok=True)
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(str(root / 'app-server-control.sock'))
        self.listener.listen()
        self.listener.settimeout(4)
        self.changed, self.pending, self.lost, self.wrong_cwd = changed, pending, lost, wrong_cwd
        self.calls, self.errors = [], []
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    @staticmethod
    def exact(stream, size):
        data = b''
        while len(data) < size:
            chunk = stream.recv(size - len(data))
            if not chunk:
                raise EOFError()
            data += chunk
        return data

    def read(self, stream):
        first, second = self.exact(stream, 2)
        assert first == 129 and second & 128
        size = second & 127
        if size == 126:
            size = struct.unpack('!H', self.exact(stream, 2))[0]
        elif size == 127:
            size = struct.unpack('!Q', self.exact(stream, 8))[0]
        mask = self.exact(stream, 4)
        return json.loads(bytes(b ^ mask[i % 4] for i, b in enumerate(self.exact(stream, size))))

    @staticmethod
    def send(stream, value):
        data = json.dumps(value, separators=(',', ':')).encode()
        header = bytes([129, len(data)]) if len(data) < 126 else bytes([129, 126]) + struct.pack('!H', len(data))
        stream.sendall(header + data)

    def serve(self):
        try:
            stream, _ = self.listener.accept()
            with stream:
                stream.settimeout(4)
                header = b''
                while not header.endswith(b'\r\n\r\n'):
                    header += self.exact(stream, 1)
                key = next(v.split(':', 1)[1].strip() for v in header.decode().splitlines() if v.startswith('Sec-WebSocket-Key:'))
                accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
                stream.sendall(('HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ' + accept + '\r\n\r\n').encode())
                while True:
                    value = self.read(stream)
                    self.calls.append(value)
                    method = value.get('method')
                    if not method:
                        assert value['id'] == 7
                        self.send(stream, dict(method='serverRequest/resolved', params=dict(threadId=NATIVE, requestId=7)))
                        continue
                    if method == 'initialized':
                        continue
                    thread = dict(id=NATIVE, cwd='/different' if self.wrong_cwd else '/fixture', status=dict(type='active' if self.pending else 'idle', activeFlags=['waitingOnUserInput'] if self.pending else []))
                    values = {'initialize': {}, 'thread/read': dict(thread=thread), 'thread/turns/list': dict(data=[dict(id='turn-new' if self.changed else 'turn-one')]), 'thread/resume': dict(thread=thread), 'thread/loaded/list': dict(data=[NATIVE]), 'turn/start': dict(turn=dict(id='turn-new', status='inProgress', error=None))}
                    if method == 'turn/start' and self.lost:
                        return
                    self.send(stream, dict(id=value['id'], result=values[method]))
                    if method == 'thread/resume' and self.pending:
                        self.send(stream, question())
        except EOFError:
            pass
        except Exception as error:
            self.errors.append(error)
        finally:
            self.listener.close()

    def join(self):
        self.thread.join(5)
        assert not self.thread.is_alive(), 'Mock server did not stop'
        assert not self.errors, self.errors


class ChannelTests(unittest.TestCase):
    def test_unexpected_post_submission_errors_never_become_retryable(self):
        with patch.object(m, 'run', side_effect=TypeError('Malformed response after send')):
            self.assertEqual(m.guarded_run(dict(action='send'))['status'], 'uncertain')
            self.assertEqual(m.guarded_run(dict(action='read'))['status'], 'blocked')

    @unix_socket
    def test_native_followup_reuses_thread_without_changing_model_or_permissions(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as directory:
            server = Server(directory)
            out = m.codex_send(dict(path=directory), target(), 'literal `$(x)`\n中文', None)
            server.join()
        self.assertEqual(out['status'], 'confirmed')
        start = next(v['params'] for v in server.calls if v.get('method') == 'turn/start')
        self.assertEqual(set(start), {'threadId', 'input'})
        self.assertEqual(start['threadId'], NATIVE)
        self.assertEqual(start['input'][0]['text'], 'literal `$(x)`\n中文')
        resume = next(v['params'] for v in server.calls if v.get('method') == 'thread/resume')
        self.assertEqual(resume, dict(threadId=NATIVE, excludeTurns=True))

    @unix_socket
    def test_stale_turn_or_workspace_never_resumes_or_starts(self):
        for params in (dict(changed=True), dict(wrong_cwd=True)):
            with self.subTest(params=params), tempfile.TemporaryDirectory(dir='/tmp') as directory:
                server = Server(directory, **params)
                out = m.codex_send(dict(path=directory), target(), 'Must be retained', None)
                server.join()
                self.assertEqual(out['status'], 'blocked')
                self.assertFalse(any(v.get('method') in ('thread/resume', 'turn/start') for v in server.calls))

    @unix_socket
    def test_pending_question_answers_exact_request(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as directory:
            server = Server(directory, pending=True)
            out = m.codex_send(dict(path=directory), target('needsInput'), '2', m.fingerprint(question()))
            server.join()
        self.assertEqual(out['status'], 'confirmed')
        reply = next(v for v in server.calls if v.get('id') == 7)
        self.assertEqual(reply['result'], dict(answers=dict(choice=dict(answers=['Two']))))

    @unix_socket
    def test_changed_question_retains_answer_and_lost_submit_is_uncertain(self):
        with tempfile.TemporaryDirectory(dir='/tmp') as directory:
            server = Server(directory, pending=True)
            out = m.codex_send(dict(path=directory), target('needsInput'), '2', 'old-fingerprint')
            server.join()
            self.assertEqual(out['status'], 'blocked')
            self.assertFalse(any(v.get('id') == 7 for v in server.calls))
        with tempfile.TemporaryDirectory(dir='/tmp') as directory:
            server = Server(directory, lost=True)
            out = m.codex_send(dict(path=directory), target(), 'Maybe received', None)
            server.join()
            self.assertEqual(out['status'], 'uncertain')
            self.assertEqual(sum(v.get('method') == 'turn/start' for v in server.calls), 1)

    def test_approval_requires_explicit_single_decision(self):
        q = dict(method='item/commandExecution/requestApproval', params=dict(availableDecisions=['accept', 'acceptForSession', 'decline', 'cancel']))
        self.assertEqual(m.answer(q, '/approve'), dict(decision='accept'))
        self.assertEqual(m.answer(q, '/decline'), dict(decision='decline'))
        for value in ('yes', 'acceptForSession', 'approve everything'):
            with self.assertRaises(ValueError):
                m.answer(q, value)

    def test_question_fingerprint_excludes_delivery_timers(self):
        q = question()
        before = m.fingerprint(q)
        q['id'] = 999
        q['params']['autoResolutionMs'] = 10
        self.assertEqual(m.fingerprint(q), before)
        q['params']['questions'][0]['question'] = 'New question'
        self.assertNotEqual(m.fingerprint(q), before)

    def test_excerpt_scope_partial_tail_and_queue_freshness(self):
        with tempfile.TemporaryDirectory() as directory:
            source, row = dict(path=directory), target()
            path = pathlib.Path(directory) / 'parent.jsonl'
            child = pathlib.Path(directory) / 'child.jsonl'
            row.update(path=str(path), logPaths=[str(child)])
            rows = [dict(type='session_meta', payload=dict(id=NATIVE, cwd='/fixture')),
                    dict(timestamp=2000, type='turn_context', payload=dict(turn_id='turn-one', cwd='/fixture')),
                    dict(timestamp=2001, type='response_item', payload=dict(type='message', role='assistant', phase='commentary', content=[dict(type='output_text', text='PRIVATE PROGRESS')])),
                    dict(timestamp=2002, type='response_item', payload=dict(type='reasoning', text='PRIVATE REASONING')),
                    dict(timestamp=2003, type='response_item', payload=dict(type='function_call_output', output='PRIVATE TOOL')),
                    dict(timestamp=2004, type='response_item', payload=dict(type='message', role='assistant', phase='final', content=[dict(type='output_text', text='Root final')]))]
            path.write_text(''.join(json.dumps(v) + '\n' for v in rows))
            other = json.loads(json.dumps(rows))
            other[0]['payload']['id'] = CHILD
            other[-1]['payload']['content'][0]['text'] = 'PRIVATE CHILD'
            child.write_text(''.join(json.dumps(v) + '\n' for v in other))
            self.assertEqual(m.excerpt(source, row)['text'], 'Root final')
            m.verify_codex_record(source, row)
            with path.open('a') as stream:
                stream.write('{"type":')
            self.assertEqual(m.excerpt(source, row)['text'], 'Root final')
            with self.assertRaises(ValueError):
                m.verify_codex_record(source, row)
            row['path'] = str(child)
            self.assertEqual(m.excerpt(source, row)['text'], '')

    def test_source_machine_and_work_alias_are_exact(self):
        row = target()
        row['session']['provider'] = 'chatgpt-work'
        source = dict(id='source', provider='codex', hostId='host', path='/fixture')
        request = dict(action='read', source=source, target=row)
        with patch.object(m, 'excerpt', return_value=dict(text='matched')) as read:
            self.assertEqual(m.run(request)['excerpt']['text'], 'matched')
            read.assert_called_once()
        for key, value in (('id', 'wrong'), ('hostId', 'local'), ('provider', 'claude')):
            wrong = dict(source, **{key: value})
            with patch.object(m, 'excerpt') as read:
                self.assertEqual(m.run(dict(request, source=wrong))['status'], 'blocked')
                read.assert_not_called()

    def test_terminal_rejects_shell_and_never_interpolates_message(self):
        row = target()
        row['session']['provider'] = 'claude'
        row['process'] = dict(pid=321, start=456)
        text = 'literal "$(danger)"\nsecond line'
        calls = []

        def run(argv, **kwargs):
            calls.append((argv, kwargs))
            listing = b'%1\t/dev/pts/2\t123\tclaude\t0\n'
            return subprocess.CompletedProcess(argv, 0, stdout=listing, stderr=b'')

        with patch.object(m, 'process_identity', return_value=([321, 123], '/dev/pts/2')), patch.object(m.subprocess, 'run', side_effect=run):
            self.assertEqual(m.tmux_send(row, text)['status'], 'submitted')
        load = next(c for c in calls if c[0][1] == 'load-buffer')
        self.assertEqual(load[1]['input'], text.encode())
        self.assertTrue(all(text not in argv and not kwargs.get('shell') for argv, kwargs in calls))
        self.assertTrue(any(argv[1] == 'paste-buffer' and argv[argv.index('-t') + 1] == '%1' for argv, _ in calls))
        calls.clear()
        with patch.object(m, 'process_identity', return_value=([321, 123], '/dev/pts/2')), patch.object(m.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, stdout=b'%1\t/dev/pts/2\t123\tzsh\t0\n')) as command:
            self.assertEqual(m.tmux_send(row, text)['status'], 'blocked')
            self.assertEqual(command.call_count, 1)

    def test_process_exit_before_paste_never_submits(self):
        row = target();row['session']['provider'] = 'claude';row['process'] = dict(pid=321, start=456)
        with patch.object(m, 'process_identity', side_effect=[([321, 123], '/dev/pts/2'), ValueError('gone')]), patch.object(m.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, stdout=b'%1\t/dev/pts/2\t123\tclaude\t0\n')) as command:
            self.assertEqual(m.tmux_send(row, 'retained')['status'], 'blocked')
            self.assertFalse(any(call.args[0][1] == 'paste-buffer' for call in command.call_args_list))


if __name__ == '__main__':
    unittest.main()
