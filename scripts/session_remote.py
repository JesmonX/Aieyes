"""Aieyes lifecycle observer/SSH collector. Standard library only; never emits conversation content."""
import hashlib
import json
import os
import pathlib
import re
import sys
import time

MARKER = '--aieyes-session-hook'
CLAUDE = ['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PermissionRequest', 'PostToolUse', 'PostToolUseFailure', 'Notification', 'SubagentStart', 'SubagentStop', 'Stop', 'StopFailure', 'SessionEnd', 'Elicitation', 'ElicitationResult']
AGY = ['SessionStart', 'PreInvocation', 'PostToolUse', 'PostInvocation', 'Stop']
ZCODE = ['SessionStart','UserPromptSubmit','PreToolUse','PermissionRequest','PostToolUse','PostToolUseFailure','Stop']
CODEBUDDY = CLAUDE + ['PermissionDenied']
CURSOR = ['sessionStart','beforeSubmitPrompt','preToolUse','postToolUse','postToolUseFailure','afterAgentThought','stop','sessionEnd']
def trae_helper():
    if '_TRAE' not in globals():
        from pathlib import Path
        source = globals().get('TRAE_SCRIPT')
        if source is None: source = Path(__file__).with_name('trae_remote.py').read_text(encoding='utf-8')
        scope = {}; exec(compile(source, '<aieyes-trae>', 'exec'), scope); globals()['_TRAE'] = scope
    return globals()['_TRAE']


def hook_names(provider):
    return CODEBUDDY if provider=='codebuddy' else CLAUDE if provider=='claude' else CURSOR if provider=='cursor' else ZCODE if provider=='zcode' else AGY
EVENT_KEYS = {'kind', 'session', 'sessionSource', 'classification', 'originator', 'parentSession', 'parentSessionPath', 'turn', 'record', 'parent', 'call', 'tool', 'at', 'pid', 'processStart', 'fullyIdle', 'failed', 'interrupted', 'notification'}

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

def codebuddy_ide():
    if '_CODEBUDDY_IDE' not in globals():
        source=globals().get('CODEBUDDY_IDE_SCRIPT')
        if source is None:source=pathlib.Path(__file__).with_name('codebuddy_ide_remote.py').read_text(encoding='utf-8')
        scope={};exec(compile(source,'<aieyes-codebuddy-ide>','exec'),scope);globals()['_CODEBUDDY_IDE']=scope
    return globals()['_CODEBUDDY_IDE']

def codex_history():
    if '_CODEX_HISTORY' not in globals():
        source = globals().get('HISTORY_SCRIPT')
        if source is None: source = pathlib.Path(__file__).with_name('remote_history.py').read_text(encoding='utf-8')
        scope = dict(AIEYES_HISTORY_LIBRARY=True)
        if 'HARNESS_SCRIPT' in globals(): scope['HARNESS_SCRIPT'] = globals()['HARNESS_SCRIPT']
        else: scope['HARNESS_SCRIPT'] = pathlib.Path(__file__).with_name('harness_remote.py').read_text(encoding='utf-8')
        exec(compile(source, '<aieyes-codex-history>', 'exec'), scope)
        globals()['_CODEX_HISTORY'] = scope
    return globals()['_CODEX_HISTORY']

def inherit_codex_originators(source, result):
    history = None
    for row in result.get('files', []):
        if not row.get('reset') or row.get('hook') or any(event.get('originator') for event in row.get('events', [])): continue
        try:
            with open(row['key'], 'rb') as stream:
                for _ in range(2):
                    line = stream.readline(65537)
                    if len(line) > 65536 or not line.endswith(b'\n'): break
                    try: value = json.loads(line)
                    except ValueError: continue
                    if value.get('type') != 'session_meta': continue
                    payload = value.get('payload') or {}
                    if payload.get('originator') or not isinstance(payload.get('history_base'), dict): break
                    if history is None: history = codex_history()['_CodexHistory'](os.path.expanduser(source.get('path') or '~/.codex'))
                    originator = history.resolve(payload['history_base']).get('originator')
                    if originator:
                        for event in row.get('events', []):
                            if event.get('kind') == 'identity' and not event.get('originator'): event['originator'] = originator
                    break
        except (OSError, ValueError): pass
    return result

def tasks():
    if '_TASKS' not in globals():
        source=globals().get('TASK_SCRIPT')
        if source is None:source=pathlib.Path(__file__).with_name('task_remote.py').read_text(encoding='utf-8')
        scope={};exec(compile(source,'<aieyes-tasks>','exec'),scope);globals()['_TASKS']=scope
    return globals()['_TASKS']

def target(source):
    path = pathlib.Path(os.path.expanduser(source.get('path') or ('~/.gemini/antigravity-cli' if source['provider'] == 'antigravity' else '~/.cursor' if source['provider']=='cursor' else '~/.zcode' if source['provider']=='zcode' else '~/.codebuddy' if source['provider']=='codebuddy' else '~/.claude')))
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

PROGRAMS = {'claude':(['claude'],['@anthropic-ai/claude-code']),'codex':(['codex'],['@openai/codex']),'deepseek-harness':(['dsh'],['@deepseek-ai/dsh']),'kimi':(['kimi','kimi-cli','kimi-code'],['@moonshot-ai/kimi-code']),'cursor':(['cursor-agent','agent'],[]),'opencode':(['opencode'],['opencode-ai','@opencode/cli']),'codebuddy':(['codebuddy'],['@tencent-ai/codebuddy-code']),'antigravity':(['agy'],[]),'qoder':(['qodercli'],['@qoder-ai/qodercli']),'zcode':(['zcode'],[]),'grok':(['grok'],[]),'pi':(['pi'],['@mariozechner/pi-coding-agent','@earendil-works/pi-coding-agent']),'omp':(['omp'],['@oh-my-pi/pi-coding-agent']),'copilot':(['copilot'],['@github/copilot']),'kiro':(['kiro-cli'],[]),'cline':(['cline'],['cline','@cline/cli']),'kilo':(['kilo','kilocode'],['@kilocode/cli']),'trae':(['trae-cli','trae_agent.cli'],[]),'trae-cn':(['trae-cli','trae_agent.cli'],[])}

def cli_args(provider, argv):
    names, packages = PROGRAMS.get(provider, ([], []))
    index = None
    for i, token in enumerate(argv[:4]):
        token = token.replace('\\', '/'); base = token.rsplit('/', 1)[-1].lower()
        for x in ('.exe', '.cmd', '.js', '.mjs', '.cjs'):
            if base.endswith(x): base = base[:-len(x)]; break
        if base in names or any('node_modules/%s/' % p in token for p in packages): index = i; break
    if index is None: return None
    args, options, positional = argv[index+1:], [], []
    valued = {'--model','-m','--provider','--api-key','--config','--config-file','--config-dir','--cwd','--directory','--workdir','--work-dir','-C','--profile','--output-format','--input-format','--format','--mode','--session','--session-id','--session-dir','--resume','-S','--log-level','--log-file','--agent','--agent-file','--add-dir','--tools','--allowed-tools','--allowedTools','--system-prompt','--append-system-prompt','--prompt','--prompt-interactive','--target','--print-timeout','--json-schema','--max-turns','--setting-sources','--thinking','--theme','--title','--hostname','--port','--trajectory-file'}
    i = 0
    while i < len(args):
        arg = args[i]; i += 1
        if arg == '--':
            positional.extend(args[i:]); break
        if not arg.startswith('-') or arg == '-':
            positional.append(arg); continue
        if '=' in arg:
            options.append(tuple(arg.split('=', 1))); continue
        takes_value = (arg in valued or arg == '-p' and provider not in ('pi','omp')
                       or arg == '-o' and provider == 'qoder' or arg == '-i' and provider == 'antigravity'
                       or arg == '-t' and provider in ('trae','trae-cn'))
        value = args[i] if takes_value and i < len(args) else None
        if value is not None: i += 1
        options.append((arg, value))
    return options, positional

