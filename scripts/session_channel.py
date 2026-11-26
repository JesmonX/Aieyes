"""On-demand SSH channel adapter. Standard library; JSON on stdin, never shell input.

Unlike the lifecycle collector, this explicitly enabled adapter returns only the
current question/final reply and last user-message receipt. It never changes how
the user's Agent is launched and never creates or forks a conversation.
"""
import base64
import datetime
import hashlib
import json
import os
import pathlib
import re
import secrets
import socket
import stat
import struct
import subprocess
import time
import uuid

LIMIT = 2 * 1024 * 1024


def clip(text, size=10000):
    data = text.encode('utf-8')
    suffix = '\n…（内容已截断）'
    return text if len(data) <= size else data[:max(0, size - len(suffix.encode()))].decode('utf-8', 'ignore') + suffix


def result(status, reason, **receipt):
    return dict(status=status, reason=reason, receipt=receipt)


def timestamp(value):
    value = value.get('timestamp', value.get('time', value.get('at')))
    try:
        if isinstance(value, (int, float)):
            return int(value * 1000 if value < 10000000000 else value)
        if isinstance(value, str):
            return int(datetime.datetime.fromisoformat(value.replace('Z', '+00:00')).timestamp() * 1000)
    except (ValueError, OverflowError):
        pass
    return 0


def text_blocks(value):
    if isinstance(value, str):
        return value
    if not isinstance(value, list):
        return ''
    return '\n'.join(p['text'] for p in value if isinstance(p, dict) and p.get('type') in ('text', 'output_text', 'input_text', 'Text') and isinstance(p.get('text'), str))


def questions(items):
    if not isinstance(items, list):
        return ''
    groups = []
    for i, question in enumerate(items):
        key = question.get('id') or question.get('header') or str(i + 1)
        lines = [str(key) + ': ' + str(question.get('question') or question.get('title') or '')]
        for j, option in enumerate(question.get('options') or []):
            label = option.get('label', '') if isinstance(option, dict) else str(option)
            detail = option.get('description', '') if isinstance(option, dict) else ''
            lines.append('  %s. %s%s' % (j + 1, label, ' — ' + detail if detail else ''))
        groups.append('\n'.join(lines))
    return '\n\n'.join(groups)


def record_session(value):
    if value.get('type') == 'session_meta':
        return (value.get('payload') or {}).get('id')
    if value.get('type') == 'session':
        return value.get('id')
    return value.get('sessionId', value.get('session_id'))


def async_answers(value):
    p = value.get('payload') or {}
    if value.get('type') != 'response_item' or p.get('type') != 'message' or p.get('role') != 'user':
        return []
    calls = []
    for part in p.get('content') or []:
        text = part.get('text', '').strip() if isinstance(part, dict) else ''
        prefix, suffix = '<send_user_message_question_reply>', '</send_user_message_question_reply>'
        if not text.startswith(prefix) or not text.endswith(suffix):
            continue
        try:
            for item in json.loads(text[len(prefix):-len(suffix)]):
                key = json.loads(item['questionItemId'])
                if key[0] == 'request_user_input_async' and isinstance(key[1], str) and 0 < len(key[1]) <= 256:
                    calls.append(key[1])
        except (ValueError, TypeError, KeyError, IndexError):
            pass
    return calls


def parse(records, target):
    out = dict(text='', question='', userText='', userAt=0)
    session = target['session']
    codex = session['provider'] in ('codex', 'chatgpt-work')
    turn, call, pending_async = '', '', False
    for value in records:
        native = record_session(value)
        if (native is not None and native != session['nativeSession']) or value.get('isSidechain') or value.get('is_sidechain') or value.get('agentId'):
            continue
        at = timestamp(value)
        p = value.get('payload') or value
        if not isinstance(p, dict):
            continue
        if codex and (value.get('type') == 'turn_context' or value.get('type') == 'event_msg' and p.get('type') == 'task_started'):
            turn = p.get('turn_id', '')
        current = p.get('turn_id') or p.get('turnId') or (p.get('internal_chat_message_metadata_passthrough') or {}).get('turn_id') or turn
        if codex and session.get('turnId') and not current:
            continue
        if session.get('turnId') and current and current != session['turnId']:
            continue
        if session.get('turnStartedAt') and (not at or at < session['turnStartedAt']):
            continue
        if session.get('turnEndedAt') and at > session['turnEndedAt']:
            continue
        if codex:
            if value.get('type') == 'retained_context' and isinstance(p.get('questions'), list):
                out['question'] = questions(p['questions']); call = p.get('call_id', ''); pending_async = False
                out.pop('asyncRequest', None)
            if value.get('type') == 'response_item' and p.get('type') == 'function_call' and p.get('name', '').rsplit('.', 1)[-1].rsplit(':', 1)[-1] in ('request_user_input', 'request_user_input_async'):
                try:
                    out['question'] = questions(json.loads(p['arguments']).get('questions'))
                    call = p.get('call_id', '')
                    pending_async = p.get('name', '').endswith('request_user_input_async')
                    if pending_async:out['asyncRequest'] = dict(callId=call, questions=json.loads(p['arguments']).get('questions'))
                    else:out.pop('asyncRequest', None)
                except (KeyError, ValueError, TypeError):
                    pass
            if value.get('type') == 'response_item' and p.get('type') == 'function_call_output' and call and p.get('call_id') == call:
                try:
                    accepted = pending_async and json.loads(p.get('output', '{}')).get('accepted') is True
                except (ValueError, TypeError, AttributeError):
                    accepted = False
                if not accepted:
                    out['question'] = ''; call = ''
                    out.pop('asyncRequest', None)
            if value.get('type') == 'response_item' and p.get('type') == 'message':
                if pending_async and call in async_answers(value):
                    out['question'] = ''; call = ''
                    out.pop('asyncRequest', None)
                if p.get('role') == 'assistant' and p.get('phase') in ('final', 'final_answer'):
                    out['text'] = text_blocks(p.get('content'))
                if p.get('role') == 'user':
                    out['userText'] = text_blocks(p.get('content')); out['userAt'] = at
            if value.get('type') == 'event_msg':
                if p.get('type') == 'user_message':
                    out['userText'] = p.get('message', ''); out['userAt'] = at
                if p.get('type') == 'agent_message' and p.get('phase') == 'final':
                    out['text'] = p.get('message', '')
                if p.get('type') == 'task_complete' and isinstance(p.get('last_agent_message'), str):
                    out['text'] = p['last_agent_message']
        else:
            message = value.get('message') if isinstance(value.get('message'), dict) else value
            role = message.get('role') or value.get('type')
            if role == 'assistant':
                text = text_blocks(message.get('content'))
                blocks = message.get('content', []) if isinstance(message.get('content'), list) else []
                tool = any(isinstance(b, dict) and b.get('type') in ('tool_use', 'toolCall') for b in blocks)
                if text and not tool and message.get('phase') not in ('commentary', 'analysis'):
                    out['text'] = text
                for block in blocks:
                    if isinstance(block, dict) and block.get('type') == 'tool_use' and block.get('name') == 'AskUserQuestion':
                        out['question'] = questions((block.get('input') or {}).get('questions'))
                        call = block.get('id', '')
                        out['claudeRequest'] = dict(callId=call, input=block.get('input') or {})
            elif role == 'user':
                text = text_blocks(message.get('content'))
                if text:
                    out['userText'] = text; out['userAt'] = at
                blocks = message.get('content', []) if isinstance(message.get('content'), list) else []
                if any(isinstance(b, dict) and b.get('type') == 'tool_result' and b.get('is_error') is not True and b.get('tool_use_id') == call for b in blocks):
                    out['question'] = ''; call = ''
                    out.pop('claudeRequest', None)
    out['text'] = clip(out['text']); out['question'] = clip(out['question']); out['userText'] = clip(out['userText'], 8192)
    return out


