import contextlib
import hashlib
import json
import os
import sys

def _harness():
    if '_HARNESS' not in globals():
        from pathlib import Path
        source=globals().get('HARNESS_SCRIPT')
        if source is None:source=Path(__file__).with_name('harness_remote.py').read_text(encoding='utf-8')
        scope={};exec(compile(source,'<aieyes-harness>','exec'),scope);globals()['_HARNESS']=scope
    return globals()['_HARNESS']

def _native_database():
    if '_NATIVE_DATABASE' not in globals():
        from pathlib import Path
        source=globals().get('NATIVE_DATABASE_SCRIPT')
        if source is None:source=Path(__file__).with_name('native_database_remote.py').read_text(encoding='utf-8')
        scope={};exec(compile(source,'<aieyes-native-database>','exec'),scope);globals()['_NATIVE_DATABASE']=scope
    return globals()['_NATIVE_DATABASE']

def _opencode_database(root):
    if os.path.isfile(root) and not root.endswith('.db'):return None
    path=root if os.path.isfile(root) else os.path.join(root,'opencode.db')
    if not os.path.isfile(path):raise FileNotFoundError('OpenCode native history database not found')
    return path

def _zcode_database(root):
    candidates=[root] if os.path.isfile(root) else [os.path.join(root,'cli/db/db.sqlite'),os.path.join(root,'db/db.sqlite')]
    return next((p for p in candidates if p.endswith('.sqlite') and os.path.isfile(p)),None)

# Matches the narrow statistical protobuf projection in core/antigravity.rs.
def _agy_varint(data, pos):
    result = 0
    for shift in range(0, 70, 7):
        if pos >= len(data):
            raise ValueError("Antigravity metadata truncated")
        byte = data[pos]; pos += 1
        if shift == 63 and byte > 1:
            raise ValueError("Antigravity metadata integer overflow")
        result |= (byte & 127) << shift
        if byte < 128:
            return result, pos
    raise ValueError("Antigravity metadata integer invalid")

def _agy_fields(data):
    if len(data) > 32 * 1024 * 1024:
        raise ValueError("Antigravity metadata too large")
    pos = 0; result = {}
    while pos < len(data):
        key, pos = _agy_varint(data, pos)
        if key >> 3 == 0:
            raise ValueError("Antigravity metadata field invalid")
        wire = key & 7
        if wire == 0:
            value, pos = _agy_varint(data, pos)
        elif wire in (1, 2, 5):
            if wire == 2:
                size, pos = _agy_varint(data, pos)
            else:
                size = 8 if wire == 1 else 4
            if size > len(data) - pos:
                raise ValueError("Antigravity metadata truncated")
            value = data[pos:pos+size]; pos += size
        else:
            raise ValueError("Antigravity metadata wire type unsupported")
        result.setdefault(key >> 3, []).append(value)
    return result

def _agy_message(fields, number):
    return _agy_fields(fields.get(number, [b''])[0])

def _agy_text(fields, number):
    value = fields.get(number, [b''])[0]
    return value.decode('utf-8') if isinstance(value, bytes) else ''

def _agy_database(path):
    return _agy_database_detailed(path)['events']


def _agy_database_detailed(path):
    events, issues = [], []
    for row in _agy_iter(path):
        if 'event' in row: events.append(row['event'])
        else: issues.append(row['issue'])
    return dict(events=events, issues=issues)

def _agy_iter(path):
    import pathlib
    import sqlite3
    db = sqlite3.connect(pathlib.Path(path).absolute().as_uri() + '?mode=ro', uri=True, timeout=2)
    try:
        db.execute('BEGIN')
        models = {}
        for (raw,) in db.execute('SELECT data FROM gen_metadata'):
            row = _agy_fields(raw)
            model = _agy_text(_agy_message(row, 1), 19) or _agy_text(_agy_message(row, 3), 28)
            if not model:
                continue
            for index in row.get(2, []):
                if isinstance(index, int):
                    models[index] = model
                else:
                    pos = 0
                    while pos < len(index):
                        step, pos = _agy_varint(index, pos); models[step] = model
        # Newer agy executor metadata supplies the complete (not 16-byte
        # truncated) model name. Join by explicit step index only.
        if db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='executor_metadata'").fetchone():
            for index, raw in db.execute('SELECT idx,data FROM executor_metadata'):
                try:
                    envelope = _agy_fields(raw)
                    config = _agy_message(envelope, 10)
                    name = _agy_text(_agy_message(config, 1), 28)
                except (ValueError, TypeError, IndexError):
                    # An optional, incompatible executor envelope must not lose usage.
                    continue
                if name and (index not in models or name.startswith(models[index])):
                    models[index] = name
        session = db.execute('SELECT cascade_id FROM trajectory_meta LIMIT 1').fetchone()[0]
        if not session:
            raise ValueError('Antigravity 会话标识缺失')
        rows = 'SELECT idx,status,metadata FROM steps WHERE length(metadata)>0 ORDER BY idx'
        identities = {}
        for index, _, raw in db.execute(rows):
            try:
                numeric = _agy_message(_agy_fields(raw), 9).get(1, [0])[0]
                if numeric and index in models:
                    identities.setdefault(numeric, set()).add(models[index])
            except (ValueError, TypeError):
                pass
        for index, status, raw in db.execute(rows):
            try:
                row = _agy_fields(raw)
                if 9 not in row:
                    continue
                usage = _agy_message(row, 9)
                end = max(_agy_message(row, 8).get(1, [0])[0], _agy_message(row, 7).get(1, [0])[0])
                if status != 3 and end == 0:
                    continue
                tokens = {name: usage.get(field, [0])[0] for name, field in [('input', 2), ('output', 3), ('cacheRead', 5), ('cacheWrite', 4), ('reasoning', 9)]}
                if sum(tokens[k] for k in ('input', 'output', 'cacheRead', 'cacheWrite')) == 0:
                    continue
                start = _agy_message(row, 1).get(1, [0])[0]
                if start <= 0 or end < start or tokens['reasoning'] > tokens['output']:
                    raise ValueError('Antigravity 调用时间或 Token 口径无效')
                numeric = usage.get(1, [0])[0]
                names = identities.get(numeric, set())
                model = models.get(index) or (next(iter(names)) if len(names) == 1 else 'antigravity-unknown-' + str(numeric))
                if model.startswith('antigravity-unknown-'):
                    yield {'issue': dict(step=index, code='unknownModel', message='模型身份未知；已保留 Token，暂不计价')}
                identity = _agy_text(usage, 7) or _agy_text(usage, 11) or 'step:' + str(index)
                yield {'event': dict(id=identity, sessionId=session, timestamp=end, intervalStart=start,
                                   intervalEvidence='antigravity-step-v1', model=model, tokens=tokens)}
            except (ValueError, TypeError, IndexError, OverflowError) as error:
                yield {'issue': dict(step=index, code='invalidRecord', message=str(error))}
    finally:
        db.close()

