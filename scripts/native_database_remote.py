"""Read-only, bounded SQLite projections for OpenCode, ZCode and WorkBuddy.

The first read enumerates metadata. Later reads follow rowids and a bounded
set of in-flight rows. Cursors and pending queues live in Aieyes, not in the
client database or a remote sidecar. No transcript fields leave SQLite.
"""
import contextlib
import hashlib
import json
import math
import pathlib
import sqlite3

LIMIT = 256
VERSION = 2


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


def schema(db, provider, history, selected=None):
    if provider == 'zcode':
        if not history:
            return 'turn_usage', "rowid,session_id,turn_id,started_at,COALESCE(NULLIF(completed_at,0),started_at),json_object('status',status)"
        columns = {row[1] for row in db.execute('PRAGMA table_info(model_usage)')}
        turn = 'turn_id' if 'turn_id' in columns else 'NULL'
        return 'model_usage', "rowid,id,json_object('id',id,'session',session_id,'turn',%s,'status',status,'model',model_id,'completed',completed_at,'input',input_tokens,'output',output_tokens,'read',cache_read_input_tokens,'write',cache_creation_input_tokens,'reasoning',reasoning_tokens,'providerTotal',provider_total_tokens,'computedTotal',computed_total_tokens)" % turn
    tables = {row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name IN ('message','session_message')")}
    if not tables:
        raise ValueError('Unsupported OpenCode database schema')
    v2 = selected == 'session_message' if selected else 'session_message' in tables and ('message' not in tables or db.execute('SELECT 1 FROM session_message LIMIT 1').fetchone() is not None)
    table = 'session_message' if v2 else 'message'
    role = 'type' if v2 else "json_extract(data,'$.role')"
    if not history:
        tool=("CASE WHEN EXISTS(SELECT 1 FROM json_each(data,'$.content') WHERE json_extract(value,'$.type')='tool' AND json_extract(value,'$.state.status') IN ('pending','streaming','running') AND json_extract(value,'$.name') IN ('question','ask_question')) THEN 'question' WHEN EXISTS(SELECT 1 FROM json_each(data,'$.content') WHERE json_extract(value,'$.type')='tool' AND json_extract(value,'$.state.status') IN ('pending','streaming','running')) THEN 'tool' END") if v2 else 'NULL'
        return table, "rowid,session_id,id,COALESCE(CASE WHEN json_valid(data) THEN json_extract(data,'$.time.created') END,time_created),COALESCE(CASE WHEN json_valid(data) THEN COALESCE(json_extract(data,'$.time.completed'),json_extract(data,'$.time.created')) END,time_created),CASE WHEN json_valid(data) THEN json_object('role',%s,'completed',json_extract(data,'$.time.completed'),'finish',json_extract(data,'$.finish'),'error',COALESCE(json_extract(data,'$.error.name'),json_extract(data,'$.error.type'),json_extract(data,'$.error._tag')),'outcome',json_extract(data,'$.outcome'),'tool',%s) ELSE '{}' END" % (role,tool)
    model, provider_key = ('$.model.id', '$.model.providerID') if v2 else ('$.modelID', '$.providerID')
    # Whitelist every token field: unknown nested fields may contain payloads.
    tokens = "CASE WHEN json_type(data,'$.tokens')='object' THEN json_object('input',json_extract(data,'$.tokens.input'),'output',json_extract(data,'$.tokens.output'),'reasoning',json_extract(data,'$.tokens.reasoning'),'cache',json_object('read',json_extract(data,'$.tokens.cache.read'),'write',json_extract(data,'$.tokens.cache.write'))) ELSE NULL END"
    completed="CASE WHEN type='compaction' AND json_extract(data,'$.status') IN ('completed','failed') THEN COALESCE(json_extract(data,'$.time.completed'),json_extract(data,'$.time.created')) ELSE json_extract(data,'$.time.completed') END" if v2 else "json_extract(data,'$.time.completed')"
    inherited='0'
    if v2 and {'fork_session_id','fork_boundary'}.issubset({row[1] for row in db.execute('PRAGMA table_info(session_v2)')}):
        inherited="EXISTS(SELECT 1 FROM session_v2 f JOIN session_message b ON b.session_id=f.fork_session_id WHERE f.id=session_message.session_id AND CASE WHEN json_valid(f.fork_boundary) THEN b.id=json_extract(f.fork_boundary,'$.messageID') AND ((json_extract(f.fork_boundary,'$.type')='before' AND session_message.seq<b.seq) OR (json_extract(f.fork_boundary,'$.type')='through' AND session_message.seq<=b.seq)) ELSE 0 END)"
    return table, "rowid,id,CASE WHEN json_valid(data) THEN json_object('session',session_id,'role',%s,'status',json_extract(data,'$.status'),'tokens',%s,'completed',%s,'model',json_extract(data,'%s'),'provider',json_extract(data,'%s'),'inherited',%s) ELSE '{}' END" % (role,tokens,completed,model,provider_key,inherited)


def tables(db,provider):
    if provider=='zcode':return ['model_usage']
    found={r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table' AND name IN ('message','session_message')")}
    if not found:raise ValueError('Unsupported OpenCode database schema')
    return [name for name in ('message','session_message') if name in found]


def history_all(path,provider,cursors,emit):
    if provider not in ('opencode','kilo'):return history(path,provider,cursors.get(str(path),{}),emit)
    with connect(path) as db:names=tables(db,provider)
    total=dict(readBytes=0,scannedRecords=0)
    for name in names:
        stats=history(path,provider,cursors.get(str(path)+'#'+name,{}),emit,selected=name)
        for k in total:total[k]+=stats[k]
    return total


def kilo_headless(db, session):
    """Use the lead session: subagents can deny questions under an interactive lead."""
    seen = set()
    for _ in range(32):
        if session in seen:
            return False
        seen.add(session)
        try:
            found = db.execute('SELECT parent_id,permission FROM session WHERE id=?', (session,)).fetchone()
            if not found:
                return False
            if found[0]:
                session = found[0]
                continue
            rules = json.loads(found[1]) if found[1] else []
        except (sqlite3.Error, ValueError, TypeError):
            return False
        return isinstance(rules, list) and any(isinstance(r, dict) and r.get('permission') == 'question' and r.get('pattern') == '*' and r.get('action') == 'deny' for r in rules)
    return False


def identity(path):
    stat = pathlib.Path(path).stat()
    return '%s:%s' % (stat.st_dev, stat.st_ino)


def workbuddy_credit_rows(session, stamp, raw):
    """The desktop's request-id -> Credits map contains no per-request model."""
    if not isinstance(session, str) or not session or len(session) > 256 or type(stamp) is not int:
        raise ValueError('Invalid WorkBuddy usage identity')
    if raw is None:
        return []
    if not isinstance(raw, str) or len(raw.encode()) > 16 * 1024 * 1024:
        raise ValueError('WorkBuddy usage metadata exceeds its limit')
    try:
        values = json.loads(raw)
    except ValueError:
        return []
    if not isinstance(values, dict):
        return []
    rows = []
    for request, credits in sorted(values.items()):
        if not request or len(request) > 256 or type(credits) not in (int, float):
            continue
        try:
            credits = float(credits)
        except (ValueError, OverflowError):
            continue
        if not math.isfinite(credits) or credits < 0:
            continue
        origin = hashlib.sha256(json.dumps([session, request], ensure_ascii=False, separators=(',', ':')).encode()).hexdigest()
        rows.append(dict(id=request, sessionId=session, timestamp=max(0, stamp) // 1000, model='unknown',
                         tokens={}, nativeCredits=dict(credits=credits),
                         inheritance=dict(origin='workbuddy-credit:' + origin, inherited=False)))
    return rows


def workbuddy_history(path, old, emit):
    """Inclusive high-water mark: rewrites at the last timestamp are read again."""
    before = fingerprint(path)
    if old.get('workbuddyVersion') == 1 and old.get('database') == before:
        return dict(readBytes=0, scannedRecords=0)
    current_identity = identity(path)
    offset = old.get('offset', 0)
    if old.get('workbuddyVersion') != 1 or old.get('identity') != current_identity or type(offset) is not int or offset < 0:
        offset = 0
    checkpoint = dict(workbuddyVersion=1, identity=current_identity, offset=offset, database=None)
    scanned, amount = 0, 0
    with connect(path) as db:
        exists = db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='session_usage'").fetchone()
        after = (offset, '')
        while exists:
            page = db.execute('SELECT session_id,updated_at,credit_json FROM session_usage WHERE updated_at>? OR (updated_at=? AND session_id>?) ORDER BY updated_at,session_id LIMIT ?',
                              (after[0], after[0], after[1], LIMIT)).fetchall()
            if not page:
                break
            batch, size = [], 0
            for session, stamp, raw in page:
                for row in workbuddy_credit_rows(session, stamp, raw):
                    encoded = len(json.dumps(row).encode())
                    if size + encoded > 768 * 1024:
                        emit('batch', path=str(path), reset=True, events=batch, cursor=None)
                        batch, size = [], 0
                    batch.append(row)
                    size += encoded
                amount += len(raw.encode()) if isinstance(raw, str) else 0
                scanned += 1
                checkpoint['offset'] = max(checkpoint['offset'], stamp)
            after = (page[-1][1], page[-1][0])
            emit('batch', path=str(path), reset=True, events=batch, cursor=dict(checkpoint))
            if len(page) < LIMIT:
                break
    if before == fingerprint(path):
        checkpoint['database'] = before
    emit('batch', path=str(path), reset=True, events=[], cursor=checkpoint)
    return dict(readBytes=amount, scannedRecords=scanned)


def history(path, provider, old, emit, selected=None):
    before = fingerprint(path)
    if old.get('nativeVersion') == VERSION and old.get('database') == before:
        return dict(readBytes=0, scannedRecords=0)
    scanned = amount = 0
    with connect(path) as db:
        table, columns = schema(db, provider, True, selected)
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
                emit('batch', path=str(path)+('#'+table if selected else ''), reset=False, nativeReset=first and reset, nativeRows=list(part), gone=list(gone_rows), events=[], cursor=dict(checkpoint))
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
    if data.get('role')=='idle':return {'succeeded':'complete','failed':'failed','interrupted':'interrupted'}.get(data.get('outcome'),'unknown')
    error, finish = data.get('error'), data.get('finish')
    kind = ('interrupted' if error in ('MessageAbortedError', 'AbortedError') else 'failed' if error
            else 'start' if data.get('role') == 'user'
            else 'complete' if data.get('completed') is not None and finish in ('stop', 'end_turn')
            else 'failed' if data.get('completed') is not None and finish == 'length'
            else 'PermissionRequest' if data.get('tool')=='question' else 'tool' if data.get('tool')=='tool' else 'thinking')
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


WORKBUDDY_PHASES = {
    **dict.fromkeys(('completed', 'complete', 'done', 'pending', 'idle', 'await_input', 'awaitinput', 'waiting_input', 'waiting_user_input', 'connected', 'archived'), 'complete'),
    **dict.fromkeys(('waiting_permission', 'waiting_question'), 'PermissionRequest'),
    **dict.fromkeys(('failed', 'error'), 'failed'),
    **dict.fromkeys(('cancelled', 'canceled', 'aborted', 'interrupted', 'terminated'), 'interrupted'),
    **dict.fromkeys(('tool_start', 'tool_end', 'tool_executing'), 'tool'),
    **dict.fromkeys(('planning', 'preparing', 'connecting', 'working', 'running', 'in_progress', 'streaming', 'processing', 'handoff', 'summarizing', 'waiting_team_members', 'model_requesting', 'model_streaming', 'model_done'), 'start'),
}


def workbuddy_phase(status):
    """Same table as Rust `native_sessions::workbuddy_phase`."""
    key = str(status or '').strip().lower().replace(' ', '_').replace('-', '_')
    return WORKBUDDY_PHASES.get(key, 'unknown')


def workbuddy_sessions(path, now):
    """WorkBuddy keeps one `sessions` row per session, updated in place. Background
    automations, playground runs and deleted rows are not user sessions."""
    with connect(path) as db:
        names = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        runs = " AND id NOT IN (SELECT thread_id FROM automation_runs)" if 'automation_runs' in names else ''
        rows = db.execute("SELECT id,CAST(COALESCE(last_activity_at,updated_at) AS TEXT),status,COALESCE(last_activity_at,updated_at) AS at FROM sessions WHERE deleted_at IS NULL AND COALESCE(is_background_automation,0)=0 AND COALESCE(is_playground,0)=0 AND COALESCE(source_mode,'')<>'background'" + runs + " AND COALESCE(last_activity_at,updated_at)>? ORDER BY at DESC LIMIT ?", ((now - 86400) * 1000, LIMIT)).fetchall()
    return [[0, row[0], row[1], row[3], row[3], {'status': workbuddy_phase(row[2])}] for row in rows if isinstance(row[0], str) and len(row[0]) <= 256]


def sessions(path, provider, cursor, now):
    before=fingerprint(path);old=cursor.get('continuation') or {}
    if old.get('nativeVersion')==VERSION and cursor.get('signature')==before:
        return cached_sessions(path,cursor,now),0
    if provider=='workbuddy':
        rows=workbuddy_sessions(path,now);files=[]
        for row in rows:
            base=dict(session=row[1],turn=row[2],at=row[4],parentSession='')
            files.append(dict(key=str(path)+'#'+row[1],modified=row[4]//1000,reset=True,database=True,events=[dict(base,kind='start'),dict(base,kind=row[5]['status'])]))
        signature=before if before==fingerprint(path) else ''
        files.append(dict(key=str(path),modified=int(pathlib.Path(path).stat().st_mtime),signature=signature,database=True,reset=True,events=[],continuation=dict(nativeVersion=VERSION,identity=identity(path),rows=rows)))
        return files,len(rows)
    scanned=0
    with connect(path) as db:
        current_identity=identity(path)
        if old.get('nativeVersion')!=VERSION or old.get('identity')!=current_identity:old={}
        state=dict(nativeVersion=VERSION,identity=current_identity,tables={})
        selected=tables(db,provider) if provider in ('opencode','kilo') else ['turn_usage']
        latest={}
        for name in selected:
            table,columns=schema(db,provider,False,name)
            saved=(old.get('tables') or {}).get(table,{})
            query='SELECT %s FROM %s'%(columns,table)
            offset=saved.get('offset',0)
            anchor=db.execute(query+' WHERE rowid=?',(offset,)).fetchone()
            if offset>0 and (anchor is None or list(anchor[1:3])!=saved.get('anchor')):saved={}
            table_state=dict(offset=saved.get('offset',0),anchor=saved.get('anchor',[]))
            rows={};ids=[r[0] for r in saved.get('rows',[])][:LIMIT]
            if ids:
                for native in db.execute(query+' WHERE rowid IN (%s)'%','.join('?' for _ in ids),ids):
                    row=_state_row(native);rows[row[1]]=row;scanned+=1
            while True:
                added=db.execute(query+' WHERE rowid>? ORDER BY rowid LIMIT 256',(table_state['offset'],)).fetchall()
                if not added:break
                for native in added:
                    row=_state_row(native);scanned+=1
                    table_state.update(offset=row[0],anchor=row[1:3]);prior=rows.get(row[1])
                    if now-row[4]//1000<86400 and (prior is None or (row[3],row[2])>=(prior[3],prior[2])):rows[row[1]]=row
                rows=_trim(rows,now)
            rows=_trim(rows,now);table_state['rows']=list(rows.values());state['tables'][table]=table_state
            for row in rows.values():
                prior=latest.get(row[1])
                if prior is None or (row[3],row[2],table)>=(prior[0][3],prior[0][2],prior[1]):latest[row[1]]=(row,table)
        has_parts=provider in ('opencode','kilo') and db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='part'").fetchone() is not None
        parents=provider in ('opencode','kilo') and 'parent_id' in {r[1] for r in db.execute('PRAGMA table_info(session)')}
        files=[];visible=[]
        for row,table in sorted(latest.values(),key=lambda pair:pair[0][4],reverse=True)[:LIMIT]:
            if provider=='kilo' and kilo_headless(db,row[1]):continue
            parent=db.execute('SELECT parent_id FROM session WHERE id=?',(row[1],)).fetchone() if parents else None
            base=dict(session=row[1],turn=row[2],at=row[4],parentSession=parent[0] if parent and isinstance(parent[0],str) else '')
            files.append(dict(key=str(path)+'#'+row[1],modified=row[4]//1000,reset=True,database=True,events=[dict(base,kind='start'),dict(base,kind=_phase(row,db,has_parts and table=='message'))]))
            visible.append(row)
        state['rows']=visible
    signature=before if before==fingerprint(path) else ''
    files.append(dict(key=str(path),modified=int(pathlib.Path(path).stat().st_mtime),signature=signature,database=True,reset=True,events=[],continuation=state))
    return files,scanned