def transcript_paths(source, target):
    root = pathlib.Path(os.path.expanduser(source['path'])).resolve()
    if root.is_file():
        root = root.parent
    paths = [target['path']] if target.get('path') else []
    paths.extend(p for p in target.get('logPaths', []) if p not in paths)
    return root, paths[:8]


def records(path, root, target):
    path = pathlib.Path(path).resolve()
    path.relative_to(root)
    session = target['session']
    with path.open('rb') as stream:
        size = os.fstat(stream.fileno()).st_size
        header = json_lines(stream.read(65536))
        identity = next((record_session(v) for v in header if record_session(v) is not None), None)
        codex = session['provider'] in ('codex', 'chatgpt-work')
        named = not codex and any(part in (session['nativeSession'], session['nativeSession'] + '.jsonl') for part in path.parts)
        if identity != session['nativeSession'] and not (identity is None and named):
            raise ValueError('会话记录身份不符')
        offset = max(0, size - LIMIT)
        stream.seek(offset)
        data = stream.read(LIMIT)
    if offset:
        data = data.split(b'\n', 1)[-1] if b'\n' in data else b''
    return json_lines(data), data.endswith(b'\n')


def json_lines(data):
    rows = []
    for line in data.splitlines(keepends=True):
        if line.endswith(b'\n'):
            try:
                value = json.loads(line)
                if isinstance(value, dict):
                    rows.append(value)
            except (ValueError, UnicodeError):
                pass
    return rows


def choice_rows(items):
    """Questions in the shape the core's own readers produce."""
    rows = []
    for i, q in enumerate(items if isinstance(items, list) else []):
        if not isinstance(q, dict):
            return []
        options = []
        for o in q.get('options') or []:
            item = dict(label=str((o.get('label') or o.get('name') or '') if isinstance(o, dict) else o))
            if isinstance(o, dict) and isinstance(o.get('description'), str) and o['description']:
                item['description'] = o['description']
            options.append(item)
        rows.append(dict(id=str(q.get('id') or q.get('header') or i + 1), question=str(q.get('question') or q.get('title') or ''),
                         multiSelect=any(q.get(k) is True for k in ('multiSelect', 'is_multi_select', 'multi_select')), options=options))
    return rows


def subject(args):
    args = args if isinstance(args, dict) else {}
    return next((args[k] for k in ('command', 'file_path', 'filePath', 'path', 'target_file', 'url') if isinstance(args.get(k), str) and args[k]), '')


def approval(out, tool, reason, command, details='', kind='tool', cwd=''):
    out['question'] = '\n'.join(s for s in (tool, reason, command) if s)
    out['structuredApproval'] = dict(toolName=tool, reason=reason, command=clip(command, 4000), details=clip(details, 2000), kind=kind, cwd=cwd)


def choices(out, items):
    rows = choice_rows(items)
    if rows:
        out['question'] = questions(rows)
        out['structuredQuestions'] = rows
    return bool(rows)


def string(value):
    return value if isinstance(value, str) else ''


def obj(value):
    """Record fields are arbitrary JSON: anything that is not an object reads as an empty one."""
    return value if isinstance(value, dict) else {}


def tail_rows(path, root):
    path = pathlib.Path(path).resolve()
    path.relative_to(root)
    with path.open('rb') as stream:
        offset = max(0, os.fstat(stream.fileno()).st_size - LIMIT)
        stream.seek(offset)
        data = stream.read(LIMIT)
    if offset:
        data = data.split(b'\n', 1)[-1] if b'\n' in data else b''
    return json_lines(data)


def kimi(rows, native, waiting):
    """Kimi Code 2.4 `wire.jsonl`: `interaction.request` stays open until `interaction.resolved`."""
    out, pending = dict(text='', question='', userText='', userAt=0), []
    for v in rows:
        kind, event = v.get('type'), obj(v.get('event'))
        if kind == 'turn.prompt':
            out.update(userText=text_blocks(v.get('input')), userAt=v.get('time') if isinstance(v.get('time'), int) else 0, text='')
            pending = []
        elif kind == 'context.append_loop_event':
            part = obj(event.get('part'))
            if event.get('type') == 'content.part' and part.get('type') == 'text':
                out['text'] = string(part.get('text'))
            elif event.get('type') == 'tool.call':
                out['text'] = ''
        elif kind == 'interaction.request':
            pending.append(v)
        elif kind == 'interaction.resolved':
            pending = [p for p in pending if p.get('id') != v.get('id')]
        elif kind == 'turn.ended' and v.get('reason') != 'blocked':
            pending = []
    if not waiting:
        return out
    row = pending[-1] if pending else None
    request = obj((row or {}).get('request'))
    if row is None or request.get('sessionId') not in (None, native.split(':')[0]):
        out['question'] = out['text']
    elif row.get('kind') == 'question':
        choices(out, request.get('questions'))
    elif row.get('kind') == 'approval':
        display = obj(request.get('display'))
        details = '- %s\n+ %s' % (string(display.get('before')), string(display.get('after'))) if display.get('operation') == 'edit' else string(display.get('content'))
        approval(out, string(request.get('toolName')) or 'Tool', string(request.get('action')), subject(display), details,
                 dict(command='command', file_io='fileChange').get(display.get('kind'), 'tool'), string(display.get('cwd')))
    return out