def _codex_home(root):
    home = os.path.dirname(root) if os.path.isfile(root) else root
    parts = home.split(os.sep)
    for index in range(len(parts) - 1, -1, -1):
        if parts[index] in ('sessions', 'archived_sessions'):
            return os.sep.join(parts[:index]) or os.sep
    return home


class _CodexHistoryError(ValueError):
    MESSAGES = dict(parentNotFound='继承历史记录尚未找到',
                    identityMismatch='继承历史文件与所属会话标识不一致',
                    boundaryInvalid='继承历史字节边界超出文件或未落在完整记录上',
                    cycle='继承历史引用形成循环或超过层数限制',
                    ambiguousHistory='继承历史存在内容不一致的同标识记录',
                    invalidRecord='继承历史包含损坏记录',
                    invalidCounter='继承历史累计用量无效',
                    invalidCompressedHistory='继承历史压缩记录损坏或不完整',
                    decoderUnavailable='读取压缩继承历史需要远程 Node 支持 Zstandard',
                    resourceLimit='继承历史超过压缩读取大小限制',
                    readError='无法读取完整继承历史')

    def __init__(self, reason):
        self.reason = reason if reason in self.MESSAGES else 'readError'
        super().__init__(self.MESSAGES[self.reason])


class _CodexHistory:
    """Resolve immutable rollout IDs; conversation text never leaves this host."""
    def __init__(self, root):
        import sqlite3
        self.paths, self.cache = {}, {}
        home = _codex_home(root)
        folders = sorted(set([root, os.path.join(home, 'sessions'), os.path.join(home, 'archived_sessions')]), key=len)
        visited = []
        def walk(folder):
            for directory, children, names in os.walk(folder, followlinks=False):
                children[:] = sorted(name for name in children if name != '.aieyes' and not os.path.islink(os.path.join(directory, name)))
                for name in sorted(names):
                    path = os.path.join(directory, name)
                    if name.endswith(('.jsonl', '.jsonl.zst')) and not os.path.islink(path):
                        yield path
        for folder in folders:
            if any(os.path.commonpath([folder, earlier]) == earlier for earlier in visited):
                continue
            visited.append(folder)
            for path in ([folder] if os.path.isfile(folder) else walk(folder)):
                self._index(path)
        database = os.path.join(home, 'state_5.sqlite')
        if os.path.isfile(database):
            try:
                from pathlib import Path
                with contextlib.closing(sqlite3.connect(Path(database).as_uri() + '?mode=ro', uri=True)) as db:
                    for _, path in db.execute('SELECT id,rollout_path FROM threads'):
                        # The DB points at the selected thread revision, not necessarily the base.
                        self._index(path)
            except (OSError, sqlite3.Error):
                pass

    @staticmethod
    def _representation(path):
        plain = path[:-4] if path.endswith('.zst') else path
        return plain if os.path.isfile(plain) else plain + '.zst' if os.path.isfile(plain + '.zst') else None

    def _index(self, path):
        import re
        from datetime import datetime
        path = self._representation(path)
        if path is None:
            return
        uuid = r'[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}'
        match = re.fullmatch(r'rollout-(.{19})-(' + uuid + r')(?:_(' + uuid + r'))?\.jsonl(?:\.zst)?', os.path.basename(path))
        if match:
            try:
                datetime.strptime(match[1], '%Y-%m-%dT%H-%M-%S')
            except ValueError:
                match = None
        try:
            if match:
                identity, rollout = match[2], match[3] or match[2]
            else:
                if path.endswith('.zst'):
                    value = _harness()['codex_prefix'](path)
                    if value.get('reason'):
                        return
                    identity = value.get('thread')
                else:
                    with open(path, 'rb') as stream:
                        row = json.loads(stream.readline(1024 * 1024))
                    if not isinstance(row, dict) or row.get('type') != 'session_meta':
                        return
                    payload = row.get('payload') or {}
                    if not isinstance(payload, dict):
                        return
                    identity = payload.get('id') or payload.get('session_id')
                if not isinstance(identity, str) or not identity:
                    return
                rollout = identity
            candidates = self.paths.setdefault(rollout, [])
            if not any(self._representation(c[0]) == path for c in candidates):
                candidates.append((path, identity, bool(match)))
        except (OSError, ValueError, TypeError):
            pass

    def resolve(self, base):
        thread, end = base.get('thread_id', ''), base.get('end_byte_offset')
        try:
            return self._resolve(thread, end, [])
        except (OSError, ValueError, TypeError, KeyError) as error:
            reason = error.reason if isinstance(error, _CodexHistoryError) else 'readError' if isinstance(error, OSError) else 'invalidRecord'
            return dict(parentThread=thread if isinstance(thread, str) else '',
                        endByteOffset=end if type(end) is int and end >= 0 else 0,
                        tokens=None, timestamp=None, proof='', issue=str(_CodexHistoryError(reason)), issueCode=reason)

    def _resolve(self, thread, end, ancestors):
        if not isinstance(thread, str) or not thread:
            raise _CodexHistoryError('identityMismatch')
        if thread in ancestors or len(ancestors) >= 64:
            raise _CodexHistoryError('cycle')
        if type(end) is not int or not 0 <= end <= 2**64 - 1:
            raise _CodexHistoryError('boundaryInvalid')
        if (thread, end) in self.cache:
            return self.cache[(thread, end)]
        candidates = self.paths.get(thread, [])
        canonical = any(c[2] for c in candidates)
        candidates = sorted(c for c in candidates if not canonical or c[2])
        result, error = None, None
        for candidate in candidates:
            try:
                value = self._read(candidate, thread, end, ancestors + [thread])
            except (OSError, ValueError, TypeError, KeyError) as problem:
                if error is None:
                    error = problem
                continue
            if result and result['proof'] != value['proof']:
                raise _CodexHistoryError('ambiguousHistory')
            result = value
        if result is None:
            raise error or _CodexHistoryError('parentNotFound')
        self.cache[(thread, end)] = result
        return result

    def _read(self, candidate, thread, end, ancestors):
        from datetime import datetime
        path, identity, _ = candidate
        path = self._representation(path)
        if path is None:
            raise _CodexHistoryError('parentNotFound')
        tokens, stamp, parent = None, None, None
        if path.endswith('.zst'):
            value = _harness()['codex_prefix'](path, end)
            if value.get('reason'):
                raise _CodexHistoryError(value['reason'])
            if end and value.get('thread') != identity:
                raise _CodexHistoryError('identityMismatch')
            tokens, stamp, parent, proof = (value.get(k) for k in ('tokens', 'timestamp', 'parent', 'proof'))
        else:
            digest, offset = hashlib.sha256(), 0
            with open(path, 'rb') as stream:
                while offset < end:
                    line = stream.readline(min(end - offset, 16 * 1024 * 1024) + 1)
                    if not line.endswith(b'\n') or len(line) > 16 * 1024 * 1024 or offset + len(line) > end:
                        raise _CodexHistoryError('boundaryInvalid')
                    first = offset == 0
                    offset += len(line)
                    digest.update(line)
                    try:
                        row = json.loads(line)
                    except (ValueError, UnicodeError):
                        raise _CodexHistoryError('invalidRecord') from None
                    if not isinstance(row, dict) or not isinstance(row.get('payload') or {}, dict):
                        raise _CodexHistoryError('invalidRecord')
                    payload = row.get('payload') or {}
                    if first:
                        if row.get('type') != 'session_meta' or (payload.get('id') or payload.get('session_id')) != identity:
                            raise _CodexHistoryError('identityMismatch')
                        base = payload.get('history_base')
                        parent = base if isinstance(base, dict) else None
                    if row.get('type') == 'event_msg' and payload.get('type') == 'token_count':
                        if not isinstance(payload.get('info'), dict):
                            continue
                        usage = (payload.get('info') or {}).get('total_token_usage')
                        if not isinstance(usage, dict):
                            continue
                        values = [usage.get(key, 0) for key in ('input_tokens', 'output_tokens', 'cached_input_tokens', 'cache_write_input_tokens', 'reasoning_output_tokens')]
                        if 'input_tokens' not in usage or any(type(v) is not int or not 0 <= v <= 2**64 - 1 for v in values) or values[0] < values[2] + values[3]:
                            raise _CodexHistoryError('invalidCounter')
                        tokens = dict(input=values[0]-values[2]-values[3], output=values[1], cacheRead=values[2], cacheWrite=values[3], reasoning=values[4])
                        stamp = row.get('timestamp')
            proof = digest.hexdigest()
        if isinstance(stamp, str):
            try:
                stamp = int(datetime.fromisoformat(stamp.replace('Z', '+00:00')).timestamp())
            except (ValueError, OverflowError):
                stamp = None
        elif type(stamp) is not int or not -(2**63) <= stamp < 2**63:
            stamp = None
        if tokens is None and parent:
            inherited = self._resolve(parent.get('thread_id'), parent.get('end_byte_offset'), ancestors)
            proof = hashlib.sha256((proof + inherited['proof']).encode()).hexdigest()
            tokens, stamp = inherited['tokens'], inherited['timestamp']
        return dict(parentThread=thread, endByteOffset=end,
                    tokens=tokens if tokens is not None else dict(input=0, output=0, cacheRead=0, cacheWrite=0, reasoning=0),
                    timestamp=stamp, proof=proof, issue='')