def headless_argv(provider, argv):
    """Mirror of session_events::headless_argv; the shared fixture checks parity."""
    parsed = cli_args(provider, argv)
    if parsed is None: return None
    options, positional = parsed
    has = lambda *flags: any(key in flags for key, _ in options)
    value = lambda flag: next((val for key, val in options if key == flag), None)
    command = next(iter(positional), None)
    H, I = 'headless', 'interactive'
    verdict = lambda headless: H if headless else I
    if provider in ('claude', 'cursor'): return verdict(has('-p', '--print'))
    if provider == 'codebuddy': return verdict(has('-p', '--print', '--acp'))
    if provider == 'codex': return H if command in ('exec', 'e', 'mcp-server') else None
    if provider == 'deepseek-harness':
        profile = value('--profile') or command
        return H if profile in ('headless', 'sdk', 'acp') else I if profile in ('tui', 'web', 'desktop') else None
    if provider == 'kimi': return verdict(has('-p', '--prompt') or command in ('acp', 'web', 'server', 'rc', 'remote'))
    if provider == 'opencode':
        if command in ('run', 'acp', 'api'): return H
        if command == 'serve': return None if has('--service') else H
        return I if command is None else None
    if provider == 'antigravity':
        if has('-i', '--prompt-interactive'): return I
        return H if has('-p', '--print', '--prompt', '--output-format', '--input-format', '--print-timeout', '--json-schema') or command in ('remote-control', 'mic-serve') else None
    if provider == 'qoder': return verdict(has('-p', '--print', '-o', '--output-format', '--remote-control'))
    if provider == 'zcode': return H if has('-p', '--prompt', '--target') or command == 'app-server' else None
    if provider == 'grok': return H if has('-p', '--single', '--output-format', '--json-schema') else None
    if provider in ('pi', 'omp'): return H if has('-p', '--print') or value('--mode') in ('json', 'rpc') else None
    if provider == 'copilot': return H if has('-p', '--prompt', '--acp') else None
    if provider == 'kiro': return H if command == 'chat' and (has('--no-interactive', '--non-interactive') or value('--output-format') == 'stream-json') else None
    if provider == 'cline':
        if has('-i', '--tui'): return I
        return H if has('--json', '-z', '--zen', '--oneshot', '-y', '--yolo') or command is not None else None
    if provider == 'kilo': return H if command in ('run', 'acp') or has('--auto') or value('--format') == 'json' else None
    if provider in ('trae', 'trae-cn'):
        if command == 'run': return H
        if command == 'interactive': return I
        return H if any(a.startswith('--docker-') or a == '--must-patch' for a, _ in options) else None
    return None

def cmdline(pid):
    try: return [a.decode('utf-8', 'replace') for a in pathlib.Path('/proc/%d/cmdline' % pid).read_bytes().split(b'\0') if a]
    except OSError: return []

