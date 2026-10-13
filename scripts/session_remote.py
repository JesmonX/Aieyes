"""Aieyes lifecycle observer/SSH collector. Standard library only; never emits conversation content."""
import hashlib
import json
import os
import pathlib
import sys
import time

MARKER = '--aieyes-session-hook'
CLAUDE = ['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PermissionRequest', 'PostToolUse', 'PostToolUseFailure', 'Notification', 'SubagentStart', 'SubagentStop', 'Stop', 'StopFailure', 'SessionEnd', 'Elicitation', 'ElicitationResult']
AGY = ['SessionStart', 'PreInvocation', 'PostToolUse', 'PostInvocation', 'Stop']
EVENT_KEYS = {'kind', 'session', 'turn', 'call', 'tool', 'at', 'pid', 'processStart', 'fullyIdle', 'failed', 'interrupted', 'notification'}

def text(value):
    return value[:256] if isinstance(value, str) else ''

def target(source):
    path = pathlib.Path(os.path.expanduser(source.get('path') or ('~/.gemini/antigravity-cli' if source['provider'] == 'antigravity' else '~/.claude')))
    if source['provider'] == 'antigravity' and path.name in ('antigravity-cli', 'antigravity'):
        home = path.parent / 'config'
        return home, home / 'hooks.json', False
    return path, path / 'settings.json', True

def atomic(path, data):
    import tempfile
    fd, name = tempfile.mkstemp(prefix='.aieyes-', dir=str(path.parent))
    try:
        with os.fdopen(fd, 'w', encoding='utf8') as f:
            f.write(data); f.flush(); os.fsync(f.fileno())
        if path.exists(): os.chmod(name, path.stat().st_mode & 0o777)
        os.replace(name, path)
    finally:
        if os.path.exists(name): os.unlink(name)

def owned(value):
    return isinstance(value, dict) and MARKER in value.get('command', '')

