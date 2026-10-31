"""CodeBuddy IDE 4.12.x request-only projection, shared by SSH collectors.

Never opens messages/*.json or returns conversation names, prompts or replies.
"""
import json
import math
import os
import pathlib
import re

MAX_FILE = 64 * 1024 * 1024
MAX_FILES = 20000


def is_path(path):
    return 'CodeBuddyExtension' in pathlib.Path(path).parts


def history_roots(root):
    root = pathlib.Path(root)
    if not root.is_dir():
        return []
    if root.name == 'history':
        return [root]
    result, pending, visited = [], [(root, 0)], 0
    while pending:
        path, depth = pending.pop()
        visited += 1
        if visited > MAX_FILES:
            raise ValueError('CodeBuddy IDE 历史目录过多')
        if path.name == 'history':
            result.append(path)
        elif depth < 6:
            for entry in path.iterdir():
                if entry.is_symlink() or not entry.is_dir() or entry.name.startswith('.') or entry.name in ('Cache', 'Logs', 'Public', 'messages', 'file-tree', 'check-point'):
                    continue
                pending.append((entry, depth + 1))
    return sorted(result)


def user_id(root):
    path = pathlib.Path(root)
    user = path.parent.name
    return user if is_path(path) and path.name == 'history' and 0 < len(user) <= 256 and user not in ('default', 'Public', 'anonymous') else None


def files(root):
    result, visited = [], 0
    for history in history_roots(root):
        for workspace in history.iterdir():
            visited += 1
            if visited > MAX_FILES * 5:
                raise ValueError('CodeBuddy IDE 历史文件过多')
            if workspace.is_symlink() or not workspace.is_dir():
                continue
            for conversation in workspace.iterdir():
                visited += 1
                if visited > MAX_FILES * 5:
                    raise ValueError('CodeBuddy IDE 历史文件过多')
                if conversation.is_symlink() or not conversation.is_dir():
                    continue
                path = conversation / 'index.json'
                if path.is_file() and not path.is_symlink():
                    result.append(path)
                    if len(result) > MAX_FILES:
                        raise ValueError('CodeBuddy IDE 历史文件过多')
    return sorted(result)


def signature(path):
    stat = path.stat()
    return 'codebuddy-ide-v1:%d:%d' % (stat.st_size, stat.st_mtime_ns)


def read(path):
    before = signature(path)
    if path.stat().st_size > MAX_FILE:
        raise ValueError('CodeBuddy IDE 历史文件过大')
    with path.open('rb') as stream:
        raw = stream.read(MAX_FILE + 1)
    if len(raw) > MAX_FILE:
        raise ValueError('CodeBuddy IDE 历史文件过大')
    try:
        value = json.loads(raw)
        requests = value.get('requests', [])
        if not isinstance(requests, list) or any(not isinstance(request, dict) for request in requests):
            raise ValueError()
        # Drop content before handing metadata to either projection.
        safe = [{key: request.get(key) for key in ('id', 'state', 'startedAt', 'usage')} for request in requests if isinstance(request, dict)]
    except (ValueError, TypeError, AttributeError):
        raise ValueError('CodeBuddy IDE 记录格式无效或正在写入') from None
    if before != signature(path):
        raise ValueError('CodeBuddy IDE 历史正在写入')
    return safe, len(raw)


def valid_id(value):
    return isinstance(value, str) and re.fullmatch(r'[a-zA-Z0-9_-]{1,256}', value) is not None


