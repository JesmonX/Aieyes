import json
import os
import sys

root = os.path.expanduser(sys.argv[1])
provider = sys.argv[2]
if not os.path.exists(root):
    raise FileNotFoundError("data directory")
roots = [root]
if provider == "codex" and os.path.isdir(os.path.join(root, "sessions")):
    roots = [os.path.join(root, part) for part in ("sessions", "archived_sessions")]
elif provider == "claude" and os.path.isdir(os.path.join(root, "projects")):
    roots = [os.path.join(root, "projects")]

files = []
for folder in roots:
    if os.path.isfile(folder):
        files.append(folder)
    else:
        for directory, _, names in os.walk(folder, followlinks=False):
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
                    event = {"type": kind, "payload": {"id": payload.get("id", payload.get("session_id"))}}
                elif kind == "turn_context":
                    event = {"type": kind, "payload": {"model": payload.get("model")}}
                elif kind == "event_msg" and payload.get("type") == "token_count":
                    event = {"type": kind, "timestamp": value.get("timestamp"), "payload": {
                        "type": "token_count", "info": payload.get("info"), "rate_limits": payload.get("rate_limits")}}
            elif provider == "claude":
                message = value.get("message") or {}
                if value.get("type") == "assistant" and message.get("usage"):
                    event = {k: value.get(k) for k in ("type", "timestamp", "sessionId", "requestId", "uuid", "isApiErrorMessage")}
                    event["message"] = {k: message.get(k) for k in ("id", "model", "usage")}
            elif "tokens" in value:
                event = {k: value.get(k) for k in ("id", "sessionId", "timestamp", "model", "tokens")}
            if event:
                size += len(json.dumps(event))
                if size > 48 * 1024 * 1024:
                    raise ValueError("history exceeds batch size")
                events.append(event)
    result.append({"path": path, "events": events})
print(json.dumps({"files": result}, separators=(",", ":")))

