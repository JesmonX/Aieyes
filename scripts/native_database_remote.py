"""Read-only, bounded SQLite projections for OpenCode and ZCode.

The first read enumerates metadata. Later reads follow rowids and a bounded
set of in-flight rows. Cursors and pending queues live in Aieyes, not in the
client database or a remote sidecar. No transcript fields leave SQLite.
"""
import contextlib
import hashlib
import json
import pathlib
import sqlite3

LIMIT = 256
VERSION = 1


def fingerprint(path):
    def stamp(file):
        try:
            s = file.stat()
            return (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns)
        except FileNotFoundError:
            return None
    path = pathlib.Path(path)
    return 'native-db:' + hashlib.sha256(repr((stamp(path), stamp(pathlib.Path(str(path) + '-wal')))).encode()).hexdigest()


def connect(path):
    db = sqlite3.connect(pathlib.Path(path).resolve().as_uri() + '?mode=ro', uri=True, timeout=.5)
    db.execute('PRAGMA query_only=ON')
    db.execute('BEGIN')
    return contextlib.closing(db)


def schema(db, provider, history):
    if provider == 'zcode':
        if not history:
            return 'turn_usage', "rowid,session_id,turn_id,started_at,COALESCE(NULLIF(completed_at,0),started_at),json_object('status',status)"
        columns = {row[1] for row in db.execute('PRAGMA table_info(model_usage)')}
        turn = 'turn_id' if 'turn_id' in columns else 'NULL'
        return 'model_usage', "rowid,id,json_object('id',id,'session',session_id,'turn',%s,'status',status,'model',model_id,'completed',completed_at,'input',input_tokens,'output',output_tokens,'read',cache_read_input_tokens,'write',cache_creation_input_tokens,'reasoning',reasoning_tokens,'providerTotal',provider_total_tokens,'computedTotal',computed_total_tokens)" % turn
    tables = {row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name IN ('message','session_message')")}
    if not tables:
        raise ValueError('Unsupported OpenCode database schema')
    v2 = 'session_message' in tables and ('message' not in tables or db.execute('SELECT 1 FROM session_message LIMIT 1').fetchone() is not None)
    table = 'session_message' if v2 else 'message'
    role = 'type' if v2 else "json_extract(data,'$.role')"
    if not history:
        return table, "rowid,session_id,id,time_created,time_updated,CASE WHEN json_valid(data) THEN json_object('role',%s,'completed',json_extract(data,'$.time.completed'),'finish',json_extract(data,'$.finish'),'error',json_extract(data,'$.error.name')) ELSE '{}' END" % role
    model, provider_key = ('$.model.id', '$.model.providerID') if v2 else ('$.modelID', '$.providerID')
    # Whitelist every token field: unknown nested fields may contain payloads.
    tokens = "CASE WHEN json_type(data,'$.tokens')='object' THEN json_object('input',json_extract(data,'$.tokens.input'),'output',json_extract(data,'$.tokens.output'),'reasoning',json_extract(data,'$.tokens.reasoning'),'cache',json_object('read',json_extract(data,'$.tokens.cache.read'),'write',json_extract(data,'$.tokens.cache.write'))) ELSE NULL END"
    return table, "rowid,id,CASE WHEN json_valid(data) THEN json_object('session',session_id,'role',%s,'tokens',%s,'completed',json_extract(data,'$.time.completed'),'model',json_extract(data,'%s'),'provider',json_extract(data,'%s')) ELSE '{}' END" % (role, tokens, model, provider_key)


def identity(path):
    stat = pathlib.Path(path).stat()
    return '%s:%s' % (stat.st_dev, stat.st_ino)


def history(path, provider, old, emit):
    before = fingerprint(path)
    if old.get('nativeVersion') == VERSION and old.get('database') == before:
        return dict(readBytes=0, scannedRecords=0)
    scanned = amount = 0
    with connect(path) as db:
        table, columns = schema(db, provider, True)
        current_identity = identity(path)
        offset = old.get('offset', 0)
        anchor = db.execute('SELECT id FROM %s WHERE rowid=?' % table, (offset,)).fetchone()
        reset = (old.get('nativeVersion') != VERSION or old.get('identity') != current_identity or old.get('table') != table
                 or (offset > 0 and (anchor is None or hashlib.sha256(str(anchor[0]).encode()).hexdigest() != old.get('anchor'))))
        checkpoint = dict(nativeVersion=VERSION, identity=current_identity, table=table, offset=0 if reset else offset,
                          anchor='' if reset else old.get('anchor', ''), database=None,
                          pendingAfter=0 if reset else old.get('pendingAfter', 0), pendingFingerprint='' if reset else old.get('pendingFingerprint', ''))
        pending = [] if reset else old.get('pending', [])
        if len(pending) > LIMIT or any(type(rowid) is not int or rowid <= 0 for rowid in pending):
            raise ValueError('Invalid native database pending cursor')
        pending_more = bool(not reset and old.get('pendingMore'))
        sweep = checkpoint['pendingFingerprint'] or before
        query = 'SELECT %s FROM %s' % (columns, table)
        first = True

        def batch(rows, gone=(), advance=False):
            nonlocal scanned, amount, first
            part = []
            used = 0

            def flush(gone_rows=()):
                nonlocal first
                emit('batch', path=str(path), reset=False, nativeReset=first and reset, nativeRows=list(part), gone=list(gone_rows), events=[], cursor=dict(checkpoint))
                first = False

            for rowid, request, raw in rows:
                scanned += 1
                anchor = hashlib.sha256(str(request).encode()).hexdigest()
                # Untrusted native strings must not exceed the transport bound.
                if len(raw.encode()) > 65536 or not isinstance(request, str) or len(request) > 4096:
                    data = dict(invalidMetadata=True)
                    request = request[:4096] if isinstance(request, str) else ''
                else:
                    data = json.loads(raw)
                row = dict(rowid=rowid, id=request, data=data)
                size = len(json.dumps(row, separators=(',', ':')).encode())
                if used + size > 768 * 1024:
                    flush()
                    part, used = [], 0
                part.append(row)
                used += size
                amount += size
                if advance:
                    checkpoint.update(offset=rowid, anchor=anchor)
            flush(gone)

        if pending:
            rows = db.execute(query + ' WHERE rowid IN (%s) ORDER BY rowid' % ','.join('?' for _ in pending), pending).fetchall()
            present = {row[0] for row in rows}
            batch(rows, [rowid for rowid in pending if rowid not in present])
            checkpoint['pendingAfter'] = pending[-1]
            checkpoint['pendingFingerprint'] = sweep
        while True:
            rows = db.execute(query + ' WHERE rowid>? ORDER BY rowid LIMIT 256', (checkpoint['offset'],)).fetchall()
            if not rows:
                break
            batch(rows, advance=True)
        stable = before == fingerprint(path)
        checkpoint['database'] = before if stable and not pending_more and sweep == before else None
        if not pending_more:
            checkpoint.update(pendingAfter=0, pendingFingerprint='')
        batch([])
    return dict(readBytes=amount, scannedRecords=scanned)


def _trim(rows, now):
    current = (row for row in rows.values() if now - row[4] // 1000 < 86400)
    return {row[1]: row for row in sorted(current, key=lambda row: row[4], reverse=True)[:LIMIT]}


def _state_row(row):
    row = list(row)
    if any(not isinstance(row[i], str) or len(row[i]) > 256 for i in (1, 2)) or len(row[5]) > 16384:
        raise ValueError('Native session metadata exceeds its bounds')
    row[5] = json.loads(row[5])
    return row


def _phase(row, db, parts):
    data = row[5]
    if 'status' in data:
        return {'completed': 'complete', 'error': 'failed', 'cancelled': 'interrupted', 'running': 'start'}.get(data['status'], 'unknown')
    error, finish = data.get('error'), data.get('finish')
    kind = ('interrupted' if error in ('MessageAbortedError', 'AbortedError') else 'failed' if error
            else 'start' if data.get('role') == 'user'
            else 'complete' if data.get('completed') is not None and finish in ('stop', 'end_turn')
            else 'failed' if data.get('completed') is not None and finish == 'length' else 'thinking')
    if parts and kind not in ('complete', 'failed', 'interrupted'):
        tools = db.execute("SELECT json_extract(data,'$.tool') FROM part WHERE message_id=? AND json_valid(data) AND json_extract(data,'$.type')='tool' AND json_extract(data,'$.state.status') IN ('pending','running') LIMIT 64", (row[2],)).fetchall()
        if tools:
            kind = 'PermissionRequest' if ('question',) in tools else 'tool'
    return kind


def cached_sessions(path, cursor, now):
    state = dict(cursor.get('continuation') or {})
    rows = _trim({r[1]: r for r in state.get('rows', [])}, now)
    state['rows'] = list(rows.values())
    files = [dict(key=str(path) + '#' + row[1], modified=row[4] // 1000, database=True, reset=False, events=[]) for row in rows.values()]
    files.append(dict(key=str(path), modified=cursor.get('modified', 0), signature=cursor.get('signature', ''), database=True, reset=True, events=[], continuation=state))
    return files


def sessions(path, provider, cursor, now):
    before = fingerprint(path)
    old = cursor.get('continuation') or {}
    if old.get('nativeVersion') == VERSION and cursor.get('signature') == before:
        return cached_sessions(path, cursor, now), 0
    scanned = 0
    with connect(path) as db:
        table, columns = schema(db, provider, False)
        current_identity = identity(path)
        offset = old.get('offset', 0)
        query = 'SELECT %s FROM %s' % (columns, table)
        anchor = db.execute(query + ' WHERE rowid=?', (offset,)).fetchone()
        if (old.get('nativeVersion') != VERSION or old.get('identity') != current_identity or old.get('table') != table
                or (offset > 0 and (anchor is None or list(anchor[1:3]) != old.get('anchor')))):
            old = dict(offset=0, anchor=[], rows=[])
        state = dict(old, nativeVersion=VERSION, identity=current_identity, table=table)
        rows = {}
        ids = [r[0] for r in old.get('rows', [])][:LIMIT]
        if ids:
            for native in db.execute(query + ' WHERE rowid IN (%s)' % ','.join('?' for _ in ids), ids):
                row = _state_row(native)
                rows[row[1]] = row
                scanned += 1
        while True:
            added = db.execute(query + ' WHERE rowid>? ORDER BY rowid LIMIT 256', (state['offset'],)).fetchall()
            if not added:
                break
            for native in added:
                row = _state_row(native)
                scanned += 1
                state.update(offset=row[0], anchor=row[1:3])
                prior = rows.get(row[1])
                if now - row[4] // 1000 < 86400 and (prior is None or (row[3], row[2]) >= (prior[3], prior[2])):
                    rows[row[1]] = row
            rows = _trim(rows, now)
        rows = _trim(rows, now)
        parts = provider == 'opencode' and table == 'message' and db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='part'").fetchone() is not None
        files = []
        for row in rows.values():
            base = dict(session=row[1], turn=row[2], at=row[4])
            files.append(dict(key=str(path) + '#' + row[1], modified=row[4] // 1000, reset=True, database=True,
                              events=[dict(base, kind='start'), dict(base, kind=_phase(row, db, parts))]))
        state['rows'] = list(rows.values())
    signature = before if before == fingerprint(path) else ''
    files.append(dict(key=str(path), modified=int(pathlib.Path(path).stat().st_mtime), signature=signature, database=True, reset=True, events=[], continuation=state))
    return files, scanned
