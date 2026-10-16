"""Aieyes loopback Responses gateway and API configuration, Python 3.8+ stdlib."""
import contextlib
import hashlib
import hmac
import http.server
import json
import math
import os
import pathlib
import re
import shlex
import signal
import socket
import sqlite3
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import urllib.parse
import uuid


def ident(value):
    if not isinstance(value, str) or not re.fullmatch(r'[A-Za-z0-9-]{1,80}', value):
        raise ValueError('API 标识无效')
    return value


def safe(path):
    path = pathlib.Path(path)
    if '..' in path.parts or any(p.is_symlink() for p in (path, *path.parents)):
        raise ValueError('API 管理路径不能包含符号链接或上级目录')
    return path


def private(path):
    safe(path).mkdir(parents=True, exist_ok=True, mode=0o700)
    if path.stat().st_uid != os.getuid():
        raise ValueError('API 目录不属于当前用户')
    path.chmod(0o700)


def read(path):
    return json.loads(safe(path).read_text())


def atomic(path, value, raw=False):
    safe(path)
    if '.aieyes' in path.parent.parts: private(path.parent)
    elif not path.parent.is_dir(): raise ValueError('配置目录不存在')
    temporary = path.with_name('.aieyes-' + uuid.uuid4().hex + '.tmp')
    try:
        fd = os.open(str(temporary), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w') as f:
            f.write(value if raw else json.dumps(value, ensure_ascii=False))
            f.flush(); os.fsync(f.fileno())
        os.replace(str(temporary), str(path))
    finally:
        with contextlib.suppress(FileNotFoundError): temporary.unlink()


def parse_toml(text):
    try:
        import tomllib
    except ImportError:
        import types
        package = types.ModuleType('_aieyes_api_tomli'); package.__path__ = []
        sys.modules['_aieyes_api_tomli'] = package
        for name in ('_types', '_re', '_parser', '__init__'):
            key = '_aieyes_api_tomli' + ('.' + name if name != '__init__' else '')
            module = package if name == '__init__' else types.ModuleType(key)
            module.__package__ = '_aieyes_api_tomli'; sys.modules[key] = module
            exec(compile(TOMLI[name], '<tomli>', 'exec'), module.__dict__)
        tomllib = package
    return tomllib.loads(text)


def toml_text(value):
    def literal(v):
        if isinstance(v, bool): return str(v).lower()
        if isinstance(v, str): return json.dumps(v, ensure_ascii=False)
        if isinstance(v, (int, float)): return str(v)
        if isinstance(v, list): return '[' + ', '.join(literal(x) for x in v) + ']'
        if isinstance(v, dict): return '{' + ', '.join(json.dumps(k) + '=' + literal(x) for k,x in v.items()) + '}'
        if hasattr(v, 'isoformat'): return v.isoformat()
        raise ValueError('不支持的 TOML 配置值')
    lines = []
    def table(v, prefix):
        if prefix: lines.append('[' + '.'.join(json.dumps(k) for k in prefix) + ']')
        for k,x in v.items():
            if not isinstance(x, dict): lines.append(json.dumps(k) + ' = ' + literal(x))
        for k,x in v.items():
            if isinstance(x,dict): table(x,prefix+[k])
    table(value, [])
    result = '\n'.join(lines) + '\n'
    if parse_toml(result) != value: raise ValueError('配置序列化验证失败')
    return result


def home_root(request):
    home = safe(pathlib.Path(os.path.expanduser(request['home'])))
    for parent in [home, *home.parents]:
        if parent.name in ('sessions', 'archived_sessions'):
            home = parent.parent
            break
    if home.suffix == '.jsonl': home = home.parent
    if not home.is_dir(): raise ValueError('Codex 目录不存在')
    return home, home / '.aieyes/api'


def config(home):
    path = safe(home / 'config.toml')
    return parse_toml(path.read_text()) if path.exists() else {}


def revision(home):
    path = safe(home / 'config.toml')
    return hashlib.sha256(path.read_bytes() if path.exists() else b'').hexdigest()


def provider(c): return 'aieyes_api_' + c['id'] + '_' + c['revision']


def launch_command(request, home, c, resume=False):
    return 'env ' + shlex.quote('CODEX_HOME='+str(home)) + ' ' + shlex.quote(os.path.expanduser(request.get('binary') or 'codex')) + ' --profile ' + shlex.quote(provider(c)) + (' resume' if resume else '')


def validate_connection(c):
    ident(c['id']); ident(c['revision'])
    url=urllib.parse.urlsplit(c['baseUrl'])
    if url.scheme not in ('http','https') or not url.hostname or url.username or url.password or url.query or url.fragment:
        raise ValueError('Base URL 应为不含凭据、查询参数的 HTTP(S) 地址')
    if not c.get('name','').strip() or len(c['name']) > 100: raise ValueError('请输入 API 名称（最多 100 字）')
    for price in c.get('prices',[]):
        if not price.get('id'): raise ValueError('请输入价格对应的模型 ID')
        for key in ('input','output','cacheRead','cacheWrite'):
            value=price.get(key)
            if value is not None and (type(value) not in (int,float) or not math.isfinite(value) or value<0): raise ValueError('价格必须为非负数')


def backup_files(paths):
    return {safe(path):path.read_bytes() if path.exists() else None for path in paths}


def restore_files(files):
    for path,value in files.items():
        if value is None:
            with contextlib.suppress(FileNotFoundError): path.unlink()
        else: atomic(path,value.decode(),raw=True)


def select_api(values, root, c, runtime):
    remove_provider_tables(values, c['id'])
    values.setdefault('model_providers', {})[provider(c)] = provider_table(root, c, runtime)
    values.update(model_provider=provider(c), model=c['model'])
    if c.get('effort'): values['model_reasoning_effort'] = c['effort']
    else: values.pop('model_reasoning_effort', None)


def remove_provider_tables(values, identifier):
    providers=values.get('model_providers',{})
    names=[name for name in providers if name.startswith('aieyes_api_'+identifier+'_')]
    for name in names: del providers[name]
    return bool(names)


def write_default(home, root, values, expected):
    if revision(home) != expected: raise ValueError('Codex 配置已变化，请刷新后重试')
    before = config(home); path = root/'default-history.json'
    history = read(path) if path.exists() else []
    original = list(history)
    history.append(dict(startedAt=time.time(), before=auth_snapshot(before)))
    atomic(path, history)
    try:
        if revision(home) != expected: raise ValueError('Codex 配置已变化，请刷新后重试')
        atomic(home/'config.toml', toml_text(values), raw=True)
    except Exception:
        atomic(path, original); raise
    history[-1]['completedAt'] = time.time()
    try: atomic(path, history)
    except Exception:
        if config(home)==values: atomic(home/'config.toml',toml_text(before),raw=True)
        atomic(path,original);raise


def auth_snapshot(values):
    providers={}
    for name,table in values.get('model_providers',{}).items():
        entry={key:table[key] for key in ('requires_openai_auth','env_key') if key in table}
        url=urllib.parse.urlsplit(table.get('base_url',''))
        if url.scheme in ('http','https') and url.hostname: entry['base_url']='https://gateway.invalid'
        if table.get('experimental_bearer_token','').strip(): entry['experimental_bearer_token']='present'
        if (table.get('auth') or {}).get('command'): entry['auth']={'command':'present'}
        providers[name]=entry
    return dict(model_provider=values.get('model_provider'),model_providers=providers)


def subscription_backup(values):
    selected=values.get('model_provider','openai')
    subscribed=selected=='openai' or values.get('model_providers',{}).get(selected,{}).get('requires_openai_auth') is True
    return {key:values.get(key) for key in ('model_provider','model','model_reasoning_effort')} if subscribed else {'model_provider':'openai'}


def provider_table(root, c, runtime):
    return dict(name=c['name'], base_url='http://127.0.0.1:%d/providers/%s/v1' % (runtime['port'], c['id']), wire_api='responses', supports_websockets=False, requires_openai_auth=False,
                auth=dict(command=sys.executable, args=[str(root/'gateway.py'), '--token', str(root), c['id']]))


def journal(root):
    db = sqlite3.connect(str(safe(root/'meter.sqlite')), timeout=5)
    db.executescript('PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS requests(id TEXT PRIMARY KEY,stamp INTEGER NOT NULL,payload TEXT NOT NULL); CREATE INDEX IF NOT EXISTS requests_stamp ON requests(stamp);')
    with db:
        db.execute('BEGIN IMMEDIATE')
        db.execute('CREATE TABLE IF NOT EXISTS meter_sequence(id INTEGER PRIMARY KEY,value INTEGER NOT NULL)')
        db.execute('INSERT OR IGNORE INTO meter_sequence VALUES(1,0)')
        if 'revision' not in [row[1] for row in db.execute('PRAGMA table_info(requests)')]:
            db.execute('ALTER TABLE requests ADD COLUMN revision INTEGER NOT NULL DEFAULT 0')
    return db


def record(root, value):
    with contextlib.closing(journal(root)) as db, db:
        db.execute('BEGIN IMMEDIATE')
        db.execute('UPDATE meter_sequence SET value=value+1 WHERE id=1')
        db.execute('INSERT INTO requests(id,stamp,payload,revision) VALUES(?,?,?,(SELECT value FROM meter_sequence WHERE id=1)) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,revision=excluded.revision', (value['id'], value['timestamp'], json.dumps(value)))


def ledger(root, after):
    path = root/'meter.sqlite'
    if not path.exists(): return dict(records=[], cursor=0)
    with contextlib.closing(journal(root)) as db, db:
        db.execute('BEGIN')
        cursor = db.execute('SELECT value FROM meter_sequence WHERE id=1').fetchone()[0]
        return dict(records=[json.loads(row[0]) for row in db.execute('SELECT payload FROM requests WHERE revision>? ORDER BY revision,id', (after,))], cursor=cursor)


def valid_usage(usage):
    if not isinstance(usage, dict): return False
    i, o = usage.get('input_tokens'), usage.get('output_tokens')
    details=usage.get('input_tokens_details') or {}
    if not isinstance(details,dict): return False
    cached = details.get('cached_tokens', 0)
    if cached is None: cached=0
    return all(type(v) is int and 0 <= v <= 2**64-1 for v in (i, o, cached)) and cached <= i and i+o <= 2**64-1


class UsageCapture:
    def __init__(self):
        self.pending, self.data = b'', b''
        self.complete, self.response = False, {}
    def value(self, value):
        if not isinstance(value, dict): return
        response = value.get('response', value)
        if not isinstance(response, dict): return
        for key in ('id','model'):
            if isinstance(response.get(key),str): self.response[key]=response[key]
        terminal = value.get('type') in ('response.completed','response.incomplete','response.failed')
        terminal = terminal or response.get('status') in ('completed','incomplete','failed') or response.get('object') == 'response.compaction'
        if terminal:
            if isinstance(response.get('usage'),dict): self.response['usage']=response['usage']
            self.complete = value.get('type') == 'response.completed' or response.get('status') == 'completed' or response.get('object') == 'response.compaction'
    def feed(self, chunk):
        self.pending += chunk
        while b'\n' in self.pending:
            line, self.pending = self.pending.split(b'\n', 1)
            if not line.strip():
                try: self.value(json.loads(self.data))
                except ValueError: pass
                self.data = b''
            elif line.startswith(b'data:'): self.data += line[5:].lstrip(b' ') + b'\n'
            if len(self.data) > 8*1024*1024: self.data = b''
        if len(self.pending) > 8*1024*1024: self.pending = b''


def opener(c):
    proxy = c['connection'].get('proxy') or {}
    env = c.get('proxyEnvironment') or {}
    proxies = {} if proxy.get('mode') == 'direct' else ({'http':proxy['url'], 'https':proxy['url']} if proxy.get('mode') == 'custom' else {k:env.get(k.upper()+'_PROXY') or env.get(k+'_proxy') or env.get('ALL_PROXY') or env.get('all_proxy') for k in ('http','https')})
    proxies = {k:v for k,v in proxies.items() if v}
    if any(v.startswith(('socks4:', 'socks5:', 'socks5h:')) for v in proxies.values()):
        raise ValueError('远程 API 计量服务请使用 HTTP 代理地址')
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *args): return None
    return urllib.request.build_opener(urllib.request.ProxyHandler(proxies), NoRedirect())


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def log_message(self, *args): pass
    def reply(self, status, value):
        body = json.dumps(value).encode()
        self.send_response(status); self.send_header('Content-Type','application/json'); self.send_header('Content-Length',str(len(body))); self.send_header('Connection','close'); self.end_headers(); self.wfile.write(body)
    def do_GET(self): self.handle_request()
    def do_POST(self): self.handle_request()
    def handle_request(self):
        rec, sent, capture = None, False, UsageCapture()
        try:
            parts = self.path.split('/')
            if len(parts)<5 or parts[1]!='providers' or parts[3]!='v1': return self.reply(404, {'error':'Unknown route'})
            identifier = ident(parts[2]); root = self.server.root
            c = read(root/'connections'/(identifier+'.json'))
            if not hmac.compare_digest(self.headers.get('Authorization',''), 'Bearer '+c['localToken']): return self.reply(401, {'error':'Local authentication required'})
            route = '/'.join(parts[4:])
            if self.command=='GET' and route=='health': return self.reply(200, {'ready':True})
            if (self.command,route) not in [('POST','responses'),('POST','responses/compact'),('GET','models')]: return self.reply(404, {'error':'Unsupported Responses route'})
            length = int(self.headers.get('Content-Length',0))
            if self.headers.get('Transfer-Encoding'): return self.reply(411, {'error':'Content-Length required'})
            if length<0 or length>32*1024*1024: return self.reply(413, {'error':'Request too large'})
            self.connection.settimeout(120)
            body = self.rfile.read(length)
            if len(body)!=length: raise ConnectionError('Truncated client request')
            data = {}
            if self.headers.get('Content-Encoding','identity')=='identity':
                with contextlib.suppress(ValueError): data = json.loads(body)
            if not isinstance(data,dict): data = {}
            try: meta = json.loads(self.headers.get('x-codex-turn-metadata','{}'))
            except ValueError: meta = {}
            if not isinstance(meta, dict): meta = {}
            if self.command=='POST':
                rec = dict(id=uuid.uuid4().hex, connectionId=identifier, sourceId=c['connection']['sourceId'], timestamp=int(time.time()), status='pending', sessionId=meta.get('session_id') or self.headers.get('session-id'), turnId=meta.get('turn_id'), model=data.get('model') or '', prices=c['connection']['prices'])
                record(root,rec)
            headers = {k:v for k,v in self.headers.items() if k.lower() in ('content-type','content-encoding','accept','originator','session-id','thread-id','x-client-request-id') or k.lower().startswith(('x-codex-','x-openai-'))}
            headers.update({'Authorization':'Bearer '+c['apiKey'], 'Accept-Encoding':'identity'})
            request = urllib.request.Request(c['connection']['baseUrl'].rstrip('/')+'/'+route, data=body if self.command=='POST' else None, headers=headers, method=self.command)
            try: response = opener(c).open(request, timeout=120)
            except urllib.error.HTTPError as error: response = error
            with response:
                status=response.code; content_type=response.headers.get('Content-Type','application/json')
                self.send_response(status); self.send_header('Content-Type',content_type); self.send_header('Transfer-Encoding','chunked'); self.send_header('Connection','close')
                for key, value in response.headers.items():
                    if key.lower() in ('retry-after','x-request-id','openai-processing-ms','content-encoding') or key.lower().startswith('x-ratelimit-'): self.send_header(key, value)
                self.end_headers(); sent=True
                whole=b''; connected=True; deadline=time.monotonic()+1800;received=0
                expected=int(response.headers['Content-Length']) if response.headers.get('Content-Length') else None
                while True:
                    if time.monotonic() > deadline: raise TimeoutError('API stream timeout')
                    chunk=response.read1(16384)
                    if not chunk: break
                    received+=len(chunk)
                    if 'event-stream' in content_type:
                        capture.feed(chunk)
                    elif len(whole)+len(chunk)<=8*1024*1024: whole+=chunk
                    if connected:
                        try: self.wfile.write(('%x\r\n'%len(chunk)).encode()+chunk+b'\r\n'); self.wfile.flush()
                        except (BrokenPipeError, ConnectionError, OSError): connected=False
                if expected is not None and received!=expected: raise ConnectionError('Truncated upstream response')
                if whole:
                    try:
                        capture.value(json.loads(whole))
                    except ValueError: pass
                capture.feed(b'\n\n')
                self.capture(rec,capture.response)
                if connected:
                    with contextlib.suppress(OSError): self.wfile.write(b'0\r\n\r\n')
                if rec:
                    rec.update(httpStatus=status, endedAt=int(time.time()), status='rejected' if status>=300 else 'completed' if capture.complete and valid_usage(rec.get('usage')) else 'incomplete')
                    record(root,rec)
        except Exception:
            if rec:
                self.capture(rec,capture.response)
                rec.update(status='incomplete', errorCode='upstream-or-stream', endedAt=int(time.time()))
                with contextlib.suppress(Exception): record(self.server.root,rec)
            if not sent:
                with contextlib.suppress(Exception): self.reply(502, {'error':'API connection or stream interrupted'})
        finally:
            self.close_connection = True
    @staticmethod
    def capture(rec, value):
        if rec is not None:
            rec['usage']=value.get('usage'); rec['responseId']=value.get('id'); rec['model']=value.get('model') or rec['model']