def rows(path, requests):
    result, invalid = [], 0
    session = path.parent.name
    if not valid_id(session):
        return [], 1
    for request in requests:
        usage = request.get('usage')
        if usage is None:
            continue
        try:
            if not isinstance(usage, dict) or not valid_id(request['id']):
                raise ValueError()
            at = request.get('startedAt')
            if type(at) is not int or not 0 < at <= 2**63 - 1:
                raise ValueError()
            def count(key):
                value = usage.get(key)
                if value is None:
                    return 0
                if type(value) is not int or not 0 <= value <= 2**64 - 1:
                    raise ValueError()
                return value
            input_tokens, output = count('inputTokens'), count('outputTokens')
            cache_read, cache_write = count('cacheTokens'), count('cachedWriteTokens')
            uncached = input_tokens - cache_read - cache_write
            total = input_tokens + output
            if uncached < 0 or total > 2**64 - 1 or usage.get('totalTokens') is not None and (type(usage['totalTokens']) is not int or usage['totalTokens'] != total):
                raise ValueError()
            credit = usage.get('credit')
            credit = credit if type(credit) in (float, int) and math.isfinite(credit) and credit > 0 else None
            if not total and credit is None:
                continue
            row = dict(type='ide-request-usage', id='ide-request:' + request['id'], sessionId='ide:' + session,
                       requestId=request['id'], timestamp=at // 1000, model='unknown',
                       tokens=dict(input=uncached, output=output, cacheRead=cache_read, cacheWrite=cache_write),
                       inheritance=dict(origin='codebuddy-ide:%s:%d' % (request['id'], at), inherited=False))
            if credit is not None:
                row['nativeCredits'] = dict(credits=credit)
            result.append(row)
        except (ValueError, TypeError, OverflowError):
            invalid += 1
    return result, invalid


def history(root, cursors, emit, issue):
    paths = files(root)
    read_bytes, scanned, failed = 0, 0, 0
    for path in paths:
        try:
            mark = signature(path)
            if cursors.get(str(path), {}).get('codebuddyIde') == mark:
                continue
            requests, amount = read(path)
            read_bytes += amount
            events, invalid = rows(path, requests)
            scanned += len(events)
            if invalid:
                issue(str(path), '部分 CodeBuddy IDE 请求缺少时间或有效用量，已保留其他计量并等待下次重试。', 'invalidRecord')
            for offset in range(0, len(events), 256):
                emit('batch', path=str(path), reset=True, events=events[offset:offset+256], cursor=None)
            emit('batch', path=str(path), reset=True, events=[], cursor=dict(codebuddyIde=mark) if not invalid and signature(path) == mark else None)
        except (OSError, ValueError):
            failed += 1
            issue(str(path), 'CodeBuddy IDE 历史文件不可读或正在写入')
    return dict(files=len(paths), readBytes=read_bytes, scannedRecords=scanned, failedFiles=failed)


def sessions(root, cursors, now):
    recent, error = [], None
    for path in files(root):
        try:
            stamp = path.stat().st_mtime_ns
            if now - stamp / 10**9 < 86400 and not (path.parent / '.hidden').exists():
                recent.append((stamp, path))
        except OSError:
            error = 'CodeBuddy IDE 历史文件不可读或正在写入'
    recent.sort(reverse=True)
    result = []
    for stamp, path in recent[:256]:
        old = cursors.get(str(path), {})
        base = dict(key=str(path), modified=stamp // 10**9, reset=True, database=True, events=[])
        try:
            mark = signature(path)
            base['signature'] = mark
            if old.get('signature') == mark:
                base['reset'] = False
                result.append(base)
                continue
            requests, _ = read(path)
            if not requests:
                continue
            request, session = requests[-1], path.parent.name
            if not valid_id(request['id']) or not valid_id(session):
                continue
            state = request.get('state')
            kind = dict(running='start', complete='complete', aborted='failed', canceled='interrupted').get(state, 'unknown') if isinstance(state, str) else 'unknown'
            event = dict(session='ide:' + session, turn=request['id'], at=stamp // 10**6)
            base['events'] = [dict(event, kind='start'), dict(event, kind=kind)]
            result.append(base)
        except (OSError, ValueError):
            error = 'CodeBuddy IDE 历史文件不可读或正在写入'
            if old.get('signature', '').startswith('codebuddy-ide-v1:'):
                base.update(signature=old['signature'], modified=old.get('modified', stamp // 10**9), reset=False)
                result.append(base)
    return dict(files=result, error=error)
