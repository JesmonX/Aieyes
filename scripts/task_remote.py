"""Bounded Cline/Roo task metadata for SSH history and live-session collection.

Only counters, native identifiers, timestamps and lifecycle flags leave this
module. Prompt text and provider credentials never appear in its output.
"""
import datetime
import hashlib
import json
import pathlib
import sqlite3

VERSION = 2
MAX_FILE = 64 * 1024 * 1024
MAX_FILES = 20000


def text(value):
    return value if isinstance(value, str) and 0 < len(value) <= 4096 else ''


def files(provider, root):
    root = pathlib.Path(root)
    if provider not in ('cline', 'roo', 'kilo'):
        return []

    def supported(path):
        return path.name in ('history_item.json', 'ui_messages.json') or (provider == 'cline' and path.name.endswith('.messages.json'))

    if root.is_file() and supported(root):
        return [root]
    names = ('sessions', 'tasks') if provider == 'cline' else ('tasks',)
    roots = [root] + [base / name for base in (root, root / 'data') for name in names]
    found = []
    database = cline_database(root) if provider == 'cline' else None
    if database:
        try:
            with sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True, timeout=.2) as db:
                found.extend(pathlib.Path(row[0]) for row in db.execute('SELECT messages_path FROM sessions WHERE messages_path IS NOT NULL LIMIT 20000')
                             if isinstance(row[0], str) and pathlib.Path(row[0]).is_absolute() and pathlib.Path(row[0]).is_file() and supported(pathlib.Path(row[0])))
        except sqlite3.Error:
            pass
    for directory in sorted(set(roots)):
        if not directory.is_dir():
            continue
        children = [directory] + sorted(p for p in directory.iterdir() if p.is_dir() and not p.name.startswith('.'))
        for child in children:
            messages = sorted(p for p in child.glob('*.messages.json') if p.is_file()) if provider == 'cline' else []
            if messages:
                found.extend(messages)
            else:
                item = next((child / name for name in ('history_item.json', 'ui_messages.json') if (child / name).is_file()), None)
                if item:
                    found.append(item)
            if len(found) >= MAX_FILES:
                return sorted(set(found[:MAX_FILES]))
    return sorted(set(found))


def read(path):
    path = pathlib.Path(path)
    if path.stat().st_size > MAX_FILE:
        raise ValueError('Task file exceeds 64 MiB')
    with path.open('rb') as stream:
        raw = stream.read(MAX_FILE + 1)
    if len(raw) > MAX_FILE:
        raise ValueError('Task file exceeds 64 MiB')
    return json.loads(raw)


def counter(value):
    if value is None:
        return 0
    if type(value) is not int or not 0 <= value <= 2**64 - 1:
        raise ValueError('Invalid task token counter')
    return value


def tokens(value, names):
    values = [counter(value.get(name)) for name in names]
    if sum(values) > 2**64 - 1:
        raise ValueError('Task token counter overflow')
    return dict(zip(('input', 'output', 'cacheRead', 'cacheWrite'), values))


def milliseconds(value):
    if isinstance(value, str):
        try:
            return max(0, int(datetime.datetime.fromisoformat(value.replace('Z', '+00:00')).timestamp() * 1000))
        except (ValueError, OverflowError):
            return 0
    return value if type(value) is int and value > 0 else 0