def manage(source, action):
    import fcntl
    import shlex
    home, config, nested = target(source)
    result = dict(sourceId=source['id'], provider=source['provider'], machineId=source.get('hostId') or 'local', configPath=str(config), state='missing', error=None)
    try:
        root = home / '.aieyes'; disabled = root / 'session-hooks-disabled'
        if disabled.exists() and action not in ('repair', 'remove'):
            result['state'] = 'disabled'; return result
        old = json.loads(config.read_text()) if config.exists() else {}
        if not isinstance(old, dict): raise ValueError('Hook config must be an object')
        if old.get('disableAllHooks') is True and action != 'remove':
            result['state'] = 'disabled'; return result
        if action == 'status':
            hooks = old.get('hooks', {}) if nested else old
            events = hooks.get('aieyes', {}) if source['provider']=='antigravity' else hooks
            if events.get('enabled') is False: result['state']='disabled'; return result
            for event in (CLAUDE if source['provider']=='claude' else AGY):
                own = [r for r in events.get(event,[]) if owned(r) or (isinstance(r,dict) and any(owned(h) for h in r.get('hooks',[])))]
                if not own: result['state']='missing'; return result
                if any(r.get('enabled') is False or any(h.get('enabled') is False for h in r.get('hooks',[]) if owned(h)) for r in own): result['state']='disabled'; return result
            result['state']='installed' if (root/'observe.py').exists() else 'missing'
            return result
        source_path = pathlib.Path(os.path.expanduser(source.get('path') or '~/.gemini/antigravity-cli'))
        if not home.exists() and not source_path.is_dir(): raise ValueError('Agent data directory does not exist')
        root.mkdir(parents=True, exist_ok=True, mode=0o700)
        with open(root / 'session-hooks.lock', 'w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            if config.is_symlink(): raise ValueError('Symlinked hook configuration was left unchanged')
            old = json.loads(config.read_text()) if config.exists() else {}
            data = json.loads(json.dumps(old))
            hooks = data.setdefault('hooks', {}) if nested else data
            if not isinstance(hooks, dict): raise ValueError('Invalid hooks object')
            if source['provider'] == 'antigravity':
                events = hooks.setdefault('aieyes', {})
                if events.get('enabled') is False and action not in ('repair', 'remove'):
                    result['state'] = 'disabled'; return result
                if action == 'repair': events.pop('enabled', None)
            else: events = hooks
            for event in (CLAUDE if source['provider'] == 'claude' else AGY):
                rows = events.get(event, [])
                if not isinstance(rows, list): raise ValueError('Invalid hook event array')
                kept = []
                for row in rows:
                    if owned(row):
                        if row.get('enabled') is False and action not in ('repair', 'remove'):
                            result['state'] = 'disabled'; return result
                        continue
                    if isinstance(row, dict) and isinstance(row.get('hooks'), list):
                        own = [h for h in row['hooks'] if owned(h)]
                        if own and (row.get('enabled') is False or any(h.get('enabled') is False for h in own)) and action not in ('repair', 'remove'):
                            result['state'] = 'disabled'; return result
                        row['hooks'] = [h for h in row['hooks'] if not owned(h)]
                        if own and not row['hooks']: continue
                    kept.append(row)
                if action != 'remove':
                    command = ' '.join(shlex.quote(s) for s in [sys.executable, str(root / 'observe.py'), MARKER, source['provider'], event, str(home)])
                    handler = dict(type='command', command=command, timeout=2)
                    kept.append(handler if source['provider'] == 'antigravity' and event != 'PostToolUse' else dict(matcher='*', hooks=[handler]))
                if kept or event in events: events[event] = kept
            if action != 'remove':
                script = globals().get('SCRIPT')
                if not script: raise ValueError('Missing observer source')
                helper = root / 'observe.py'
                if not helper.exists() or helper.read_text() != script: atomic(helper, script)
                (root / 'session-events').mkdir(exist_ok=True, mode=0o700)
            if data != old: atomic(config, json.dumps(data, ensure_ascii=False, indent=2) + '\n')
            if action == 'remove': disabled.write_text('disabled\n')
            elif action == 'repair' and disabled.exists(): disabled.unlink()
            result['state'] = 'disabled' if action == 'remove' else 'installed'
    except Exception as error:
        result.update(state='error', error=str(error))
    return result

def process_info(pid):
    try:
        raw = pathlib.Path('/proc/%d/stat' % pid).read_text()
        end = raw.rfind(')'); fields = raw[end+2:].split()
        name = raw[raw.index('(')+1:end]
        boot = next(int(line.split()[1]) for line in pathlib.Path('/proc/stat').read_text().splitlines() if line.startswith('btime '))
        return name, int(fields[1]), boot + int(fields[19]) // os.sysconf('SC_CLK_TCK')
    except (OSError, ValueError, StopIteration): return None

def ancestor(provider):
    pid = os.getppid()
    for _ in range(8):
        p = process_info(pid)
        if not p: break
        if (provider == 'claude' and p[0].startswith('claude')) or (provider == 'antigravity' and p[0] == 'agy'): return pid, p[2]
        pid = p[1]
    return 0, 0

def observe(provider, kind, home):
    import threading
    import queue
    import fcntl
    at = int(time.time() * 1000)
    inbox = queue.Queue(1)
    def read():
        try: inbox.put(sys.stdin.buffer.read(1024 * 1024 + 1))
        except Exception: pass
    threading.Thread(target=read, daemon=True).start()
    try:
        data = inbox.get(timeout=.5)
        if len(data) > 1024*1024: return
        v = json.loads(data)
        session = text(v.get('session_id') or v.get('conversationId'))
        if not session: return
        row = dict(kind=kind, at=at, session=session, turn=text(v.get('turn_id') or v.get('turnId')), call=text(v.get('tool_use_id') or v.get('agent_id') or v.get('elicitation_id')), tool=text((v.get('toolCall') or {}).get('name') if provider == 'antigravity' else v.get('tool_name')), notification=text(v.get('notification_type')))
        if isinstance(v.get('fullyIdle'), bool): row['fullyIdle'] = v['fullyIdle']
        row['failed'] = kind == 'StopFailure' or (kind == 'Stop' and (v.get('terminationReason') == 'error' or bool(v.get('error'))))
        row['interrupted'] = any(s in str(v.get('terminationReason', '')) for s in ['cancel', 'interrupt'])
        row['pid'], row['processStart'] = ancestor(provider)
        root = pathlib.Path(home) / '.aieyes'
        if (root / 'session-hooks-disabled').exists(): return
        path = root / 'session-events' / (hashlib.sha256(session.encode()).hexdigest() + '.jsonl')
        with open(path.with_suffix('.lock'), 'w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            if path.exists() and path.stat().st_size > 4*1024*1024: path.unlink()
            fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
            try: os.write(fd, (json.dumps(row) + '\n').encode())
            finally: os.close(fd)
    except Exception: pass

def milliseconds(value):
    if not isinstance(value, str): return 0
    from datetime import datetime
    try: return max(0, int(datetime.fromisoformat(value.replace('Z', '+00:00')).timestamp() * 1000))
    except ValueError: return 0

def metadata(v, provider, hook):
    if hook: return [{k: value for k, value in v.items() if k in EVENT_KEYS}]
    at = milliseconds(v.get('timestamp'))
    if provider == 'codex':
        p = v.get('payload') or {}; key = (v.get('type'), p.get('type'))
        kinds = {('event_msg','task_started'):'start',('event_msg','task_complete'):'complete',('event_msg','turn_aborted'):'interrupted',('event_msg','error'):'failed',('response_item','reasoning'):'thinking',('response_item','function_call'):'tool',('response_item','custom_tool_call'):'tool',('response_item','function_call_output'):'output',('response_item','custom_tool_call_output'):'output'}
        kind = 'identity' if v.get('type') == 'session_meta' else kinds.get(key)
        return [dict(kind=kind,at=at,session=text(p.get('id')) if kind=='identity' else '',turn=text(p.get('turn_id')),call=text(p.get('call_id')),tool=text(p.get('name')))] if kind else []
    rows = []; message = v.get('message') or {}; content = message.get('content')
    content = content if isinstance(content,list) else []
    base = dict(at=at,session=text(v.get('sessionId')))
    if v.get('type') == 'user':
        results = [p for p in content if isinstance(p,dict) and p.get('type')=='tool_result']
        if not results and not v.get('isMeta'): rows.append(dict(base,kind='start',turn=text(v.get('uuid'))))
        for part in results: rows.append(dict(base,kind='output',call=text(part.get('tool_use_id'))))
    elif v.get('type') == 'assistant':
        for part in content:
            if not isinstance(part,dict): continue
            kind = {'thinking':'thinking','tool_use':'tool'}.get(part.get('type'))
            if kind: rows.append(dict(base,kind=kind,tool=text(part.get('name')),call=text(part.get('id'))))
        if message.get('stop_reason') == 'end_turn': rows.append(dict(base,kind='complete'))
    return rows

def read_file(path, provider, hook, cursor):
    stat = path.stat(); size = stat.st_size; modified = int(stat.st_mtime)
    identity = str(stat.st_ino)
    reset = cursor.get('identity') != identity or size < cursor.get('offset',0) or (size == cursor.get('size') and str(stat.st_mtime_ns) != cursor.get('signature'))
    offset = max(0,size-262144) if reset else cursor.get('offset',0)
    with path.open('rb') as f:
        f.seek(offset); data = f.read(524288)
        rows = []
        if reset and offset:
            prefix, sep, data = data.partition(b'\n'); offset += len(prefix) + len(sep)
        if reset:
            f.seek(0)
            try: rows.extend(e for e in metadata(json.loads(f.readline(65536)), provider, hook) if e.get('kind') == 'identity')
            except (ValueError, TypeError): pass
        complete = data.rfind(b'\n') + 1
        for line in data[:complete].splitlines():
            try: rows.extend(metadata(json.loads(line),provider,hook))
            except (ValueError,TypeError,AttributeError): pass
        if not complete and len(data) == 524288: complete=len(data)
    pid = cursor.get('pid',0); start = cursor.get('processStart',0)
    for row in rows:
        if row.get('pid'): pid=row['pid']; start=row.get('processStart',0)
    alive = None
    if pid and pathlib.Path('/proc').is_dir():
        p=process_info(pid)
        alive = (not start or p[2]==start) if p else (None if pathlib.Path('/proc/%d' % pid).exists() else False)
    return dict(key=str(path), identity=identity, signature=str(stat.st_mtime_ns), size=size, modified=modified, offset=offset+complete, reset=reset, events=rows, hook=hook,alive=alive)

def read_db(path):
    import sqlite3
    stat=path.stat(); wal=pathlib.Path(str(path)+'-wal')
    modified=int(max(stat.st_mtime, wal.stat().st_mtime if wal.exists() else 0))
    uri=path.resolve().as_uri()+'?mode=ro'
    with sqlite3.connect(uri,uri=True,timeout=.1) as db:
        db.execute('PRAGMA query_only=ON')
        session=db.execute('SELECT cascade_id FROM trajectory_meta LIMIT 1').fetchone()[0]
        steps=db.execute('SELECT idx,step_type,status FROM steps ORDER BY idx DESC LIMIT 2048').fetchall()[::-1]
    return dict(key=str(path),identity=str(stat.st_ino),size=0,offset=0,modified=modified,reset=True,events=[],hook=False,session=session,steps=steps)

def collect(source,cursors):
    path=pathlib.Path(os.path.expanduser(source.get('path') or '~/.gemini/antigravity-cli'))
    provider=source['provider']; result=dict(sourceId=source['id'],files=[],error=None)
    try:
        if not path.exists(): raise FileNotFoundError('Agent data directory does not exist')
        roots=[(path / ('sessions' if provider=='codex' else 'projects' if provider=='claude' else 'conversations'),False)]
        if not roots[0][0].is_dir(): roots=[(path,False)]
        if provider in ('claude','antigravity'): roots.append((target(source)[0]/'.aieyes/session-events',True))
        files=[]
        for root,hook in roots:
            if not root.exists(): continue
            candidates=[root] if root.is_file() else root.rglob('*.jsonl' if hook or provider!='antigravity' else '*.db')
            for f in candidates:
                if f.is_symlink() or (not hook and '.aieyes' in f.parts): continue
                stamp=max(f.stat().st_mtime, pathlib.Path(str(f)+'-wal').stat().st_mtime if pathlib.Path(str(f)+'-wal').exists() else 0)
                if time.time()-stamp < 86400: files.append((stamp,f,hook))
        if len(files)>256: result['error']='Too many recent sessions; showing the most recent 256'
        for _,f,hook in sorted(files,reverse=True)[:256]:
            try: result['files'].append(read_db(f) if not hook and provider=='antigravity' else read_file(f,provider,hook,cursors.get(str(f),{})))
            except Exception as error: result['error']=str(error)
    except Exception as error: result['error']=str(error)
    return result

def main(request):
    action=request.get('action','collect'); sources=request.get('sources',[])
    hooks=[]; seen=set()
    for s in sources:
        if s['provider'] not in ('claude','antigravity'): continue
        key=(s['provider'],str(target(s)[0]))
        if key in seen: continue
        seen.add(key); hooks.append(manage(s,'ensure' if action=='collect' else action))
    polls=[collect(s,request.get('cursors',{})) for s in sources] if action=='collect' else []
    return dict(hooks=hooks,polls=polls,now=int(time.time()))

if __name__ == '__main__':
    if MARKER in sys.argv:
        index=sys.argv.index(MARKER); args=sys.argv[index+1:]
        provider,kind=(args+['',''])[:2]
        try:
            if len(args)==3: observe(*args)
        finally:
            if provider=='antigravity': print('{"decision":"allow"}' if kind=='Stop' else '{}')
    else:
        request=globals().get('REQUEST')
        if request is None: request=json.loads(sys.argv[1])
        print(json.dumps(main(request),ensure_ascii=False))
