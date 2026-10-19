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
ZCODE = ['SessionStart','UserPromptSubmit','PreToolUse','PermissionRequest','PostToolUse','PostToolUseFailure','Stop']
CURSOR = ['sessionStart','beforeSubmitPrompt','preToolUse','postToolUse','postToolUseFailure','afterAgentThought','stop','sessionEnd']
def hook_names(provider):
    return CLAUDE if provider=='claude' else CURSOR if provider=='cursor' else ZCODE if provider=='zcode' else AGY
EVENT_KEYS = {'kind', 'session', 'turn', 'record', 'parent', 'call', 'tool', 'at', 'pid', 'processStart', 'fullyIdle', 'failed', 'interrupted', 'notification'}

def text(value):
    return value[:256] if isinstance(value, str) else ''

def harness():
    if '_HARNESS' not in globals():
        source=globals().get('HARNESS_SCRIPT')
        if source is None:source=pathlib.Path(__file__).with_name('harness_remote.py').read_text(encoding='utf-8')
        scope={};exec(compile(source,'<aieyes-harness>','exec'),scope);globals()['_HARNESS']=scope
    return globals()['_HARNESS']

def native_database():
    if '_NATIVE_DATABASE' not in globals():
        source=globals().get('NATIVE_DATABASE_SCRIPT')
        if source is None:source=pathlib.Path(__file__).with_name('native_database_remote.py').read_text(encoding='utf-8')
        scope={};exec(compile(source,'<aieyes-native-database>','exec'),scope);globals()['_NATIVE_DATABASE']=scope
    return globals()['_NATIVE_DATABASE']