def ancestor(provider):
    """(pid, start, argv verdict). Node-hosted CLIs are matched by argv only, never pid-bound."""
    bind = provider not in ('cursor','zcode','codebuddy')
    pid = os.getppid(); verdict = None
    for _ in range(8):
        p = process_info(pid)
        if not p: break
        argv = cmdline(pid)
        if verdict is None and not any(MARKER in a for a in argv): verdict = headless_argv(provider, argv)
        if bind and ((provider == 'claude' and p[0].startswith('claude')) or (provider == 'antigravity' and p[0] == 'agy')): return pid, p[2], verdict
        pid = p[1]
    return 0, 0, verdict

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
        session = text((v.get('conversation_id') or v.get('session_id')) if provider=='cursor' else v.get('session_id') or v.get('conversationId') or v.get('sessionId'))
        if not session: return
        row = dict(kind=kind, at=at, session=session, turn=text(v.get('turn_id') or v.get('turnId') or v.get('prompt_id')), call=text(v.get('tool_use_id') or v.get('agent_id') or v.get('elicitation_id')), tool=text((v.get('toolCall') or {}).get('name') if provider == 'antigravity' else v.get('tool_name')), notification=text(v.get('notification_type')))
        if provider=='claude':row['sessionSource']=text(os.environ.get('CLAUDE_CODE_ENTRYPOINT') or v.get('entrypoint'))
        row['parentSession']=text(v.get('parent_session_id') or v.get('parentSessionId'))
        if provider in ('claude','codebuddy') and kind not in ('SubagentStart','SubagentStop') and text(v.get('agent_id')):
            row['parentSession']=session
            session=row['session']=session+':'+text(v.get('agent_id'))
        if provider=='codebuddy' and isinstance(v.get('background_tasks'),list):row['fullyIdle']=not v['background_tasks']
        if isinstance(v.get('fullyIdle'), bool): row['fullyIdle'] = v['fullyIdle']
        row['failed'] = kind == 'StopFailure' or (kind == 'Stop' and (v.get('terminationReason') == 'error' or bool(v.get('error'))))
        row['interrupted'] = any(s in str(v.get('terminationReason', '')) for s in ['cancel', 'interrupt'])
        if kind=='PostToolUseFailure' and v.get('is_interrupt') is True:row.update(kind='interrupted',interrupted=True)
        if provider=='cursor':
            terminal=lambda status: {'completed':'complete','error':'failed','aborted':'interrupted','window_close':'interrupted','user_close':'interrupted'}.get(status,'unknown')
            event={'sessionStart':'SessionStart','beforeSubmitPrompt':'start','preToolUse':'PreToolUse','postToolUse':'PostToolUse','postToolUseFailure':'PostToolUseFailure','afterAgentThought':'thinking'}.get(kind,'unknown')
            if kind in ('stop','sessionEnd'):event=terminal(v.get('status' if kind=='stop' else 'reason'))
            row.update(kind=event,session=text(v.get('conversation_id') or v.get('session_id')),turn=text(v.get('generation_id')),call=text(v.get('tool_use_id') or v.get('tool_call_id')),tool='ask_question' if v.get('tool_name')=='AskQuestion' else text(v.get('tool_name')),failed=False,interrupted=False)
            row.pop('fullyIdle',None);row['notification']=''
            if v.get('is_background_agent') is True:row['classification']='headless'
            session=row['session']
            if not session:return
        row['pid'], row['processStart'], verdict = ancestor(provider)
        if os.environ.get('AIEYES_INTERNAL'): row.update(sessionSource='aieyes-internal', classification='internal')
        elif verdict and not row.get('classification'): row['classification'] = verdict
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
    if provider in ('kimi','codebuddy','deepseek-harness','grok','copilot'):
        return native_metadata(v,provider)
    at = milliseconds(v.get('timestamp'))
    if provider == 'codex':
        p = v.get('payload') or {}; key = (v.get('type'), p.get('type'))
        kinds = {('event_msg','task_started'):'start',('event_msg','task_complete'):'complete',('event_msg','turn_aborted'):'interrupted',('event_msg','error'):'failed',('response_item','reasoning'):'thinking',('response_item','function_call'):'tool',('response_item','custom_tool_call'):'tool',('response_item','function_call_output'):'output',('response_item','custom_tool_call_output'):'output'}
        kind = 'identity' if v.get('type') == 'session_meta' else kinds.get(key)
        if not kind:return []
        row=dict(kind=kind,at=at,session=text(p.get('id')) if kind=='identity' else '',turn=text(p.get('turn_id')),call=text(p.get('call_id')),tool=text(p.get('name')))
        if kind=='identity':
            row['sessionSource']=text(p.get('source'))
            if text(p.get('originator')):row['originator']=text(p['originator'])
            source=p.get('source')
            if isinstance(source,dict):row['parentSession']=text(((source.get('subagent') or {}).get('thread_spawn') or {}).get('parent_thread_id'))
        return [row]
    rows = []; message = v.get('message') or {}; content = message.get('content')
    content = content if isinstance(content,list) else []
    base = dict(at=at,session=text(v.get('sessionId')),parentSession=text(v.get('parentSessionId') or v.get('parent_session_id')))
    if provider=='claude' and text(v.get('entrypoint')):base['sessionSource']=text(v['entrypoint'])
    if v.get('isSidechain') is True and text(v.get('agentId')):
        base['parentSession']=base['session'];base['session']+=':'+text(v.get('agentId'))
    if base.get('sessionSource'):rows.append(dict(base,kind='identity'))
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
    base=dict(at=milliseconds(v.get('timestamp')),record=text(v.get('id')),parent=text(v.get('parentId')),parentSession=text(v.get('parentSessionId') or v.get('parent_session_id')))
    if v.get('type')=='session':return [dict(base,kind='identity',session=text(v.get('id')),parentSessionPath=text(v.get('parentSession')))]
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
        kind={'turn.prompt':'start','llm.request':'thinking','interaction.request':'Elicitation','interaction.resolved':'ElicitationResult'}.get(v.get('type'))
        if v.get('type') in ('interaction.request','interaction.resolved'):row.update(call=text(v.get('id') or v.get('interactionId')),tool=text(v.get('toolName')))
        if v.get('type')=='turn.ended':
            kind={'completed':'complete','cancelled':'interrupted','canceled':'interrupted','interrupted':'interrupted','error':'failed','failed':'failed','blocked':'PermissionRequest'}.get(v.get('reason'))
        if v.get('type')=='context.append_loop_event':
            event=v.get('event') or {}
            kind={'tool.call':'tool','tool.result':'output'}.get(event.get('type'))
            row.update(call=text(event.get('toolCallId')),tool=text(event.get('name')))
        if isinstance(v.get('message'),dict):
            row['at']=int(v.get('timestamp',0)*1000)
            kind={'TurnBegin':'start','StepBegin':'thinking','ToolCall':'tool','ToolResult':'output','TurnEnd':'complete','TurnInterrupted':'interrupted'}.get(v['message'].get('type'))
    elif provider=='grok':
        row=dict(at=milliseconds(v.get('ts')),session=text(v.get('session_id')),turn=str(v['turn_number']) if 'turn_number' in v else '')
        kind={'turn_started':'start','tool_started':'tool','tool_completed':'output','tool_finished':'output','tool_failed':'output','permission_requested':'PermissionRequest','permission_resolved':'ElicitationResult','permission_response':'ElicitationResult','permission_responded':'ElicitationResult'}.get(v.get('type'))
        if kind in ('tool','output'):row.update(call=text(v.get('tool_call_id') or v.get('call_id') or v.get('id')),tool=text(v.get('tool_name') or v.get('name')))
        if kind=='tool' and not row.get('call'):kind='anonymous_tool'
        if kind in ('PermissionRequest','ElicitationResult'):row['call']=text(v.get('permission_id') or v.get('request_id') or v.get('tool_call_id') or v.get('call_id'))
        if v.get('type')=='phase_changed' and v.get('phase')=='tool_execution':kind='tool_phase'
        if v.get('type')=='phase_changed' and v.get('phase')=='waiting_for_model':kind='thinking'
        if v.get('type')=='turn_ended':kind={'completed':'complete','cancelled':'interrupted','interrupted':'interrupted','failed':'failed','error':'failed'}.get(v.get('outcome'))
    elif provider=='copilot':
        data=v.get('data') or {}
        row=dict(at=milliseconds(v.get('timestamp')),turn=text(data.get('interactionId')))
        t=v.get('type')
        kind={'user.message':'start','assistant.turn_start':'thinking','tool.execution_start':'tool','tool.execution_complete':'output','abort':'interrupted','session.abort':'interrupted','assistant.abort':'interrupted','session.error':'failed'}.get(t)
        if t in ('session.start','session.resume'):kind='identity';row['session']=text(data.get('sessionId'))
        if t=='system.message' and isinstance(data.get('content'),str) and 'You are running in non-interactive mode' in data['content']:kind='identity';row['classification']='headless'
        if t=='assistant.message' and not data.get('toolRequests'):kind='complete'
        if kind in ('tool','output'):row.update(call=text(data.get('toolCallId')),tool=text(data.get('toolName')))
    elif provider=='codebuddy':
        p=v.get('providerData') or {}
        row=dict(at=v.get('timestamp',0),session=text(v.get('sessionId')),turn=text(p.get('conversationRequestId')),record=text(v.get('id')),parent=text(v.get('parentId')))
        kind={'reasoning':'thinking','function_call':'tool','function_call_output':'output','function_call_result':'output'}.get(v.get('type'))
        if v.get('type')=='message':
            if v.get('role')=='user':
                kind='start'
                if not row['turn']:row['turn']=row['record']
            elif v.get('role')=='assistant' and v.get('status')=='completed':
                kind='thinking' if isinstance(p.get('messageId'),str) and p.get('rawUsage') is None and p.get('usage') is None else 'complete'
            elif v.get('role')=='assistant' and v.get('status')=='incomplete' and p.get('skipRun') is True and isinstance(p.get('error'),dict):
                error=p['error']
                kind='interrupted' if error.get('message')=='Interrupted by user' else 'failed' if error.get('isRetryable') is False else 'unknown'
        if v.get('type') in ('message','function_call_output','function_call_result') and v.get('status')=='incomplete' and p.get('skipRun') is True and (p.get('error') or {}).get('message')=='Interrupted by user':kind='interrupted'
        row.update(call=text(v.get('callId') or v.get('call_id')),tool=text(v.get('name')))
        if not kind and row['record']:kind='identity'
        if v.get('isSidechain') is True and text(v.get('agentId')):
            row['parentSession']=row['session'];row['session']+=':'+text(v.get('agentId'))
    else:
        data=v.get('data') or {}
        row=dict(at=v.get('time',0),turn=str(data['turn']) if 'turn' in data else '')
        kind={'session':'identity','turn/start':'start','step/start':'thinking','tool/call':'tool','tool/result':'output'}.get(v.get('type'))
        if v.get('type')=='session':row['session']=text(v.get('id'))
        if v.get('type')=='turn/end':
            kind={'completed':'complete','aborted':'interrupted','interrupted':'interrupted','error':'failed','max-tokens':'failed','blocked':'PermissionRequest'}.get((data.get('reason') or {}).get('kind'))
        row.update(call=text(data.get('callId')),tool=text(data.get('name')))
    if kind:
        row['kind']=kind
        if 'parentSession' not in row:row['parentSession']=text(v.get('parentSessionId') or v.get('parent_session_id'))
        return [row]
    return []

