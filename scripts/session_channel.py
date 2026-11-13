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


def parse(records, target):
    out = dict(text='', question='', userText='', userAt=0)
    session = target['session']
    codex = session['provider'] in ('codex', 'chatgpt-work')
    turn, call = '', ''
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
        current = p.get('turn_id') or p.get('turnId') or turn
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
                out['question'] = questions(p['questions']); call = p.get('call_id', '')
            if value.get('type') == 'response_item' and p.get('type') == 'function_call' and p.get('name', '').endswith('request_user_input'):
                try:
                    out['question'] = questions(json.loads(p['arguments']).get('questions'))
                    call = p.get('call_id', '')
                except (KeyError, ValueError, TypeError):
                    pass
            if value.get('type') == 'response_item' and p.get('type') == 'function_call_output' and call and p.get('call_id') == call:
                out['question'] = ''; call = ''
            if value.get('type') == 'response_item' and p.get('type') == 'message':
                if p.get('role') == 'assistant' and p.get('phase') == 'final':
                    out['text'] = text_blocks(p.get('content'))
                if p.get('role') == 'user':
                    out['userText'] = text_blocks(p.get('content')); out['userAt'] = at
            if value.get('type') == 'event_msg':
                if p.get('type') == 'user_message':
                    out['userText'] = p.get('message', ''); out['userAt'] = at
                if p.get('type') == 'agent_message' and p.get('phase') == 'final':
                    out['text'] = p.get('message', '')
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
            elif role == 'user':
                text = text_blocks(message.get('content'))
                if text:
                    out['userText'] = text; out['userAt'] = at
                blocks = message.get('content', []) if isinstance(message.get('content'), list) else []
                if any(isinstance(b, dict) and b.get('type') == 'tool_result' and b.get('tool_use_id') == call for b in blocks):
                    out['question'] = ''; call = ''
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


def excerpt(source, target):
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
        return next((r for r in reversed(self.pending) if r['params'].get('threadId') == session['nativeSession'] and (not session.get('turnId') or r['params'].get('turnId') == session['turnId'])), None)


def fingerprint(request):
    keys = ('threadId', 'turnId', 'itemId', 'questions', 'command', 'reason', 'availableDecisions', 'cwd', 'grantRoot', 'commandActions', 'permissions', 'changes', 'networkApprovalContext', 'proposedExecpolicyAmendment')
    params = {key: request['params'].get(key) for key in keys}
    return hashlib.sha256(json.dumps(dict(method=request['method'], params=params), sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()


def describe(request, language='zh-CN'):
    p, method = request['params'], request['method']
    english = language == 'en'
    if method in ('item/tool/requestUserInput', 'tool/requestUserInput'):
        text = questions(p.get('questions'))
        if len(p.get('questions') or []) > 1:
            text += '\n\n' + ('Reply with one question_id=answer per line.' if english else '请按「问题编号=答案」逐行回复。')
        return text
    if method in ('item/commandExecution/requestApproval', 'item/fileChange/requestApproval'):
        command = p.get('command') or ('File changes' if english else '文件修改')
        if isinstance(command, list):
            command = ' '.join(command)
        reason = p.get('reason') or ('The original session requests approval' if english else '原会话请求确认')
        instructions = 'Quote this message and reply /approve to allow this operation once, or /decline to deny.' if english else '引用此消息回复 /approve 允许本次操作，/decline 拒绝。'
        return str(reason) + '\n' + clip(str(command), 6000) + '\n\n' + instructions
    return 'Handle this request in the original window.' if english else '此类输入需要回到原窗口处理。'


def answer(request, text):
    p, method = request['params'], request['method']
    if method in ('item/tool/requestUserInput', 'tool/requestUserInput'):
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
    if method in ('item/commandExecution/requestApproval', 'item/fileChange/requestApproval'):
        decision = {'/approve': 'accept', '/decline': 'decline', '/cancel-request': 'cancel'}.get(text.strip())
        if not decision or ('availableDecisions' in p and decision not in p['availableDecisions']):
            raise ValueError('此操作需要明确回复 /approve 或 /decline')
        return dict(decision=decision)
    raise ValueError('此类输入需要回到原窗口处理，指令已保留')


def codex_send(source, target, text, prompt_hash):
    root = pathlib.Path(os.path.expanduser(source['path']))
    ws = None
    try:
        ws = WebSocket(root / 'app-server-control/app-server-control.sock')
    except (OSError, ValueError, Rejected, TimeoutError):
        pass
    if ws:
        try:
            thread = ws.attach(target, target['session']['phase'] == 'needsInput')
            if target['session']['phase'] == 'needsInput':
                request = ws.question(target)
                if not request or not prompt_hash or fingerprint(request) != prompt_hash:
                    return result('blocked', '问题已改变或尚未同步，请回复最新的问题通知')
                try:
                    reply = answer(request, text)
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
                        if value and value.get('method') == 'serverRequest/resolved' and value['params']['requestId'] == request['id'] and value['params']['threadId'] == request['params']['threadId']:
                            return result('confirmed', '原会话已接收回答', adapter='codex', requestResolved=True)
                    return result('submitted', '回答已提交，等待原会话确认', adapter='codex', answer=True)
                except (OSError, ValueError):
                    return result('uncertain', '回答可能已提交，请核对原会话')
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
    if target['session']['phase'] == 'needsInput':
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
        out = dict(excerpt=excerpt(source, target), question=None, promptHash=None, approval=False)
        if session['provider'] in ('codex', 'chatgpt-work') and session['phase'] == 'needsInput':
            ws = None
            try:
                ws = WebSocket(pathlib.Path(os.path.expanduser(source['path'])) / 'app-server-control/app-server-control.sock')
                ws.attach(target, True)
                pending = ws.question(target)
                if pending:
                    out['question'] = clip(describe(pending, request.get('language'))); out['promptHash'] = fingerprint(pending)
                    out['approval'] = pending['method'] in ('item/commandExecution/requestApproval', 'item/fileChange/requestApproval')
            except (OSError, ValueError, Rejected, TimeoutError):
                pass
            finally:
                if ws:
                    ws.close()
        return out
    if request['action'] != 'send':
        return result('blocked', '不支持的渠道操作')
    text = request.get('text', '')
    if not isinstance(text, str) or not text or len(text.encode()) > 8192 or any(ord(c) < 32 and c not in '\n\t' for c in text) or '\x7f' in text:
        return result('blocked', '输入内容无效')
    if session['provider'] in ('codex', 'chatgpt-work'):
        return codex_send(source, target, text, request.get('promptHash'))
    if not request.get('allowTerminal'):
        return result('blocked', '此会话需要启用终端自动化，指令已保留')
    if session.get('subagentCount', 0) and session['phase'] == 'needsInput':
        return result('blocked', '等待输入可能来自子 Agent，请在原窗口确认目标')
    if text.strip() in ('/approve', '/decline', '/cancel-request'):
        return result('blocked', '此终端没有可核验的审批接口，请在原窗口确认')
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