def overlapping_legacy_tasks(provider, found):
    if provider != 'cline':
        return set()
    legacy = {path.parent.name: path for path in found if path.name == 'ui_messages.json'}
    if not legacy:
        return set()
    parents = {}
    for path in found:
        if path.name == 'ui_messages.json':
            continue
        try:
            value = read(manifest(path) or path)
            if not isinstance(value, dict):
                continue
            identity = text(value.get('session_id') or value.get('sessionId') or value.get('id'))
            parent = text(value.get('parent_session_id') or (value.get('origin') or {}).get('parentThreadId') or value.get('parentTaskId') or value.get('rootTaskId'))
            if identity and parent:
                parents[identity] = parent
        except (OSError, ValueError, TypeError, AttributeError):
            continue
    related = set()
    for parent in parents.values():
        seen = set()
        for _ in range(32):
            if parent in seen:
                break
            seen.add(parent)
            if parent in legacy:
                related.add(legacy[parent])
            parent = parents.get(parent)
            if not parent:
                break
    overlapping = set()
    for path in related:
        try:
            value = read(path)
            if isinstance(value, list) and any(isinstance(m, dict) and m.get('type') == 'say' and m.get('say') == 'subagent_usage' for m in value):
                overlapping.add(path)
        except (OSError, ValueError, TypeError, AttributeError):
            pass
    return overlapping