def kimi_parent(path):
    if path.parent.parent.name!='agents':return ''
    root=path.parent.parent.parent
    try:
        with (root/'state.json').open('rb') as f:raw=f.read(1024*1024+1)
        if len(raw)>1024*1024:return ''
        data=json.loads(raw);agents=data.get('agents') or {};agent=path.parent.name
        parent=(agents.get(agent) or {}).get('parentAgentId')
        if data.get('id')==root.name and isinstance(parent,str) and parent and parent!=agent and isinstance(agents.get(parent),dict):return root.name+':'+parent
    except (OSError,ValueError,TypeError,AttributeError):pass
    return ''

def read_file(path, provider, hook, cursor):
    limit=16*1024*1024
    stat=path.stat();size=stat.st_size;modified=int(stat.st_mtime)
    identity='%s:%s'%(stat.st_dev,stat.st_ino)
    signature='%s:%s'%(stat.st_mtime_ns,stat.st_ctime_ns)
    reset=(cursor.get('identity')!=identity or size<cursor.get('offset',0)
           or size==cursor.get('size') and signature!=cursor.get('signature'))
    offset=cursor.get('offset',0)
    rows=[];anchor=cursor.get('anchor','')
    if reset or offset!=size or signature!=cursor.get('signature'):
        with path.open('rb') as f:
            if not reset and anchor:
                f.seek(max(0,offset-128))
                reset=hashlib.sha256(f.read(min(offset,128))).hexdigest()!=anchor
            if reset:
                if provider=='codebuddy' and not hook and path.parent.name=='subagents' and path.stem.startswith('agent-'):
                    rows.append(dict(kind='identity',parentSessionPath=str(path.parent.parent.with_suffix('.jsonl'))))
                if provider=='kimi' and not hook:
                    parent=kimi_parent(path)
                    if parent:rows.append(dict(kind='identity',parentSession=parent))
                offset=max(0,size-262144)
                target=offset
                while offset:
                    start=max(0,offset-65536);f.seek(start);chunk=f.read(offset-start)
                    index=chunk.rfind(b'\n')
                    if index>=0:offset=start+index+1;break
                    offset=start
                    if target-offset>limit:raise ValueError('Session record exceeds 16 MiB; retry position retained')
                f.seek(0)
                for _ in range(2 if provider in ('codex','claude','pi','omp') and not hook else 1):
                    line=f.readline(limit+1)
                    if len(line)>limit:raise ValueError('Session record exceeds 16 MiB; retry position retained')
                    if not line.endswith(b'\n'):break
                    try:rows.extend(e for e in metadata(json.loads(line),provider,hook) if e.get('kind')=='identity' and (e.get('session') or e.get('sessionSource')))
                    except (ValueError,TypeError):pass
            f.seek(offset);start=offset
            while True:
                line=f.readline(limit+1)
                if len(line)>limit:raise ValueError('Session record exceeds 16 MiB; retry position retained')
                if not line.endswith(b'\n'):break
                offset+=len(line)
                try:
                    value=json.loads(line)
                    if provider=='kimi' and value.get('type')=='forked':rows=[r for r in rows if r.get('kind')=='identity' and r.get('parentSession')]
                    rows.extend(metadata(value,provider,hook))
                except (ValueError,TypeError,AttributeError):pass
                if offset-start>=524288:break
            f.seek(max(0,offset-128));anchor=hashlib.sha256(f.read(min(offset,128))).hexdigest()
    pid=cursor.get('pid',0);start=cursor.get('processStart',0)
    for row in rows:
        if row.get('pid'):pid=row['pid'];start=row.get('processStart',0)
    alive=None
    if pid and pathlib.Path('/proc').is_dir():
        p=process_info(pid)
        alive=(not start or p[2]==start) if p else (None if pathlib.Path('/proc/%d'%pid).exists() else False)
    return dict(key=str(path),identity=identity,signature=signature,anchor=anchor,size=size,modified=modified,offset=offset,reset=reset,events=rows,hook=hook,alive=alive)

def database_fingerprint(path):
    def stamp(p):
        try:
            stat=p.stat()
            return (stat.st_dev,stat.st_ino,stat.st_size,stat.st_mtime_ns,stat.st_ctime_ns)
        except FileNotFoundError:return None
    return 'db:'+hashlib.sha256(repr((stamp(path),stamp(pathlib.Path(str(path)+'-wal')))).encode()).hexdigest()

def agy_activity_at(raw):
    # Narrow protobuf projection matching core/antigravity.rs. Values other
    # than lifecycle timestamps are skipped and never returned over SSH.
    def varint(data, pos):
        value=0
        for shift in range(0,70,7):
            byte=data[pos];pos+=1
            if shift==63 and byte>1:raise ValueError('metadata integer overflow')
            value|=(byte&127)<<shift
            if byte<128:return value,pos
        raise ValueError('metadata integer invalid')
    def fields(data):
        pos=0
        while pos<len(data):
            tag,pos=varint(data,pos);wire=tag&7;value=None
            if not tag>>3:raise ValueError('metadata field invalid')
            if wire==0:value,pos=varint(data,pos)
            elif wire==2:
                size,pos=varint(data,pos);value=data[pos:pos+size];pos+=size
            elif wire in (1,5):pos+=8 if wire==1 else 4
            else:raise ValueError('metadata wire invalid')
            if pos>len(data):raise ValueError('metadata truncated')
            yield tag>>3,value
    try:
        return max((seconds for key,value in fields(raw) if key in (1,6,7,8,22,32) and isinstance(value,bytes)
                    for field,seconds in fields(value) if field==1 and isinstance(seconds,int) and seconds>0),default=0)
    except (IndexError,ValueError,TypeError):return 0

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
        try:
            activity_at=max((agy_activity_at(raw) for (raw,) in db.execute('SELECT metadata FROM steps WHERE length(metadata)>0 ORDER BY idx DESC LIMIT 32')),default=0)
        except sqlite3.OperationalError:activity_at=0 # Legacy schemas have no metadata column.
    if signature!=database_fingerprint(path):raise RuntimeError('Session database changed during read; retry')
    return dict(metadata,reset=True,session=session,steps=steps,activityAt=activity_at)