def target(source):
    path = pathlib.Path(os.path.expanduser(source.get('path') or ('~/.gemini/antigravity-cli' if source['provider'] == 'antigravity' else '~/.cursor' if source['provider']=='cursor' else '~/.zcode' if source['provider']=='zcode' else '~/.claude')))
    if source['provider']=='zcode':
        home=path if path.name=='cli' else path.parent.parent if path.suffix=='.sqlite' and path.parent.name=='db' else path/'cli'
        return home,home/'config.json',True
    if source['provider']=='cursor':return path,path/'hooks.json',True
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
    return isinstance(value, dict) and (MARKER in value.get('command', '') or any(isinstance(v,str) and MARKER in v for v in (value.get('args') if isinstance(value.get('args'),list) else [])))

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
        if source['provider']=='cursor' and (type(old.get('version',1)) is not int or old.get('version',1)!=1):raise ValueError('Unsupported Cursor Hooks version; original config preserved')
        if source['provider']=='cursor' and home.name.lower()!='.cursor':raise ValueError('Cursor Hooks require the actual user or project .cursor directory; CURSOR_CONFIG_DIR does not relocate user hooks')
        if old.get('disableAllHooks') is True and action != 'remove':
            result['state'] = 'disabled'; return result
        if action == 'status':
            hooks = old.get('hooks', {}) if nested else old
            if source['provider']=='zcode' and hooks.get('enabled') is not True:
                result['state']='disabled' if hooks.get('enabled') is False else 'missing';return result
            events = hooks.get('events', {}) if source['provider']=='zcode' else hooks.get('aieyes', {}) if source['provider']=='antigravity' else hooks
            if events.get('enabled') is False: result['state']='disabled'; return result
            for event in hook_names(source['provider']):
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
            if source['provider']=='cursor':
                if type(data.get('version',1)) is not int or data.get('version',1)!=1:raise ValueError('Unsupported Cursor Hooks version; original config preserved')
                if action!='remove':data['version']=1
            hooks = data.setdefault('hooks', {}) if nested else data
            if not isinstance(hooks, dict): raise ValueError('Invalid hooks object')
            if source['provider']=='zcode':
                if hooks.get('enabled') is False and action not in ('repair','remove'):
                    result['state']='disabled';return result
                if action!='remove':hooks['enabled']=True
                events=hooks.setdefault('events',{})
                if not isinstance(events,dict):raise ValueError('Invalid ZCode hook events')
            elif source['provider'] == 'antigravity':
                events = hooks.setdefault('aieyes', {})
                if events.get('enabled') is False and action not in ('repair', 'remove'):
                    result['state'] = 'disabled'; return result
                if action == 'repair': events.pop('enabled', None)
            else: events = hooks
            for event in hook_names(source['provider']):
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
                    if source['provider']=='zcode':
                        handler=dict(type='process',command=sys.executable,args=[str(root/'observe.py'),MARKER,'zcode',event,str(home)],timeoutMs=2000)
                        kept.append(dict(hooks=[handler]))
                    else:kept.append(handler if source['provider']=='cursor' or source['provider'] == 'antigravity' and event != 'PostToolUse' else dict(matcher='*', hooks=[handler]))
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
    if provider in ('cursor','zcode'):return 0,0
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
        session = text((v.get('conversation_id') or v.get('session_id')) if provider=='cursor' else v.get('session_id') or v.get('conversationId'))
        if not session: return
        row = dict(kind=kind, at=at, session=session, turn=text(v.get('turn_id') or v.get('turnId')), call=text(v.get('tool_use_id') or v.get('agent_id') or v.get('elicitation_id')), tool=text((v.get('toolCall') or {}).get('name') if provider == 'antigravity' else v.get('tool_name')), notification=text(v.get('notification_type')))
        if isinstance(v.get('fullyIdle'), bool): row['fullyIdle'] = v['fullyIdle']
        row['failed'] = kind == 'StopFailure' or (kind == 'Stop' and (v.get('terminationReason') == 'error' or bool(v.get('error'))))
        row['interrupted'] = any(s in str(v.get('terminationReason', '')) for s in ['cancel', 'interrupt'])
        if provider=='cursor':
            terminal=lambda status: {'completed':'complete','error':'failed','aborted':'interrupted','window_close':'interrupted','user_close':'interrupted'}.get(status,'unknown')
            event={'sessionStart':'SessionStart','beforeSubmitPrompt':'start','preToolUse':'PreToolUse','postToolUse':'PostToolUse','postToolUseFailure':'PostToolUseFailure','afterAgentThought':'thinking'}.get(kind,'unknown')
            if kind in ('stop','sessionEnd'):event=terminal(v.get('status' if kind=='stop' else 'reason'))
            row.update(kind=event,session=text(v.get('conversation_id') or v.get('session_id')),turn=text(v.get('generation_id')),call=text(v.get('tool_use_id') or v.get('tool_call_id')),tool='ask_question' if v.get('tool_name')=='AskQuestion' else text(v.get('tool_name')),failed=False,interrupted=False)
            row.pop('fullyIdle',None);row['notification']=''
            session=row['session']
            if not session:return
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
    if provider in ('pi','omp'):return pi_metadata(v)
    if provider in ('kimi','codebuddy','deepseek-harness','grok'):
        return native_metadata(v,provider)
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
        if v.get('isApiErrorMessage') is True: return [dict(base,kind='failed')]
        for part in content:
            if not isinstance(part,dict): continue
            kind = {'thinking':'thinking','tool_use':'tool'}.get(part.get('type'))
            if kind: rows.append(dict(base,kind=kind,tool=text(part.get('name')),call=text(part.get('id'))))
        if message.get('stop_reason') == 'end_turn': rows.append(dict(base,kind='complete'))
    return rows