def serve(root):
    import fcntl
    private(root)
    lock=open(root/'service.lock','a+')
    try: fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
    except OSError: return
    runtime=read(root/'runtime.json')
    server=http.server.ThreadingHTTPServer(('127.0.0.1',runtime['port']),Handler); server.root=root
    server.daemon_threads=False
    server.timeout=.1
    with contextlib.closing(journal(root)): pass
    interrupt_pending(root,'service-restarted')
    try:
        while has_connections(root): server.handle_request()
    finally: server.server_close();lock.close()


def has_connections(root): return any((root/'connections').glob('*.json'))


def interrupt_pending(root, reason):
    for rec in ledger(root,-1)['records']:
        if rec['status']=='pending':
            rec.update(status='incomplete',errorCode=reason,endedAt=int(time.time()));record(root,rec)


def healthy(root):
    if not (root/'runtime.json').exists(): return False
    runtime=read(root/'runtime.json')
    for p in (root/'connections').glob('*.json'):
        c=read(p)
        req=urllib.request.Request('http://127.0.0.1:%d/providers/%s/v1/health'%(runtime['port'],c['connection']['id']),headers={'Authorization':'Bearer '+c['localToken']})
        try:
            with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req,timeout=.4) as response: return json.load(response).get('ready') is True
        except Exception: return False
    return False