class FileWatch:
    """Bounded directory/file notifications; unavailable backends use reconciliation."""
    LIMIT = 8192

    def __init__(self):
        self.fd = None; self.queue = None; self.paths = {}; self.registered = set()
        try:
            if sys.platform.startswith('linux'):
                import ctypes
                self.libc = ctypes.CDLL(None, use_errno=True)
                self.fd = self.libc.inotify_init1(os.O_NONBLOCK | os.O_CLOEXEC)
                if self.fd < 0: raise OSError('inotify unavailable')
            else:
                import select
                self.queue = select.kqueue()
        except (OSError, AttributeError): self.close()

    @property
    def active(self):
        return self.fd is not None or self.queue is not None

    def add(self, path, directory):
        if not self.active or path in self.registered or self.fd is not None and not directory: return
        if len(self.paths) >= self.LIMIT: self.close(); return
        try:
            if self.fd is not None:
                # MODIFY, ATTRIB, CLOSE_WRITE, moves, create/delete and self changes.
                token = self.libc.inotify_add_watch(self.fd, os.fsencode(path), 0x00000fce)
                if token < 0: raise OSError('inotify watch unavailable')
            else:
                import select
                token = os.open(path, os.O_RDONLY | getattr(os, 'O_CLOEXEC', 0) | getattr(os, 'O_EVTONLY', 0))
                try:
                    event = select.kevent(token, filter=select.KQ_FILTER_VNODE,
                        flags=select.KQ_EV_ADD | select.KQ_EV_CLEAR,
                        fflags=select.KQ_NOTE_WRITE | select.KQ_NOTE_EXTEND | select.KQ_NOTE_ATTRIB | select.KQ_NOTE_DELETE | select.KQ_NOTE_RENAME | select.KQ_NOTE_REVOKE)
                    self.queue.control([event], 0, 0)
                except Exception: os.close(token); raise
            self.paths[token] = (path, directory); self.registered.add(path)
        except OSError: self.close()

    def changes(self):
        dirty = set(); lost = False
        try:
            if self.fd is not None:
                import struct
                while True:
                    try: data = os.read(self.fd, 65536)
                    except BlockingIOError: break
                    if not data: break
                    offset = 0
                    while offset + 16 <= len(data):
                        token, mask, _, length = struct.unpack_from('iIII', data, offset)
                        name = os.fsdecode(data[offset+16:offset+16+length].split(b'\0', 1)[0]); offset += 16 + length
                        if mask & (0x4000 | 0x8000) or token not in self.paths: lost = True; continue
                        root, _ = self.paths[token]
                        if mask & 0x40000000 or not name: lost = True
                        else: dirty.add(root / name)
            elif self.queue is not None:
                import select
                while True:
                    events = self.queue.control([], 256, 0)
                    for event in events:
                        path, directory = self.paths[event.ident]
                        if directory or event.fflags & (select.KQ_NOTE_DELETE | select.KQ_NOTE_RENAME | select.KQ_NOTE_REVOKE): lost = True
                        else: dirty.add(path)
                    if len(events) < 256: break
        except (OSError, KeyError): self.close(); lost = True
        return dirty, lost

    def close(self):
        if self.fd is not None and self.fd >= 0: os.close(self.fd)
        if self.queue is not None:
            self.queue.close()
            for token in self.paths: os.close(token)
        self.fd = None; self.queue = None; self.paths.clear(); self.registered.clear()


class SessionDiscovery:
    def __init__(self, provider, roots):
        self.provider = provider; self.roots = roots; self.files = {}; self.watch = FileWatch()
        self.scanned = 0; self.wall = 0; self.overflow = False

    def candidate(self, path, hook):
        if path.suffix != ('.db' if self.provider == 'antigravity' and not hook else '.jsonl'): return False
        if not hook and '.aieyes' in path.parts: return False
        return not ((self.provider == 'kimi' and not hook and path.name != 'wire.jsonl') or
                    (self.provider in ('grok','copilot') and not hook and path.name != 'events.jsonl'))

    def update(self, path, hook, now):
        self.files.pop(path, None)
        if not self.candidate(path, hook) or path.is_symlink(): return
        try:
            stamp = path.stat().st_mtime
            if not hook and self.provider == 'antigravity':
                try: stamp = max(stamp, pathlib.Path(str(path)+'-wal').stat().st_mtime)
                except FileNotFoundError: pass
            self.watch.add(path, False)
            if now - stamp < 86400: self.files[path] = (stamp, hook)
        except FileNotFoundError: pass

    def scan(self, now):
        self.watch.close(); self.watch = FileWatch(); self.files.clear(); self.overflow = False
        for root, hook in self.roots:
            if not root.exists():
                # A first-run hook creates its event directory after collection
                # starts. Watch the closest existing parent to discover it.
                parent = next((p for p in root.parents if p.is_dir()), None)
                if parent is not None: self.watch.add(parent, True)
                continue
            if root.is_file(): self.update(root, hook, now); continue
            pending = [root]
            while pending:
                directory = pending.pop(); self.watch.add(directory, True)
                with os.scandir(directory) as entries:
                    for entry in entries:
                        if entry.is_symlink() or not hook and entry.name == '.aieyes': continue
                        path = pathlib.Path(entry.path)
                        if entry.is_dir(follow_symlinks=False): pending.append(path)
                        elif entry.is_file(follow_symlinks=False): self.update(path, hook, now)
                if len(self.files) > 4096:
                    self.overflow = True
                    self.files = dict(sorted(self.files.items(), key=lambda item: item[1][0], reverse=True)[:257])
        if self.overflow: self.watch.close()
        self.scanned = time.monotonic(); self.wall = now

    def recent(self):
        now = time.time(); dirty, lost = self.watch.changes()
        if lost or not self.scanned or now < self.wall or time.monotonic()-self.scanned >= (60 if self.watch.active else 5):
            self.scan(now)
        else:
            for path in dirty:
                if self.provider == 'antigravity' and str(path).endswith('-wal'): path = pathlib.Path(str(path)[:-4])
                for root, hook in reversed(self.roots):
                    if path == root or root in path.parents:
                        self.update(path, hook, now); break
        self.files = {path: value for path, value in self.files.items() if now-value[0] < 86400}
        return sorted(((stamp, path, hook) for path, (stamp, hook) in self.files.items()), reverse=True)


_DISCOVERY = {}
def recent_files(provider, roots):
    key = (provider, tuple((str(path), hook) for path, hook in roots))
    if key not in _DISCOVERY:
        if len(_DISCOVERY) >= 16: _DISCOVERY.pop(next(iter(_DISCOVERY))).watch.close()
        _DISCOVERY[key] = SessionDiscovery(provider, roots)
    discovery = _DISCOVERY[key]
    return discovery.recent(), discovery.overflow


def kiro_process_classification(lock):
    """Inspect Kiro's lock-holder ancestry; never return argv or environment values."""
    pid = lock.get('pid')
    if not isinstance(pid, int) or pid <= 0:
        return None
    verdict, tui, keyed = None, False, False
    for depth in range(6):
        process = process_info(pid)
        if not process:
            break
        if depth == 0 and lock.get('started_at'):
            try:
                from datetime import datetime
                started = datetime.fromisoformat(lock['started_at'].replace('Z', '+00:00')).timestamp()
                if abs(process[2] - started) > 120:
                    return None
            except (ValueError, TypeError):
                pass
        argv = cmdline(pid)
        verdict = verdict or headless_argv('kiro', argv)
        tui = tui or any(a.replace('\\', '/').endswith('/tui.js') and 'kiro' in a for a in argv[:4])
        try:
            variables = pathlib.Path('/proc/%d/environ' % pid).read_bytes().split(b'\0')
            if any(v.startswith(b'AIEYES_INTERNAL=') and v != b'AIEYES_INTERNAL=' for v in variables):
                return 'internal'
            keyed = keyed or any(v.startswith(b'KIRO_API_KEY=') and v != b'KIRO_API_KEY=' for v in variables)
        except OSError:
            pass
        pid = process[1]
    return verdict or ('interactive' if tui else 'headless' if keyed else None)