def grok(events, updates, waiting):
    """Grok CLI 1.0: `events.jsonl` names the tool awaiting permission, `updates.jsonl` its call."""
    out, calls, pending = dict(text='', question='', userText='', userAt=0), [], []
    for v in updates:
        u = obj(obj(v.get('params')).get('update'))
        call, kind = string(u.get('toolCallId')), u.get('sessionUpdate')
        if kind == 'user_message_chunk':
            out.update(userText=string(obj(u.get('content')).get('text')), userAt=(v.get('timestamp') if isinstance(v.get('timestamp'), int) else 0) * 1000, text='')
            calls = []
        elif kind == 'agent_message_chunk':
            out['text'] += string(obj(u.get('content')).get('text'))
        elif kind == 'tool_call' and call:
            meta = obj(obj(u.get('_meta')).get('x.ai/tool'))
            out['text'] = ''
            calls = [c for c in calls if c[0] != call] + [(call, string(meta.get('name')) or string(u.get('title')), string(meta.get('kind')), obj(u.get('rawInput')))]
        elif kind == 'tool_call_update' and u.get('status') in ('completed', 'failed', 'cancelled', 'canceled', 'rejected'):
            calls = [c for c in calls if c[0] != call]
    # The tools with an open permission dialog, oldest first; a row names only its tool.
    for v in events:
        kind, tool = v.get('type'), string(v.get('tool_name'))
        last = max((i for i, t in enumerate(pending) if t == tool), default=None)
        if kind == 'permission_requested':
            pending.append(tool)
        elif kind in ('permission_resolved', 'permission_response', 'permission_responded'):
            if not tool:
                del pending[-1:]
            elif last is not None:
                del pending[last]
        elif kind in ('tool_completed', 'tool_finished', 'tool_failed'):
            # Another tool finishing says nothing about this one's dialog.
            if tool and last is not None:
                del pending[last]
        elif kind in ('turn_started', 'turn_ended'):
            pending = []
    if not waiting:
        return out
    permission = pending[-1] if pending else ''
    if permission:
        same = [c for c in calls if c[1] == permission]
        # The permission row names only its tool: with two calls of it, neither is shown.
        only = same[0] if len(same) == 1 else ('', '', '', {})
        approval(out, permission, string(only[3].get('description')), subject(only[3]), '',
                 'command' if only[2] == 'execute' else 'fileChange' if only[2] in ('edit', 'write', 'delete', 'move') else 'tool')
        del out['structuredApproval']['cwd']
        return out
    ask = next((c for c in reversed(calls) if c[1] == 'ask_user_question' or c[2] == 'ask_user'), None)
    if not (ask and choices(out, ask[3].get('questions'))):
        out['question'] = out['text']
    return out


def copilot(rows, waiting):
    """Copilot CLI 1.0 `events.jsonl`: `permission.requested` / `permission.completed`, `ask_user`."""
    out, permissions, asks = dict(text='', question='', userText='', userAt=0), [], []
    for v in rows:
        kind, d = v.get('type'), obj(v.get('data'))
        if kind == 'user.message':
            out.update(userText=string(d.get('content')), userAt=timestamp(v), text='')
            permissions, asks = [], []
        elif kind == 'assistant.message':
            out['text'] = '' if d.get('toolRequests') else string(d.get('content'))
        elif kind == 'tool.execution_start' and d.get('toolName') == 'ask_user':
            asks.append((d.get('toolCallId'), obj(d.get('arguments'))))
        elif kind == 'tool.execution_complete':
            asks = [a for a in asks if a[0] != d.get('toolCallId')]
        elif kind == 'permission.requested':
            permissions.append(d)
        elif kind == 'permission.completed':
            permissions = [p for p in permissions if p.get('requestId') != d.get('requestId')]
        elif kind in ('abort', 'session.abort', 'assistant.abort', 'session.error'):
            permissions, asks = [], []
    if not waiting:
        return out
    if permissions:
        request = obj(permissions[-1].get('permissionRequest'))
        kind = string(request.get('kind')) or 'tool'
        command = next((request[k] for k in ('fullCommandText', 'fileName', 'path', 'url', 'toolName') if isinstance(request.get(k), str) and request[k]), '')
        approval(out, kind, string(request.get('intention')), command, string(request.get('diff')), dict(shell='command', write='fileChange').get(kind, 'tool'))
        del out['structuredApproval']['cwd']
    elif asks:
        args = asks[-1][1]
        message, fields = string(args.get('message')), obj(args.get('requestedSchema')).get('properties')
        fields = fields if isinstance(fields, dict) and 0 < len(fields) <= 32 else {}
        # Only a form made of plain choices is shown as one; anything else is its message.
        # Numbers and booleans are shown as written (`true`, not `True`), like the core's reader.
        plain = lambda o: isinstance(o, (str, int, float))
        label = lambda o: o if isinstance(o, str) else json.dumps(o)
        if fields and all(isinstance(f, dict) and isinstance(f.get('enum'), list) and f['enum'] and all(plain(o) for o in f['enum']) for f in fields.values()):
            single = len(fields) == 1 and message
            choices(out, [dict(id=key, question=message if single else string(f.get('title')) or string(f.get('description')) or key, options=[label(o) for o in f['enum']]) for key, f in fields.items()])
            if not single and message:
                out['question'] = message + '\n' + out['question']
        else:
            out['question'] = message
    else:
        out['question'] = out['text']
    return out


def codebuddy(rows, waiting, asked):
    """CodeBuddy / WorkBuddy transcript: a `function_call` is open until a result names its callId."""
    out, calls = dict(text='', question='', userText='', userAt=0), []
    for v in rows:
        if v.get('isSidechain') is True:
            continue
        kind, role = v.get('type'), v.get('role')
        if kind == 'message' and role == 'user':
            out.update(userText=text_blocks(v.get('content')), userAt=v.get('timestamp') if isinstance(v.get('timestamp'), int) else 0, text='')
            calls = []
        elif kind == 'message' and role == 'assistant' and v.get('status') == 'completed':
            out['text'] = text_blocks(v.get('content'))
        elif kind == 'function_call':
            out['text'] = ''
            calls.append((v.get('callId'), string(v.get('name')), string(v.get('arguments'))))
        elif kind in ('function_call_output', 'function_call_result'):
            calls = [c for c in calls if c[0] != v.get('callId')]
    if not waiting:
        return out

    def args(raw):
        try:
            value = json.loads(raw)
            return value if isinstance(value, dict) else {}
        except ValueError:
            return {}
    ask = next((c for c in reversed(calls) if c[1] == 'AskUserQuestion'), None)
    if ask and choices(out, args(ask[2]).get('questions')):
        return out
    if asked and len(calls) == 1:
        # With several calls in flight the transcript does not say which one is being asked.
        value = args(calls[0][2])
        details = next((value[k] for k in ('content', 'new_string') if isinstance(value.get(k), str)), '')
        approval(out, calls[0][1], string(value.get('description')), subject(value), details, 'command' if isinstance(value.get('command'), str) else 'tool')
        del out['structuredApproval']['cwd']
    elif not asked:
        out['question'] = out['text']
    return out


