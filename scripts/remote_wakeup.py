"""Aieyes user-owned SSH deployment and headless runner (Python 3 stdlib only)."""
from contextlib import closing, contextmanager
import datetime
import concurrent.futures
import hashlib
import json
import os
import pathlib
import queue
import re
import shutil
import signal
import sqlite3
import subprocess
import sys
import threading
import time

CLEAR_ENV = ('OPENAI_API_KEY', 'CODEX_API_KEY', 'ANTHROPIC_API_KEY', 'ANTHROPIC_AUTH_TOKEN', 'GEMINI_API_KEY', 'GOOGLE_API_KEY', 'CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX', 'CLAUDE_CODE_USE_FOUNDRY')
CLI_READY = b'AIEYES_WAKEUP_CLI_READY'
CLIENT_VERSION = globals().get('_aieyes_client_version', 'development')


def shell_argv(m, argv, diagnostic=False):
    import shlex
    if not m.get('preCommand', '').strip():
        return argv
    # Keep banners out of CLI JSON/help and keep initialization from consuming RPC input.
    script = 'set -e\n{\n' + m['preCommand'] + '\n} </dev/null >&2\n'
    if diagnostic:
        script += "printf '\\nAIEYES_WAKEUP_CLI_READY\\n' >&2\n"
    return [m.get('shell') or '/bin/bash', '-c', script + 'exec ' + ' '.join(shlex.quote(a) for a in argv)]


def call_stage(args):
    if args == ['--help']:
        return '读取 CLI 帮助'
    if args == ['exec', '--help']:
        return '检查 Codex exec 能力'
    if args[:2] == ['auth', 'status']:
        return '检查订阅登录'
    if args == ['models']:
        return '读取模型列表'
    return 'CLI 调用'


def private_dir(path):
    path.mkdir(parents=True, exist_ok=True)
    if path.is_symlink():
        raise ValueError('任务目录不能是符号链接')
    path.chmod(0o700)


