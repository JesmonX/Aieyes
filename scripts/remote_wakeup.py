"""Aieyes user-owned SSH deployment and headless runner (Python 3 stdlib only)."""
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
    proxy = m.get('proxy') or {}
    if proxy.get('mode') in ('direct', 'custom'):
        for key in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'http_proxy', 'https_proxy', 'all_proxy', 'NO_PROXY', 'no_proxy'):
            env.pop(key, None)
    if proxy.get('mode') == 'custom':
        for key in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'http_proxy', 'https_proxy', 'all_proxy'):
            env[key] = proxy['url']
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


def codex_rpc(m, method, params):
    env = dict(os.environ, CODEX_HOME=m['configPath'])
    for key in CLEAR_ENV:
        env.pop(key, None)
    proxy = m.get('proxy') or {}
    if proxy.get('mode') in ('direct', 'custom'):
        for key in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'http_proxy', 'https_proxy', 'all_proxy', 'NO_PROXY', 'no_proxy'):
            env.pop(key, None)
    if proxy.get('mode') == 'custom':
        env.update({k: proxy['url'] for k in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY')})
    argv = shell_argv(m, [m['binary'], 'app-server', '--stdio'])
    process = subprocess.Popen(argv, env=env, cwd=str(pathlib.Path.home()), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, start_new_session=True)
    replies = queue.Queue()
    def reader():
        for line in process.stdout:
            try:
                replies.put(json.loads(line))
            except ValueError:
                pass
    threading.Thread(target=reader, daemon=True).start()
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
        send({'id': 1, 'method': 'initialize', 'params': {'clientInfo': {'name': 'aieyes', 'version': '0.1.5'}, 'capabilities': {'experimentalApi': True}}})
        wait(1)
        send({'method': 'initialized'})
        send({'id': 2, 'method': method, 'params': params})
        return wait(2)
    finally:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()


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
    if m['provider'] == 'agy':
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
    if m['provider'] == 'agy':
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
    db.executescript('CREATE TABLE IF NOT EXISTS runs(id INTEGER PRIMARY KEY,task TEXT,occurrence TEXT UNIQUE,started INTEGER,ended INTEGER,status TEXT,model TEXT,effort TEXT); CREATE TABLE IF NOT EXISTS leases(account TEXT PRIMARY KEY,expires INTEGER);')
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
        try:
            cwd = root / 'work' / m['id']
            private_dir(cwd)
            auth(m)
            output = call(m, m['args'], cwd, 120)
            rows = []
            for line in output.splitlines():
                try:
                    rows.append(json.loads(line))
                except ValueError:
                    pass
            if any(response_ok(m['provider'], v) for v in rows):
                status = 'success'
        except ValueError as e:
            if '超时' in str(e):
                status = 'timeout'
        except Exception:
            pass
        db = sqlite3.connect(str(root / 'runner.sqlite'), timeout=3)
        try:
            with db:
                db.execute('UPDATE runs SET status=?,ended=? WHERE id=?', (status, int(time.time()), row))
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
    rows = [{'startedAt': s, 'endedAt': e, 'status': st, 'model': m, 'effort': ef} for s,e,st,m,ef in db.execute('SELECT started,ended,status,model,effort FROM runs WHERE task=? ORDER BY id DESC LIMIT 30', (identifier,))]
    db.close()
    return rows


def manage(request):
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
        private_dir(root / 'tasks')
        script = root / 'runner.py'
        old_script = script.read_text() if script.exists() else None
        atomic(script, request['script'])
        path = root / 'tasks' / (m['id'] + '.json')
        old = path.read_text() if path.exists() else None
        atomic(path, json.dumps(m))
        try:
            crontab(root)
        except Exception:
            if old_script is not None:
                atomic(script, old_script)
            else:
                script.unlink()
            if old is not None:
                atomic(path, old)
            else:
                path.unlink()
            raise
        return {'root': str(root), 'timezone': timezone()}
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
        installed = bool(m) and tag(root) in cron.stdout.decode()
        now = datetime.datetime.now().replace(second=0, microsecond=0)
        next_run = next(((now + datetime.timedelta(minutes=i)).timestamp() for i in range(1, 2881) if (now+datetime.timedelta(minutes=i)).strftime('%H:%M') in m['times']), None) if installed and m['enabled'] else None
        return {'installed': installed, 'enabled': bool(m and m['enabled']), 'timezone': timezone(), 'nextRunAt': next_run, 'history': history(root, identifier)}
    if action == 'run':
        if not path.exists():
            raise ValueError('部署文件不存在')
        subprocess.Popen([sys.executable, str(root/'runner.py'), '--tick', str(root), identifier], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        return {'started': True}
    if action in ('enable', 'disable'):
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
            print(json.dumps({'error': str(error) if isinstance(error, ValueError) else '目标依赖或任务文件不可用；请检查 Python 3、crontab 和 CLI'}, ensure_ascii=False))
        except Exception:
            print(json.dumps({'error': '远程任务操作失败，请检查目标机器连接和运行环境'}, ensure_ascii=False))