def pi_metadata(v):
    base=dict(at=milliseconds(v.get('timestamp')),record=text(v.get('id')),parent=text(v.get('parentId')))
    if v.get('type')=='session':return [dict(base,kind='identity',session=text(v.get('id')))]
    data=v.get('data') or {}
    if v.get('type')=='custom' and v.get('customType')=='tool_execution_start':
        return [dict(base,kind='tool',call=text(data.get('toolCallId')),tool=text(data.get('toolName')))]
    if v.get('type')=='custom' and v.get('customType')=='session_exit':
        kind='failed' if data.get('kind')=='fatal' else 'complete' if data.get('kind')=='normal' and not data.get('pendingToolCalls') else 'interrupted'
        return [dict(base,kind=kind)]
    if v.get('type')!='message':return [dict(base,kind='identity')]
    message=v.get('message') or {};role=message.get('role')
    if role=='user':return [dict(base,kind='start',turn=base['record'])]
    if role=='toolResult':return [dict(base,kind='output',call=text(message.get('toolCallId')),tool=text(message.get('toolName')))]
    if role!='assistant':return [dict(base,kind='identity')]
    rows=[dict(base,kind='tool',call=text(p.get('id')),tool=text(p.get('name'))) for p in message.get('content',[]) if isinstance(p,dict) and p.get('type')=='toolCall']
    reason=message.get('stopReason')
    if reason!='toolUse':rows.append(dict(base,kind={'stop':'complete','length':'failed','error':'failed','aborted':'interrupted'}.get(reason,'unknown')))
    return rows

def native_metadata(v, provider):
    kind=None; row={}
    if provider=='kimi':
        row=dict(at=v.get('time',0),turn=str(v['turnId']) if 'turnId' in v else '')
        kind={'turn.prompt':'start','llm.request':'thinking'}.get(v.get('type'))
        if v.get('type')=='turn.ended':
            kind={'completed':'complete','cancelled':'interrupted','failed':'failed','blocked':'PermissionRequest'}.get(v.get('reason'))
        if v.get('type')=='context.append_loop_event':
            event=v.get('event') or {}
            kind={'tool.call':'tool','tool.result':'output'}.get(event.get('type'))
            row.update(call=text(event.get('toolCallId')),tool=text(event.get('name')))
        if isinstance(v.get('message'),dict):
            row['at']=int(v.get('timestamp',0)*1000)
            kind={'TurnBegin':'start','StepBegin':'thinking','ToolCall':'tool','ToolResult':'output','TurnEnd':'complete','TurnInterrupted':'interrupted'}.get(v['message'].get('type'))
    elif provider=='grok':
        row=dict(at=milliseconds(v.get('ts')),session=text(v.get('session_id')),turn=str(v['turn_number']) if 'turn_number' in v else '')
        kind={'turn_started':'start'}.get(v.get('type'))
        if v.get('type')=='phase_changed' and v.get('phase')=='waiting_for_model':kind='thinking'
        if v.get('type')=='turn_ended':kind={'completed':'complete','cancelled':'interrupted','interrupted':'interrupted','failed':'failed','error':'failed'}.get(v.get('outcome'))
    elif provider=='codebuddy':
        p=v.get('providerData') or {}
        row=dict(at=v.get('timestamp',0),session=text(v.get('sessionId')),turn=text(p.get('conversationRequestId')),record=text(v.get('id')),parent=text(v.get('parentId')))
        kind={'reasoning':'thinking','function_call':'tool','function_call_output':'output','function_call_result':'output'}.get(v.get('type'))
        if v.get('type')=='message':
            if v.get('role')=='user':
                kind='start'
                if not row['turn']:row['turn']=row['record']
            elif v.get('role')=='assistant' and v.get('status')=='completed':kind='complete'
        row.update(call=text(v.get('callId') or v.get('call_id')),tool=text(v.get('name')))
        if not kind and row['record']:kind='identity'
    else:
        data=v.get('data') or {}
        row=dict(at=v.get('time',0),turn=str(data['turn']) if 'turn' in data else '')
        kind={'session':'identity','turn/start':'start','step/start':'thinking','tool/call':'tool','tool/result':'output'}.get(v.get('type'))
        if v.get('type')=='session':row['session']=text(v.get('id'))
        if v.get('type')=='turn/end':
            kind={'completed':'complete','aborted':'interrupted','interrupted':'interrupted','error':'failed','max-tokens':'failed','blocked':'PermissionRequest'}.get((data.get('reason') or {}).get('kind'))
        row.update(call=text(data.get('callId')),tool=text(data.get('name')))
    if kind: row['kind']=kind; return [row]
    return []

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
            for _ in range(2 if provider=='omp' else 1):
                try: rows.extend(e for e in metadata(json.loads(f.readline(65536)), provider, hook) if e.get('kind') == 'identity' and e.get('session'))
                except (ValueError, TypeError): pass
        complete = data.rfind(b'\n') + 1
        for line in data[:complete].splitlines():
            try:
                value=json.loads(line)
                if provider=='kimi' and value.get('type')=='forked':rows=[]
                rows.extend(metadata(value,provider,hook))
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

