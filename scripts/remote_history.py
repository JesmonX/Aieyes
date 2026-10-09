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
            raise ValueError('Antigravity conversation identity missing')
        events = []
        for index, status, raw in db.execute('SELECT idx,status,metadata FROM steps WHERE length(metadata)>0 ORDER BY idx'):
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
                raise ValueError('Antigravity usage time or token range invalid')
            if index not in models:
                raise ValueError('Antigravity actual model missing; update the CLI')
            identity = _agy_text(usage, 7) or _agy_text(usage, 11) or 'step:' + str(index)
            events.append(dict(id=identity, sessionId=session, timestamp=end, intervalStart=start,
                               intervalEvidence='antigravity-step-v1', model=models[index], tokens=tokens))
        return events
    finally:
        db.close()

root = os.path.expanduser(sys.argv[1])
provider = sys.argv[2]
if not os.path.exists(root):
    raise FileNotFoundError("data directory")
native = root if os.path.basename(os.path.normpath(root)) == 'conversations' else os.path.join(root, 'conversations')
if provider in ('antigravity', 'agy') and os.path.isdir(native):
    result = []
    size = 0
    for name in sorted(os.listdir(native)):
        path = os.path.join(native, name)
        if not name.endswith('.db') or os.path.islink(path) or not os.path.isfile(path):
            continue
        events = _agy_database(path)
        size += len(json.dumps(events))
        if size > 48 * 1024 * 1024:
            raise ValueError('history exceeds batch size')
        result.append(dict(path=path, events=events))
    print(json.dumps(dict(files=result), separators=(',', ':')))
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
            if event:
                size += len(json.dumps(event))
                if size > 48 * 1024 * 1024:
                    raise ValueError("history exceeds batch size")
                events.append(event)
    result.append({"path": path, "events": events})
print(json.dumps({"files": result}, separators=(",", ":")))