def _project(value, provider):
    if provider in ('kimi','codebuddy','grok','qoder','pi','omp') and isinstance(value.get('tokens'),dict):
        event={k:value.get(k) for k in ('id','sessionId','timestamp','model','tokens','billing','intervalStart','intervalEvidence')}
        if provider in ('qoder','codebuddy') and isinstance(value.get('nativeCredits'),dict):
            event['nativeCredits']={k:value['nativeCredits'].get(k) for k in ('credits','originalCredits','billable')}
        return event
    event = None
    if provider == "codex":
        payload = value.get("payload") or {}
        kind = value.get("type")
        if kind == "session_meta":
            event = {"type": kind, "timestamp": value.get("timestamp"), "payload": {"timestamp": payload.get("timestamp"), "id": payload.get("id", payload.get("session_id")), "model_provider": payload.get("model_provider")}}
            user, account = payload.get('creator_user_id'), payload.get('creator_account_id')
            if isinstance(user, str) and user and isinstance(account, str) and account:
                event['payload']['creator_identity_key'] = hashlib.sha256((user + '\0' + account).encode()).hexdigest()
            if isinstance(payload.get('history_base'), dict):
                event['payload']['history_base'] = {k: payload['history_base'].get(k) for k in ('thread_id', 'end_byte_offset')}
        elif kind == "turn_context":
            event = {"type": kind, "payload": {"model": payload.get("model"), "turn_id": payload.get("turn_id")}}
        elif kind == "event_msg" and payload.get("type") in ("task_started", "task_complete", "turn_aborted"):
            event = {"type": kind, "timestamp": value.get("timestamp"), "payload": {"type": payload["type"]}}
            if isinstance(payload.get("turn_id"), str) and payload["turn_id"]:
                event["payload"]["turn_id"] = payload["turn_id"]
        elif kind == "event_msg" and payload.get("type") == "token_count":
            event = {"type": kind, "timestamp": value.get("timestamp"), "payload": {
                "type": "token_count", "info": payload.get("info"), "rate_limits": payload.get("rate_limits")}}
    elif provider in ('pi','omp'):
        kind=value.get('type')
        if kind=='session':event={k:value[k] for k in ('type','id','timestamp','version') if k in value}
        elif kind in ('message','usage','model_usage','compaction','branch_summary'):
            data=value.get('message') if kind=='message' else value
            if isinstance(data,dict) and (kind!='message' or data.get('role') in ('user','assistant')):
                event={k:value[k] for k in ('type','id','timestamp','parentId') if k in value}
                fields={k:data[k] for k in ('role','model','provider','timestamp') if k in data}
                if isinstance(data.get('usage'),dict):fields['usage']={k:data['usage'][k] for k in ('input','output','cacheRead','cacheWrite','reasoning','totalTokens') if k in data['usage']}
                if kind=='message':event['message']=fields
                else:event.update(fields)
    elif provider == "kimi":
        if value.get('type') in ('metadata','llm.request','turn.prompt','usage.record'):
            event={k:value[k] for k in ('type','model','provider','time','turnId') if k in value}
            if value.get('type')=='usage.record':
                event['usage']={k:(value.get('usage') or {}).get(k) for k in ('inputOther','output','inputCacheRead','inputCacheCreation')}
        elif (value.get('message') or {}).get('type')=='StatusUpdate':
            payload=value['message'].get('payload') or {}
            event=dict(timestamp=value.get('timestamp'),message=dict(type='StatusUpdate',payload=dict(message_id=payload.get('message_id'),token_usage={k:(payload.get('token_usage') or {}).get(k) for k in ('input_other','output','input_cache_read','input_cache_creation')})))
    elif provider == "codebuddy":
        if value.get('type')=='message' and value.get('role')=='user' and isinstance(value.get('sessionId'),str):
            event=dict(type='identity',sessionId=value['sessionId'])
        elif value.get('type') in ('function_call','model-usage','credit-usage') or value.get('type')=='message' and value.get('role')=='assistant':
            p=value.get('providerData') or {}; u=p.get('rawUsage') or {}
            event={k:value.get(k) for k in ('type','role','id','status','sessionId','timestamp')}
            raw={k:u[k] for k in ('prompt_tokens','completion_tokens','input_tokens','output_tokens','credit') if k in u}
            for detail,key in [('prompt_tokens_details','cached_tokens'),('completion_tokens_details','reasoning_tokens'),('input_tokens_details','cached_tokens'),('output_tokens_details','reasoning_tokens')]:
                if isinstance(u.get(detail),dict):raw[detail]={key:u[detail].get(key)}
            event['providerData']=dict(model=p.get('model'),messageId=p.get('messageId'),conversationRequestId=p.get('conversationRequestId'),rawUsage=raw)
            event['providerData'].update({k:p[k] for k in ('requestId','requestModelId','credit','source','stageId') if k in p})
    elif provider == "grok":
        p=value.get('params') or {}; update=p.get('update') or {}; meta=p.get('_meta') or {}
        if value.get('method')=='_x.ai/session/update' and update.get('sessionUpdate')=='turn_completed':
            models=(update.get('usage') or {}).get('modelUsage') or {}
            usage={model:{k:counts.get(k) for k in ('inputTokens','outputTokens','cachedReadTokens','cacheCreationTokens','reasoningTokens','totalTokens')} for model,counts in models.items() if isinstance(counts,dict)}
            event=dict(method=value['method'],params=dict(sessionId=p.get('sessionId'),update=dict(sessionUpdate='turn_completed',prompt_id=update.get('prompt_id'),usage=dict(modelUsage=usage)),_meta={k:meta.get(k) for k in ('eventId','agentTimestampMs')}))
    elif provider in ("claude","qoder"):
        message = value.get("message") or {}
        if value.get("type") == "assistant" and (message.get("usage") or provider == 'qoder' and isinstance(message.get('model'), str)):
            event = {k: value.get(k) for k in ("type", "timestamp", "sessionId", "requestId", "uuid", "isApiErrorMessage")}
            usage=message.get('usage') or {}
            keys=['input_tokens','output_tokens','cache_read_input_tokens','cache_creation_input_tokens']
            if provider=='qoder':
                keys+=['credits','original_credits','billable']
                event['modelSource']=value.get('modelSource')
                event['isSidechain']=value.get('isSidechain') is True
            event["message"] = {k: message.get(k) for k in ("id", "model")}
            event["message"]['usage']={k:usage[k] for k in keys if k in usage}
    elif "tokens" in value:
        event = {k: value.get(k) for k in ("id", "sessionId", "timestamp", "model", "tokens", "billing", "intervalStart", "intervalEvidence")}
    return event