def collect_kiro(path, cursors):
    """Kiro V2 CLI and V3/IDE session metadata only; no message JSONL is read."""
    root = path.parent if path.name == 'cli' and path.parent.name == 'sessions' else path if path.name == 'sessions' else path / 'sessions'
    if not root.is_dir():
        return [], 0
    files = list((root / 'cli').glob('*.json')) + list(root.glob('*/sess_*/session.json'))
    recent = []
    for file in files:
        try:
            stamp = file.stat().st_mtime
            if file.name == 'session.json' and (file.parent / 'messages.jsonl').is_file():
                stamp = max(stamp, (file.parent / 'messages.jsonl').stat().st_mtime)
            if time.time() - stamp < 86400:
                recent.append((stamp, file))
        except OSError:
            continue
    recent.sort(reverse=True)
    result = []
    for stamp, file in recent[:256]:
        if file.stat().st_size > 16 * 1024 * 1024:
            raise ValueError('Kiro session metadata exceeds 16 MiB')
        value = json.loads(file.read_text())
        v3 = file.name == 'session.json'
        session = text(value.get('session_id') or value.get('id') or (file.parent.name if v3 else file.stem))
        cursor = cursors.get(str(file), {})
        classification = (cursor.get('continuation') or {}).get('classification', 'unknown')
        if classification not in ('headless', 'internal'):
            cwd = value.get('cwd') or next(iter(value.get('workspacePaths') or value.get('rootPaths') or []), '')
            if cwd and pathlib.Path(cwd).resolve() == (pathlib.Path.home() / '.aieyes/kiro-probe').resolve():
                classification = 'internal'
            elif not v3:
                lock = file.with_suffix('.lock')
                try:
                    if lock.stat().st_size <= 4096:
                        classification = kiro_process_classification(json.loads(lock.read_text())) or classification
                except (OSError, ValueError):
                    pass
        parent = text(value.get('parentSessionId')) if v3 and value.get('createdReason') == 'subagent' else ''
        if v3:
            kind = {'in_progress': 'start', 'provisioning': 'start', 'waiting_on_user': 'PermissionRequest', 'completed': 'complete', 'idle': 'complete', 'failed': 'failed'}.get(value.get('status'), 'unknown')
        else:
            metadata = (value.get('session_state') or {}).get('conversation_metadata') or {}
            last = (metadata.get('user_turn_metadatas') or [{}])[-1]
            kind = 'start' if metadata.get('user_turn_start_request') is not None else 'failed' if 'Err' in (last.get('result') or {}) else {'UserTurnEnd': 'complete', 'ToolUseRejected': 'interrupted', 'DidNotRun': 'interrupted'}.get(last.get('end_reason'), 'unknown')
        event = dict(session=session, turn=session, at=int(stamp * 1000), classification=classification)
        result.append(dict(key=str(file), identity=session, signature='kiro:' + str(stamp), modified=int(stamp), reset=True, database=True,
                           events=[dict(event, kind='start', parentSession=parent), dict(event, kind=kind)], continuation=dict(classification=classification)))
    return result, max(len(recent) - 256, 0)


def collect_native(source,cursors,database_cache=False,harness_deadline=None):
    path=pathlib.Path(os.path.expanduser(source.get('path') or '~/.gemini/antigravity-cli'))
    provider=source['provider']; result=dict(sourceId=source['id'],files=[],error=None,ignored=[],overflow=0)
    try:
        if provider == 'codebuddy' and codebuddy_ide()['is_path'](path):
            result.update(codebuddy_ide()['sessions'](path, cursors, int(time.time())))
            return result
        # Only CLI trajectory metadata is readable; encrypted IDE sources expose
        # a main-process status without synthesizing conversations.
        if provider in ('trae','trae-cn'):
            helper = trae_helper()
            result['files'], result['overflow'], errors = helper['sessions'](path, cursors, headless_argv)
            if errors: result['error'] = errors[0]
            if not (path if path.name == 'trajectories' else path / 'trajectories').is_dir():
                result['processRunning'] = helper['ide_running'](provider)
            return result
        if provider == 'kiro':
            if not path.exists():
                raise FileNotFoundError('Agent data directory does not exist')
            result['files'], result['overflow'] = collect_kiro(path, cursors)
            if result['overflow']:
                result['error'] = 'Too many recent Kiro sessions; showing the most recent 256'
            return result
        if provider in ('cline','roo') or (provider=='kilo' and tasks()['files'](provider,path)
            and not any((path/name).is_file() for name in ('opencode.db','kilo.db'))):
            if not path.exists():raise FileNotFoundError('Agent data directory does not exist')
            result['files'],errors=tasks()['sessions'](path,provider,cursors,int(time.time()))
            if errors:result['error']=errors[0]
            return result
        if provider in ('opencode','kilo','zcode','workbuddy'):
            database=(path if path.is_file() else next((p for p in (path/'opencode.db',path/'kilo.db') if p.is_file()),path/'opencode.db')) if provider in ('opencode','kilo') else (path if path.is_file() else path/'workbuddy.db') if provider=='workbuddy' else next((p for p in ([path] if path.is_file() else [path/'cli/db/db.sqlite',path/'db/db.sqlite']) if p.is_file()),None)
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
            files,issues=harness()['discover'](path)
            if issues:result['error']=issues[0]['message']
            stamps=((f.stat().st_mtime,f) for f in files)
            recent=[f for _,f in sorted((row for row in stamps if time.time()-row[0]<86400),reverse=True)]
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
        roots=[] if provider=='cursor' else [(path / ('sessions' if provider in ('codex','kimi','grok','pi','omp') else 'session-state' if provider=='copilot' else 'projects' if provider in ('claude','codebuddy','qoder') else 'conversations'),False)]
        if roots and not roots[0][0].is_dir(): roots=[(path,False)]
        if provider in ('claude','antigravity','cursor','codebuddy'): roots.append((target(source)[0]/'.aieyes/session-events',True))
        files,truncated=recent_files(provider,roots)
        if truncated: result['error']='Too many recent sessions; showing the most recent 256'
        selected=[]
        for stamp,f,hook in files:
            cursor=cursors.get(str(f),{})
            # Known non-interactive transcripts use no read slot until they change.
            if cursor.get('nonInteractive') and cursor.get('modified')==int(stamp): result['ignored'].append(str(f))
            else: selected.append((f,hook))
        result['overflow']=max(len(selected)-256,0)
        for f,hook in selected[:256]:
            try: result['files'].append(read_db(f,cursors.get(str(f),{}) if database_cache else None) if not hook and provider=='antigravity' else read_file(f,provider,hook,cursors.get(str(f),{})))
            except Exception as error:
                result['error']=str(error)
                cursor=cursors.get(str(f),{})
                if database_cache and not hook and provider=='antigravity' and cursor.get('signature','').startswith('db:'):
                    result['files'].append(dict(key=str(f),identity=cursor.get('identity',''),signature=cursor['signature'],size=0,offset=0,modified=cursor.get('modified',0),reset=False,events=[],database=True,steps=None))
    except Exception as error: result['error']=str(error)
    return result