def database_fingerprint(path):
    def stamp(p):
        try:
            stat=p.stat()
            return (stat.st_dev,stat.st_ino,stat.st_size,stat.st_mtime_ns,stat.st_ctime_ns)
        except FileNotFoundError:return None
    return 'db:'+hashlib.sha256(repr((stamp(path),stamp(pathlib.Path(str(path)+'-wal')))).encode()).hexdigest()

def read_db(path,cursor=None):
    import sqlite3
    from contextlib import closing
    stat=path.stat(); wal=pathlib.Path(str(path)+'-wal')
    signature=database_fingerprint(path)
    modified=int(max(stat.st_mtime, wal.stat().st_mtime if wal.exists() else 0))
    metadata=dict(key=str(path),identity=str(stat.st_ino),signature=signature,size=0,offset=0,modified=modified,events=[],hook=False,database=True)
    if cursor and cursor.get('signature')==signature:
        return dict(metadata,reset=False,steps=None)
    uri=path.resolve().as_uri()+'?mode=ro'
    with closing(sqlite3.connect(uri,uri=True,timeout=.1)) as db:
        db.execute('PRAGMA query_only=ON')
        session=db.execute('SELECT cascade_id FROM trajectory_meta LIMIT 1').fetchone()[0]
        steps=db.execute('SELECT idx,step_type,status FROM steps ORDER BY idx DESC LIMIT 2048').fetchall()[::-1]
    if signature!=database_fingerprint(path):raise RuntimeError('Session database changed during read; retry')
    return dict(metadata,reset=True,session=session,steps=steps)

