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
    def __init__(self, directory, changed=False, pending=False, lost=False, wrong_cwd=False, requests=None):
        root = pathlib.Path(directory) / 'app-server-control'
        root.mkdir(exist_ok=True)
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(str(root / 'app-server-control.sock'))
        self.listener.listen()
        self.listener.settimeout(4)
        self.changed, self.pending, self.lost, self.wrong_cwd = changed, pending, lost, wrong_cwd
        self.calls, self.errors = [], []
        self.requests = requests or [question()]
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
                    values = {'initialize': {}, 'thread/read': dict(thread=thread), 'thread/turns/list': dict(data=[dict(id='turn-new' if self.changed else 'turn-one')]), 'thread/resume': dict(thread=thread), 'thread/loaded/list': dict(data=[NATIVE]), 'turn/start': dict(turn=dict(id='turn-new', status='inProgress', error=None)), 'turn/steer': dict(turnId='turn-one')}
                    if method == 'turn/start' and self.lost:
                        return
                    self.send(stream, dict(id=value['id'], result=values[method]))
                    if method == 'thread/resume' and self.pending:
                        for request in self.requests:
                            self.send(stream, request)
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
    @unix_socket
    def test_async_reply_steers_the_same_turn_and_follows_owned_socket_symlink(self):
        for changed in (False,True):
            with self.subTest(changed=changed), tempfile.TemporaryDirectory(dir='/tmp') as directory:
                server=Server(directory,pending=True,changed=changed)
                link=pathlib.Path(directory)/'app-server-control/app-server-control.sock'
                real=pathlib.Path(directory)/'daemon.sock';link.rename(real);link.symlink_to(real)
                row=target('needsInput')
                value=dict(asyncRequest=dict(callId='ask',questions=[dict(title='测试问题',options=['收到','没有'])]))
                request=m.async_request(row,value)
                with patch.object(m,'excerpt',return_value=value):
                    out=m.codex_send(dict(path=directory),row,'1',m.fingerprint(request))
                server.join()
                self.assertEqual(out['status'],'blocked' if changed else 'confirmed')
                self.assertFalse(any(v.get('method') in ('turn/start','thread/resume') for v in server.calls))
                sent=next((v['params'] for v in server.calls if v.get('method')=='turn/steer'),None)
                if changed:self.assertIsNone(sent);continue
                self.assertEqual(sent['expectedTurnId'],'turn-one')
                self.assertEqual(sent['threadId'],NATIVE)
                answer=json.loads(sent['input'][0]['text'].split('\n')[1])[0]
                self.assertEqual(answer['answer'],'收到')
                self.assertEqual(json.loads(answer['questionItemId']),['request_user_input_async','ask',0])

    def test_async_question_receipt_is_not_an_answer_and_new_final_phase_is_readable(self):
        records = [
            dict(timestamp=1000, type='event_msg', payload=dict(type='task_started', turn_id='turn-one')),
            dict(timestamp=1001, type='response_item', payload=dict(type='function_call', name='functions.request_user_input_async', call_id='ask', arguments=json.dumps(dict(questions=[dict(title='测试问题', options=['收到', '没有'])])))),
            dict(timestamp=1002, type='response_item', payload=dict(type='function_call_output', call_id='ask', output='{"accepted":true}')),
            dict(timestamp=1003, type='response_item', payload=dict(type='message', role='assistant', phase='final_answer', content=[dict(type='output_text', text='Current final')])),
        ]
        excerpt = m.parse(records, target())
        self.assertEqual(excerpt['question'], '1: 测试问题\n  1. 收到\n  2. 没有')
        self.assertEqual(excerpt['text'], 'Current final')
        answer = dict(timestamp=1004, type='response_item', payload=dict(type='message', role='user', content=[dict(type='input_text', text='<send_user_message_question_reply>\n' + json.dumps([dict(answer='收到', questionItemId=json.dumps(['request_user_input_async','ask',0]))]) + '\n</send_user_message_question_reply>')]))
        self.assertEqual(m.parse(records + [answer], target())['question'], '')
        answer['payload']['content'][0]['text'] = answer['payload']['content'][0]['text'].replace('ask', 'other')
        self.assertEqual(m.parse(records + [answer], target())['question'], excerpt['question'])
        records.append(dict(timestamp=1004, type='event_msg', payload=dict(type='task_complete', turn_id='turn-one', last_agent_message='Completion final')))
        records.append(dict(timestamp=1005, type='event_msg', payload=dict(type='task_complete', turn_id='other', last_agent_message='PRIVATE OTHER TURN')))
        self.assertEqual(m.parse(records, target())['text'], 'Completion final')
        records[2]['payload']['output'] = '{"accepted":false}'
        self.assertEqual(m.parse(records, target())['question'], '')

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

    def test_plain_decisions_follow_each_request_type(self):
        command = dict(method='item/commandExecution/requestApproval', params=dict(kind='command', command='touch a', availableDecisions=['accept', dict(acceptWithExecpolicyAmendment=dict(execpolicy_amendment=['touch', 'a'])), 'cancel']))
        # 0.172.0 lists no plain refusal for a command but takes one.
        self.assertEqual(m.plain_replies(command), {'/approve': dict(decision='accept'), '/decline': dict(decision='decline'), '/cancel-request': dict(decision='cancel')})
        for kind in ('writeStdin', 'humanCommand'):
            held = dict(method=command['method'], params=dict(command['params'], kind=kind))
            self.assertEqual(m.plain_replies(held), {})
            with self.assertRaises(ValueError):
                m.answer(held, '/approve')
        profile = dict(network=dict(enabled=True), fileSystem=dict(read=None, write=['/tmp/out'], entries=[dict(path=dict(type='path', path='/tmp/out'), access='write')]))
        permissions = dict(method='item/permissions/requestApproval', params=dict(reason='fetch', permissions=profile))
        self.assertEqual(m.answer(permissions, '/approve'), dict(permissions=dict(network=dict(enabled=True), fileSystem=dict(write=['/tmp/out'], entries=profile['fileSystem']['entries'])), scope='turn'))
        self.assertEqual(m.answer(permissions, '/decline'), dict(permissions={}, scope='turn'))
        with self.assertRaises(ValueError):
            m.answer(permissions, '/cancel-request')
        self.assertIn('write: /tmp/out', m.describe(permissions, 'en'))
        tool = dict(method='mcpServer/elicitation/request', params=dict(mode='form', serverName='probe', message='Allow?', requestedSchema=dict(type='object', properties={}), _meta=dict(tool_name='delete_record', tool_params=dict(id='42'))))
        self.assertTrue(m.is_approval(tool))
        self.assertEqual(m.answer(tool, '/approve'), dict(action='accept', content={}))
        self.assertEqual(m.answer(tool, '/cancel-request'), dict(action='cancel'))
        self.assertIn('probe · delete_record\nid: 42', m.describe(tool, 'en'))
        form = dict(method='mcpServer/elicitation/request', params=dict(mode='form', serverName='probe', message='Pick', requestedSchema=dict(type='object', properties=dict(region=dict(type='string')))))
        self.assertFalse(m.is_approval(form))
        with self.assertRaises(ValueError):
            m.answer(form, '/approve')
        self.assertEqual(m.describe(form, 'en'), 'Handle this request in the original window.')
        legacy = dict(method='execCommandApproval', params=dict(conversationId=NATIVE, callId='c', command=['ls', '-la']))
        self.assertEqual(m.answer(legacy, '/approve'), dict(decision='approved'))
        self.assertEqual(m.answer(legacy, '/decline'), dict(decision=dict(denied=dict(rejection=''))))
        self.assertIn('ls -la', m.describe(legacy, 'en'))
        patch_request = dict(method='item/fileChange/requestApproval', params=dict(itemId='call'))
        self.assertEqual(m.answer(patch_request, '/decline'), dict(decision='decline'))

    def test_core_built_reply_must_fit_the_pending_request(self):
        amendment = dict(acceptWithExecpolicyAmendment=dict(execpolicy_amendment=['touch', 'a']))
        command = dict(method='item/commandExecution/requestApproval', params=dict(availableDecisions=['accept', amendment, 'cancel']))
        for good in (dict(decision='accept'), dict(decision=amendment), dict(decision='decline'), dict(decision='cancel')):
            self.assertEqual(m.checked_reply(command, good), good)
        other = dict(acceptWithExecpolicyAmendment=dict(execpolicy_amendment=['rm']))
        for bad in (dict(decision='acceptForSession'), dict(decision=other), dict(decision='accept', extra=1), dict(action='accept'), 'accept', None):
            with self.assertRaises(ValueError, msg=bad):
                m.checked_reply(command, bad)
        held = dict(method=command['method'], params=dict(kind='writeStdin', availableDecisions=['accept']))
        nothing = dict(method=command['method'], params=dict(availableDecisions=[]))
        for request in (held, nothing):
            for reply in (dict(decision='accept'), dict(decision='decline')):
                with self.assertRaises(ValueError):
                    m.checked_reply(request, reply)
        profile = dict(network=dict(enabled=True), fileSystem=None)
        permissions = dict(method='item/permissions/requestApproval', params=dict(permissions=profile))
        granted = dict(network=dict(enabled=True))
        for good in (dict(permissions=granted, scope='session'), dict(permissions={}, scope='turn')):
            self.assertEqual(m.checked_reply(permissions, good), good)
        for bad in (dict(permissions=dict(network=dict(enabled=True), fileSystem=dict(write=['/'])), scope='turn'), dict(permissions=granted, scope='always'), dict(decision='accept')):
            with self.assertRaises(ValueError):
                m.checked_reply(permissions, bad)
        form = dict(method='mcpServer/elicitation/request', params=dict(mode='form', requestedSchema=dict(type='object', properties=dict(region={}, replicas={}))))
        for good in (dict(action='accept', content=dict(region='eu-west', replicas=3)), dict(action='decline'), dict(action='accept', content={}, _meta=dict(persist='session'))):
            self.assertEqual(m.checked_reply(form, good), good)
        for bad in (dict(action='accept', content=dict(zone='x')), dict(action='accept'), dict(action='submit'), dict(action='decline', decision='accept')):
            with self.assertRaises(ValueError):
                m.checked_reply(form, bad)
        with self.assertRaises(ValueError):
            m.checked_reply(dict(method='mcpServer/elicitation/request', params=dict(mode='url', url='https://example.com')), dict(action='accept', content={}))
        legacy = dict(method='applyPatchApproval', params={})
        for good in (dict(decision='approved_for_session'), dict(decision=dict(denied=dict(rejection='not now')))):
            self.assertEqual(m.checked_reply(legacy, good), good)
        with self.assertRaises(ValueError):
            m.checked_reply(legacy, dict(decision='accept'))
        # Several choices for one question stay separate answers.
        many = dict(answers=dict(choice=dict(answers=['One', 'Two'])))
        self.assertEqual(m.checked_reply(question(), many), many)
        for bad in (dict(answers=dict(other=dict(answers=['One']))), dict(answers=dict(choice=dict(answers=[]))), dict(answers={}), dict(decision='accept')):
            with self.assertRaises(ValueError):
                m.checked_reply(question(), bad)
        with self.assertRaises(ValueError):
            m.checked_reply(dict(method='item/tool/call', params={}), dict(decision='accept'))

    @unix_socket
    def test_native_reply_reaches_only_the_request_it_was_built_for(self):
        amendment = dict(acceptWithExecpolicyAmendment=dict(execpolicy_amendment=['touch', 'a']))
        approval = dict(id=7, method='item/commandExecution/requestApproval', params=dict(threadId=NATIVE, turnId='turn-one', itemId='call', command='touch a', availableDecisions=['accept', amendment, 'cancel']))
        later = dict(id=8, method='mcpServer/elicitation/request', params=dict(threadId=NATIVE, turnId=None, serverName='probe', mode='form', message='Allow?', requestedSchema=dict(type='object', properties={}), _meta=dict(tool_name='t')))
        expected = dict(approval, params=dict(approval['params'], pendingCount=2))
        with tempfile.TemporaryDirectory(dir='/tmp') as directory:
            server = Server(directory, pending=True, requests=[approval, later])
            out = m.codex_send(dict(path=directory), target('needsInput'), '', m.fingerprint(expected), dict(decision=amendment))
            server.join()
        self.assertEqual(out['status'], 'confirmed', out)
        # The oldest request is the one answered; the second stays pending.
        self.assertEqual([v['result'] for v in server.calls if not v.get('method')], [dict(decision=amendment)])
        self.assertEqual(next(v for v in server.calls if not v.get('method'))['id'], 7)
        for native, prompt_hash in ((dict(decision='acceptForSession'), m.fingerprint(expected)), (dict(decision='accept'), m.fingerprint(later))):
            with tempfile.TemporaryDirectory(dir='/tmp') as directory:
                server = Server(directory, pending=True, requests=[approval, later])
                out = m.codex_send(dict(path=directory), target('needsInput'), '', prompt_hash, native)
                server.join()
            self.assertEqual(out['status'], 'blocked', out)
            self.assertFalse(any(not v.get('method') for v in server.calls))

    @unix_socket
    def test_read_returns_the_request_and_send_takes_a_native_reply_alone(self):
        tool = dict(id=7, method='mcpServer/elicitation/request', params=dict(threadId=NATIVE, turnId=None, serverName='probe', mode='form', message='Allow?', requestedSchema=dict(type='object', properties={}), _meta=dict(persist=['session'], tool_name='t')))
        native = dict(action='accept', content={}, _meta=dict(persist='session'))
        with tempfile.TemporaryDirectory(dir='/tmp') as directory:
            source = dict(id='source', provider='codex', hostId='host', path=directory)
            request = dict(source=source, target=target('needsInput'))
            server = Server(directory, pending=True, requests=[tool])
            read = m.run(dict(request, action='read', language='en'))
            server.join()
            self.assertTrue(read['approval'])
            self.assertEqual(read['approvalRequest']['params']['_meta'], tool['params']['_meta'])
            self.assertEqual(read['approvalRequest']['params']['pendingCount'], 1)
            self.assertIsNone(read['questionRequest'])
            # Text beside a native reply, or a native reply for a finished turn, goes nowhere.
            self.assertEqual(m.run(dict(request, action='send', text='/approve', reply=native, promptHash=read['promptHash']))['status'], 'blocked')
            self.assertEqual(m.run(dict(request, target=target(), action='send', reply=native, promptHash=read['promptHash']))['status'], 'blocked')
        with tempfile.TemporaryDirectory(dir='/tmp') as directory:
            source = dict(id='source', provider='codex', hostId='host', path=directory)
            server = Server(directory, pending=True, requests=[tool])
            out = m.run(dict(source=source, target=target('needsInput'), action='send', reply=native, promptHash=read['promptHash']))
            server.join()
        self.assertEqual(out['status'], 'confirmed', out)
        self.assertEqual([v['result'] for v in server.calls if not v.get('method')], [native])

    def test_fingerprint_tells_similar_requests_apart(self):
        def tool(params):
            return dict(method='mcpServer/elicitation/request', params=dict(threadId=NATIVE, turnId=None, serverName='probe', mode='form', message='Allow?', requestedSchema=dict(type='object', properties={}), _meta=dict(tool_name='delete_record', tool_params=params)))
        self.assertNotEqual(m.fingerprint(tool(dict(id='42'))), m.fingerprint(tool(dict(id='43'))))
        legacy = dict(method='execCommandApproval', params=dict(conversationId=NATIVE, callId='a', command=['ls']))
        self.assertNotEqual(m.fingerprint(legacy), m.fingerprint(dict(legacy, params=dict(legacy['params'], callId='b'))))

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

    def test_terminal_text_never_reaches_an_approval_form_option_menu_or_running_turn(self):
        source = dict(id='source', provider='claude', hostId='host', path='/fixture')
        ask = dict(type='assistant', sessionId=NATIVE, timestamp='2026-01-01T00:00:02Z', message=dict(role='assistant', content=[
            dict(type='tool_use', id='call-1', name='AskUserQuestion', input=dict(questions=[dict(question='Pick', options=[dict(label='A'), dict(label='B')])]))]))
        failed = dict(type='user', sessionId=NATIVE, timestamp='2026-01-01T00:00:03Z', message=dict(role='user', content=[
            dict(type='tool_result', tool_use_id='call-1', is_error=True, content='hook error')]))
        answered = dict(failed, message=dict(role='user', content=[dict(type='tool_result', tool_use_id='call-1', content='A')]))

        def row(phase, **extra):
            value = target(phase)
            value['session'].update(provider='claude', turnId='', turnStartedAt=0, **extra)
            return value

        # A failed tool result leaves the question open; a successful one closes it.
        self.assertEqual(m.parse([ask, failed], row('needsInput'))['claudeRequest']['callId'], 'call-1')
        self.assertNotIn('claudeRequest', m.parse([ask, answered], row('needsInput')))
        request = lambda value: dict(action='send', source=source, target=value, text='yes', allowTerminal=True)
        with patch.object(m, 'tmux_send', return_value=m.result('submitted', 'sent')) as send, patch.object(m, 'excerpt', return_value=m.parse([ask], row('needsInput'))):
            for kind in ('approval', 'elicitation'):
                self.assertEqual(m.run(request(row('needsInput', inputKind=kind)))['status'], 'blocked')
            for phase in ('working', 'thinking', 'tool', 'unknown'):
                self.assertEqual(m.run(request(row(phase)))['status'], 'blocked')
            self.assertEqual(m.run(request(row('needsInput', inputKind='question')))['status'], 'blocked')
            send.assert_not_called()
        with patch.object(m, 'tmux_send', return_value=m.result('submitted', 'sent')) as send, patch.object(m, 'excerpt', return_value=m.parse([ask, answered], row('needsInput'))):
            self.assertEqual(m.run(request(row('needsInput', inputKind='notification')))['status'], 'submitted')
            self.assertEqual(m.run(request(row('complete')))['status'], 'submitted')
            self.assertEqual(send.call_count, 2)

    def test_other_agents_pending_prompts_are_read_and_never_typed_into(self):
        """Rows as written by Kimi Code 2.4.1, Grok CLI 1.0.92, Copilot CLI 1.0.105 and CodeBuddy 2.187.0."""
        def save(path, rows):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(''.join(json.dumps(r) + '\n' for r in rows))

        def read(provider, root, path, native, phase='needsInput', kind=''):
            source = dict(id='source', provider=provider, hostId='host', path=str(root))
            row = target(phase)
            row['path'] = str(path) if path else ''
            row['session'].update(provider=provider, nativeSession=native, inputKind=kind)
            return source, row, m.excerpt(source, row)

        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory).resolve()
            # Kimi
            folder = 'session_' + NATIVE
            wire = home / 'kimi/sessions/wd_fixture' / folder / 'agents/main/wire.jsonl'
            rows = [dict(type='turn.prompt', input=[dict(type='text', text='Clean up')], turnId=0, time=1795631203441),
                    dict(type='context.append_loop_event', event=dict(type='content.part', part=dict(type='text', text='PRIVATE PROGRESS'))),
                    dict(type='context.append_loop_event', event=dict(type='tool.call', toolCallId='c0')),
                    dict(type='interaction.request', id='a0', kind='approval', request=dict(sessionId=folder, toolName='Bash', action='Running: OLD', display=dict(kind='command', command='OLD'))),
                    dict(type='interaction.resolved', id='a0', response=dict(decision='approved')),
                    dict(type='interaction.request', id='a1', kind='approval', request=dict(sessionId=folder, toolName='Bash', action='Running: rm -rf probe_dir', display=dict(kind='command', command='rm -rf probe_dir', cwd='/fixture')))]
            save(wire, rows)
            source, row, out = read('kimi', home / 'kimi', wire, folder + ':main', kind='approval')
            self.assertEqual(out['structuredApproval'], dict(toolName='Bash', reason='Running: rm -rf probe_dir', command='rm -rf probe_dir', details='', kind='command', cwd='/fixture'))
            self.assertEqual((out['userText'], out['userAt']), ('Clean up', 1795631203441))
            self.assertNotIn('PRIVATE', json.dumps(out)); self.assertNotIn('OLD', json.dumps(out))
            # The helper refuses to type even when the session's kind was not reported.
            row['session']['inputKind'] = ''
            with patch.object(m, 'tmux_send', return_value=m.result('submitted', 'sent')) as send:
                self.assertEqual(m.run(dict(action='send', source=source, target=row, text='y', allowTerminal=True))['status'], 'blocked')
                send.assert_not_called()
            rows += [dict(type='interaction.resolved', id='a1', response=dict(decision='approved')),
                     dict(type='interaction.request', id='q1', kind='question', request=dict(questions=[
                         dict(question='Which color?', header='Color', options=[dict(label='Red', description='Warm'), dict(label='Blue')]),
                         dict(question='Which fruits?', header='Fruit', multiSelect=True, options=[dict(label='Apple')])]))]
            save(wire, rows)
            out = read('kimi', home / 'kimi', wire, folder + ':main')[2]
            self.assertEqual(out['structuredQuestions'][0], dict(id='Color', question='Which color?', multiSelect=False, options=[dict(label='Red', description='Warm'), dict(label='Blue')]))
            self.assertTrue(out['structuredQuestions'][1]['multiSelect'])
            self.assertIsNone(m.agent_excerpt(*read('kimi', home / 'kimi', wire, folder + ':coder')[:2]))
            self.assertIsNone(m.agent_excerpt(*read('kimi', home / 'other', wire, folder + ':main')[:2]))

            # Grok
            session = home / 'grok/sessions/%2Ffixture' / NATIVE
            update = lambda u: dict(timestamp=1795631363, method='session/update', params=dict(sessionId=NATIVE, update=u))
            call = lambda i, tool, kind, raw: update({'sessionUpdate': 'tool_call', 'toolCallId': i, 'title': tool, 'rawInput': raw, '_meta': {'x.ai/tool': dict(name=tool, kind=kind)}})
            events = [dict(type='turn_started', session_id=NATIVE), dict(type='tool_started', tool_name='run_terminal_command'), dict(type='permission_requested', tool_name='run_terminal_command')]
            updates = [update(dict(sessionUpdate='user_message_chunk', content=dict(type='text', text='Clean up'))),
                       call('c1', 'run_terminal_command', 'execute', dict(command='rm -rf probe_dir', description='Remove it'))]
            save(session / 'events.jsonl', events); save(session / 'updates.jsonl', updates)
            out = read('grok', home / 'grok', session / 'events.jsonl', NATIVE)[2]
            self.assertEqual(out['structuredApproval'], dict(toolName='run_terminal_command', reason='Remove it', command='rm -rf probe_dir', details='', kind='command'))
            save(session / 'updates.jsonl', updates + [call('c2', 'run_terminal_command', 'execute', dict(command='OTHER'))])
            self.assertEqual(read('grok', home / 'grok', session / 'events.jsonl', NATIVE)[2]['structuredApproval']['command'], '')
            events += [dict(type='permission_resolved', tool_name='run_terminal_command', decision='deny'), dict(type='tool_started', tool_name='ask_user_question'),
                       dict(type='permission_requested', tool_name='ask_user_question'), dict(type='permission_resolved', tool_name='ask_user_question', decision='allow')]
            updates += [update(dict(sessionUpdate='tool_call_update', toolCallId='c1', status='failed')),
                        call('c3', 'ask_user_question', 'ask_user', dict(questions=[dict(question='Which fruits?', options=[dict(label='Apple')], multi_select=True)]))]
            save(session / 'events.jsonl', events); save(session / 'updates.jsonl', updates)
            out = read('grok', home / 'grok', session / 'events.jsonl', NATIVE)[2]
            self.assertEqual(out['structuredQuestions'], [dict(id='1', question='Which fruits?', multiSelect=True, options=[dict(label='Apple')])])
            self.assertNotIn('structuredApproval', out)

            # Copilot
            log = home / 'copilot/session-state' / NATIVE / 'events.jsonl'
            item = lambda kind, data: dict(type=kind, data=data, timestamp='2026-11-25T18:56:26.573Z')
            rows = [item('session.start', dict(sessionId=NATIVE)), item('user.message', dict(content='Clean up')),
                    item('permission.requested', dict(requestId='r0', permissionRequest=dict(kind='shell', fullCommandText='OLD'))),
                    item('permission.completed', dict(requestId='r0', result=dict(kind='approved'))),
                    item('permission.requested', dict(requestId='r1', permissionRequest=dict(kind='shell', fullCommandText='rm -rf probe_dir', intention='Remove it')))]
            save(log, rows)
            out = read('copilot', home / 'copilot', log, NATIVE)[2]
            self.assertEqual(out['structuredApproval'], dict(toolName='shell', reason='Remove it', command='rm -rf probe_dir', details='', kind='command'))
            rows += [item('permission.completed', dict(requestId='r1', result=dict(kind='approved'))),
                     item('tool.execution_start', dict(toolCallId='c2', toolName='ask_user', arguments=dict(message='Which color?', requestedSchema=dict(properties=dict(color=dict(type='string', enum=['red', 'blue']))))))]
            save(log, rows)
            out = read('copilot', home / 'copilot', log, NATIVE)[2]
            self.assertEqual(out['structuredQuestions'], [dict(id='color', question='Which color?', multiSelect=False, options=[dict(label='red'), dict(label='blue')])])
            rows += [item('tool.execution_complete', dict(toolCallId='c2')),
                     item('tool.execution_start', dict(toolCallId='c3', toolName='ask_user', arguments=dict(message='What name?', requestedSchema=dict(properties=dict(name=dict(type='string'))))))]
            save(log, rows)
            out = read('copilot', home / 'copilot', log, NATIVE)[2]
            self.assertEqual(out['question'], 'What name?'); self.assertNotIn('structuredQuestions', out)

            # CodeBuddy, and WorkBuddy whose session has no transcript path of its own
            for provider in ('codebuddy', 'workbuddy'):
                transcript = home / provider / 'projects/fixture' / (NATIVE + '.jsonl')
                asked = json.dumps(dict(questions=[dict(question='Which color?', header='Color', options=[dict(label='Red')], multiSelect=False)]))
                rows = [dict(type='message', role='user', content=[dict(type='input_text', text='Ask me')], timestamp=5, sessionId=NATIVE),
                        dict(type='function_call', callId='c1', name='AskUserQuestion', arguments=asked, sessionId=NATIVE)]
                save(transcript, rows)
                path = transcript if provider == 'codebuddy' else ''
                out = read(provider, home / provider, path, NATIVE, kind='question')[2]
                self.assertEqual(out['structuredQuestions'], [dict(id='Color', question='Which color?', multiSelect=False, options=[dict(label='Red')])])
                rows += [dict(type='function_call_result', callId='c1', sessionId=NATIVE),
                         dict(type='function_call', callId='c2', name='Bash', arguments='{"command":"rm -rf probe_dir"}', sessionId=NATIVE)]
                save(transcript, rows)
                out = read(provider, home / provider, path, NATIVE, kind='approval')[2]
                self.assertEqual(out['structuredApproval']['command'], 'rm -rf probe_dir')
                save(transcript, rows + [dict(type='function_call', callId='c3', name='Bash', arguments='{"command":"OTHER"}', sessionId=NATIVE)])
                out = read(provider, home / provider, path, NATIVE, kind='approval')[2]
                self.assertNotIn('structuredApproval', out); self.assertEqual(out['question'], '')
                done = rows + [dict(type='function_call_output', callId='c2', sessionId=NATIVE),
                               dict(type='message', role='assistant', status='completed', content=[dict(type='output_text', text='All done')], sessionId=NATIVE)]
                save(transcript, done)
                out = read(provider, home / provider, path, NATIVE, phase='complete')[2]
                self.assertEqual((out['text'], out['question']), ('All done', ''))

    def test_readers_accept_any_json_shape_and_follow_each_open_permission(self):
        # Nested fields are arbitrary JSON: a string, a list or a number where an object is expected.
        permission = [dict(type='permission_requested', tool_name='x')]
        update = lambda u: dict(params=dict(update=u))
        raw = update({'sessionUpdate': 'tool_call', 'toolCallId': 'c1', 'title': 'x', 'rawInput': 'ls -la'})
        out = m.grok(permission, [raw], True)
        self.assertEqual((out['structuredApproval']['toolName'], out['structuredApproval']['command']), ('x', ''))
        ask = update({'sessionUpdate': 'tool_call', 'toolCallId': 'c1', 'title': 'ask_user_question', 'rawInput': 'text'})
        self.assertEqual(m.grok([], [ask], True)['question'], '')
        chunk = update(dict(sessionUpdate='agent_message_chunk', content=[dict(type='text', text='x')]))
        self.assertEqual(m.grok([], [chunk, dict(params='x'), dict(params=dict(update=3))], True)['text'], '')
        start = dict(type='tool.execution_start', data=dict(toolCallId='c', toolName='ask_user', arguments='{"message":"x"}'))
        self.assertEqual(m.copilot([start, dict(type='user.message', data='x')], True)['question'], '')
        self.assertEqual(m.copilot([dict(type='permission.requested', data=dict(requestId='r', permissionRequest='x'))], True)['structuredApproval']['toolName'], 'tool')
        rows = [dict(type='context.append_loop_event', event='x'), dict(type='context.append_loop_event', event=dict(type='content.part', part='x')),
                dict(type='interaction.request', id='a', kind='approval', request='x')]
        self.assertEqual(m.kimi(rows, 's:main', True)['structuredApproval']['toolName'], 'Tool')
        self.assertNotIn('structuredApproval', m.kimi([dict(type='interaction.request', id='a', kind='approval', request=dict(sessionId='s', display='x'))], 'other:main', True))

        # Grok: a parallel tool that is allowed automatically and finishes leaves the open dialog.
        call = lambda i, tool, raw: update({'sessionUpdate': 'tool_call', 'toolCallId': i, 'title': tool, 'rawInput': raw, '_meta': {'x.ai/tool': dict(name=tool, kind='execute')}})
        updates = [call('c1', 'run_terminal_command', dict(command='rm -rf probe_dir'))]
        events = [dict(type='turn_started'), dict(type='permission_requested', tool_name='run_terminal_command'),
                  dict(type='permission_requested', tool_name='list_dir'), dict(type='permission_resolved', tool_name='list_dir', decision='allow'),
                  dict(type='tool_completed', tool_name='list_dir', tool_call_id='c9')]
        out = m.grok(events, updates, True)
        self.assertEqual((out['structuredApproval']['toolName'], out['structuredApproval']['command']), ('run_terminal_command', 'rm -rf probe_dir'))
        # Two calls of one tool each hold a dialog; one answer closes one.
        twice = events + [dict(type='permission_requested', tool_name='run_terminal_command'), dict(type='permission_resolved', tool_name='run_terminal_command')]
        self.assertEqual(m.grok(twice, updates, True)['structuredApproval']['toolName'], 'run_terminal_command')
        self.assertNotIn('structuredApproval', m.grok(twice + [dict(type='permission_resolved', tool_name='run_terminal_command')], updates, True))
        self.assertNotIn('structuredApproval', m.grok(events + [dict(type='turn_ended')], updates, True))

        # Copilot: number and boolean choices are shown as written, fields in the order of the form.
        form = dict(properties=dict(zeta=dict(type='integer', enum=[1, 2]), alpha=dict(type='boolean', enum=[True, False]), mid=dict(enum=[1.5])))
        ask = lambda schema: [dict(type='tool.execution_start', data=dict(toolCallId='c', toolName='ask_user', arguments=dict(message='Set up', requestedSchema=schema)))]
        out = m.copilot(ask(form), True)
        self.assertEqual([(q['id'], [o['label'] for o in q['options']]) for q in out['structuredQuestions']],
                         [('zeta', ['1', '2']), ('alpha', ['true', 'false']), ('mid', ['1.5'])])
        odd = m.copilot(ask(dict(properties=dict(zeta=dict(enum=[dict(const=1)]), alpha=dict(enum=['a'])))), True)
        self.assertEqual(odd['question'], 'Set up'); self.assertNotIn('structuredQuestions', odd)

    def test_unreadable_record_is_a_fallback_for_read_and_a_refusal_before_send(self):
        with tempfile.TemporaryDirectory() as directory:
            home = pathlib.Path(directory).resolve()
            log = home / 'copilot/session-state' / NATIVE / 'events.jsonl'
            log.parent.mkdir(parents=True)
            log.write_text(json.dumps(dict(type='session.start', data=dict(sessionId=NATIVE))) + '\n')
            source = dict(id='src', provider='copilot', path=str(home / 'copilot'))
            row = target('needsInput'); row['path'] = str(log); row['logPaths'] = []
            row['session'].update(provider='copilot', sourceId='src', nativeSession=NATIVE, machineId='local', inputKind='')
            # A reader that fails on a shape it does not know is not an error of the read.
            with patch.object(m, 'copilot', side_effect=AttributeError('shape')):
                self.assertIsNone(m.agent_excerpt(source, row))
                self.assertIn('excerpt', m.guarded_run(dict(action='read', source=source, target=row)))
            # Before anything is typed, a failed check is a refusal: nothing may have been sent.
            with patch.object(m, 'excerpt', side_effect=RuntimeError('unreadable')), patch.object(m, 'tmux_send') as send:
                out = m.guarded_run(dict(action='send', source=source, target=row, text='y', allowTerminal=True))
                self.assertEqual(out['status'], 'blocked'); send.assert_not_called()

    def test_process_exit_before_paste_never_submits(self):
        row = target();row['session']['provider'] = 'claude';row['process'] = dict(pid=321, start=456)
        with patch.object(m, 'process_identity', side_effect=[([321, 123], '/dev/pts/2'), ValueError('gone')]), patch.object(m.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, stdout=b'%1\t/dev/pts/2\t123\tclaude\t0\n')) as command:
            self.assertEqual(m.tmux_send(row, 'retained')['status'], 'blocked')
            self.assertFalse(any(call.args[0][1] == 'paste-buffer' for call in command.call_args_list))


if __name__ == '__main__':
    unittest.main()