# Native CLI association mirrors session_processes.rs. No argv/environment data
# is returned to the UI; only a proved process identity and its classification.
_PROCESS_CONTEXTS = {}

def _absolute(value):
    return os.path.realpath(value) if isinstance(value, str) and 0 < len(value) <= 4096 and os.path.isabs(value) else None

def _seconds(value):
    if isinstance(value, str):
        return milliseconds(value) // 1000
    if type(value) is int and value > 0:
        return value // 1000 if value > 10000000000 else value
    return 0

def _session_context(provider, key, session):
    base, sep, native_id = key.rpartition('#')
    database = bool(sep and os.path.isfile(base))
    path = os.path.realpath(base if database else key)
    result = dict(path=path, session=session or key, workspace=None, created=0)
    def metadata(value):
        if not isinstance(value, dict): return
        if value.get('type') == 'session_meta': value = value.get('payload') or {}
        if not isinstance(value, dict): return
        result['workspace'] = next((_absolute(value.get(k)) for k in ('cwd','workingDirectory','working_directory','workspace','directory') if _absolute(value.get(k))), result['workspace'])
        if not result['created']:
            result['created'] = next((_seconds(value.get(k)) for k in ('createdAt','created_at','timestamp','time') if _seconds(value.get(k))), 0)
    if database:
        result['session'] = native_id
        if provider in ('opencode', 'kilo'):
            import contextlib, sqlite3
            try:
                with contextlib.closing(sqlite3.connect(pathlib.Path(path).as_uri() + '?mode=ro', uri=True, timeout=.2)) as db:
                    for table in ('session_v2', 'session'):
                        try:
                            row = db.execute('SELECT directory,time_created FROM ' + table + ' WHERE id=?', (native_id,)).fetchone()
                        except sqlite3.Error: continue
                        if row:
                            result.update(workspace=_absolute(row[0]), created=_seconds(row[1])); break
            except (OSError, sqlite3.Error): pass
        return result
    if provider == 'kimi':
        root = next((p for p in pathlib.Path(path).parents if p.name.startswith('session_')), None)
        if root:
            result['session'] = root.name
            try:
                with (root/'state.json').open('rb') as f: raw = f.read(1024*1024+1)
                value = json.loads(raw) if len(raw) <= 1024*1024 else {}
                if value.get('id') == root.name: metadata(value)
            except (OSError, ValueError, AttributeError): pass
    if not result['workspace']:
        try:
            with open(path, 'rb') as f:
                for _ in range(4):
                    row = f.readline(65537)
                    if not row or len(row) > 65536: break
                    try: metadata(json.loads(row))
                    except ValueError: pass
                    if result['workspace']: break
        except OSError: pass
    return result

def _process_root(provider, options, env):
    value = lambda *keys: next((v for k, v in options if k in keys), None)
    if provider in ('pi','omp') and _absolute(value('--session-dir')): return _absolute(value('--session-dir')), True
    if _absolute(value('--config-dir')): return _absolute(value('--config-dir')), True
    variable = {'codex':'CODEX_HOME','claude':'CLAUDE_CONFIG_DIR','kimi':'KIMI_CODE_HOME','codebuddy':'CODEBUDDY_CONFIG_DIR','qoder':'QODER_CONFIG_DIR','grok':'GROK_HOME','deepseek-harness':'DSH_HOME','pi':'PI_CODING_AGENT_DIR','omp':'PI_CODING_AGENT_DIR'}.get(provider)
    if _absolute(env.get(variable)): return _absolute(env[variable]), True
    if provider in ('opencode','kilo') and _absolute(env.get('XDG_DATA_HOME')): return os.path.join(_absolute(env['XDG_DATA_HOME']), provider), True
    default = {'codex':'~/.codex','claude':'~/.claude','kimi':'~/.kimi-code','codebuddy':'~/.codebuddy','qoder':'~/.qoder','grok':'~/.grok','deepseek-harness':'~/.dsh','pi':'~/.pi/agent','omp':'~/.omp/agent','opencode':'~/.local/share/opencode','kilo':'~/.local/share/kilo','antigravity':'~/.gemini/antigravity-cli'}.get(provider)
    return (os.path.realpath(os.path.expanduser(default)) if default else None), False

_NATIVE_PROCESS_CACHE = {}

def retain_launch_evidence(process, previous):
    if not previous or any(process[key] != previous[key] for key in ('provider','pid','start')): return
    process['internal'] = process['internal'] or previous['internal']
    if previous.get('classification') == 'headless' or process.get('classification') is None:
        process['classification'] = previous.get('classification')
    if not process['rootObserved'] and previous['rootObserved']:
        process.update(root=previous['root'],rootObserved=True)
    if not process.get('selectedSession'): process['selectedSession'] = previous.get('selectedSession')
    if not process.get('workspace'): process['workspace'] = previous.get('workspace')

def native_processes(providers):
    result = []
    try: pids = [int(p.name) for p in pathlib.Path('/proc').iterdir() if p.name.isdigit()]
    except OSError: return []
    for pid in pids:
        argv = cmdline(pid)
        if not argv or any(MARKER in a for a in argv): continue
        matched = [(provider, cli_args(provider, argv)) for provider in providers]
        matched = [(p, args) for p, args in matched if args is not None or any(key[:2] == (p,pid) for key in _NATIVE_PROCESS_CACHE)]
        if not matched: continue
        info = process_info(pid)
        if not info: continue
        try: cwd = _absolute(os.readlink('/proc/%d/cwd' % pid))
        except OSError: cwd = None
        env = {}
        try:
            for item in pathlib.Path('/proc/%d/environ' % pid).read_bytes().split(b'\0'):
                key, sep, value = item.partition(b'=')
                if sep and key in (b'AIEYES_INTERNAL', b'CODEX_HOME', b'CLAUDE_CONFIG_DIR', b'KIMI_CODE_HOME', b'CODEBUDDY_CONFIG_DIR', b'QODER_CONFIG_DIR', b'GROK_HOME', b'DSH_HOME', b'PI_CODING_AGENT_DIR', b'XDG_DATA_HOME'):
                    env[key.decode()] = value.decode('utf-8', 'replace')
        except OSError: pass
        for provider, parsed in matched:
            previous = _NATIVE_PROCESS_CACHE.get((provider,pid,info[2]))
            if parsed is None and previous is None: continue
            options, positional = parsed or ([], [])
            if (provider == 'opencode' and positional[:1] == ['serve'] and any(k == '--service' for k, _ in options)) or (provider == 'kilo' and positional[:1] == ['serve']): continue
            workspace = next((_absolute(v) for k, v in options if k in ('--cwd','--directory','--workdir','--work-dir','-C') and _absolute(v)), cwd)
            files = set()
            if provider in ('deepseek-harness','grok'):
                try:
                    for fd in list(pathlib.Path('/proc/%d/fd' % pid).iterdir())[:512]:
                        try:
                            path = _absolute(os.readlink(fd))
                            if path: files.add(path)
                        except OSError: pass
                except OSError: pass
            root, root_observed = _process_root(provider,options,env)
            verdict = headless_argv(provider,argv)
            if verdict is None and parsed is not None and provider in ('pi','omp'): verdict = 'interactive'
            # process.title may overwrite argv, leaving environment entries in
            # the old argument slots. That title cannot prove interactive mode.
            if verdict != 'headless' and any('=' in arg and re.fullmatch(r'[A-Z0-9_]+',arg.split('=',1)[0]) for arg in argv[1:]): verdict = None
            process = dict(pid=pid,start=info[2],provider=provider,classification=verdict,observedAt=time.monotonic(),alive=True,
                           selectedSession=next((v for k,v in options if k in ('--session','--session-id','--resume','-S')),None),
                           workspace=workspace,root=root,rootObserved=root_observed,internal=bool(env.get('AIEYES_INTERNAL')),files=files)
            retain_launch_evidence(process,previous)
            result.append(process)
        if len(result) >= 256: break
    present = {(p['provider'],p['pid'],p['start']) for p in result}
    for identity, previous in _NATIVE_PROCESS_CACHE.items():
        if len(result) >= 256: break
        if identity not in present and previous['provider'] in providers and time.monotonic()-previous['observedAt'] <= 15:
            result.append(dict(previous,alive=False))
    _NATIVE_PROCESS_CACHE.clear()
    _NATIVE_PROCESS_CACHE.update({(p['provider'],p['pid'],p['start']):p.copy() for p in result})
    return result