def collect_native(source,cursors,database_cache=False,harness_deadline=None):
    path=pathlib.Path(os.path.expanduser(source.get('path') or '~/.gemini/antigravity-cli'))
    provider=source['provider']; result=dict(sourceId=source['id'],files=[],error=None)
    try:
        if provider in ('opencode','zcode'):
            database=(path if path.is_file() else path/'opencode.db') if provider=='opencode' else next((p for p in ([path] if path.is_file() else [path/'cli/db/db.sqlite',path/'db/db.sqlite']) if p.is_file()),None)
            if database is None or not database.is_file():raise FileNotFoundError('Native session database not found')
            cursor=cursors.get(str(database),{}) if database_cache else {}
            try:
                result['files'],result['scannedRecords']=native_database()['sessions'](database,provider,cursor,int(time.time()))
            except Exception as error:
                result['error']=str(error)
                if cursor.get('continuation'):
                    result['files']=native_database()['cached_sessions'](database,cursor,int(time.time()))
            return result
        if provider=='deepseek-harness':
            if harness_deadline is None:harness_deadline=time.monotonic()+8
            if not path.exists():raise FileNotFoundError('Harness data directory does not exist')
            files=harness()['paths'](path)
            recent=sorted((f for f in files if time.time()-f.stat().st_mtime<86400),key=lambda f:f.stat().st_mtime,reverse=True)
            if len(recent)>256:result['error']='Too many recent sessions; showing the most recent 256'
            for file in recent[:256]:
                cursor=cursors.get(str(file),{})
                try:
                    remaining=harness_deadline-time.monotonic()
                    # Cached files do not start Node; unfinished first reads
                    # resume on the next poll instead of timing out all sources.
                    if remaining<=0:raise RuntimeError('Harness state scan budget reached; remaining sessions will retry next poll')
                    result['files'].append(harness()['session'](file,cursor,timeout=remaining))
                except Exception as error:
                    result['error']=str(error)
                    if cursor.get('signature','').startswith('dsh:'):
                        result['files'].append(dict(key=str(file),identity=cursor.get('identity',''),signature=cursor['signature'],size=cursor.get('size',0),offset=cursor.get('offset',0),modified=cursor.get('modified',0),reset=False,events=[],hook=False,continuation=cursor.get('continuation')))
            return result
        if not path.exists(): raise FileNotFoundError('Agent data directory does not exist')
        roots=[] if provider=='cursor' else [(path / ('sessions' if provider in ('codex','kimi','grok','pi','omp') else 'projects' if provider in ('claude','codebuddy','qoder') else 'conversations'),False)]
        if roots and not roots[0][0].is_dir(): roots=[(path,False)]
        if provider in ('claude','antigravity','cursor'): roots.append((target(source)[0]/'.aieyes/session-events',True))
        files=[]
        for root,hook in roots:
            if not root.exists(): continue
            candidates=[root] if root.is_file() else root.rglob('*.jsonl' if hook or provider!='antigravity' else '*.db')
            for f in candidates:
                if provider=='kimi' and f.name!='wire.jsonl':continue
                if provider=='grok' and f.name!='events.jsonl':continue
                if f.is_symlink() or (not hook and '.aieyes' in f.parts): continue
                stamp=max(f.stat().st_mtime, pathlib.Path(str(f)+'-wal').stat().st_mtime if pathlib.Path(str(f)+'-wal').exists() else 0)
                if time.time()-stamp < 86400: files.append((stamp,f,hook))
        if len(files)>256: result['error']='Too many recent sessions; showing the most recent 256'
        for _,f,hook in sorted(files,reverse=True)[:256]:
            try: result['files'].append(read_db(f,cursors.get(str(f),{}) if database_cache else None) if not hook and provider=='antigravity' else read_file(f,provider,hook,cursors.get(str(f),{})))
            except Exception as error:
                result['error']=str(error)
                cursor=cursors.get(str(f),{})
                if database_cache and not hook and provider=='antigravity' and cursor.get('signature','').startswith('db:'):
                    result['files'].append(dict(key=str(f),identity=cursor.get('identity',''),signature=cursor['signature'],size=0,offset=0,modified=cursor.get('modified',0),reset=False,events=[],database=True,steps=None))
    except Exception as error: result['error']=str(error)
    return result

def collect(source,cursors,database_cache=False,harness_deadline=None):
    result=collect_native(source,cursors,database_cache,harness_deadline)
    if source['provider']=='zcode':
        root=target(source)[0]/'.aieyes/session-events'
        if root.is_dir():
            paths=sorted((p for p in root.glob('*.jsonl') if p.is_file() and time.time()-p.stat().st_mtime<86400),key=lambda p:p.stat().st_mtime,reverse=True)
            if len(paths)>256:result['error']='Too many recent ZCode state files; showing latest 256'
            for path in paths[:256]:
                try:result['files'].append(read_file(path,'zcode',True,cursors.get(str(path),{})))
                except Exception as error:result['error']=str(error)
    return result

def main(request):
    action=request.get('action','collect'); sources=request.get('sources',[])
    hooks=[]; seen=set()
    for s in sources:
        if s['provider'] not in ('claude','antigravity','cursor','zcode'): continue
        key=(s['provider'],str(target(s)[0]))
        if key in seen: continue
        seen.add(key); hooks.append(manage(s,'ensure' if action=='collect' else action))
    deadline=time.monotonic()+8
    polls=[collect(s,request.get('cursors',{}),request.get('databaseCache') is True,harness_deadline=deadline) for s in sources] if action=='collect' else []
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
