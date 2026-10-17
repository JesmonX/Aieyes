import contextlib
import json
import os
import sys

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


class _CodexHistory:
    """Only export a verified counter and its proof; parent conversation text stays remote."""
    def __init__(self, root):
        import sqlite3
        self.paths, self.cache = {}, {}
        home = _codex_home(root)
        folders = sorted(set([root, os.path.join(home, 'sessions'), os.path.join(home, 'archived_sessions')]), key=len)
        visited = []
        def walk(folder):
            for directory, children, names in os.walk(folder, followlinks=False):
                children[:] = [name for name in children if name != '.aieyes' and not os.path.islink(os.path.join(directory, name))]
                for name in names:
                    path = os.path.join(directory, name)
                    if name.endswith('.jsonl') and not os.path.islink(path):
                        yield path
        for folder in folders:
            if any(os.path.commonpath([folder, earlier]) == earlier for earlier in visited):
                continue
            visited.append(folder)
            paths = [folder] if os.path.isfile(folder) else walk(folder)
            for path in paths:
                try:
                    # Imported filenames can carry an unrelated UUID suffix.
                    # Resolve references using the actual session metadata.
                    with open(path, 'rb') as stream:
                        row = json.loads(stream.readline(1024 * 1024))
                    if row.get('type') == 'session_meta':
                        payload = row.get('payload') or {}
                        identity = payload.get('id') or payload.get('session_id')
                        if identity:
                            self.paths.setdefault(identity, path)
                except (OSError, ValueError):
                    pass
        database = os.path.join(home, 'state_5.sqlite')
        if os.path.isfile(database):
            try:
                from pathlib import Path
                with contextlib.closing(sqlite3.connect(Path(database).as_uri() + '?mode=ro', uri=True)) as db:
                    for identity, path in db.execute('SELECT id,rollout_path FROM threads'):
                        if os.path.isfile(path):
                            self.paths[identity] = path
            except (OSError, sqlite3.Error):
                pass

    def resolve(self, base):
        thread, end = base.get('thread_id', ''), base.get('end_byte_offset')
        try:
            return self._resolve(thread, end, [])
        except (OSError, ValueError, TypeError, KeyError):
            return dict(parentThread=thread, endByteOffset=end if isinstance(end, int) and end >= 0 else 0,
                        tokens=None, timestamp=None, proof='', issue='父会话历史缺失、损坏或边界无效')

    def _resolve(self, thread, end, ancestors):
        import hashlib
        from datetime import datetime
        if not thread or thread in ancestors or len(ancestors) >= 64 or type(end) is not int or end < 0:
            raise ValueError('invalid history reference')
        if (thread, end) in self.cache:
            return self.cache[(thread, end)]
        digest, offset, tokens, stamp, parent = hashlib.sha256(), 0, None, None, None
        with open(self.paths[thread], 'rb') as stream:
            if end > os.fstat(stream.fileno()).st_size:
                raise ValueError('incomplete parent')
            while offset < end:
                line = stream.readline(min(end - offset, 16 * 1024 * 1024) + 1)
                if not line.endswith(b'\n') or len(line) > 16 * 1024 * 1024 or offset + len(line) > end:
                    raise ValueError('invalid boundary')
                offset += len(line)
                digest.update(line)
                row = json.loads(line)
                payload = row.get('payload') or {}
                if row.get('type') == 'session_meta':
                    if (payload.get('id') or payload.get('session_id')) != thread:
                        raise ValueError('wrong parent')
                    parent = payload.get('history_base')
                if row.get('type') == 'event_msg' and payload.get('type') == 'token_count':
                    usage = (payload.get('info') or {}).get('total_token_usage')
                    if not isinstance(usage, dict):
                        continue
                    values = [usage.get(key, 0) for key in ('input_tokens', 'output_tokens', 'cached_input_tokens', 'cache_write_input_tokens', 'reasoning_output_tokens')]
                    if 'input_tokens' not in usage or any(type(v) is not int or v < 0 for v in values) or values[0] < values[2] + values[3]:
                        raise ValueError('invalid counter')
                    tokens = dict(input=values[0]-values[2]-values[3], output=values[1], cacheRead=values[2], cacheWrite=values[3], reasoning=values[4])
                    raw = row.get('timestamp')
                    stamp = int(datetime.fromisoformat(raw.replace('Z', '+00:00')).timestamp()) if isinstance(raw, str) else raw
        if parent:
            inherited = self._resolve(parent.get('thread_id'), parent.get('end_byte_offset'), ancestors + [thread])
            digest.update(inherited['proof'].encode())
            if tokens is None:
                tokens, stamp = inherited['tokens'], inherited['timestamp']
        result = dict(parentThread=thread, endByteOffset=end, tokens=tokens or dict(input=0, output=0, cacheRead=0, cacheWrite=0, reasoning=0), timestamp=stamp, proof=digest.hexdigest(), issue='')
        self.cache[(thread, end)] = result
        return result