def _native_routes(root, cursor, emit):
    import hashlib
    import re
    import sqlite3
    from pathlib import Path
    from urllib.parse import urlsplit
    home = _codex_home(root)
    path = os.path.join(home, 'logs_2.sqlite')
    if not os.path.isfile(path):
        return
    metadata = os.stat(path)
    identity = [metadata.st_dev, metadata.st_ino]
    db = sqlite3.connect(Path(path).as_uri() + '?mode=ro', uri=True, timeout=5)
    try:
        db.execute('PRAGMA query_only=ON')
        db.execute('BEGIN')
        maximum = db.execute('SELECT COALESCE(MAX(id),0) FROM logs').fetchone()[0]
        after = cursor.get('after', 0) if cursor.get('identity') == identity else 0
        if after > maximum:
            after = 0
        targets = ('codex_http_client::client', 'codex_api::endpoint::responses_websocket', 'codex_api::sse::responses', 'codex_core::client')
        patterns = {}
        def fields(body, key):
            if key not in patterns:
                patterns[key] = re.compile(r'(?<![\w.])' + re.escape(key) + r'=(?:"((?:\\.|[^"\\])*)"|([^\s}\]]+))')
            return [json.loads('"' + m.group(1) + '"') if m.group(1) is not None else m.group(2) for m in patterns[key].finditer(body)]
        def unique(body, keys, fallback=None):
            values = {v for key in keys for v in fields(body, key) if re.fullmatch(r'[\w-]{1,128}', v, re.ASCII)}
            if fallback:
                values.add(fallback)
            return next(iter(values)) if len(values) == 1 else None
        batch = []
        query = db.execute('SELECT id,ts,ts_nanos,target,feedback_log_body,thread_id,process_uuid FROM logs WHERE id>? AND id<=? AND target IN (?,?,?,?) ORDER BY id', (after, maximum, *targets))
        for row_id, stamp, nanos, target, body, thread, process in query:
            body = body or ''
            urls = fields(body, 'url') + fields(body, 'endpoint')
            if not urls and target != 'codex_http_client::client':
                urls = re.findall(r'(?:https|http|wss|ws)://[^\s"<>]+', body)
            session, turn = unique(body, ('thread_id', 'thread.id', 'conversation.id'), thread), unique(body, ('turn_id', 'turn.id'))
            statuses = fields(body, 'status')
            status = int(statuses[-1]) if statuses and statuses[-1].isdigit() else None
            for raw in set(urls):
                url = urlsplit(raw.rstrip('),.;\\'))
                path_name = url.path.rstrip('/')
                if url.scheme not in ('http', 'https', 'ws', 'wss') or not url.hostname or not path_name.endswith(('/responses', '/responses/compact', '/chat/completions')):
                    continue
                if len(url.hostname) > 253 or len(path_name) > 4096:
                    continue
                category = 'subscription' if url.hostname == 'chatgpt.com' and path_name.startswith('/backend-api/codex/') else 'api'
                safe_id = hashlib.sha256(json.dumps([row_id, stamp, nanos, process, url.hostname, path_name]).encode()).hexdigest()
                batch.append(dict(id=safe_id, stamp=stamp, nanos=nanos, sessionId=session, turnId=turn, processId=process, host=url.hostname, path=path_name, category=category, status=status))
            if len(batch) >= 256:
                emit('routes', routes=batch, cursor=dict(identity=identity, after=row_id))
                batch = []
        if batch or maximum != after or cursor.get('identity') != identity:
            emit('routes', routes=batch, cursor=dict(identity=identity, after=maximum))
    finally:
        db.close()