def associate_native_processes(provider, sessions, processes):
    matches, result = [], {}
    for si, (_, context, activity) in enumerate(sessions):
        for pi, p in enumerate(processes):
            if p['provider'] != provider: continue
            exact_pid = activity.get('pid') == p['pid'] and activity.get('processStart', 0) > 0 and activity['processStart'] == p['start']
            transcript = '#' not in sessions[si][0] and (context['path'] in p['files'] or provider == 'deepseek-harness' and str(pathlib.Path(context['path']).parent/'session.lock') in p['files'])
            root = p['root']
            in_root = bool(root and (context['path'] == root or context['path'].startswith(root + os.sep)))
            options, _ = cli_args(provider, p['argv']) if 'argv' in p else ([], [])
            selected = p.get('selectedSession') or next((v for k, v in options if k in ('--session','--session-id','--resume','-S')), None)
            explicit = bool(selected and (selected in (context['session'], activity.get('session')) or _absolute(selected) == context['path']))
            same_workspace = context['workspace'] is not None and context['workspace'] == p['workspace']
            started = p['start'] <= context['created'] <= p['start'] + 120
            turn = activity.get('turnStartedAt', 0) // 1000
            resumed = activity.get('phase') in ('working','thinking','tool','needsInput') and p['start'] <= turn <= p['start'] + 120
            strength = 3 if exact_pid or transcript else 2 if in_root and explicit else 1 if in_root and same_workspace and (started or resumed) else 0
            if strength: matches.append((si,pi,strength))
    for si, (key, _, _) in enumerate(sessions):
        best = max((strength for s, _, strength in matches if s == si), default=0)
        candidates = [(pi,strength) for s,pi,strength in matches if s == si and strength == best]
        if len(candidates) != 1: continue
        pi, strength = candidates[0]
        if strength == 1 and len({sessions[s][1]['session'] for s,p,_ in matches if p == pi}) != 1: continue
        p = processes[pi]
        verdict = 'internal' if p['internal'] else p['classification'] if 'classification' in p else headless_argv(provider, p['argv'])
        if verdict is None and 'classification' not in p and provider in ('pi','omp'): verdict = 'interactive'
        result[key] = dict(kind='identity',pid=p['pid'],processStart=p['start'])
        if verdict: result[key]['classification'] = verdict
    return result

def apply_process_evidence(source, result, cursors, processes):
    sessions = []
    for row in result.get('files', []):
        if row.get('hook'): continue
        key = row['key']
        activity = dict(cursors.get(key, {})) if not row.get('reset') else {}
        for event in row.get('events', []):
            if event.get('session'): activity['session'] = event['session']
            if event.get('pid'): activity.update(pid=event['pid'], processStart=event.get('processStart', 0))
            kind = event.get('kind')
            if kind in ('start','UserPromptSubmit'):
                activity.update(phase='working',turnStartedAt=event.get('at',0))
            elif kind in ('thinking','tool','complete','failed','interrupted'):
                activity['phase'] = kind
        cached = _PROCESS_CONTEXTS.get(key)
        if cached is None or cached[0] != row.get('identity') or row.get('reset'):
            cached = (row.get('identity'), _session_context(source['provider'], key, activity.get('session','')))
            _PROCESS_CONTEXTS[key] = cached
        sessions.append((key, cached[1], activity))
    evidence = associate_native_processes(source['provider'], sessions, processes)
    for row in result.get('files', []):
        if row['key'] in evidence:
            event = evidence[row['key']]
            row.setdefault('events', []).append(event)
            row['alive'] = any(p['pid'] == event['pid'] and p['start'] == event['processStart'] and p.get('alive', True) for p in processes)
    if len(_PROCESS_CONTEXTS) > 4096:
        keep = {row['key'] for row in result.get('files', [])}
        for key in list(_PROCESS_CONTEXTS):
            if key not in keep: _PROCESS_CONTEXTS.pop(key, None)
    return result


def collect(source,cursors,database_cache=False,harness_deadline=None,processes=None):
    result=collect_native(source,cursors,database_cache,harness_deadline)
    if source['provider']=='zcode':
        root=target(source)[0]/'.aieyes/session-events'
        if root.is_dir():
            paths=[p for _,p,_ in recent_files('zcode',[(root,True)])[0]]
            if len(paths)>256:result['error']='Too many recent ZCode state files; showing latest 256'
            for path in paths[:256]:
                try:result['files'].append(read_file(path,'zcode',True,cursors.get(str(path),{})))
                except Exception as error:result['error']=str(error)
    if source['provider'] == 'codex': inherit_codex_originators(source,result)
    return apply_process_evidence(source,result,cursors,native_processes([source["provider"]]) if processes is None else processes)

def main(request):
    action=request.get('action','collect'); sources=request.get('sources',[])
    hooks=[]; seen=set()
    for s in sources:
        if s['provider'] not in ('claude','antigravity','cursor','zcode','codebuddy'): continue
        if s['provider'] == 'codebuddy' and codebuddy_ide()['is_path'](s.get('path', '')): continue
        key=(s['provider'],str(target(s)[0]))
        if key in seen: continue
        seen.add(key); hooks.append(manage(s,'ensure' if action=='collect' else action))
    deadline=time.monotonic()+8
    processes=native_processes({s["provider"] for s in sources}) if action=="collect" else []
    polls=[collect(s,request.get('cursors',{}),request.get('databaseCache') is True,harness_deadline=deadline,processes=processes) for s in sources] if action=='collect' else []
    return dict(hooks=hooks,polls=polls,now=int(time.time()))

def serve(request):
    try:
        while True:
            try: response=dict(aieyesSession=1,result=main(request))
            except Exception as error: response=dict(aieyesSession=1,error=str(error))
            print(json.dumps(response,ensure_ascii=False),flush=True)
            line=sys.stdin.readline(64*1024*1024+1)
            if not line or len(line)>64*1024*1024: break
            request=json.loads(line)
    finally:
        for discovery in _DISCOVERY.values(): discovery.watch.close()
        _DISCOVERY.clear()


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