def _project(value, provider):
    event = None
    if provider == "codex":
        payload = value.get("payload") or {}
        kind = value.get("type")
        if kind == "session_meta":
            event = {"type": kind, "timestamp": value.get("timestamp"), "payload": {"timestamp": payload.get("timestamp"), "id": payload.get("id", payload.get("session_id")), "model_provider": payload.get("model_provider")}}
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
    elif provider == "claude":
        message = value.get("message") or {}
        if value.get("type") == "assistant" and message.get("usage"):
            event = {k: value.get(k) for k in ("type", "timestamp", "sessionId", "requestId", "uuid", "isApiErrorMessage")}
            event["message"] = {k: message.get(k) for k in ("id", "model", "usage")}
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


def _incremental(request):
    import hashlib
    root = os.path.abspath(os.path.expanduser(request['path']))
    provider = request['provider']
    cursors = request.get('cursors', {})
    precise = bool(request.get('codexPreciseCounting', False))
    history = _CodexHistory(root) if provider == 'codex' and precise else None
    fork_issues = []
    issues, failed, invalid_records, read_bytes, file_count = [], 0, 0, 0, 0

    def emit(kind, **fields):
        line = json.dumps(dict(protocol=2, kind=kind, **fields), separators=(',', ':'))
        if len(line.encode('utf-8')) + 1 > 1024 * 1024:
            raise ValueError('远程统计批次超过 1 MiB')
        print(line, flush=True)

    def issue(path, error, code='fileRead'):
        nonlocal invalid_records
        if code == 'missingHistoryBase':
            if len(fork_issues) < 128:
                fork_issues.append(dict(path=os.path.basename(path), code=code, message=str(error)[:400]))
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
            if (refreshed.get('tokens'), refreshed.get('proof'), refreshed.get('issue')) != (inherited.get('tokens'), inherited.get('proof'), inherited.get('issue')):
                old = {}
                inherited = refreshed
        if old and inherited and inherited.get('tokens') is None:
            issue(path, '父会话历史尚未找到；首段累计暂不计入', 'missingHistoryBase')
        with open(path, 'rb') as stream:
            metadata = os.fstat(stream.fileno())
            size = metadata.st_size
            identity = [metadata.st_dev, metadata.st_ino]
            offset = int(old.get('offset', 0))
            prefix_size = int(old.get('prefixBytes', min(size, 128)))
            valid = (old.get('identity') == identity and 0 <= offset <= size
                     and 0 <= prefix_size <= min(size, 128)
                     and old.get('prefix') == digest(stream, 0, prefix_size)
                     and old.get('anchor') == digest(stream, max(0, offset - 128), min(offset, 128))
                     and not (size == old.get('size') and metadata.st_mtime_ns != old.get('mtimeNs')))
            if not valid:
                offset, prefix_size = 0, min(size, 128)
            prefix_size = min(size, 128)
            prefix = digest(stream, 0, prefix_size)
            reset, events, bytes_in_batch = not valid, [], 0
            last_commit = offset

            def flush():
                nonlocal reset, events, bytes_in_batch, last_commit
                cursor = dict(identity=identity, prefixBytes=prefix_size, prefix=prefix,
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
                    if inherited['tokens'] is None:
                        issue(path, inherited['issue'], 'missingHistoryBase')
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

    try:
        if not os.path.exists(root):
            raise FileNotFoundError('数据目录不存在')
        native = root if os.path.basename(root) == 'conversations' else os.path.join(root, 'conversations')
        if provider in ('antigravity', 'agy') and os.path.isdir(native):
            for name in sorted(os.listdir(native)):
                path = os.path.join(native, name)
                if not name.endswith('.db') or os.path.islink(path) or not os.path.isfile(path):
                    continue
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
            elif provider == 'claude' and os.path.isdir(os.path.join(root, 'projects')):
                roots = [os.path.join(root, 'projects')]
            for folder in roots:
                if folder != root and not os.path.exists(folder):
                    continue
                if os.path.isfile(folder):
                    paths = iter([folder])
                else:
                    def walk_error(error):
                        nonlocal failed
                        failed += 1
                        issue(folder, error)
                    def walk():
                        for directory, directories, names in os.walk(folder, followlinks=False, onerror=walk_error):
                            directories[:] = sorted(n for n in directories if n != '.aieyes')
                            for name in sorted(names):
                                if name.endswith('.jsonl'):
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
        emit('done', files=file_count, readBytes=read_bytes, issues=issues,
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