def _cursor_private_history(root):
    if os.path.isfile(root):return os.path.splitext(root)[1] in ('.db','.vscdb')
    for name in ('chats','acp-sessions'):
        base=os.path.join(root,name)
        for directory,dirs,files in os.walk(base,followlinks=False):
            if 'store.db' in files and not os.path.islink(os.path.join(directory,'store.db')):return True
            dirs[:]=[] if len(os.path.relpath(directory,base).split(os.sep))>=2 else [d for d in dirs if not os.path.islink(os.path.join(directory,d))]
    base=os.path.join(root,'projects')
    for directory,dirs,files in os.walk(base,followlinks=False):
        if 'agent-transcripts' in dirs:return True
        dirs[:]=[] if len(os.path.relpath(directory,base).split(os.sep))>=2 else [d for d in dirs if not os.path.islink(os.path.join(directory,d))]
    return False

def _qoder_keys(root):
    """Read the embedded state key only for the lifetime of this collector."""
    import base64
    import re
    import shutil
    candidates = []
    configured = os.environ.get('QODER_CLI_PATH')
    if configured:
        candidates.append(configured)
    found = shutil.which('qodercli')
    if found:
        candidates.append(found)
    home = os.path.expanduser('~')
    candidates.extend([
        os.path.join(home, '.local', 'bin', 'qodercli'),
        os.path.join(home, 'bin', 'qodercli'),
        '/usr/local/bin/qodercli',
        '/opt/homebrew/bin/qodercli',
    ])
    directory = os.path.join(root, 'bin', 'qodercli')
    if os.path.isdir(directory):
        try:
            candidates.extend(os.path.join(directory, name) for name in sorted(os.listdir(directory)))
        except OSError:
            pass
    keys = []
    # The minified variable name changes between platforms and CLI versions.
    marker = b'Buffer.from(['
    pattern = re.compile(re.escape(marker) + rb'([\d,]+)\]\)')
    for path in dict.fromkeys(candidates):
        if not path or not os.path.isfile(path):
            continue
        try:
            with open(path, 'rb') as stream:
                carry = b''
                while True:
                    chunk = stream.read(4 * 1024 * 1024)
                    if not chunk:
                        break
                    data = carry + chunk
                    for match in pattern.finditer(data):
                        try:
                            key = bytes(int(part) for part in match.group(1).split(b','))
                        except ValueError:
                            continue
                        if len(key) == 32 and key not in keys:
                            keys.append(key)
                    carry = data[-(len(marker) + 128):]
        except OSError:
            continue
        if len(keys) >= 8:
            break
    return [base64.b64encode(key).decode('ascii') for key in keys]


def _qoder_hash(path):
    import hashlib
    normalized = os.path.abspath(os.path.expanduser(path)).replace('\\', '/').lower()
    return hashlib.sha256(normalized.encode('utf-8')).hexdigest()


def _qoder_project_hashes(path, value, session_id):
    roots = []
    directories = value.get('workspaceDirectories')
    for root in directories if isinstance(directories, list) else []:
        if isinstance(root, str) and root:
            roots.append(root)
    project = os.path.dirname(os.path.dirname(path))
    transcript = os.path.join(project, session_id + '.jsonl')
    try:
        with open(transcript, 'rb') as stream:
            for _ in range(128):
                line = stream.readline(1024 * 1024)
                if not line:
                    break
                try:
                    row = json.loads(line)
                except (ValueError, UnicodeDecodeError):
                    continue
                cwd = row.get('cwd') if isinstance(row, dict) else None
                if isinstance(cwd, str) and cwd:
                    roots.append(cwd)
    except OSError:
        pass
    name = os.path.basename(project)
    if name.startswith('-') and len(name) > 1:
        # Best-effort fallback for legacy state files without workspace data.
        roots.append('/' + name[1:].replace('-', '/'))
    result = []
    for root in roots:
        digest = _qoder_hash(root)
        if digest not in result:
            result.append(digest)
    return result