def agent_excerpt(source, target):
    """Agents whose record is not a Claude or Codex transcript. None: no reader, or no record."""
    session = target['session']
    provider, native, waiting = session['provider'], session['nativeSession'], session['phase'] == 'needsInput'
    if provider not in ('kimi', 'grok', 'copilot', 'codebuddy', 'workbuddy'):
        return None
    root, paths = transcript_paths(source, target)
    paths = [pathlib.Path(p) for p in paths]
    try:
        if provider == 'kimi':
            def name(p):
                return '%s:%s' % (p.parent.parent.parent.name, p.parent.name) if p.parent.parent.name == 'agents' else p.parent.name
            path = next(p for p in paths if p.name == 'wire.jsonl' and name(p) == native)
            out = kimi(tail_rows(path, root), native, waiting)
        elif not re.fullmatch(r'[A-Za-z0-9_-]{1,128}', native):
            return None
        elif provider in ('grok', 'copilot'):
            path = next(p for p in paths if p.name == 'events.jsonl' and p.parent.name == native)
            rows = records(path, root, target)[0]
            if provider == 'copilot':
                out = copilot(rows, waiting)
            else:
                try:
                    updates = records(path.with_name('updates.jsonl'), root, target)[0]
                except (OSError, ValueError):
                    updates = []
                out = grok(rows, updates, waiting)
        else:
            path = next((p for p in paths if p.suffix == '.jsonl' and p.stem == native), None)
            if path is None:
                path = next(p for p in (d / (native + '.jsonl') for d in sorted((root / 'projects').iterdir())[:4096]) if p.is_file())
            out = codebuddy(records(path, root, target)[0], waiting, session.get('inputKind') == 'approval')
    except (OSError, ValueError, StopIteration, AttributeError, TypeError, KeyError, IndexError):
        # A record in a shape no reader knows is read as a plain transcript, never guessed at.
        return None
    out['text'] = clip(out['text']); out['question'] = clip(out['question']); out['userText'] = clip(out['userText'], 8192)
    return out


def excerpt(source, target):
    out = agent_excerpt(source, target)
    if out is not None:
        return out
    root, paths = transcript_paths(source, target)
    for name in paths[:8]:
        try:
            rows, _ = records(name, root, target)
            out = parse(rows, target)
            if out['text'] or out['question'] or out['userAt']:
                return out
        except (OSError, ValueError):
            continue
    return parse([], target)


def verify_codex_record(source, target):
    session = target['session']
    if not session.get('turnId'):
        raise ValueError('原会话缺少轮次标识')
    root, paths = transcript_paths(source, target)
    for path in paths:
        try:
            rows, complete = records(path, root, target)
        except (OSError, ValueError):
            continue
        latest = next((v for v in reversed(rows) if v.get('type') == 'turn_context' or v.get('type') == 'event_msg' and (v.get('payload') or {}).get('type') == 'task_started'), {})
        if not complete or (latest.get('payload') or {}).get('turn_id') != session['turnId']:
            raise ValueError('原会话记录已改变或无法核对')
        if session.get('workspace'):
            cwd = next(((v.get('payload') or {}).get('cwd') for v in reversed(rows) if v.get('type') in ('turn_context', 'session_meta') and (v.get('payload') or {}).get('cwd') is not None), None)
            if cwd != session['workspace']:
                raise ValueError('原会话工作目录已改变或无法核对')
        return
    raise ValueError('原会话记录不可用')


class Rejected(Exception):
    pass