def atomic(path, data):
    private_dir(path.parent)
    temp = path.with_name(path.name + '.%d.%d.tmp' % (os.getpid(), time.time_ns()))
    try:
        fd = os.open(str(temp), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w') as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        os.replace(str(temp), str(path))
    finally:
        if temp.exists():
            temp.unlink()


def call(m, args, cwd=None, timeout=20):
    env = dict(os.environ)
    for key in CLEAR_ENV:
        env.pop(key, None)
    env['PATH'] = os.path.expanduser('~/.local/bin:~/bin:~/.codex/packages/standalone/current/bin').replace(':~/', ':' + os.path.expanduser('~/')) + ':/usr/local/bin:/usr/bin:/bin:' + env.get('PATH', '')
    if m['provider'] == 'codex':
        env['CODEX_HOME'] = m['configPath']
    if m['provider'] == 'claude':
        env['CLAUDE_CONFIG_DIR'] = m['configPath']
    # Legacy remote proxy fields are ignored. The account's pre-command owns
    # initialization, including whether to inherit, clear or replace proxy env.
    argv = shell_argv(m, [m['binary']] + args, diagnostic=True)
    process = subprocess.Popen(argv, cwd=str(cwd or pathlib.Path.home()), env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
    try:
        output, errors = process.communicate(timeout=timeout)
        # Report only fixed labels and exit codes; never expose command output or credentials.
        if m.get('preCommand', '').strip() and CLI_READY not in errors.splitlines():
            raise ValueError('%s：前置命令未完成（退出码 %s）；请检查脚本、shell 及非交互环境' % (call_stage(args), process.returncode))
        if process.returncode:
            raise ValueError('%s失败（退出码 %s）；请检查 CLI 路径、运行环境及配置' % (call_stage(args), process.returncode))
        if len(output) > 4 * 1024 * 1024:
            raise ValueError('CLI 输出超过限制')
        return output.decode('utf-8')
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.communicate()
        raise ValueError('CLI 调用超时')


def auth_helper():
    global _AUTH_HELPER
    if '_AUTH_HELPER' not in globals():
        import importlib.util
        spec = importlib.util.spec_from_file_location('aieyes_codex_auth', pathlib.Path(__file__).with_name('codex_auth.py'))
        _AUTH_HELPER = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(_AUTH_HELPER)
    return _AUTH_HELPER


def auth_location(m):
    return dict(path=m['configPath'], binary=m['binary'], proxy={'mode': 'system'},
                preCommand=m.get('preCommand', ''), shell=m.get('shell') or '/bin/bash')


def codex_rpc(m, method, params):
    if m.get('codexProfileId'):
        info = auth_helper().call(dict(version=1, location=auth_location(m), method='model.list', params=dict(profileId=m['codexProfileId'].split(':', 1)[1])))
        return info['models'] if method == 'model/list' else info
    env = dict(os.environ, CODEX_HOME=m['configPath'])
    for key in CLEAR_ENV:
        env.pop(key, None)
    # Stdio is the default, including CLI versions without the --stdio alias.
    argv = shell_argv(m, [m['binary'], 'app-server'])
    process = subprocess.Popen(argv, env=env, cwd=str(pathlib.Path.home()), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, start_new_session=True)
    replies = queue.Queue()
    def reader():
        for line in process.stdout:
            try:
                replies.put(json.loads(line))
            except ValueError:
                pass
    reader_thread = threading.Thread(target=reader, daemon=True)
    reader_thread.start()
    def send(v):
        process.stdin.write(json.dumps(v) + '\n')
        process.stdin.flush()
    def wait(identifier):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            v = replies.get(timeout=max(0.01, deadline - time.monotonic()))
            if v.get('id') == identifier:
                if 'error' in v:
                    raise ValueError('Codex 能力查询失败')
                return v.get('result', {})
        raise ValueError('Codex 能力查询超时')
    try:
        send({'id': 1, 'method': 'initialize', 'params': {'clientInfo': {'name': 'aieyes', 'version': CLIENT_VERSION}, 'capabilities': {'experimentalApi': True}}})
        wait(1)
        send({'method': 'initialized'})
        send({'id': 2, 'method': method, 'params': params})
        return wait(2)
    finally:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        reader_thread.join()
        process.stdin.close()
        process.stdout.close()


def resolve(m):
    m = dict(m)
    m['configPath'] = os.path.expanduser(m['configPath'])
    binary = os.path.expanduser(m['binary'])
    candidates = [binary, os.path.expanduser('~/.local/bin/' + binary), os.path.expanduser('~/.codex/packages/standalone/current/bin/' + binary), shutil.which(binary)]
    found = next((str(pathlib.Path(p).absolute()) for p in candidates if p and pathlib.Path(p).is_file()), None)
    if not found:
        raise ValueError('目标机器找不到 CLI；请填写可执行文件完整路径')
    m['binary'] = found
    return m


def auth(m):
    if m['provider'] == 'claude':
        v = json.loads(call(m, ['auth', 'status']))
        if not v.get('loggedIn') or v.get('authMethod') not in ('claude.ai', 'oauth_token'):
            raise ValueError('目标 Claude CLI 未使用订阅登录')
    if m['provider'] in ('agy', 'antigravity'):
        if pathlib.Path(m['configPath']) != pathlib.Path.home() / '.gemini/antigravity-cli':
            raise ValueError('agy 唤醒仅支持目标用户的默认登录目录')
        rows = [json.loads(l) for l in call(m, ['--print', '/usage', '--output-format', 'json', '--print-timeout', '20s'], timeout=25).splitlines() if l.startswith('{')]
        if not any(v.get('status') == 'SUCCESS' and v.get('command', {}).get('name') == 'usage' for v in rows):
            raise ValueError('目标 agy CLI 未提供订阅额度')


def probe(m):
    m = resolve(m)
    help_text = call(m, ['--help'])
    if '--model' not in help_text:
        raise ValueError('请升级 CLI 以支持指定模型')
    auth(m)
    models = []
    if m['provider'] == 'codex':
        account = codex_rpc(m, 'account/read', {'refreshToken': False})
        if (account.get('account') or {}).get('type') not in ('chatgpt', 'chatgptAuthTokens'):
            raise ValueError('Codex 唤醒需要 ChatGPT 订阅登录')
        if '--ignore-user-config' not in call(m, ['exec', '--help']):
            raise ValueError('请升级 Codex CLI 以支持隔离唤醒配置')
        for row in codex_rpc(m, 'model/list', {'includeHidden': False}).get('data', []):
            models.append({'id': row.get('model') or row.get('id'), 'efforts': [v['reasoningEffort'] for v in row.get('supportedReasoningEfforts', [])], 'defaultEffort': row.get('defaultReasoningEffort')})
    elif m['provider'] == 'claude' and ('--safe-mode' not in help_text or '--tools' not in help_text):
        raise ValueError('请升级 Claude Code CLI')
    if m['provider'] in ('agy', 'antigravity'):
        for line in call(m, ['models']).splitlines():
            if '\t' not in line:
                continue
            identifier = line.split('\t')[0]
            if not re.fullmatch('[a-zA-Z0-9._/-]+', identifier):
                continue
            suffix = identifier.rsplit('-', 1)[-1]
            efforts = [suffix] if suffix in ('low', 'medium', 'high', 'xhigh', 'max') else []
            models.append({'id': identifier, 'efforts': efforts, 'defaultEffort': efforts[0] if efforts else None})
    if m.get('model') and models:
        selected = next((r for r in models if r['id'] == m['model']), None)
        if not selected or (m.get('effort') and m['effort'] not in selected['efforts']):
            raise ValueError('所选模型或 effort 当前不可用')
    if m.get('effort') and m['provider'] != 'codex' and '--effort' not in help_text:
        raise ValueError('当前 CLI 不支持 effort，请选择模型默认值')
    return {'binary': m['binary'], 'models': models, 'efforts': ['low', 'medium', 'high', 'xhigh', 'max'] if '--effort' in help_text or m['provider'] == 'codex' else [], 'timezone': timezone(), 'message': '按 API 等价价格推荐；无法列出模型时请填写模型 ID，可用性将在实际运行时验证'}


def timezone():
    return datetime.datetime.now().astimezone().strftime('%Z %z')


def next_run(times, now=None, occurred=(), task_id=''):
    start = int((time.time() if now is None else now) // 60) * 60
    # Walk real minutes, not wall-clock additions that can invent a DST gap.
    return next((stamp for stamp in range(start + 60, start + 2881 * 60, 60)
                 if time.strftime('%H:%M', time.localtime(stamp)) in times
                 and task_id + ':' + time.strftime('%Y-%m-%d:%H:%M', time.localtime(stamp)) not in occurred), None)


def scheduled_occurrences(root, identifier, now):
    if not (root / 'runner.sqlite').exists():
        return set()
    with closing(sqlite3.connect((root / 'runner.sqlite').as_uri() + '?mode=ro', uri=True)) as db:
        return {row[0] for row in db.execute('SELECT occurrence FROM runs WHERE task=? AND started>=?', (identifier, now - 172800))}


def tag(root):
    return '# app.aieyes.wakeup.' + hashlib.sha256(str(root).encode()).hexdigest()[:20]


def crontab(root, remove=False):
    import shlex
    old = subprocess.run(['crontab', '-l'], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if old.returncode and not (old.returncode == 1 and (not old.stderr or b'no crontab' in old.stderr)):
        raise ValueError('无法读取已有 crontab')
    lines = [line for line in old.stdout.decode().splitlines() if not line.endswith(tag(root))]
    if not remove:
        command = ' '.join(shlex.quote(a) for a in [sys.executable, str(root / 'runner.py'), '--tick', str(root)])
        lines.append('* * * * * ' + command.replace('%', '\\%') + ' >/dev/null 2>&1 ' + tag(root))
    installed = subprocess.run(['crontab', '-'], input=('\n'.join(lines) + '\n').encode(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    if installed.returncode:
        raise ValueError('安装 crontab 失败；需要目标用户的 crontab 权限')


def tick(root, manual=None):
    private_dir(root)
    db = sqlite3.connect(str(root / 'runner.sqlite'), timeout=3)
    db.executescript('CREATE TABLE IF NOT EXISTS runs(id INTEGER PRIMARY KEY,task TEXT,occurrence TEXT UNIQUE,started INTEGER,ended INTEGER,status TEXT,model TEXT,effort TEXT); CREATE TABLE IF NOT EXISTS leases(account TEXT PRIMARY KEY,expires INTEGER); CREATE TABLE IF NOT EXISTS run_diagnostics(run_id INTEGER PRIMARY KEY,stage TEXT,code TEXT,message TEXT);')
    local = datetime.datetime.now()
    slot = local.strftime('%H:%M')
    jobs = []
    for path in sorted((root / 'tasks').glob('*.json')):
        m = json.loads(path.read_text())
        if manual and m['id'] != manual:
            continue
        if not manual and (not m['enabled'] or slot not in m['times'] or datetime.datetime.now().strftime('%H:%M') != slot):
            continue
        occurrence = m['id'] + (':manual:%d' % time.time_ns() if manual else ':' + local.strftime('%Y-%m-%d') + ':' + slot)
        stamp = int(time.time())
        with db:
            db.execute('BEGIN IMMEDIATE')
            db.execute("UPDATE runs SET status='interrupted',ended=? WHERE status='running' AND started<?", (stamp, stamp-150))
            db.execute('DELETE FROM leases WHERE expires<?', (stamp,))
            if db.execute('SELECT 1 FROM runs WHERE occurrence=?', (occurrence,)).fetchone():
                continue
            acquired = db.execute('INSERT OR IGNORE INTO leases VALUES(?,?)', (m['accountKey'], stamp+150)).rowcount > 0
            row = db.execute('INSERT INTO runs(task,occurrence,started,status,model,effort) VALUES(?,?,?,?,?,?)', (m['id'], occurrence, stamp, 'running' if acquired else 'skipped-overlap', m['model'], m['effort'])).lastrowid
        if not acquired:
            continue
        jobs.append((m, row))
    db.close()

    def execute(job):
        m, row = job
        status = 'failed'
        stage, detail = 'prepare', None
        try:
            cwd = root / 'work' / m['id']
            private_dir(cwd)
            stage = 'authentication'
            auth(m)
            stage = 'model-request'
            if m.get('codexProfileId'):
                stage = 'codex-turn'
                auth_helper().wakeup(auth_location(m), m['codexProfileId'].split(':', 1)[1], m, cwd)
                output = '{"type":"turn.completed"}'
            else:
                if m['provider'] == 'codex' and (pathlib.Path(m['configPath']) / '.aieyes/state/home.json').exists():
                    raise ValueError('旧唤醒任务需绑定账号后重新部署')
                output = call(m, m['args'], cwd, 120)
            rows = []
            for line in output.splitlines():
                try:
                    rows.append(json.loads(line))
                except ValueError:
                    pass
            if any(response_ok(m['provider'], v) for v in rows):
                status = 'success'
            else:
                detail = ('missing-completion', '模型没有返回成功完成事件')
        except Exception as e:
            detail = diagnostic(e)
            if detail[0] == 'timeout':
                status = 'timeout'
        db = sqlite3.connect(str(root / 'runner.sqlite'), timeout=3)
        try:
            with db:
                db.execute('UPDATE runs SET status=?,ended=? WHERE id=?', (status, int(time.time()), row))
                if detail:
                    db.execute('INSERT OR REPLACE INTO run_diagnostics VALUES(?,?,?,?)', (row, stage, detail[0], detail[1]))
                db.execute('DELETE FROM leases WHERE account=?', (m['accountKey'],))
        finally:
            db.close()

    if jobs:
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(jobs)) as pool:
            list(pool.map(execute, jobs))


def response_ok(provider, v):
    if provider == 'codex':
        return v.get('type') == 'turn.completed'
    if provider == 'claude':
        return v.get('type') == 'result' and v.get('subtype') == 'success' and v.get('is_error') is False
    return v.get('status') == 'SUCCESS' or (v.get('event') == 'result' and v.get('result', {}).get('status') == 'SUCCESS')


def history(root, identifier):
    if not (root / 'runner.sqlite').exists():
        return []
    db = sqlite3.connect('file:' + str(root / 'runner.sqlite') + '?mode=ro', uri=True)
    details = db.execute("SELECT 1 FROM sqlite_master WHERE name='run_diagnostics'").fetchone()
    query = 'SELECT r.started,r.ended,r.status,r.model,r.effort,d.stage,d.code,d.message FROM runs r LEFT JOIN run_diagnostics d ON d.run_id=r.id WHERE r.task=? ORDER BY r.id DESC LIMIT 30' if details else 'SELECT started,ended,status,model,effort,NULL,NULL,NULL FROM runs WHERE task=? ORDER BY id DESC LIMIT 30'
    rows = [dict(startedAt=s, endedAt=e, status=st, model=m, effort=ef, stage=stage, errorCode=code, message=message) for s,e,st,m,ef,stage,code,message in db.execute(query, (identifier,))]
    db.close()
    return rows


def diagnostic(error):
    text = str(error)
    if isinstance(error, (TimeoutError, subprocess.TimeoutExpired)) or '超时' in text:
        return 'timeout', '调用超时；请检查目标网络与代理'
    if isinstance(error, FileNotFoundError):
        return 'dependency-missing', '任务文件或可执行文件不存在；请重新部署并检查 CLI 路径'
    if isinstance(error, PermissionError):
        return 'permission-denied', '任务文件或命令权限不足；请检查目标用户权限'
    for needle, code, message in (
        ('前置命令', 'pre-command', '前置命令失败；请检查非交互 shell 环境'),
        ('接口不兼容', 'incompatible-cli', 'CLI 接口不兼容；请升级 Codex'),
        ('正在执行的账号操作', 'account-busy', '账号管理操作进行中；本次唤醒未执行'),
        ('身份', 'identity-changed', '账号身份已变化；请检查任务绑定'),
        ('认证', 'authentication', '订阅认证不可用；请刷新登录状态后重试'),
        ('登录', 'authentication', '订阅认证不可用；请刷新登录状态后重试'),
        ('凭据', 'authentication', '订阅认证不可用；请刷新登录状态后重试')):
        if needle in text:
            return code, message
    return 'model-request', '模型请求未成功完成；请检查模型、认证与代理'


@contextmanager
def scheduler_lock(base):
    # A user has one crontab; serialize its read/modify/write across namespaces.
    # Probing/authentication happens before this short local transaction.
    private_dir(base)
    with closing(sqlite3.connect(base / 'deployment.sqlite', timeout=25)) as db:
        db.execute('BEGIN IMMEDIATE')
        try:
            yield
        finally:
            db.rollback()


def install(root, m, request):
    private_dir(root / 'tasks')
    script = root / 'runner.py'
    old_script = script.read_text() if script.exists() else None
    atomic(script, request['script'])
    helper = root / 'codex_auth.py'
    old_helper = helper.read_text() if helper.exists() else None
    if request.get('authHelper'):
        atomic(helper, request['authHelper'])
    path = root / 'tasks' / (m['id'] + '.json')
    old = path.read_text() if path.exists() else None
    atomic(path, json.dumps(m))
    try:
        crontab(root)
        check = subprocess.run(['crontab', '-l'], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        if check.returncode or tag(root) not in check.stdout.decode() or json.loads(path.read_text()) != m:
            raise ValueError('部署回读验证失败；请重新部署')
    except Exception:
        if old_helper is not None:
            atomic(helper, old_helper)
        elif helper.exists():
            helper.unlink()
        if old_script is not None:
            atomic(script, old_script)
        else:
            script.unlink()
        if old is not None:
            atomic(path, old)
        else:
            path.unlink()
        try:
            crontab(root, remove=not any((root / 'tasks').glob('*.json')))
        except Exception:
            pass
        raise
    return {'root': str(root), 'timezone': timezone()}


def manage(request):
    global _AUTH_HELPER
    if request.get('authHelper'):
        import types
        _AUTH_HELPER = types.ModuleType('aieyes_codex_auth')
        exec(compile(request['authHelper'], '<aieyes-codex-auth>', 'exec'), _AUTH_HELPER.__dict__)
    action = request['action']
    if action == 'probe':
        return probe(request['manifest'])
    base = pathlib.Path.home() / '.local/share/aieyes-wakeups'
    if action == 'deploy':
        namespace = request['namespace']
        if not re.fullmatch('[a-f0-9]{20}', namespace):
            raise ValueError('部署 ID 无效')
        if sys.platform != 'linux':
            raise ValueError('SSH 自动部署当前支持 Linux 服务器')
        root = base / namespace
        m = resolve(request['manifest'])
        probe(m)
        with scheduler_lock(base):
            return install(root, m, request)
    root = pathlib.Path(os.path.expanduser(request['root']))
    if root.parent != base or not re.fullmatch('[a-f0-9]{20}', root.name):
        raise ValueError('部署目录不属于 Aieyes')
    identifier = request['id']
    if not re.fullmatch('wake-[a-zA-Z0-9-]{1,59}', identifier):
        raise ValueError('任务 ID 无效')
    path = root / 'tasks' / (identifier + '.json')
    if action == 'history':
        return history(root, identifier)
    if action == 'status':
        m = json.loads(path.read_text()) if path.exists() else None
        cron = subprocess.run(['crontab','-l'], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        installed = bool(m) and (root / 'runner.py').is_file() and (not m.get('codexProfileId') or (root / 'codex_auth.py').is_file()) and cron.returncode == 0 and tag(root) in cron.stdout.decode()
        now = time.time()
        next_run_at = next_run(m['times'], now, scheduled_occurrences(root, identifier, now), identifier) if installed and m['enabled'] else None
        return {'installed': installed, 'enabled': bool(m and m['enabled']), 'timezone': timezone(), 'nextRunAt': next_run_at, 'history': history(root, identifier)}
    if action == 'run':
        if not path.exists():
            raise ValueError('任务文件缺失；请重新部署')
        subprocess.Popen([sys.executable, str(root/'runner.py'), '--tick', str(root), identifier], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        return {'started': True}
    if action in ('enable', 'disable', 'remove'):
        with scheduler_lock(base):
            if action in ('enable', 'disable'):
                if not path.is_file():
                    raise ValueError('任务文件缺失；请重新部署')
                m = json.loads(path.read_text())
                m['enabled'] = action == 'enable'
                atomic(path, json.dumps(m))
                return {'enabled': m['enabled']}
            if action == 'remove':
                if not any(p != path for p in (root/'tasks').glob('*.json')):
                    crontab(root, remove=True)
                if path.exists():
                    path.unlink()
                return {'removed': True}

    raise ValueError('未知部署操作')


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--tick':
        tick(pathlib.Path(sys.argv[2]), sys.argv[3] if len(sys.argv) > 3 else None)
    else:
        try:
            print(json.dumps(manage(json.loads(sys.argv[1])), ensure_ascii=False))
        except (ValueError, FileNotFoundError, PermissionError) as error:
            # Fixed messages only; do not print subprocess output/configuration.
            code, message = diagnostic(error)
            print(json.dumps({'error': str(error) if isinstance(error, ValueError) else message, 'errorCode':code}, ensure_ascii=False))
        except Exception:
            print(json.dumps({'error': '远程任务操作失败，请检查目标机器连接和运行环境'}, ensure_ascii=False))