def ensure_started(root):
    if healthy(root): return
    subprocess.Popen([sys.executable,str(root/'gateway.py'),'--serve',str(root)],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)
    for _ in range(30):
        if healthy(root): return
        time.sleep(.1)
    raise ValueError('API 计量服务未就绪；请检查监听端口')


def cron_tag(root): return '# app.aieyes.api.'+hashlib.sha256(str(root).encode()).hexdigest()[:20]


def cron_lines():
    old=subprocess.run(['crontab','-l'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=dict(os.environ,LC_ALL='C'))
    if old.returncode and not (old.returncode==1 and (not old.stderr or b'no crontab' in old.stderr)):
        raise ValueError('无法读取 crontab')
    return old.stdout.decode().splitlines()


def set_cron(root,owned):
    old=cron_lines();tag=cron_tag(root)
    lines=[line for line in old if not line.endswith(tag)]+owned
    if lines==old: return
    p=subprocess.run(['crontab','-'],input=('\n'.join(lines)+'\n').encode(),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    if p.returncode: raise ValueError('更新 API 计量服务 crontab 失败')


def register(root):
    command=' '.join(shlex.quote(v) for v in [sys.executable,str(root/'gateway.py'),'--serve',str(root)])
    set_cron(root,['* * * * * '+command.replace('%','\\%')+' >/dev/null 2>&1 '+cron_tag(root)])


def manage(request, script, tomli):
    home,root=home_root(request); action=request['action']; params=request.get('params') or {}
    if action=='ledger': return ledger(root, params.get('cursor',-1))
    if action=='status': return dict(configRevision=revision(home), defaultProvider=config(home).get('model_provider'), installed=(root/'runtime.json').exists() and has_connections(root), ready=healthy(root))
    private(root);private(root/'connections')
    import fcntl
    with open(root/'config.lock','a+') as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        if action=='deploy':
            c=params['connection']; identifier=ident(c['id']); path=root/'connections'/(identifier+'.json')
            validate_connection(c)
            old=read(path) if path.exists() else None
            if old and request.get('sourceId',c['sourceId']) != old['connection']['sourceId']: raise ValueError('API 连接不属于所选设备目录')
            if (old or {}).get('connection', {}).get('revision', '') != params.get('baseRevision', ''): raise ValueError('目标设备 API 设置已变化，请重新读取')
            is_default = config(home).get('model_provider', '').startswith('aieyes_api_'+identifier+'_')
            if is_default and params.get('configRevision') != revision(home): raise ValueError('Codex 默认配置已变化，请刷新后重试')
            key=params.get('apiKey') or (old or {}).get('apiKey')
            if not key or any(x in key for x in ('\n','\r','\0')): raise ValueError('请输入有效的 API Key')
            if (root/'runtime.json').exists(): runtime=read(root/'runtime.json')
            else:
                with socket.socket() as probe: probe.bind(('127.0.0.1',0)); runtime={'port':probe.getsockname()[1]}
            private_connection=dict(connection=c,apiKey=key,localToken=(old or {}).get('localToken') or uuid.uuid4().hex+uuid.uuid4().hex,proxyEnvironment={k:v for k,v in os.environ.items() if k in ('HTTP_PROXY','HTTPS_PROXY','ALL_PROXY','http_proxy','https_proxy','all_proxy')})
            opener(private_connection)
            values=dict(model_provider=provider(c),model=c['model'],model_providers={provider(c):provider_table(root,c,runtime)})
            if c.get('effort'): values['model_reasoning_effort']=c['effort']
            # Do not chmod the user's Codex home.
            profile=home/(provider(c)+'.config.toml')
            files=backup_files([path,profile,root/'runtime.json',root/'gateway.py'])
            cron=[line for line in cron_lines() if line.endswith(cron_tag(root))]
            try:
                atomic(root/'runtime.json',runtime)
                atomic(root/'gateway.py','TOMLI='+repr(tomli)+'\n'+script,raw=True)
                atomic(path,private_connection)
                atomic(profile,toml_text(values),raw=True)
                register(root);ensure_started(root)
                if is_default:
                    values = config(home); select_api(values, root, c, runtime)
                    write_default(home, root, values, params['configRevision'])
            except Exception:
                restore_files(files);set_cron(root,cron)
                raise
            return dict(launchCommand=launch_command(request, home, c),resumeCommand=launch_command(request,home,c,True),configRevision=revision(home))
        if action=='default':
            if params.get('configRevision')!=revision(home): raise ValueError('Codex 配置已变化，请刷新后重试')
            values=config(home);backup=root/'default-backup.json';keys=('model_provider','model','model_reasoning_effort')
            if params['id']=='subscription':
                if backup.exists():
                    saved=read(backup)
                    for key in keys:
                        if saved.get(key) is None: values.pop(key,None)
                        else: values[key]=saved[key]
                else: values['model_provider']='openai'
            else:
                c=read(root/'connections'/(ident(params['id'])+'.json'))['connection'];runtime=read(root/'runtime.json')
                if request.get('sourceId',c['sourceId'])!=c['sourceId']: raise ValueError('API 连接不属于所选设备目录')
                if params.get('revision')!=c['revision']: raise ValueError('目标设备 API 设置已变化，请重新读取')
                if not values.get('model_provider','').startswith('aieyes_api_'): atomic(backup,subscription_backup(values))
                select_api(values,root,c,runtime)
            write_default(home, root, values, params['configRevision'])
            return dict(configRevision=revision(home))
        identifier=ident(params['id']);path=root/'connections'/(identifier+'.json');c=read(path)
        if request.get('sourceId',c['connection']['sourceId'])!=c['connection']['sourceId']: raise ValueError('API 连接不属于所选设备目录')
        if action=='command': return dict(launchCommand=launch_command(request, home, c['connection']),resumeCommand=launch_command(request,home,c['connection'],True))
        if action=='probe':
            req=urllib.request.Request(c['connection']['baseUrl'].rstrip('/')+'/models',headers={'Authorization':'Bearer '+c['apiKey']})
            try:
                with opener(c).open(req,timeout=20) as response: status=response.code
            except urllib.error.HTTPError as error: status=error.code
            return dict(reachable=200<=status<300,httpStatus=status,message='API 认证与模型列表接口可用；尚未发起计费调用' if status<300 else '模型列表查询未成功；部分接口不支持此操作')
        if action=='remove':
            if params.get('revision')!=c['connection']['revision']: raise ValueError('目标设备 API 设置已变化，请重新读取')
            expected=revision(home);values=config(home)
            if values.get('model_provider','').startswith('aieyes_api_'+identifier+'_'): raise ValueError('请先切回订阅或其他默认 API，再移除此连接')
            config_changed=remove_provider_tables(values,identifier)
            files=backup_files([path]+list(home.glob('aieyes_api_'+identifier+'_*.config.toml')))
            cron=[line for line in cron_lines() if line.endswith(cron_tag(root))]
            last=len(list((root/'connections').glob('*.json')))==1
            try:
                if last: set_cron(root,[])
                for path in files: path.unlink()
                if last: interrupt_pending(root,'connection-removed')
                if config_changed: write_default(home,root,values,expected)
            except Exception:
                restore_files(files);set_cron(root,cron);raise
            return dict(removed=True)
        raise ValueError('未知 API 操作')


def direct_index(root, identifier): return root/'direct/current'/(ident(identifier)+'.json')
def direct_key(root, identifier, version): return root/'direct/keys'/(ident(identifier)+'-'+ident(version)+'.json')


def direct_latest(root, identifier):
    pointer = direct_index(root, identifier)
    if pointer.exists():
        c = read(pointer)
        return read(direct_key(root, identifier, c['revision']))
    return read(root/'connections'/(ident(identifier)+'.json'))


def direct_simple(c):
    return {key: c[key] for key in ('id', 'sourceId', 'name', 'baseUrl', 'revision')}


def direct_store(root, c, key):
    path, pointer = direct_key(root, c['id'], c['revision']), direct_index(root, c['id'])
    private(path.parent); private(pointer.parent)
    saved = dict(connection=direct_simple(c), apiKey=key)
    if path.exists() and read(path) != saved:
        raise ValueError('API 修订已存在且内容不同，请重新保存')
    if not path.exists(): atomic(path, saved)
    atomic(pointer, saved['connection'])


def direct_selected(home):
    values = config(home)
    provider_name = values.get('model_provider', 'openai')
    table = values.get('model_providers', {}).get(provider_name, {})
    if provider_name == 'openai' or table.get('requires_openai_auth') is True:
        auth = read(home/'auth.json') if (home/'auth.json').exists() else {}
        return dict(kind='api' if auth.get('auth_mode') == 'apikey' or auth.get('OPENAI_API_KEY') else 'account', provider=provider_name)
    result = dict(kind='api', provider=provider_name, name=table.get('name'))
    if provider_name.startswith('aieyes_api_') and '_' in provider_name[len('aieyes_api_'):]:
        result['connectionId'], result['revision'] = provider_name[len('aieyes_api_'):].rsplit('_', 1)
    return result


DIRECT_READER = '''import json,os,pathlib,re,sys
root=pathlib.Path(sys.argv[1])
identity,version=sys.argv[2:4]
if not all(re.fullmatch(r"[A-Za-z0-9_-]+",v) for v in (identity,version)): raise SystemExit("Invalid API identity")
path=root/"direct/keys"/(identity+"-"+version+".json")
if any(p.is_symlink() for p in [path,*path.parents]): raise SystemExit("Unsafe API path")
with path.open() as file: key=json.load(file)["apiKey"]
if not key or any(c in key for c in (chr(0),chr(10),chr(13))): raise SystemExit("Invalid API key")
print(key)
'''


def direct_prepare(home, target):
    root = home/'.aieyes/api'
    result = dict(target)
    if target.get('kind') == 'api':
        saved = direct_latest(root, target['connectionId']); c = direct_simple(saved['connection'])
        if c['revision'] != target.get('revision') or target.get('sourceId', c['sourceId']) != c['sourceId']:
            raise ValueError('API 设置或设备已变化，请重新检查')
        validate_connection(c)
        if not saved.get('apiKey') or any(char in saved['apiKey'] for char in ('\n', '\r', '\0')):
            raise ValueError('API Key 无效')
        if config(home).get('forced_login_method') not in (None, 'api'):
            raise ValueError('此 home 限制了登录方式')
        direct_store(root, c, saved['apiKey'])
        helper = root/('direct/key_reader_' + hashlib.sha256(DIRECT_READER.encode()).hexdigest()[:16] + '.py')
        if not helper.exists(): atomic(helper, DIRECT_READER, raw=True)
        result.update(helper=str(helper), name=c['name'])
    elif target.get('kind') != 'account':
        raise ValueError('切换目标无效')
    result['configRevision'] = revision(home)
    return result


def direct_select(home, target):
    root = home/'.aieyes/api'; private(root)
    import fcntl
    with open(root/'config.lock', 'a+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if revision(home) != target.get('configRevision'): raise ValueError('Codex 配置已变化，请重新检查')
        values = config(home); backup = root/'account-selection.json'
        if target['kind'] == 'api':
            saved = direct_latest(root, target['connectionId']); c = saved['connection']
            if c['revision'] != target['revision']: raise ValueError('API 设置已变化，请重新检查')
            if direct_selected(home)['kind'] == 'account': atomic(backup, dict(model_provider=values.get('model_provider')))
            name = provider(c)
            values.setdefault('model_providers', {})[name] = dict(name=c['name'], base_url=c['baseUrl'], wire_api='responses', requires_openai_auth=False, supports_websockets=False,
                auth=dict(command=sys.executable, args=[target['helper'], str(root), c['id'], c['revision']]))
            values['model_provider'] = name
        else:
            old = backup if backup.exists() else root/'default-backup.json'
            saved = read(old) if old.exists() else {}
            name = saved.get('model_provider') or 'openai'
            if name != 'openai' and values.get('model_providers', {}).get(name, {}).get('requires_openai_auth') is not True: name = 'openai'
            values['model_provider'] = name
        write_default(home, root, values, target['configRevision'])
        return revision(home)


def direct_restore(home, values, written):
    write_default(home, home/'.aieyes/api', values, written)


def direct_running(home):
    for entry in pathlib.Path('/proc').iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid(): continue
        try:
            if entry.stat().st_uid != os.getuid(): continue
            if not (entry/'comm').read_text().strip().lower().startswith('codex'): continue
            env = dict(item.decode().split('=',1) for item in (entry/'environ').read_bytes().split(b'\0') if b'=' in item)
            location = pathlib.Path(env['CODEX_HOME']) if env.get('CODEX_HOME') else pathlib.Path(env['HOME'])/'.codex'
            if location.resolve() == home.resolve(): return True
        except (OSError,ValueError,KeyError):
            if entry.exists(): return True
    return False


def direct_manage(request, script, tomli):
    home, root = home_root(request); action = request['action']; params = request.get('params') or {}
    if action == 'ledger': return ledger(root, params.get('cursor',-1))
    if action == 'status': return dict(configRevision=revision(home), defaultProvider=config(home).get('model_provider'), selected=direct_selected(home))
    if action in ('default','command'): raise ValueError('请通过账户切换检查后切换 API')
    private(root)
    import fcntl
    with open(root/'config.lock','a+') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if action == 'deploy':
            c = direct_simple(params['connection']); validate_connection(c)
            try: old = direct_latest(root, c['id'])
            except FileNotFoundError: old = {}
            if old.get('connection',{}).get('revision','') != params.get('baseRevision',''): raise ValueError('目标设备 API 设置已变化，请重新读取')
            if old and old['connection']['sourceId'] != request['sourceId']: raise ValueError('API 连接不属于所选设备目录')
            key = params.get('apiKey') or old.get('apiKey')
            if not key or any(c in key for c in ('\n','\r','\0')): raise ValueError('请输入有效的 API Key')
            direct_store(root, c, key)
            return dict(saved=True, configRevision=revision(home))
        saved = direct_latest(root, params['id']); c = saved['connection']
        if c['sourceId'] != request['sourceId']: raise ValueError('API 连接不属于所选设备目录')
        if action == 'probe':
            req=urllib.request.Request(c['baseUrl'].rstrip('/')+'/models',headers={'Authorization':'Bearer '+saved['apiKey']})
            class NoRedirect(urllib.request.HTTPRedirectHandler):
                def redirect_request(self, *args): return None
            try:
                with urllib.request.build_opener(NoRedirect()).open(req,timeout=20) as response: status=response.code
            except urllib.error.HTTPError as error: status=error.code
            return dict(reachable=200<=status<300,httpStatus=status,message='API 认证可用' if status<300 else '模型列表查询未成功；部分服务不支持此接口')
        if action == 'remove':
            if c['revision'] != params.get('revision'): raise ValueError('API 设置已变化，请刷新后重试')
            if direct_selected(home).get('connectionId') == c['id']: raise ValueError('请先切换到账户或其他 API，再移除此连接')
            if direct_running(home): raise ValueError('此目录仍有 Codex 会话，退出后再移除 API 连接')
            identifier = ident(c['id'])
            belongs = lambda name: name.startswith('aieyes_api_') and name[len('aieyes_api_'):].rsplit('_', 1)[0] == identifier
            legacy = root/'connections'/(identifier+'.json')
            last = legacy.exists() and len(list((root/'connections').glob('*.json'))) == 1
            paths = [direct_index(root, identifier), legacy]
            paths += [path for path in (root/'direct/keys').glob(identifier+'-*.json') if read(path)['connection']['id'] == identifier]
            paths += [path for path in home.glob('aieyes_api_*.config.toml') if belongs(path.name[:-len('.config.toml')])]
            files = backup_files(paths)
            cron = [line for line in cron_lines() if line.endswith(cron_tag(root))] if last else None
            expected, values = revision(home), config(home)
            providers = values.get('model_providers', {})
            names = [name for name in providers if belongs(name)]
            for name in names: del providers[name]
            try:
                if last: set_cron(root, [])
                for path in paths:
                    if path.exists(): safe(path).unlink()
                if names: write_default(home, root, values, expected)
            except Exception:
                restore_files(files)
                if cron is not None: set_cron(root, cron)
                raise
            return dict(removed=True, legacyLedger=(root/'meter.sqlite').exists())
        raise ValueError('未知 API 操作')


def dispatch(request,script,tomli):
    try: return direct_manage(request,script,tomli)
    except ValueError as error: return dict(error=str(error))
    except FileNotFoundError: return dict(error='API 文件或目标依赖缺失；请检查 Python 3 与连接配置')
    except PermissionError: return dict(error='API 文件或目标命令权限不足')
    except Exception: return dict(error='API 操作失败；请检查目标连接与配置')


if __name__=='__main__':
    root=safe(pathlib.Path(sys.argv[2]))
    if sys.argv[1]=='--serve': serve(root)
    elif sys.argv[1]=='--token':
        c=read(root/'connections'/(ident(sys.argv[3])+'.json'));ensure_started(root);print(c['localToken'])