def _qoder_states(root, request, emit, issue):
    import hashlib
    provider = request.get('provider')
    if provider != 'qoder':
        return
    projects = os.path.join(root, 'projects') if os.path.isdir(os.path.join(root, 'projects')) else root
    if not os.path.isdir(projects):
        return
    paths = []
    try:
        for project in sorted(os.listdir(projects)):
            project_path = os.path.join(projects, project)
            if not os.path.isdir(project_path) or os.path.islink(project_path):
                continue
            for session in sorted(os.listdir(project_path)):
                session_path = os.path.join(project_path, session)
                state = os.path.join(session_path, 'state.json')
                if os.path.isdir(session_path) and os.path.isfile(state) and not os.path.islink(state):
                    paths.append(state)
    except OSError as error:
        issue(projects, error, 'qoderStateRead')
        return
    cursors = request.get('qoderStateCursors') or {}
    reset = bool(request.get('qoderStateReset'))
    keys = None
    batch, amount = [], 0

    def flush():
        nonlocal batch, amount
        if batch:
            emit('qoderState', keys=keys or [], states=batch)
            batch, amount = [], 0

    for path in paths:
        try:
            with open(path, 'rb') as stream:
                raw = stream.read()
            signature = hashlib.sha256(raw).hexdigest()
            if not reset and cursors.get(path) == signature:
                continue
            value = json.loads(raw)
            if not isinstance(value, dict):
                raise ValueError('Qoder 会话状态必须是对象')
        except (OSError, ValueError, UnicodeDecodeError) as error:
            issue(path, error, 'qoderStateRead')
            continue
        session_id = value.get('sessionId')
        if not isinstance(session_id, str) or not session_id:
            session_id = os.path.basename(os.path.dirname(path))
        item = ((value.get('items') or {}).get('s0')) if isinstance(value.get('items'), dict) else None
        if not isinstance(item, dict) or not all(isinstance(item.get(k), str) for k in ('n', 'p', 't')):
            continue
        hashes = _qoder_project_hashes(path, value, session_id)
        if not hashes:
            issue(path, '无法确定 Qoder 会话项目路径，历史总量暂未补足', 'qoderStateRead')
            continue
        if keys is None:
            keys = _qoder_keys(root)
        if not keys:
            issue(path, '未找到可读取的 qodercli 状态密钥，历史 Token 总量暂未补足', 'qoderStateKey')
            return
        state = dict(path=path, signature=signature, sessionId=session_id,
                     createdAt=value.get('createdAt'), updatedAt=value.get('updatedAt'),
                     segmentKey='s0', projectHashes=hashes,
                     n=item['n'], p=item['p'], t=item['t'])
        encoded = len(json.dumps(state).encode('utf-8'))
        if batch and amount + encoded > 700 * 1024:
            flush()
        batch.append(state)
        amount += encoded
    flush()