class WebSocket:
    """Only the local Codex Unix socket; no internet listener, TLS or credentials."""
    def __init__(self, path):
        meta = os.lstat(path)
        if not (stat.S_ISSOCK(meta.st_mode) or stat.S_ISLNK(meta.st_mode)) or meta.st_uid != os.geteuid():
            raise ValueError('会话控制接口身份不符')
        path = pathlib.Path(path).resolve(strict=True)
        meta = os.lstat(path)
        if not stat.S_ISSOCK(meta.st_mode) or meta.st_uid != os.geteuid():
            raise ValueError('会话控制接口身份不符')
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.settimeout(3)
        self.socket.connect(str(path))
        self.buffer = bytearray()
        key = base64.b64encode(secrets.token_bytes(16)).decode()
        self.socket.sendall(('GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: ' + key + '\r\n\r\n').encode())
        while b'\r\n\r\n' not in self.buffer:
            received = self.socket.recv(4096)
            if not received:
                raise OSError('会话连接已关闭')
            self.buffer.extend(received)
            if len(self.buffer) > 16384:
                raise ValueError('会话握手响应过大')
        head, body = bytes(self.buffer).split(b'\r\n\r\n', 1)
        self.buffer = bytearray(body)
        lines = head.decode('ascii').split('\r\n')
        headers = dict((k.lower(), v.strip()) for k, v in (line.split(':', 1) for line in lines[1:] if ':' in line))
        accept = base64.b64encode(hashlib.sha1((key + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').encode()).digest()).decode()
        if lines[0].split()[1] != '101' or headers.get('sec-websocket-accept') != accept:
            raise ValueError('会话握手失败')
        self.socket.settimeout(.15)
        self.fragments = bytearray()
        self.pending = []
        self.sequence = 0
        self.call('initialize', dict(clientInfo=dict(name='aieyes', version='1'), capabilities=dict(experimentalApi=True)))
        self.send(dict(method='initialized'))

    def close(self):
        self.socket.close()

    def frame(self, data, opcode=1):
        mask = secrets.token_bytes(4)
        length = len(data)
        header = bytes([128 | opcode, 128 | length]) if length < 126 else bytes([128 | opcode, 128 | 126]) + struct.pack('!H', length) if length < 65536 else bytes([128 | opcode, 128 | 127]) + struct.pack('!Q', length)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
        self.socket.sendall(header + mask + masked)

    def send(self, value):
        self.frame(json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode())

    def fill(self, size):
        if size > LIMIT:
            raise ValueError('会话响应过大')
        while len(self.buffer) < size:
            data = self.socket.recv(min(65536, size - len(self.buffer)))
            if not data:
                raise OSError('会话连接已关闭')
            self.buffer.extend(data)

    def receive(self):
        try:
            self.fill(2)
            first, second = self.buffer[:2]
            if second & 128 or first & 112:
                raise ValueError('会话帧格式无效')
            length, offset = second & 127, 2
            if length == 126:
                self.fill(4); length = struct.unpack('!H', self.buffer[2:4])[0]; offset = 4
            elif length == 127:
                self.fill(10); length = struct.unpack('!Q', self.buffer[2:10])[0]; offset = 10
            self.fill(offset + length)
            body = bytes(self.buffer[offset:offset + length]); del self.buffer[:offset + length]
            opcode = first & 15
            if opcode == 8:
                raise OSError('会话连接已关闭')
            if opcode == 9:
                self.frame(body, 10); return None
            if opcode in (0, 1):
                self.fragments.extend(body)
                if len(self.fragments) > LIMIT:
                    raise ValueError('会话响应过大')
                if first & 128:
                    data = bytes(self.fragments); self.fragments.clear()
                    return json.loads(data)
            return None
        except socket.timeout:
            return None

    def capture(self, value):
        if not isinstance(value, dict):
            raise ValueError('会话响应格式无效')
        if 'id' in value and 'method' in value and len(self.pending) < 32:
            self.pending.append(value)
        if value.get('method') == 'serverRequest/resolved':
            self.pending = [r for r in self.pending if r['id'] != value['params']['requestId']]

    def call(self, method, params):
        self.sequence += 1
        key = 'aieyes-' + str(self.sequence)
        self.send(dict(id=key, method=method, params=params))
        until = time.monotonic() + 4
        while time.monotonic() < until:
            value = self.receive()
            if value:
                if value.get('id') == key:
                    if 'error' in value:
                        raise Rejected('原会话拒绝此请求')
                    return value['result']
                self.capture(value)
        raise TimeoutError('会话控制接口响应超时')

    def attach(self, target, only_loaded=False):
        session = target['session']; native = session['nativeSession']
        uuid.UUID(native)
        self.verify(target)
        if only_loaded and native not in self.call('thread/loaded/list', {})['data']:
            raise Rejected('原会话未在控制服务中运行')
        thread = self.call('thread/resume', dict(threadId=native, excludeTurns=True))['thread']
        if thread['id'] != native:
            raise Rejected('原会话身份不符')
        return thread

    def verify(self, target):
        session = target['session']; native = session['nativeSession']
        if not session.get('turnId'):
            raise Rejected('原会话缺少轮次标识')
        thread = self.call('thread/read', dict(threadId=native, includeTurns=False))['thread']
        if thread.get('id') != native:
            raise Rejected('原会话身份不符')
        if session.get('workspace') and thread.get('cwd') != session['workspace']:
            raise Rejected('原会话工作目录已改变或无法核对')
        turns = self.call('thread/turns/list', dict(threadId=native, limit=1, sortDirection='desc', itemsView='notLoaded')).get('data') or []
        if not turns or turns[0].get('id') != session['turnId']:
            raise Rejected('原会话已进入其他轮次，请发送新指令')
        return thread

    def question(self, target):
        until = time.monotonic() + .45
        while time.monotonic() < until:
            value = self.receive()
            if value:
                self.capture(value)
        session = target['session']
        # Oldest first, the order the original window shows its own dialogs in. MCP elicitations
        # and legacy approvals may carry no turn; verify() ties the latest turn to the target.
        waiting = [r for r in self.pending if isinstance(r.get('params'), dict) and (r['params'].get('threadId') or r['params'].get('conversationId')) == session['nativeSession'] and (not session.get('turnId') or r['params'].get('turnId') == session['turnId'] or r['params'].get('turnId') is None and r['method'] in TURN_OPTIONAL)]
        if not waiting:
            return None
        waiting[0]['params']['pendingCount'] = len(waiting)
        return waiting[0]


COMMAND, FILE_CHANGE, PERMISSIONS, ELICITATION = 'item/commandExecution/requestApproval', 'item/fileChange/requestApproval', 'item/permissions/requestApproval', 'mcpServer/elicitation/request'
LEGACY = ('execCommandApproval', 'applyPatchApproval')
USER_INPUT = ('item/tool/requestUserInput', 'tool/requestUserInput')
TURN_OPTIONAL = (ELICITATION,) + LEGACY


def fingerprint(request):
    keys = ('threadId', 'conversationId', 'turnId', 'itemId', 'callId', 'approvalId', 'kind', 'questions', 'command', 'reason', 'availableDecisions', 'cwd', 'grantRoot', 'commandActions', 'permissions', 'changes', 'networkApprovalContext', 'proposedExecpolicyAmendment', 'additionalPermissions', 'fileChanges', 'serverName', 'mode', 'message', 'requestedSchema', '_meta', 'url', 'elicitationId')
    params = {key: request['params'].get(key) for key in keys}
    return hashlib.sha256(json.dumps(dict(method=request['method'], params=params), sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()


def consent(request):
    """A form-mode elicitation that asks for nothing but consent, such as an MCP tool call approval."""
    p = request['params']
    schema = p.get('requestedSchema')
    return request['method'] == ELICITATION and p.get('mode') == 'form' and isinstance(schema, dict) and schema.get('properties') == {}


def is_approval(request):
    return request['method'] in (COMMAND, FILE_CHANGE, PERMISSIONS) + LEGACY or consent(request)


def without_nulls(value):
    return {k: without_nulls(v) for k, v in value.items() if v is not None} if isinstance(value, dict) else value


def permission_lines(profile):
    if not isinstance(profile, dict):
        return []
    lines = ['network'] if (profile.get('network') or {}).get('enabled') is True else []
    files = profile.get('fileSystem') or {}
    if files.get('entries'):
        for entry in files['entries']:
            path = entry.get('path') or {}
            name = path.get('path') or path.get('pattern') or json.dumps(path.get('value'), ensure_ascii=False)
            lines.append('%s: %s' % (entry.get('access'), name))
    else:
        lines += ['%s: %s' % (access, path) for access in ('read', 'write') for path in files.get(access) or []]
    return lines


def plain_replies(request):
    """Native replies for /approve, /decline and /cancel-request, by the rules of codex/card.rs.

    Scoped and persistent decisions are only offered by the desktop card, which sends the
    native reply itself (see checked_reply)."""
    p, method = request['params'], request['method']
    if method == COMMAND:
        # Input for a running terminal and managed checkpoints stay in the original window.
        if p.get('kind') not in (None, 'command'):
            return {}
        listed = p.get('availableDecisions')
        names = ['accept', 'decline'] if listed is None else [d for d in listed if isinstance(d, str) and d in ('accept', 'decline', 'cancel')]
        # 0.172.0 takes a plain refusal even when the prompt does not list one.
        if names and 'decline' not in names:
            names.append('decline')
        return {slash: dict(decision=name) for slash, name in (('/approve', 'accept'), ('/decline', 'decline'), ('/cancel-request', 'cancel')) if name in names}
    if method == FILE_CHANGE:
        return {'/approve': dict(decision='accept'), '/decline': dict(decision='decline'), '/cancel-request': dict(decision='cancel')}
    if method == PERMISSIONS:
        if not permission_lines(p.get('permissions')):
            return {}
        return {'/approve': dict(permissions=without_nulls(p['permissions']), scope='turn'), '/decline': dict(permissions={}, scope='turn')}
    if consent(request):
        return {'/approve': dict(action='accept', content={}), '/decline': dict(action='decline'), '/cancel-request': dict(action='cancel')}
    if method in LEGACY:
        return {'/approve': dict(decision='approved'), '/decline': dict(decision=dict(denied=dict(rejection=''))), '/cancel-request': dict(decision='abort')}
    return {}


def checked_reply(request, reply):
    """A native reply built by the desktop core for exactly this request. Only shapes the
    request's own reply type allows are passed on; listed decision objects must be echoed."""
    p, method = request['params'], request['method']
    bad = ValueError('原会话不允许此审批选项')
    if not isinstance(reply, dict):
        raise bad
    if method == COMMAND:
        decision, listed = reply.get('decision'), p.get('availableDecisions')
        if set(reply) != {'decision'} or p.get('kind') not in (None, 'command'):
            raise bad
        offered = ['accept'] if listed is None else [d for d in listed if isinstance(d, dict) or d in ('accept', 'acceptForSession', 'cancel')]
        if not offered or decision != 'decline' and decision not in offered:
            raise bad
    elif method == FILE_CHANGE:
        if set(reply) != {'decision'} or reply['decision'] not in ('accept', 'acceptForSession', 'decline', 'cancel'):
            raise bad
    elif method == PERMISSIONS:
        if set(reply) != {'permissions', 'scope'} or reply['scope'] not in ('turn', 'session') or reply['permissions'] not in ({}, without_nulls(p.get('permissions'))):
            raise bad
    elif method == ELICITATION:
        if p.get('mode') != 'form' or reply.get('action') not in ('accept', 'decline', 'cancel') or not set(reply) <= {'action', 'content', '_meta'}:
            raise bad
        fields = (p.get('requestedSchema') or {}).get('properties')
        if reply['action'] == 'accept' and (not isinstance(fields, dict) or not isinstance(reply.get('content'), dict) or not set(reply['content']) <= set(fields)):
            raise bad
    elif method in LEGACY:
        decision = reply.get('decision')
        denied = isinstance(decision, dict) and set(decision) == {'denied'} and isinstance(decision['denied'], dict) and isinstance(decision['denied'].get('rejection'), str)
        if set(reply) != {'decision'} or not (denied or decision in ('approved', 'approved_for_session', 'abort')):
            raise bad
    elif method in USER_INPUT:
        answers, ids = reply.get('answers'), [q.get('id') for q in p.get('questions') or []]
        if set(reply) != {'answers'} or not isinstance(answers, dict) or not answers or not set(answers) <= set(ids):
            raise ValueError('回答与原问题不符，请刷新后再操作')
        for value in answers.values():
            if not isinstance(value, dict) or set(value) != {'answers'} or not value['answers'] or not all(isinstance(a, str) and a for a in value['answers']):
                raise ValueError('问题回答不能为空')
    else:
        raise ValueError('此类输入需要回到原窗口处理，指令已保留')
    return reply


def describe(request, language='zh-CN'):
    p, method = request['params'], request['method']
    english = language == 'en'
    if method in USER_INPUT:
        text = questions(p.get('questions'))
        if len(p.get('questions') or []) > 1:
            text += '\n\n' + ('Reply with one question_id=answer per line.' if english else '请按「问题编号=答案」逐行回复。')
        return text
    if is_approval(request) and plain_replies(request):
        meta = p.get('_meta') if isinstance(p.get('_meta'), dict) else {}
        if method == PERMISSIONS:
            body = '\n'.join(permission_lines(p.get('permissions')))
        elif method == ELICITATION:
            rows = ['%s: %s' % (k, v if isinstance(v, str) else json.dumps(v, ensure_ascii=False)) for k, v in (meta.get('tool_params') or {}).items()] if isinstance(meta.get('tool_params'), dict) else []
            body = '\n'.join([' · '.join(x for x in (p.get('serverName'), meta.get('tool_name')) if x)] + rows)
        elif method == 'applyPatchApproval':
            body = '\n'.join(p.get('fileChanges') or {})
        else:
            body = p.get('command') or ('File changes' if english else '文件修改')
            if isinstance(body, list):
                body = ' '.join(body)
        reason = p.get('reason') or p.get('message') or ('The original session requests approval' if english else '原会话请求确认')
        instructions = 'Quote this message and reply /approve to allow this operation once, or /decline to deny.' if english else '引用此消息回复 /approve 允许本次操作，/decline 拒绝。'
        return str(reason) + '\n' + clip(str(body), 6000) + '\n\n' + instructions
    return 'Handle this request in the original window.' if english else '此类输入需要回到原窗口处理。'


def answer(request, text):
    p, method = request['params'], request['method']
    if method in USER_INPUT + ('aieyes/asyncQuestion',):
        items = p.get('questions') or []
        if not items:
            raise ValueError('原会话问题已不可用')
        answers = {}
        for q in items:
            if len(items) == 1:
                selected = text.strip()
            else:
                values = [line.split('=', 1)[1].strip() for line in text.splitlines() if '=' in line and line.split('=', 1)[0].strip() == q['id']]
                if len(values) != 1:
                    raise ValueError('请按「问题编号=答案」逐行回答所有问题')
                selected = values[0]
            if not selected:
                raise ValueError('问题回答不能为空')
            options = q.get('options') or []
            if selected.isdigit() and 0 < int(selected) <= len(options):
                selected = options[int(selected) - 1]['label']
            answers[q['id']] = dict(answers=[selected])
        return dict(answers=answers)
    if is_approval(request):
        if text.strip() not in ('/approve', '/decline', '/cancel-request'):
            raise ValueError('此操作需要明确回复 /approve 或 /decline')
        reply = plain_replies(request).get(text.strip())
        if reply is None:
            raise ValueError('原会话不允许此审批选项')
        return reply
    raise ValueError('此类输入需要回到原窗口处理，指令已保留')


def call_patch(source, target, item):
    """The apply_patch text of exactly the tool call `item` in the rollout, for a file-change card."""
    def find(value):
        if isinstance(value, str):
            return value if '*** Begin Patch' in value else None
        children = value if isinstance(value, list) else list(value.values()) if isinstance(value, dict) else []
        return next((hit for hit in map(find, children) if hit), None)
    root, paths = transcript_paths(source, target)
    for name in paths[:8]:
        try:
            rows, _ = records(name, root, target)
        except (OSError, ValueError):
            continue
        for row in reversed(rows):
            payload = row.get('payload') if isinstance(row.get('payload'), dict) else {}
            if payload.get('call_id') == item and payload.get('type') in ('function_call', 'custom_tool_call'):
                arguments = payload.get('arguments')
                try:
                    arguments = json.loads(arguments) if isinstance(arguments, str) else arguments
                except ValueError:
                    pass
                patch = find(arguments) or find(payload.get('input'))
                return clip(patch, 65536) if patch else None
    return None


def async_request(target, value):
    raw = value.get('asyncRequest') or {}
    if not raw.get('callId') or not raw.get('questions'):return None
    normalized = []
    for i, q in enumerate(raw['questions']):
        options = [dict(label=o if isinstance(o,str) else o.get('label','')) for o in q.get('options') or []]
        normalized.append(dict(id=str(i+1), questionIndex=i, question=q.get('question') or q.get('title') or '', options=options))
    return dict(method='aieyes/asyncQuestion', params=dict(threadId=target['session']['nativeSession'], turnId=target['session']['turnId'], itemId=raw['callId'], questions=normalized))


def async_answer(request, text):
    answers = answer(request, text)['answers']; p=request['params']
    rows = [dict(answer=answers[q['id']]['answers'][0], question=q['question'], questionItemId=json.dumps(['request_user_input_async',p['itemId'],q['questionIndex']],separators=(',',':'),ensure_ascii=False)) for q in p['questions']]
    return '<send_user_message_question_reply>\n'+json.dumps(rows,separators=(',',':'),ensure_ascii=False)+'\n</send_user_message_question_reply>'


def codex_send(source, target, text, prompt_hash, native=None):
    root = pathlib.Path(os.path.expanduser(source['path']))
    ws = None
    try:
        ws = WebSocket(root / 'app-server-control/app-server-control.sock')
    except (OSError, ValueError, Rejected, TimeoutError):
        pass
    if ws:
        try:
            request = async_request(target, excerpt(source,target)) if target['session']['phase'] == 'needsInput' else None
            if request:
                if not prompt_hash or fingerprint(request) != prompt_hash:
                    return result('blocked', '问题已改变或尚未同步，请回复最新的问题通知')
                if native is not None:
                    return result('blocked', '问题已改变或尚未同步，请回复最新的问题通知')
                reply = async_answer(request,text)
                if (ws.verify(target).get('status') or {}).get('type') != 'active':
                    return result('blocked', '原会话状态已改变，指令已保留')
                try:
                    response = ws.call('turn/steer',dict(threadId=target['session']['nativeSession'],expectedTurnId=target['session']['turnId'],input=[dict(type='text',text=reply,text_elements=[])]))
                    if response.get('turnId') == target['session']['turnId']:
                        return result('confirmed','原会话已接收回答',adapter='codex',asyncAnswer=True,turnId=response['turnId'])
                    return result('uncertain','原会话返回的执行结果不完整，请核对原窗口')
                except Rejected:
                    return result('blocked','原会话拒绝了指令，请核对原窗口')
                except (OSError,ValueError,TimeoutError):
                    return result('uncertain','回答可能已提交，请核对原会话')
            thread = ws.attach(target, target['session']['phase'] == 'needsInput')
            if target['session']['phase'] == 'needsInput':
                request = ws.question(target)
                if not request or not prompt_hash or fingerprint(request) != prompt_hash:
                    return result('blocked', '问题已改变或尚未同步，请回复最新的问题通知')
                try:
                    reply = checked_reply(request, native) if native is not None else answer(request, text)
                except ValueError as error:
                    return result('blocked', str(error))
                ws.verify(target)
                if not any(r['id'] == request['id'] and fingerprint(r) == fingerprint(request) for r in ws.pending):
                    return result('blocked', '原会话的问题已结束或改变，请回复最新通知')
                try:
                    ws.send(dict(id=request['id'], result=reply))
                    until = time.monotonic() + 4
                    while time.monotonic() < until:
                        value = ws.receive()
                        if value and value.get('method') == 'serverRequest/resolved' and value['params']['requestId'] == request['id'] and value['params']['threadId'] == (request['params'].get('threadId') or request['params'].get('conversationId')):
                            return result('confirmed', '原会话已接收回答', adapter='codex', requestResolved=True)
                    return result('submitted', '回答已提交，等待原会话确认', adapter='codex', answer=True)
                except (OSError, ValueError):
                    return result('uncertain', '回答可能已提交，请核对原会话')
            if native is not None:
                return result('blocked', '原会话没有可回答的请求，请核对原窗口')
            if (thread.get('status') or {}).get('type') == 'idle':
                if text.strip() in ('/approve', '/decline', '/cancel-request'):
                    return result('blocked', '当前没有审批请求，请回复具体问题通知')
                if (ws.verify(target).get('status') or {}).get('type') != 'idle':
                    return result('blocked', '原会话状态已改变，指令已保留')
                try:
                    response = ws.call('turn/start', dict(threadId=target['session']['nativeSession'], input=[dict(type='text', text=text, text_elements=[])]))
                    turn = response.get('turn') or {}
                    if turn.get('id') and turn['id'] != target['session'].get('turnId') and turn.get('status') in ('inProgress', 'completed') and not turn.get('error'):
                        return result('confirmed', '原会话已开始执行', adapter='codex', turnId=turn['id'])
                    return result('uncertain', '原会话返回的执行结果不完整，请核对原窗口')
                except Rejected:
                    return result('blocked', '原会话拒绝了指令，请核对原窗口')
                except (OSError, ValueError, TimeoutError):
                    return result('uncertain', '指令可能已提交，请核对原会话；系统不会重发')
            if (thread.get('status') or {}).get('activeFlags'):
                return result('blocked', '原会话正在等待回答，请回复最新的问题通知')
        except (OSError, ValueError, Rejected, TimeoutError):
            return result('blocked', '无法核对原会话控制状态，指令已保留')
        finally:
            ws.close()
    if target['session']['phase'] == 'needsInput' or native is not None:
        return result('blocked', '无法连接原会话的提问接口，回答已保留')
    if text.strip() in ('/approve', '/decline', '/cancel-request'):
        return result('blocked', '当前没有可确认的审批请求')
    try:
        uuid.UUID(target['session']['nativeSession'])
        env = dict(os.environ, CODEX_HOME=str(root))
        binary = os.path.expanduser(source.get('codexBinary') or 'codex')
        help_result = subprocess.run([binary, 'queue', '--help'], env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=4)
        if help_result.returncode or b'--thread' not in help_result.stdout:
            return result('blocked', '此 Codex 版本没有既有会话排队接口')
        verify_codex_record(source, target)
    except (ValueError, OSError, subprocess.TimeoutExpired):
        return result('blocked', '无法找到原会话的排队接口')
    try:
        queued = subprocess.run([binary, 'queue', '--thread', target['session']['nativeSession'], '--message', text, '--resume'], env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=12)
        if queued.returncode == 0:
            return result('submitted', '已排队，等待原会话恢复；排队成功不代表已开始执行', adapter='codexQueue')
    except (OSError, subprocess.TimeoutExpired):
        pass
    return result('uncertain', '排队结果无法确认，请核对原会话；系统不会重发')


def process_identity(pid, expected):
    if not isinstance(pid, int) or pid <= 0 or not expected:
        raise ValueError('缺少原会话进程身份')
    boot = next(int(line.split()[1]) for line in pathlib.Path('/proc/stat').read_text().splitlines() if line.startswith('btime '))
    ticks = os.sysconf('SC_CLK_TCK')
    parents, current = [], pid
    for _ in range(64):
        fields = pathlib.Path('/proc/%s/stat' % current).read_text().rsplit(')', 1)[1].split()
        started = boot + int(fields[19]) // ticks
        if current == pid:
            if started != expected:
                raise ValueError('原会话进程身份已改变')
            if int(fields[2]) <= 0 or fields[2] != fields[5]:
                raise ValueError('原 Agent 已不在终端前台')
        parents.append(current)
        parent = int(fields[1])
        if parent <= 1 or parent in parents:
            break
        current = parent
    tty = os.readlink('/proc/%s/fd/0' % pid)
    if not re.fullmatch(r'/dev/(pts/[0-9]+|tty[A-Za-z0-9]+)', tty):
        raise ValueError('原会话没有可定位的终端')
    return parents, tty


FOREGROUND = {'claude': {'claude'}, 'kimi': {'kimi'}, 'pi': {'pi'}, 'omp': {'omp'}, 'opencode': {'opencode'}, 'qoder': {'qoder', 'qodercli'}, 'qoder-cn': {'qoder', 'qodercli'}, 'codebuddy': {'codebuddy', 'c'}, 'antigravity': {'agy', 'antigravity'}, 'deepseek': {'dsh', 'deepseek'}, 'deepseek-harness': {'dsh', 'deepseek'}, 'zcode': {'zcode'}, 'copilot': {'copilot'}, 'gemini': {'gemini'}, 'kilo': {'kilo'}, 'cline': {'cline'}, 'kiro': {'kiro-cli'}}


def tmux_send(target, text):
    session = target['session']
    process = target.get('process') or dict(pid=session.get('pid'), start=session.get('processStart'))
    if session['phase'] not in ('needsInput', 'complete', 'interrupted', 'failed'):
        return result('blocked', '原终端仍在运行，请等待本轮结束后重试')
    if time.time() - target.get('observedAt', 0) > 20:
        return result('blocked', '缺少原终端的最新状态，指令已保留')
    try:
        parents, tty = process_identity(process.get('pid'), process.get('start'))
        listing = subprocess.run(['tmux', 'list-panes', '-a', '-F', '#{pane_id}\t#{pane_tty}\t#{pane_pid}\t#{pane_current_command}\t#{pane_dead}'], stdin=subprocess.DEVNULL, capture_output=True, timeout=3, check=True).stdout.decode()
        hits = [p for p in (line.split('\t') for line in listing.splitlines()) if len(p) == 5 and p[1] == tty]
        if len(hits) != 1:
            return result('blocked', '无法唯一定位原 tmux 分屏，指令已保留')
        pane, _, pane_pid, command, dead = hits[0]
        if int(pane_pid) not in parents or dead != '0' or pathlib.Path(command).name.lower() not in FOREGROUND.get(session['provider'], set()):
            return result('blocked', '目标分屏的前台进程不是原 Agent，指令已保留')
        buffer = 'aieyes-' + secrets.token_hex(16)
        subprocess.run(['tmux', 'load-buffer', '-b', buffer, '-'], input=text.encode(), capture_output=True, timeout=3, check=True)
    except (OSError, ValueError, subprocess.SubprocessError, StopIteration):
        return result('blocked', '原终端不可用或身份无法核对，指令已保留')
    try:
        process_identity(process.get('pid'), process.get('start'))
        try:
            subprocess.run(['tmux', 'paste-buffer', '-p', '-d', '-b', buffer, '-t', pane, ';', 'send-keys', '-t', pane, 'Enter'], stdin=subprocess.DEVNULL, capture_output=True, timeout=3, check=True)
            return result('submitted', '已输入原 tmux 分屏，等待会话确认', adapter='tmux')
        except (OSError, subprocess.SubprocessError):
            return result('uncertain', '终端输入结果无法确认，请核对原分屏；系统不会重发')
    except (OSError, ValueError, StopIteration):
        return result('blocked', '原会话进程已退出，指令已保留')
    finally:
        try:
            subprocess.run(['tmux', 'delete-buffer', '-b', buffer], stdin=subprocess.DEVNULL, capture_output=True, timeout=1)
        except (OSError, subprocess.SubprocessError):
            pass


def run(request):
    source, target = request['source'], request['target']
    session = target['session']
    provider_matches = source['provider'] == session['provider'] or source['provider'] == 'codex' and session['provider'] == 'chatgpt-work'
    if source['id'] != session['sourceId'] or not provider_matches or (source.get('hostId') or 'local') != session['machineId']:
        return result('blocked', '会话来源身份不符')
    if request['action'] == 'read':
        out = dict(excerpt=excerpt(source, target), question=None, promptHash=None, approval=False, questionRequest=None, approvalRequest=None, approvalPatch=None)
        pending_async = async_request(target,out['excerpt']) if session['phase'] == 'needsInput' else None
        if pending_async:
            out['promptHash']=fingerprint(pending_async)
            out['questionRequest']=pending_async
            return out
        if session['provider'] in ('codex', 'chatgpt-work') and session['phase'] == 'needsInput':
            ws = None
            try:
                ws = WebSocket(pathlib.Path(os.path.expanduser(source['path'])) / 'app-server-control/app-server-control.sock')
                ws.attach(target, True)
                pending = ws.question(target)
                if pending:
                    out['question'] = clip(describe(pending, request.get('language'))); out['promptHash'] = fingerprint(pending)
                    out['approval'] = is_approval(pending)
                    if out['approval']:
                        out['approvalRequest'] = pending
                        if pending['method'] == FILE_CHANGE and isinstance(pending['params'].get('itemId'), str):
                            out['approvalPatch'] = call_patch(source, target, pending['params']['itemId'])
                    else:out['questionRequest']=pending
            except (OSError, ValueError, Rejected, TimeoutError):
                pass
            finally:
                if ws:
                    ws.close()
        return out
    if request['action'] != 'send':
        return result('blocked', '不支持的渠道操作')
    text, native = request.get('text', ''), request.get('reply')
    if native is not None:
        # A native reply only ever answers a structured Codex request; it is never typed.
        if session['provider'] not in ('codex', 'chatgpt-work') or session['phase'] != 'needsInput' or text or len(json.dumps(native).encode()) > 65536:
            return result('blocked', '输入内容无效')
        return codex_send(source, target, '', request.get('promptHash'), native)
    if not isinstance(text, str) or not text or len(text.encode()) > 8192 or any(ord(c) < 32 and c not in '\n\t' for c in text) or '\x7f' in text:
        return result('blocked', '输入内容无效')
    if session['provider'] in ('codex', 'chatgpt-work'):
        return codex_send(source, target, text, request.get('promptHash'))
    if not request.get('allowTerminal'):
        return result('blocked', '此会话需要启用终端自动化，指令已保留')
    if session.get('subagentCount', 0) and session['phase'] == 'needsInput':
        return result('blocked', '等待输入可能来自子 Agent，请在原窗口确认目标')
    # Text followed by Enter must never land on a permission dialog, a form or an option menu.
    if text.strip() in ('/approve', '/decline', '/cancel-request') or session['phase'] == 'needsInput' and session.get('inputKind') in ('approval', 'elicitation'):
        return result('blocked', '此终端没有可核验的审批接口，请在原窗口确认')
    if session['phase'] not in ('needsInput', 'complete', 'interrupted', 'failed'):
        return result('blocked', '原终端仍在运行，请等待本轮结束后重试')
    # A question tool opens a menu whether or not its options could be read.
    try:
        pending = excerpt(source, target) if session['phase'] == 'needsInput' else {}
    except Exception:
        # Nothing has been typed yet: this is a refusal, not an uncertain send.
        return result('blocked', '无法核对原会话，指令已保留')
    if session['phase'] == 'needsInput' and (session.get('inputKind') == 'question' or any(pending.get(k) for k in ('claudeRequest', 'structuredQuestions', 'structuredApproval'))):
        return result('blocked', '原会话正在等待选择或确认，终端输入无法可靠送达，请在原窗口处理')
    return tmux_send(target, text)


def guarded_run(request):
    try:
        return run(request)
    except Exception:
        if isinstance(request, dict) and request.get('action') == 'send':
            return result('uncertain', '无法确认原会话提交结果，请核对原窗口；系统不会重发')
        return result('blocked', '无法核对原会话，指令已保留')


if __name__ == '__main__':
    try:
        request = globals().get('REQUEST')
        if request is None:
            request = json.load(__import__('sys').stdin)
        output = guarded_run(request)
    except Exception:
        output = result('blocked', '无法核对原会话，指令已保留')
    print(json.dumps(output, ensure_ascii=False, separators=(',', ':')))