def rows(provider, value, path, exclude_subagents=False):
    path = pathlib.Path(path)
    if isinstance(value, list):
        usage, current, suppressed_at = [], None, 0
        for message in value:
            if not isinstance(message, dict) or message.get('type') != 'say':
                continue
            kind = message.get('say')
            if kind not in ('api_req_started', 'api_req_finished', 'deleted_api_reqs', 'subagent_usage') or not isinstance(message.get('text'), str):
                continue
            data = json.loads(message['text'])
            if not isinstance(data, dict):
                raise ValueError('Invalid extension usage metadata')
            if kind == 'api_req_finished':
                if current is not None:
                    usage[current][0].update(data)
                    current = None
                continue
            stamp = milliseconds(message.get('ts'))
            if not stamp:
                raise ValueError('Invalid extension usage timestamp')
            if exclude_subagents and kind == 'subagent_usage':
                suppressed_at, current = max(suppressed_at, stamp), None
                continue
            model = text((message.get('modelInfo') or {}).get('modelId')) if kind == 'api_req_started' else ''
            current = len(usage) if kind == 'api_req_started' else None
            usage.append([data, stamp, model or 'unknown'])
        total = dict(input=0, output=0, cacheRead=0, cacheWrite=0)
        models, stamp = set(), suppressed_at
        for data, ts, model in usage:
            if not any(key in data for key in ('tokensIn', 'tokensOut', 'cacheReads', 'cacheWrites')):
                continue
            for key, count in tokens(data, ('tokensIn', 'tokensOut', 'cacheReads', 'cacheWrites')).items():
                total[key] += count
            if sum(total.values()) > 2**64 - 1:
                raise ValueError('Task token counter overflow')
            models.add(model)
            stamp = max(stamp, ts)
        if not stamp:
            return []
        row = dict(id='task:' + path.parent.name, sessionId=path.parent.name, timestamp=stamp // 1000,
                   model=next(iter(models)) if len(models) == 1 else 'unknown', tokens=total)
        if provider == 'cline':
            row['clineTaskUsageVersion'] = 3 if exclude_subagents else 2
        return [row]
    if not isinstance(value, dict):
        raise ValueError('Invalid task file')
    if provider == 'cline' and isinstance(value.get('messages'), list):
        session = text(value.get('sessionId'))
        if not session:
            raise ValueError('Missing Cline session ID')
        result = []
        for message in value['messages']:
            if not isinstance(message, dict) or message.get('role') != 'assistant' or not isinstance(message.get('metrics'), dict):
                continue
            identity, stamp = text(message.get('id')), milliseconds(message.get('ts'))
            if not identity or not stamp:
                raise ValueError('Invalid Cline request identity')
            result.append(dict(id='msg:' + identity, sessionId=session, timestamp=stamp // 1000,
                               model=text((message.get('modelInfo') or {}).get('id')) or 'unknown',
                               tokens=tokens(message['metrics'], ('inputTokens', 'outputTokens', 'cacheReadTokens', 'cacheWriteTokens')),
                               inheritance=dict(origin='cline-message:%s:%s' % (identity, stamp), inherited=False)))
        return result
    identity, stamp = text(value.get('id')), milliseconds(value.get('ts'))
    if not identity or not stamp or type(value.get('tokensIn')) is not int or type(value.get('tokensOut')) is not int:
        raise ValueError('Invalid task summary')
    return [dict(id='task:' + identity, sessionId=text(value.get('rootTaskId')) or text(value.get('parentTaskId')) or identity,
                 timestamp=stamp // 1000, model='unknown', tokens=tokens(value, ('tokensIn', 'tokensOut', 'cacheReads', 'cacheWrites')))]


def stat(path):
    try:
        meta = pathlib.Path(path).stat()
        return [meta.st_dev, meta.st_ino, meta.st_size, meta.st_mtime_ns]
    except OSError:
        return None


def history(root, provider, cursors, emit, issue, precise=True, generation=0):
    found = files(provider, root)
    overlapping = overlapping_legacy_tasks(provider, found)
    stats = dict(files=len(found), failedFiles=0, readBytes=0, scannedRecords=0)
    for file in found:
        path = str(file)
        exclude_subagents = file in overlapping
        if exclude_subagents:
            issue(path, 'Subagent summary overlaps explicitly related child records; only individually verifiable usage was counted', 'overlappingSubagentSummary')
        signature = dict(taskVersion=VERSION, taskFile=stat(file), preciseCounting=precise, countingGeneration=generation, excludeSubagents=exclude_subagents)
        if all(cursors.get(path, {}).get(key) == value for key, value in signature.items()):
            continue
        try:
            projected = rows(provider, read(file), file, exclude_subagents)
            stats['readBytes'] += file.stat().st_size
            stats['scannedRecords'] += len(projected)
            batch, amount = [], 0
            for event in projected:
                size = len(json.dumps(event).encode())
                if amount + size > 768 * 1024:
                    emit('batch', path=path, reset=True, events=batch, cursor=None)
                    batch, amount = [], 0
                batch.append(event)
                amount += size
            unchanged = signature['taskFile'] == stat(file)
            emit('batch', path=path, reset=True, events=batch, cursor=signature if unchanged else None)
            if not unchanged:
                stats['failedFiles'] += 1
                issue(path, 'Task changed during read; retrying next scan', 'changedDuringRead')
        except (OSError, ValueError, TypeError, AttributeError) as error:
            stats['failedFiles'] += 1
            # json.JSONDecodeError can contain source text in repr; report only a
            # fixed description, never task text or a client response.
            issue(path, 'Task metadata is invalid or is being rewritten', 'fileRead')
    return stats


def phase(value):
    if isinstance(value, dict) and isinstance(value.get('status'), str):
        return {'idle': 'complete', 'completed': 'complete', 'running': 'start', 'active': 'start', 'delegated': 'start',
                'pending': 'PermissionRequest', 'failed': 'failed', 'cancelled': 'interrupted'}.get(value['status'], 'unknown')
    messages = value if isinstance(value, list) else value.get('messages', [])
    for message in reversed(messages):
        if not isinstance(message, dict):
            continue
        ask = message.get('ask')
        if ask in ('completion_result', 'resume_completed_task'):
            return 'complete'
        if ask == 'api_req_failed':
            return 'failed'
        if ask in ('followup', 'plan_mode_respond', 'act_mode_respond', 'command', 'tool', 'use_mcp_server'):
            return 'PermissionRequest'
        kind = message.get('say')
        if kind in ('completion_result', 'api_req_finished'):
            return 'complete'
        if kind == 'error':
            return 'failed'
        if kind == 'api_req_started':
            try:
                data = json.loads(message.get('text') or '{}')
                reason = data.get('cancelReason')
                return 'interrupted' if reason == 'user_cancelled' else 'failed' if reason else 'complete' if 'cost' in data else 'start'
            except (ValueError, AttributeError):
                return 'unknown'
        if kind in ('tool', 'command'):
            return 'tool'
        if message.get('role') == 'user':
            return 'start'
        if message.get('role') == 'assistant' and isinstance(message.get('metrics'), dict):
            return 'complete'
    return 'unknown'


def cline_database(root):
    root = pathlib.Path(root)
    if root.is_file():
        return root if root.name == 'sessions.db' else None
    return next((root / name for name in ('sessions.db', 'db/sessions.db', 'data/db/sessions.db') if (root / name).is_file()), None)


def manifest(file):
    if file.name.endswith('.messages.json'):
        path = file.with_name(file.name[:-len('.messages.json')] + '.json')
        if path.is_file():
            return path
    return None


def metadata_paths(provider, found):
    if provider != 'cline':
        return [path for path in found if path.name == 'history_item.json']
    paths = []
    for directory in sorted({path.parent for path in found}):
        for path in sorted(directory.glob('*.json')):
            if path.is_file() and not path.name.endswith('.messages.json') and path.name not in ('ui_messages.json', 'api_conversation_history.json'):
                paths.append(path)
                if len(paths) == MAX_FILES:
                    return paths
    return paths


def session_record(provider, value, file, stamp):
    current = provider == 'cline' and isinstance(value, dict) and bool(value.get('session_id') or value.get('sessionId'))
    if current:
        origin = value.get('origin') or {}
        return dict(id=text(value.get('session_id') or value.get('sessionId')),
                    parent=text(value.get('parent_session_id') or origin.get('parentThreadId')),
                    at=milliseconds(value.get('updated_at')) or milliseconds(value.get('ended_at')) or int(stamp * 1000),
                    kind=phase(value), interactive=value.get('interactive') if type(value.get('interactive')) is bool else None,
                    source=text(value.get('source') or origin.get('source')))
    if isinstance(value, list):
        return dict(id=file.parent.name, parent='', at=max([milliseconds(m.get('ts')) for m in value if isinstance(m, dict)] or [int(stamp * 1000)]), kind=phase(value))
    if isinstance(value, dict) and type(value.get('tokensIn')) is int:
        return dict(id=text(value.get('id')) or file.parent.name, parent=text(value.get('parentTaskId') or value.get('rootTaskId')),
                    at=milliseconds(value.get('ts')) or int(stamp * 1000), kind=phase(value))
    raise ValueError('Invalid task metadata')


def add_ancestors(provider, found, indexed):
    wanted = {row['parent'] for row in indexed.values() if row.get('parent') and row['parent'] not in indexed}
    if not wanted:
        return
    paths = metadata_paths(provider, found)
    paths.sort(key=lambda path: path.stem not in wanted)
    for path in paths:
        try:
            meta = path.stat()
            if meta.st_size > 1024 * 1024:
                continue
            value = read(path)
            if provider == 'cline' and (not isinstance(value, dict) or not (value.get('session_id') or value.get('sessionId'))):
                continue
            row = session_record(provider, value, path, meta.st_mtime)
            if not row['id']:
                continue
        except (OSError, ValueError, TypeError, AttributeError):
            continue
        indexed.setdefault(row['id'], row)
        if row['id'] in wanted:
            wanted.remove(row['id'])
            seen = set()
            for _ in range(32):
                if not row.get('parent') or row['id'] in seen:
                    break
                seen.add(row['id'])
                if row['parent'] not in indexed:
                    wanted.add(row['parent'])
                    break
                row = indexed[row['parent']]
        if not wanted:
            break


def excluded(row, all_rows):
    seen = set()
    for _ in range(32):
        if row['id'] in seen:
            return False
        seen.add(row['id'])
        if not row.get('parent'):
            return row.get('interactive') is False and row.get('source') != 'subagent'
        row = all_rows.get(row['parent'])
        if row is None:
            return False
    return False


def session_batch(key, row, signature, changed=True):
    event = dict(session=row['id'], turn=row['id'], at=row['at'])
    return dict(key=key, identity=row['id'], signature=signature, modified=row['at'] // 1000,
                reset=changed, database=True, continuation=dict(taskVersion=VERSION, row=row),
                events=[dict(event, kind='start', parentSession=row.get('parent', '')), dict(event, kind=row['kind'])] if changed else [])


def cached_row(value):
    if not isinstance(value, dict) or not text(value.get('id')) or not milliseconds(value.get('at')):
        return None
    return dict(id=text(value['id']), parent=text(value.get('parent')), at=milliseconds(value['at']),
                kind=value.get('kind') if value.get('kind') in ('start','complete','tool','thinking','PermissionRequest','failed','interrupted') else 'unknown',
                source=text(value.get('source')), interactive=value.get('interactive') if type(value.get('interactive')) is bool else None)


def sessions(root, provider, cursors, now):
    database = cline_database(root) if provider == 'cline' else None
    indexed, descriptors, errors = {}, {}, []
    if database:
        signature = 'cline-db:' + hashlib.sha256(json.dumps([stat(database), stat(str(database) + '-wal')]).encode()).hexdigest()
        with sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True, timeout=.2) as db:
            since = datetime.datetime.fromtimestamp(now - 86400, datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%S')
            columns = "session_id,status,updated_at,COALESCE(parent_session_id,''),interactive,source"

            def add(raw):
                identity, status, at, parent, interactive, source = raw
                row = dict(id=identity, parent=parent, at=milliseconds(at), kind=phase(dict(status=status)),
                           interactive=bool(interactive) if interactive is not None else None, source=source)
                indexed[identity] = row
                key = str(database) + '#' + identity
                descriptors[identity] = (key, signature, cursors.get(key, {}).get('signature') != signature)

            for raw in db.execute('SELECT ' + columns + ' FROM sessions WHERE updated_at>? ORDER BY updated_at DESC LIMIT 2048', (since,)):
                add(raw)
            parents, seen = [row['parent'] for row in indexed.values() if row['parent']], set()
            for _ in range(32):
                following = []
                for identity in parents:
                    if identity in indexed or identity in seen:
                        continue
                    seen.add(identity)
                    raw = db.execute('SELECT ' + columns + ' FROM sessions WHERE session_id=?', (identity,)).fetchone()
                    if raw:
                        add(raw)
                        following.append(raw[3])
                if not following:
                    break
                parents = following
    else:
        recent = []
        found = files(provider, root)
        for file in found:
            companion = manifest(file)
            stamp = max(file.stat().st_mtime, companion.stat().st_mtime if companion else 0)
            if now - stamp < 86400:
                recent.append((stamp, str(file), file, companion))
        recent.sort(reverse=True)
        for stamp, key, file, companion in recent[:256]:
            signature = 'task:' + hashlib.sha256(json.dumps([VERSION, stat(file), stat(companion) if companion else None]).encode()).hexdigest()
            old = cursors.get(key, {})
            row = cached_row((old.get('continuation') or {}).get('row'))
            changed = old.get('signature') != signature
            try:
                if changed or not isinstance(row, dict):
                    value = read(companion or file)
                    row = session_record(provider, value, file, stamp)
            except (OSError, ValueError, TypeError, AttributeError):
                errors.append('Task metadata is invalid or is being rewritten')
                changed = False
            if isinstance(row, dict) and text(row.get('id')):
                indexed[row['id']] = row
                descriptors[row['id']] = (key, signature if changed else old.get('signature', ''), changed)
        add_ancestors(provider, found, indexed)
    visible = sorted((row for row in indexed.values() if row['id'] in descriptors and now - row['at'] // 1000 < 86400 and not excluded(row, indexed)), key=lambda row: (-row['at'], row['id']))[:256]
    return [session_batch(descriptors[row['id']][0], row, descriptors[row['id']][1], descriptors[row['id']][2]) for row in visible], errors