def _incremental(request):
    import hashlib
    root = os.path.abspath(os.path.expanduser(request['path']))
    provider = request['provider']
    cursor_native=provider=='cursor' and (os.path.basename(root)=='.cursor' or _cursor_private_history(root))
    grok_native = provider=='grok' and os.path.isdir(os.path.join(root,'sessions'))
    cursors = request.get('cursors', {})
    precise = bool(request.get('codexPreciseCounting', False))
    history = _CodexHistory(root) if provider == 'codex' and precise else None
    fork_issues = []
    issues, failed, invalid_records, read_bytes, file_count, scanned_records = [], 0, 0, 0, 0, 0

    def emit(kind, **fields):
        line = json.dumps(dict(protocol=2, kind=kind, **fields), separators=(',', ':'))
        if len(line.encode('utf-8')) + 1 > 1024 * 1024:
            raise ValueError('远程统计批次超过 1 MiB')
        print(line, flush=True)

    def issue(path, error, code='fileRead', history=None):
        nonlocal invalid_records
        if code == 'missingHistoryBase':
            if len(fork_issues) < 128:
                detail = dict(path=os.path.basename(path), code=code, message=str(error)[:400])
                if history:
                    detail.update(reason=history.get('issueCode', ''), parentRolloutId=history.get('parentThread', ''), endByteOffset=history.get('endByteOffset', 0))
                fork_issues.append(detail)
            return
        if code == 'invalidRecord': invalid_records += 1
        if len(issues) < 128:
            issues.append(dict(path=os.path.basename(path), code=code, message=str(error)[:400]))

    def stat(path):
        try:
            value = os.stat(path)
            return [value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns]
        except FileNotFoundError:
            return None

    def digest(stream, start, size):
        stream.seek(start)
        return hashlib.sha256(stream.read(size)).hexdigest()

    def jsonl(path):
        nonlocal read_bytes
        old = cursors.get(path, {})
        if provider == 'codex' and old.get('codexPreciseCounting') != precise:
            old = {}
        inherited = old.get('inheritedHistory')
        if inherited and history:
            refreshed = history.resolve(dict(thread_id=inherited['parentThread'], end_byte_offset=inherited['endByteOffset']))
            if any(refreshed.get(k) != inherited.get(k) for k in ('tokens', 'proof', 'issue', 'issueCode')):
                old = {}
                inherited = refreshed
        with open(path, 'rb') as stream:
            metadata = os.fstat(stream.fileno())
            size = metadata.st_size
            identity = [metadata.st_dev, metadata.st_ino]
            offset = int(old.get('offset', 0))
            prefix_start=0
            title_digest=None
            if provider=='omp':
                stream.seek(0);slot=stream.read(256)
                try:
                    if len(slot)==256 and slot.endswith(b'\n') and json.loads(slot).get('type')=='title':
                        prefix_start=256;title_digest=hashlib.sha256(slot).hexdigest()
                except (ValueError,UnicodeError):pass
            prefix_size = int(old.get('prefixBytes', min(size-prefix_start, 128)))
            valid = (old.get('identity') == identity and 0 <= offset <= size
                     and 0 <= prefix_size <= min(size-prefix_start, 128) and old.get('prefixStart',0)==prefix_start
                     and old.get('prefix') == digest(stream, prefix_start, prefix_size)
                     and old.get('anchor') == digest(stream, max(0, offset - 128), min(offset, 128))
                     and not (size == old.get('size') and metadata.st_mtime_ns != old.get('mtimeNs')
                              and not (title_digest and old.get('titleDigest') and title_digest!=old['titleDigest'])))
            if not valid:
                inherited = None
                offset, prefix_size = 0, min(size-prefix_start, 128)
                if provider=='kimi':
                    stream.seek(0)
                    for line in stream:
                        if len(line)>16*1024*1024:raise ValueError('Kimi 单条记录过大')
                        try:
                            if json.loads(line).get('type')=='forked':offset=stream.tell()
                        except (ValueError,UnicodeDecodeError):pass
            prefix_size = min(size-prefix_start, 128)
            prefix = digest(stream, prefix_start, prefix_size)
            reset, events, bytes_in_batch = not valid, [], 0
            last_commit = offset

            def flush():
                nonlocal reset, events, bytes_in_batch, last_commit
                cursor = dict(identity=identity, prefixStart=prefix_start, titleDigest=title_digest, prefixBytes=prefix_size, prefix=prefix,
                              offset=offset, size=size, mtimeNs=metadata.st_mtime_ns,
                              anchor=digest(stream, max(0, offset - 128), min(offset, 128)))
                if provider == 'codex':
                    cursor['codexPreciseCounting'] = precise
                if inherited:
                    cursor['inheritedHistory'] = inherited
                emit('batch', path=path, reset=reset, events=events, cursor=cursor)
                reset, events, bytes_in_batch, last_commit = False, [], 0, offset
                stream.seek(offset)

            stream.seek(offset)
            while offset < size:
                start = stream.tell()
                line = stream.readline(min(size - start, 64 * 1024 * 1024 + 1))
                read_bytes += len(line)
                if len(line) > 64 * 1024 * 1024:
                    raise ValueError('单条日志超过 64 MiB，保留上次游标')
                if not line.endswith(b'\n'):
                    break
                try:
                    value = json.loads(line)
                except (ValueError, UnicodeDecodeError):
                    # Match legacy collectors: complete malformed lines do not contain usable events.
                    offset = stream.tell()
                    if offset - last_commit >= 8 * 1024 * 1024:
                        flush()
                    continue
                event = _project(value, provider)
                if event and history and event['type'] == 'session_meta' and 'history_base' in event['payload']:
                    inherited = history.resolve(event['payload']['history_base'])
                    event['inheritedHistory'] = inherited
                encoded = len(json.dumps(event).encode('utf-8')) if event else 0
                if encoded > 768 * 1024:
                    raise ValueError('单条统计记录过大，保留上次游标')
                if bytes_in_batch + encoded > 768 * 1024:
                    # Commit before adding this record; seek back to the current record afterwards.
                    next_offset = stream.tell()
                    flush()
                    stream.seek(next_offset)
                offset = stream.tell()
                if event:
                    events.append(event)
                    bytes_in_batch += encoded
                if offset - last_commit >= 8 * 1024 * 1024:
                    flush()
            if offset != last_commit or reset:
                flush()
            if inherited and history and inherited.get('tokens') is None:
                issue(path, inherited.get('issue') or '继承历史尚未确认；首段累计暂不计入', 'missingHistoryBase', inherited)

    try:
        if not os.path.exists(root):
            raise FileNotFoundError('数据目录不存在')
        native = root if os.path.basename(root) == 'conversations' else os.path.join(root, 'conversations')
        zcode_db = _zcode_database(root) if provider=='zcode' else None
        opencode_db = _opencode_database(root) if provider=='opencode' else None
        harness_paths=_harness()['paths'](root) if provider=='deepseek-harness' else []
        if harness_paths:
            for path in harness_paths:
                path=str(path);file_count+=1
                signature=_harness()['fingerprint'](path)
                if cursors.get(path,{}).get('harness')==signature:continue
                try:
                    # project() validates the entire frame stream before yielding.
                    checkpoint={}
                    rows=_harness()['project'](path,'history',timeout=35,cursor=cursors.get(path,{}),checkpoint=checkpoint)
                    batch=[];amount=0;header=None;seeded=False
                    for event in rows:
                        encoded=len(json.dumps(event).encode())
                        if amount+encoded>768*1024:
                            emit('batch',path=path,reset=True,events=batch,cursor=None)
                            batch=[dict(header,isSeeded=seeded)];amount=len(json.dumps(batch[0]).encode())
                        if event.get('type')=='session':header=event;seeded=event.get('isSeeded') is True
                        elif event.get('type')=='session/end-seed':seeded=False
                        batch.append(event);amount+=encoded
                    read_bytes+=checkpoint.pop('readBytes');checkpoint.pop('reset')
                    emit('batch',path=path,reset=True,events=batch,cursor=checkpoint)
                except Exception as error:
                    failed+=1;issue(path,error)
        elif zcode_db or opencode_db:
            path=zcode_db or opencode_db;file_count+=1
            if os.path.islink(path):raise ValueError('Select the actual native database rather than a symbolic link')
            try:
                stats=_native_database()['history'](path,provider,cursors.get(path,{}),emit)
                read_bytes+=stats['readBytes'];scanned_records+=stats['scannedRecords']
            except Exception as error:
                failed+=1;issue(path,error)
        elif provider in ('antigravity', 'agy') and os.path.isdir(native):
            paths=[os.path.join(native,name) for name in sorted(os.listdir(native)) if name.endswith('.db')]
            for path in paths:
                if os.path.islink(path) or not os.path.isfile(path):continue
                file_count += 1
                signature = [stat(path), stat(path + '-wal')]
                if cursors.get(path, {}).get('database') == signature:
                    continue
                events, byte_count, clean = [], 0, True
                try:
                    for row in _agy_iter(path):
                        if 'issue' in row:
                            value = row['issue']
                            if value['code'] != 'unknownModel':
                                clean = False
                            issue(path, value['message'], value['code'])
                            continue
                        event = row['event']
                        amount = len(json.dumps(event).encode('utf-8'))
                        if amount > 768 * 1024:
                            raise ValueError('单条统计记录过大')
                        if amount + byte_count > 768 * 1024:
                            emit('batch', path=path, reset=True, events=events, cursor=None)
                            events, byte_count = [], 0
                        events.append(event)
                        byte_count += amount
                        read_bytes += amount
                    unchanged = signature == [stat(path), stat(path + '-wal')]
                    if not unchanged:
                        failed += 1
                        issue(path, '读取期间数据库发生变化，下次同步将重新读取', 'changedDuringRead')
                    emit('batch', path=path, reset=True, events=events,
                         cursor=dict(database=signature) if clean and unchanged else None)
                except Exception as error:
                    if events:
                        emit('batch', path=path, reset=True, events=events, cursor=None)
                    failed += 1
                    issue(path, error)
        else:
            roots = [root]
            if provider == 'codex' and os.path.isdir(root) and any(os.path.exists(os.path.join(root, p)) for p in ('sessions', 'archived_sessions', 'auth.json', 'config.toml', '.aieyes')) and os.path.basename(root) not in ('sessions', 'archived_sessions'):
                roots = [os.path.join(root, p) for p in ('sessions', 'archived_sessions')]
            elif provider in ('claude','codebuddy','qoder') and os.path.isdir(os.path.join(root, 'projects')):
                roots = [os.path.join(root, 'projects')]
            elif provider in ('kimi','grok','pi','omp') and os.path.isdir(os.path.join(root,'sessions')):
                roots = [os.path.join(root,'sessions')]
            for folder in roots:
                if folder != root and not os.path.exists(folder):
                    continue
                if os.path.isfile(folder):
                    paths = iter([] if provider=='cursor' and _cursor_private_history(folder) else [folder])
                else:
                    def walk_error(error):
                        nonlocal failed
                        failed += 1
                        issue(folder, error)
                    def walk():
                        for directory, directories, names in os.walk(folder, followlinks=False, onerror=walk_error):
                            directories[:] = sorted(n for n in directories if n != '.aieyes' and not (cursor_native and directory==folder and n in ('chats','acp-sessions','projects')))
                            for name in sorted(names):
                                if name.endswith('.jsonl') and (provider!='kimi' or name=='wire.jsonl') and (not grok_native or name=='updates.jsonl'):
                                    yield os.path.join(directory, name)
                    paths = walk()
                for path in paths:
                    file_count += 1
                    try:
                        jsonl(path)
                    except Exception as error:
                        failed += 1
                        issue(path, error)
        if provider == 'codex':
            _native_routes(root, request.get('routeCursor') or {}, emit)
        _qoder_states(root, request, emit, issue)
        if provider=='cursor' and _cursor_private_history(root):
            issue(root,'Cursor 私有会话库的请求 Token 暂未接入；仅导入标准 JSONL，上下文占用不计入累计用量。','unsupportedNativeHistory')
        emit('done', files=file_count, readBytes=read_bytes, scannedRecords=scanned_records, issues=issues,
             forkHistoryIssues=fork_issues,
             partial=any(row['code'] != 'unknownModel' for row in issues),
             failedFiles=failed, invalidRecords=invalid_records)
    except Exception as error:
        emit('error', error='远程记录读取失败：' + str(error)[:400])


