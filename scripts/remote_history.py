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

def _project(value, provider):
    event = None
    if provider == "codex":
        payload = value.get("payload") or {}
        kind = value.get("type")
        if kind == "session_meta":
            event = {"type": kind, "timestamp": value.get("timestamp"), "payload": {"timestamp": payload.get("timestamp"), "id": payload.get("id", payload.get("session_id")), "model_provider": payload.get("model_provider")}}
        elif kind == "turn_context":
            event = {"type": kind, "payload": {"model": payload.get("model")}}
        elif kind == "event_msg" and payload.get("type") in ("task_started", "task_complete", "turn_aborted"):
            event = {"type": kind, "timestamp": value.get("timestamp"), "payload": {"type": payload["type"]}}
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


def _incremental(request):
    import hashlib
    root = os.path.abspath(os.path.expanduser(request['path']))
    provider = request['provider']
    cursors = request.get('cursors', {})
    issues, failed, invalid_records, read_bytes, file_count = [], 0, 0, 0, 0

    def emit(kind, **fields):
        line = json.dumps(dict(protocol=2, kind=kind, **fields), separators=(',', ':'))
        if len(line.encode('utf-8')) + 1 > 1024 * 1024:
            raise ValueError('远程统计批次超过 1 MiB')
        print(line, flush=True)

    def issue(path, error, code='fileRead'):
        nonlocal invalid_records
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
                            clean = False
                            value = row['issue']
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
        emit('done', files=file_count, readBytes=read_bytes, issues=issues, partial=bool(issues), failedFiles=failed, invalidRecords=invalid_records)
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
        response = dict(files=result, issues=issues, partial=bool(issues), failedFiles=failed)
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