if 'REQUEST' in globals():
    _incremental(REQUEST)
    raise SystemExit(0)

try:
    root = os.path.expanduser(sys.argv[1])
    provider = sys.argv[2]
    if not os.path.exists(root):
        raise FileNotFoundError("data directory")
    native = root if os.path.basename(os.path.normpath(root)) == 'conversations' else os.path.join(root, 'conversations')
    if provider in ('antigravity', 'agy') and os.path.isdir(native):
        result, issues = [], []
        size, failed = 0, 0
        for name in sorted(os.listdir(native)):
            path = os.path.join(native, name)
            if not name.endswith('.db') or os.path.islink(path) or not os.path.isfile(path):
                continue
            try:
                data = _agy_database_detailed(path)
            except Exception as error:
                failed += 1
                issues.append(dict(path=name, code='fileRead', message=str(error)))
                continue
            events = data['events']
            issues.extend(dict(issue, path=name) for issue in data['issues'])
            size += len(json.dumps(events))
            if size > 48 * 1024 * 1024:
                raise ValueError('远程记录超过单批大小，请缩小数据目录范围')
            result.append(dict(path=path, events=events))
        response = dict(files=result, issues=issues,
                        partial=any(row['code'] != 'unknownModel' for row in issues), failedFiles=failed)
        if failed and not result:
            response['error'] = 'Antigravity 会话文件读取失败：' + '；'.join(issue['path'] + ' · ' + issue['message'] for issue in issues[:3])
        print(json.dumps(response, separators=(',', ':')))
        raise SystemExit(0)

    roots = [root]
    if provider == "codex" and os.path.isdir(root) and (os.path.basename(root) == ".codex" or any(os.path.exists(os.path.join(root, name)) for name in ("sessions", "archived_sessions", "auth.json", "config.toml", ".aieyes"))) and os.path.basename(os.path.normpath(root)) not in ("sessions", "archived_sessions"):
        roots = [os.path.join(root, part) for part in ("sessions", "archived_sessions")]
    elif provider == "claude" and os.path.isdir(os.path.join(root, "projects")):
        roots = [os.path.join(root, "projects")]

    files = []
    for folder in roots:
        if os.path.isfile(folder):
            files.append(folder)
        else:
            for directory, directories, names in os.walk(folder, followlinks=False):
                directories[:] = [n for n in directories if n != '.aieyes']
                files.extend(os.path.join(directory, n) for n in names if n.endswith(".jsonl"))

    result = []
    size = 0
    for path in sorted(files):
        events = []
        with open(path, "rb") as stream:
            for line in stream:
                if not line.endswith(b"\n"):
                    continue
                try:
                    value = json.loads(line)
                except (ValueError, UnicodeDecodeError):
                    continue
                event = _project(value, provider)
                if event:
                    size += len(json.dumps(event))
                    if size > 48 * 1024 * 1024:
                        raise ValueError("history exceeds batch size")
                    events.append(event)
        result.append({"path": path, "events": events})
    print(json.dumps({"files": result}, separators=(",", ":")))
except Exception as error:
    print(json.dumps(dict(error='远程记录读取失败：' + str(error)[:400]), separators=(',', ':')))
