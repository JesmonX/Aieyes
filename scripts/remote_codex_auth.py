"""Aieyes Codex auth protocol v1, Linux/SSH only; credentials never leave this host.

Also imported by the deployed wake runner. Only Python standard-library dependencies.
"""
import base64
import contextlib
import hashlib
import json
import os
import pathlib
import queue
import re
import signal
import subprocess
import sys
import threading
import time
import uuid

VERSION = 1


def identifier(value):
    if not isinstance(value, str) or not re.fullmatch(r'[A-Za-z0-9-]{1,80}', value):
        raise ValueError('管理标识无效')
    return value


def safe(path):
    path = pathlib.Path(path)
    if '..' in path.parts:
        raise ValueError('管理路径不能包含上级目录')
    for p in (path, *path.parents):
        if p.is_symlink():
            raise ValueError('管理路径不能包含符号链接')
    return path


def private(path):
    path = safe(path)
    if not path.exists() and ".aieyes" in path.parent.parts:
        private(path.parent)
    path.mkdir(parents=True, exist_ok=True, mode=0o700)
    if path.stat().st_uid != os.getuid():
        raise ValueError('管理目录不属于当前用户')
    path.chmod(0o700)
    return path


def read(path):
    with safe(path).open('rb') as f:
        raw = f.read(1024 * 1024 + 1)
    if len(raw) > 1024 * 1024:
        raise ValueError('管理记录过大')
    try:
        return json.loads(raw)
    except (ValueError, UnicodeError):
        raise ValueError('凭据或管理记录格式无效') from None


def atomic(path, value):
    path = safe(path)
    temp = path.with_name('.aieyes-' + uuid.uuid4().hex + '.tmp')
    try:
        fd = os.open(str(temp), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w') as f:
            json.dump(value, f, separators=(',', ':'), ensure_ascii=False)
            f.flush()
            os.fsync(f.fileno())
        os.replace(temp, path)
        fd = os.open(str(path.parent), os.O_RDONLY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    finally:
        with contextlib.suppress(FileNotFoundError):
            temp.unlink()


def revision(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()).hexdigest()


class Lock:
    def __init__(self, root):
        import fcntl
        path = private(root / '.aieyes/state') / 'coordinator.lock'
        fd = os.open(str(safe(path)), os.O_RDWR | os.O_CREAT, 0o600)
        self.file = os.fdopen(fd, 'r+')
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            self.file.close()
            raise ValueError('此 Codex 目录有正在执行的账号操作，请稍后重试') from None

    def __enter__(self):
        return self

    def __exit__(self, *args):
        # Closing, rather than explicit LOCK_UN, preserves a forked login worker's lock.
        self.file.close()


def claims(token):
    try:
        payload = token.split('.')[1]
        return json.loads(base64.urlsafe_b64decode(payload + '=' * (-len(payload) % 4)))
    except (ValueError, IndexError, UnicodeError):
        return {}


def identity(auth):
    if auth.get('auth_mode') == 'apikey':
        raise ValueError('需要 ChatGPT 订阅登录')
    tokens = auth.get('tokens') or {}
    access = tokens.get('access_token')
    if not access:
        raise ValueError('缺少 ChatGPT 访问凭据')
    a, i = claims(access), claims(tokens.get('id_token', ''))
    meta, imeta = a.get('https://api.openai.com/auth', {}), i.get('https://api.openai.com/auth', {})
    workspace = tokens.get('account_id') or meta.get('chatgpt_account_id') or imeta.get('chatgpt_account_id')
    user = meta.get('chatgpt_user_id') or imeta.get('chatgpt_user_id') or i.get('sub')
    if not user or not workspace:
        raise ValueError('无法确认 ChatGPT 用户与工作区，请重新登录')
    return dict(key=hashlib.sha256((user + '\0' + workspace).encode()).hexdigest(), workspace=workspace,
                email=i.get('email') or a.get('https://api.openai.com/profile', {}).get('email', ''),
                plan=meta.get('chatgpt_plan_type') or imeta.get('chatgpt_plan_type') or 'unknown')


def external(auth):
    i = identity(auth)
    return dict(type='chatgptAuthTokens', accessToken=auth['tokens']['access_token'], chatgptAccountId=i['workspace'], chatgptPlanType=i['plan'])


def profile_path(root, profile):
    return root / '.aieyes/accounts' / identifier(profile)


def profiles(root):
    folder = safe(root / '.aieyes/accounts')
    if not folder.exists():
        return []
    return sorted((read(p / 'metadata.json') for p in folder.iterdir() if (p / 'metadata.json').exists()), key=lambda v: v['id'])


def profile(root, profile_id):
    return read(profile_path(root, profile_id) / 'metadata.json')


def save_profile(root, profile_id, name, auth):
    folder = private(profile_path(root, profile_id))
    ident = identity(auth)
    if (folder / 'metadata.json').exists() and profile(root, profile_id)['identity']['key'] != ident['key']:
        raise ValueError('账号身份发生变化，原档案已保留')
    value = dict(id=profile_id, name=name.strip() or ident['email'].strip() or 'Codex 账户', identity=ident, updatedAt=int(time.time()))
    atomic(folder / 'auth.json', auth)
    atomic(folder / 'metadata.json', value)
    return value


def active(root):
    try:
        return identity(read(root / 'auth.json'))['key']
    except (OSError, ValueError, KeyError):
        return None


def authority(root, p):
    path = root / 'auth.json'
    if path.exists():
        auth = read(path)
        if auth.get('auth_mode') != 'apikey' and identity(auth)['key'] == p['identity']['key']:
            return path
    return profile_path(root, p['id']) / 'auth.json'


def current_auth(root, p):
    auth = read(authority(root, p))
    if identity(auth)['key'] != p['identity']['key']:
        raise ValueError('账号身份改变，已停止操作')
    return auth


def running(root):
    result = []
    for entry in pathlib.Path('/proc').iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        name = ''
        try:
            if entry.stat().st_uid != os.getuid():
                continue
            name = (entry / 'comm').read_text().strip()
            if name.lower() != 'codex' and not name.lower().startswith('codex-'):
                continue
            env = (entry / 'environ').read_bytes().split(b'\0')
            home = next((v[len(b'CODEX_HOME='):].decode() for v in env if v.startswith(b'CODEX_HOME=')), str(pathlib.Path.home() / '.codex'))
            actual = pathlib.Path(home)
            if actual != root:
                continue
            stat = (entry / 'stat').read_text().rsplit(')', 1)[1].split()
            result.append(dict(pid=int(entry.name), fingerprint=revision([entry.name, stat[19]]), name='Codex', canClose=True))
        except (OSError, ValueError, IndexError):
            # If the process remains alive but cannot be identified, do not guess ownership.
            if entry.exists() and name.lower().startswith('codex'):
                result.append(dict(pid=int(entry.name), fingerprint='unknown', name='Codex（请手动关闭）', canClose=False))
    return sorted(result, key=lambda v: v['pid'])


def close_processes(root, expected):
    actual = running(root)
    if actual != expected:
        raise ValueError('运行进程已变化，请重新检查后切换')
    if any(not p['canClose'] for p in actual):
        raise ValueError('无法确认部分进程所属目录，请手动关闭 Codex 后重试')
    for p in actual:
        os.kill(p['pid'], signal.SIGTERM)
    end = time.monotonic() + 10
    while running(root):
        if time.monotonic() >= end:
            raise ValueError('Codex 未退出或已自动重启，账号未切换')
        time.sleep(.25)


def config(root):
    path = safe(root / 'config.toml')
    if not path.exists():
        return {}
    try:
        import tomllib
    except ImportError:
        raise ValueError('账号管理需要 Python 3.11 或更新版本来读取 Codex 配置') from None
    try:
        return tomllib.loads(path.read_text())
    except ValueError:
        raise ValueError('Codex 配置格式无效') from None


def mode(root):
    c = config(root)
    if 'profile' in c:
        raise ValueError('请先移除 home 配置的默认 profile，避免认证策略歧义')
    return c.get('cli_auth_credentials_store', 'file')


def policy(root, auth):
    c = config(root)
    if c.get('forced_login_method', 'chatgpt') != 'chatgpt':
        raise ValueError('此 home 限制了登录方式')
    if c.get('forced_chatgpt_workspace_id', identity(auth)['workspace']) != identity(auth)['workspace']:
        raise ValueError('此 home 限制了 ChatGPT 工作区')


def require_file(root):
    if mode(root) != 'file':
        raise ValueError('此 home 使用 keyring/auto 或其他认证存储，不能直接切换 auth.json')


def layout(root, profile_id, kind):
    return private(root / '.aieyes/codex' / identifier(profile_id) / kind)


class Rpc:
    def __init__(self, loc, root, managed=False):
        env = dict(os.environ, CODEX_HOME=str(root))
        for key in ('OPENAI_API_KEY', 'CODEX_API_KEY', 'CODEX_ACCESS_TOKEN', 'CHATGPT_ACCESS_TOKEN'):
            env.pop(key, None)
        proxy = loc.get('proxy') or {}
        if proxy.get('mode') in ('custom', 'direct'):
            for key in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'http_proxy', 'https_proxy', 'all_proxy', 'NO_PROXY', 'no_proxy'):
                env.pop(key, None)
        if proxy.get('mode') == 'custom':
            env.update({key: proxy['url'] for key in ('HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY')})
        binary = os.path.expanduser(loc.get('binary') or 'codex')
        if not managed:
            try:
                version = subprocess.check_output([binary, '--version'], timeout=5, stderr=subprocess.DEVNULL).decode()
                match = re.search(r'(\d+)\.(\d+)\.\d+', version)
                if not match or (int(match[1]), int(match[2])) < (0, 99):
                    raise ValueError('接口不兼容：内存凭据查询需要 Codex 0.99.0 或更新版本')
            except (OSError, subprocess.SubprocessError):
                raise ValueError('接口不兼容：无法确认 Codex CLI 版本') from None
        self.child = subprocess.Popen([binary, '-c', 'cli_auth_credentials_store="%s"' % ('file' if managed else 'ephemeral'), 'app-server'], cwd=root, env=env,
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, start_new_session=True)
        self.messages, self.notifications, self.next = queue.Queue(maxsize=64), [], 0
        def reader():
            try:
                while True:
                    line = self.child.stdout.readline(4 * 1024 * 1024 + 1)
                    if not line or len(line) > 4 * 1024 * 1024:
                        break
                    try:
                        self.messages.put(json.loads(line))
                    except ValueError:
                        continue
            finally:
                self.messages.put(None)
        threading.Thread(target=reader, daemon=True).start()
        try:
            self.call('initialize', dict(clientInfo=dict(name='aieyes', title='Aieyes', version='0.1.21'), capabilities=dict(experimentalApi=True)))
            self.send(dict(method='initialized'))
        except Exception:
            self.close()
            raise

    def send(self, value):
        self.child.stdin.write(json.dumps(value, separators=(',', ':')) + '\n')
        self.child.stdin.flush()

    def receive(self, timeout=30):
        if self.notifications:
            return self.notifications.pop(0)
        return self.wire(timeout)

    def wire(self, timeout):
        try:
            value = self.messages.get(timeout=max(timeout, 0))
        except queue.Empty:
            raise TimeoutError('Codex 响应超时') from None
        if value is None:
            raise ValueError('Codex 连接已断开')
        return value

    def call(self, method, params, refresh=None):
        self.next += 1
        request = self.next
        self.send(dict(id=request, method=method, params=params))
        deadline = time.monotonic() + 30
        while True:
            v = self.wire(deadline - time.monotonic())
            if 'method' in v:
                if 'id' in v:
                    result = refresh(v.get('params') or {}) if refresh and v['method'] == 'account/chatgptAuthTokens/refresh' else None
                    self.send(dict(id=v['id'], result=result) if result else dict(id=v['id'], error=dict(code=-32000, message='Credential refresh deferred by host')))
                elif len(self.notifications) < 64:
                    self.notifications.append(v)
                continue
            if v.get('id') != request:
                continue
            if 'error' in v:
                text = str(v['error'].get('message', '')).lower()
                code = v['error'].get('code', -32000)
                kind = '认证已失效' if any(s in text for s in ('401', 'unauthorized', 'refresh token', 'not authenticated', 'not logged in')) else ('接口不兼容' if code in (-32601, -32602) or 'experimental' in text else '请求失败')
                raise ValueError('Codex %s：%s（%s）' % (method, kind, code))
            return v.get('result')

    def login(self, auth):
        self.call('account/login/start', external(auth))
        response = self.call('account/read', dict(refreshToken=False))
        a, ident = response.get('account') or {}, identity(auth)
        if a.get('type') != 'chatgpt':
            raise ValueError('查询进程未使用 ChatGPT 登录')
        workspace = (response.get('workspaceRouting') or {}).get('chatgptAccountId')
        if workspace and workspace != ident['workspace'] or a.get('email') and ident['email'] and a['email'].lower() != ident['email'].lower():
            raise ValueError('查询用户或工作区与档案不一致')
        return response

    def close(self):
        if self.child.poll() is None:
            with contextlib.suppress(ProcessLookupError):
                os.killpg(self.child.pid, signal.SIGKILL)
        self.child.wait(timeout=5)
        self.child.stdin.close()
        self.child.stdout.close()

    def __enter__(self):
        return self

    def __exit__(self, *args):
        self.close()


def recover(root):
    journal = root / '.aieyes/state/refresh.json'
    if not journal.exists():
        return
    record = read(journal)
    p = profile(root, record['profileId'])
    auth = read(layout(root, p['id'], 'managed') / 'auth.json')
    if identity(auth)['key'] != p['identity']['key']:
        raise ValueError('续期恢复身份冲突，请重新登录')
    target = root / 'auth.json' if record['current'] else profile_path(root, p['id']) / 'auth.json'
    original = read(target)
    if revision(original) not in (record['revision'], revision(auth)):
        raise ValueError('凭据在续期期间被外部修改；已保留两份结果，请重新登录解除冲突')
    if record['current'] and running(root):
        raise ValueError('续期待恢复，请先关闭此位置的 Codex')
    atomic(target, auth)
    save_profile(root, p['id'], p['name'], auth)
    journal.unlink()


def renew(loc, root, p):
    recover(root)
    is_current = active(root) == p['identity']['key']
    if is_current and running(root):
        raise ValueError('当前账号由运行中的 Codex 续期；暂时显示缓存额度')
    auth = current_auth(root, p)
    managed = layout(root, p['id'], 'managed')
    atomic(managed / 'auth.json', auth)
    atomic(root / '.aieyes/state/refresh.json', dict(profileId=p['id'], current=is_current, revision=revision(auth)))
    try:
        with Rpc(loc, managed, True) as rpc:
            rpc.call('account/read', dict(refreshToken=True))
    finally:
        recover(root)
    return current_auth(root, p)


def quota(loc, root, profile_id):
    with Lock(root):
        require_file(root)
        recover(root)
        p = profile(root, profile_id)
        active_before = active(root)
        auth = current_auth(root, p)
        query = layout(root, profile_id, 'query')
        def attempt(auth):
            with Rpc(loc, query) as rpc:
                rpc.login(auth)
                def refresh(params):
                    if params.get('previousAccountId') not in (None, p['identity']['workspace']):
                        raise ValueError('刷新请求工作区不一致')
                    latest = current_auth(root, p)
                    if latest['tokens']['access_token'] == auth['tokens']['access_token']:
                        return None
                    value = external(latest)
                    value.pop('type')
                    return value
                result = rpc.call('account/rateLimits/read', None, refresh)
                if active(root) != active_before:
                    raise ValueError('日常账号已改变，已丢弃旧查询结果')
                return result
        try:
            return attempt(auth)
        except ValueError as error:
            latest = current_auth(root, p)
            if revision(latest) != revision(auth):
                return attempt(latest)
            if active_before == p['identity']['key'] and running(root):
                raise
            if not any(s in str(error) for s in ('认证已失效', '接口不兼容')):
                raise
            fresh = renew(loc, root, p)
            try:
                return attempt(fresh)
            except ValueError as second:
                if '接口不兼容' not in str(second):
                    raise
                if active(root) == p['identity']['key'] and running(root):
                    raise ValueError('当前账号暂时显示缓存额度') from None
                managed = layout(root, profile_id, 'managed')
                atomic(managed / 'auth.json', fresh)
                atomic(root / '.aieyes/state/refresh.json', dict(profileId=p['id'], current=active(root) == p['identity']['key'], revision=revision(fresh)))
                try:
                    with Rpc(loc, managed, True) as rpc:
                        return rpc.call('account/rateLimits/read', None)
                finally:
                    recover(root)


def operation(root, op):
    return root / '.aieyes/state' / (identifier(op) + '.json')


def login_worker(loc, root, op, name, replacement=None, before=None):
    profile_id = uuid.uuid4().hex
    managed = layout(root, profile_id, 'managed')
    cancelled = root / '.aieyes/state' / (op + '.cancel')
    try:
        with Rpc(loc, managed, True) as rpc:
            auth = rpc.call('account/login/start', dict(type='chatgptDeviceCode'))
            url = auth.get('verificationUrl') or ''
            from urllib.parse import urlparse
            parsed = urlparse(url)
            if parsed.scheme != 'https' or parsed.hostname not in ('auth.openai.com', 'chatgpt.com', 'auth0.openai.com'):
                raise ValueError('CLI 未提供受支持的设备码登录地址，请升级 CLI 或在远端手动登录后接入')
            atomic(operation(root, op), dict(operationId=op, status='awaitingAuthorization', authUrl=url, userCode=auth.get('userCode'), startedAt=int(time.time())))
            deadline = time.monotonic() + 600
            while True:
                if cancelled.exists():
                    rpc.call('account/login/cancel', dict(loginId=auth['loginId']))
                    raise ValueError('登录已取消')
                if time.monotonic() >= deadline:
                    raise ValueError('登录等待超时，请重试')
                try:
                    v = rpc.receive(.25)
                except TimeoutError:
                    continue
                if v.get('method') == 'account/login/completed' and v.get('params', {}).get('loginId') == auth['loginId']:
                    if not v['params'].get('success'):
                        raise ValueError('登录未成功，请重试')
                    break
            response = rpc.call('account/read', dict(refreshToken=False))
            if (response.get('account') or {}).get('type') != 'chatgpt':
                raise ValueError('需要 ChatGPT 登录')
        credential = read(managed / 'auth.json')
        ident = identity(credential)
        policy(root, credential)
        with Rpc(loc, layout(root, profile_id, 'query')) as rpc:
            rpc.login(credential)
            rpc.call('account/rateLimits/read', None)
        if replacement:
            if replacement['identity']['key'] != ident['key']:
                raise ValueError('重新登录必须使用原用户与工作区')
            if snapshot(root) != before:
                raise ValueError('授权期间凭据已改变，原凭据已保留，请重试')
            if active(root) == ident['key']:
                if running(root):
                    raise ValueError('请关闭日常 Codex 后重新授权')
            original = current_auth(root, replacement)
            atomic(layout(root, replacement['id'], 'managed') / 'auth.json', credential)
            atomic(root / '.aieyes/state/refresh.json', dict(profileId=replacement['id'], current=active(root)==ident['key'], revision=revision(original)))
            recover(root)
            p = save_profile(root, replacement['id'], name, credential)
        else:
            if any(p['identity']['key'] == ident['key'] for p in profiles(root)):
                raise ValueError('此用户与工作区已有档案，请使用重新登录')
            p = save_profile(root, profile_id, name, credential)
        state = dict(operationId=op, status='succeeded', result=dict(profile=p), finishedAt=int(time.time()))
    except Exception as error:
        state = dict(operationId=op, status='cancelled' if cancelled.exists() else 'failed', error=public_error(error), finishedAt=int(time.time()))
    import shutil
    shutil.rmtree(safe(root / '.aieyes/codex' / profile_id), ignore_errors=True)
    atomic(operation(root, op), state)


def public_error(error):
    return str(error) if isinstance(error, (ValueError, TimeoutError)) else '账号操作失败，请检查权限、CLI 与连接；凭据内容未写入诊断'


def snapshot(root):
    try:
        auth = revision(read(root / 'auth.json'))
    except FileNotFoundError:
        auth = None
    return dict(auth=auth, profiles=[dict(id=p['id'], auth=revision(read(profile_path(root, p['id']) / 'auth.json'))) for p in profiles(root)])


def call(request):
    if request.get('version') != VERSION:
        raise ValueError('账号助手版本不兼容')
    loc, method, params = request['location'], request['method'], request.get('params') or {}
    root = safe(pathlib.Path(os.path.expanduser(loc['path'])))
    if not root.is_dir():
        raise ValueError('Codex 目录不存在')
    root = root.resolve()
    if method == 'status':
        active_key = active(root)
        rows = []
        for p in profiles(root):
            try:
                available = identity(current_auth(root, p))['key'] == p['identity']['key']
            except (OSError, ValueError, KeyError):
                available = False
            rows.append(dict(id=p['id'], identityKey=p['identity']['key'], credential=available))
        return dict(storageMode=mode(root), currentIdentity=active_key, currentCredential=active_key is not None, profiles=rows)
    if method in ('inspect', 'list'):
        rows = profiles(root)
        for p in rows:
            p['current'] = active(root) == p['identity']['key']
        try:
            version = subprocess.check_output([os.path.expanduser(loc.get('binary') or 'codex'), '--version'], timeout=5, stderr=subprocess.DEVNULL).decode().strip()[:80]
        except (OSError, subprocess.SubprocessError):
            version = None
        return dict(protocolVersion=1, path=str(root), version=version, storageMode=mode(root), currentIdentity=active(root), profiles=rows, processes=running(root))
    if method == 'model.list':
        with Lock(root):
            require_file(root)
            recover(root)
            p = profile(root, params['profileId'])
            with Rpc(loc, layout(root, p['id'], 'query')) as rpc:
                info = rpc.login(current_auth(root, p))
                info['models'] = rpc.call('model/list', dict(includeHidden=False))
                return info
    if method == 'quota':
        return quota(loc, root, params['profileId'])
    if method == 'login.status':
        op = identifier(params['operationId'])
        state = read(operation(root, op))
        if state['status'] in ('running', 'awaitingAuthorization'):
            try:
                with Lock(root):
                    state = dict(operationId=op, status='failed', error='登录进程已退出，请重新登录')
                    atomic(operation(root, op), state)
            except ValueError:
                pass
        return state
    if method == 'login.cancel':
        op = identifier(params['operationId'])
        read(operation(root, op))
        atomic(root / '.aieyes/state' / (op + '.cancel'), True)
        return dict(cancelRequested=True)
    with Lock(root):
        if method == 'enable':
            require_file(root)
            marker = root / '.aieyes/state/home.json'
            if not marker.exists():
                atomic(marker, dict(homeId=uuid.uuid4().hex, version=1))
            return read(marker)
        replacement = None
        if method == 'login.start' and params.get('profileId'):
            require_file(root)
            replacement = profile(root, params['profileId'])
            if active(root) == replacement['identity']['key'] and running(root):
                raise ValueError('请先关闭日常 Codex，再重新登录')
            journal = root / '.aieyes/state/refresh.json'
            if journal.exists() and read(journal)['profileId'] != replacement['id']:
                raise ValueError('请先重新登录发生续期冲突的账号')
        else:
            recover(root)
        if method == 'login.start':
            require_file(root)
            before = snapshot(root)
            name = (params.get('name') or '').strip()
            if len(name) > 100:
                raise ValueError('账号名称不能超过 100 字')
            op = uuid.uuid4().hex
            atomic(operation(root, op), dict(operationId=op, status='running', startedAt=int(time.time())))
            pid = os.fork()
            if pid == 0:
                os.setsid()
                fd = os.open(os.devnull, os.O_RDWR)
                for target in (0, 1, 2):
                    os.dup2(fd, target)
                if fd > 2:
                    os.close(fd)
                try:
                    login_worker(loc, root, op, name, replacement, before)
                finally:
                    os._exit(0)
            return dict(operationId=op, status='running')
        if method == 'adopt':
            require_file(root)
            auth = read(root / 'auth.json')
            ident = identity(auth)
            old = next((p for p in profiles(root) if p['identity']['key'] == ident['key']), None)
            if old:
                return dict(profile=old)
            with Rpc(loc, layout(root, 'verify', 'query')) as rpc:
                rpc.login(auth)
                policy(root, auth)
                rpc.call('account/rateLimits/read', None)
            if revision(read(root / 'auth.json')) != revision(auth):
                raise ValueError('登录状态已改变，请重新读取')
            return dict(profile=save_profile(root, uuid.uuid4().hex, params.get('name') or '', auth))
        if method == 'switch.prepare':
            require_file(root)
            p = profile(root, params['profileId'])
            op = uuid.uuid4().hex
            value = dict(operationId=op, status='prepared', profileId=p['id'], revision=revision(snapshot(root)), processes=running(root), expiresAt=int(time.time()) + 120)
            atomic(operation(root, op), value)
            return value
        if method == 'switch.commit':
            require_file(root)
            op = identifier(params['operationId'])
            record = read(operation(root, op))
            if record['status'] == 'succeeded':
                return record
            if record['status'] == 'committing':
                if revision(read(root / 'auth.json')) == record['targetRevision']:
                    record['status'] = 'succeeded'
                    atomic(operation(root, op), record)
                    return record
                raise ValueError('上次切换未完成，请重新检查当前账号')
            if record['status'] != 'prepared' or record['expiresAt'] < time.time():
                raise ValueError('切换检查已过期，请重新检查')
            if record['revision'] != revision(snapshot(root)):
                raise ValueError('凭据已更新，请重新检查后切换')
            if record['processes'] and not params.get('closeProcesses'):
                raise ValueError('Codex 正在运行，尚未授权关闭')
            close_processes(root, record['processes'])
            p = profile(root, record['profileId'])
            auth = current_auth(root, p)
            policy(root, auth)
            with Rpc(loc, layout(root, p['id'], 'query')) as rpc:
                rpc.login(auth)
                rpc.call('account/rateLimits/read', None)
            if running(root):
                raise ValueError('Codex 已重新启动，账号未切换')
            if record['revision'] != revision(snapshot(root)):
                raise ValueError('验证期间凭据已改变，请重新检查后切换')
            old = read(root / 'auth.json') if (root / 'auth.json').exists() else None
            if old:
                ident = identity(old)
                prior = next((p for p in profiles(root) if p['identity']['key'] == ident['key']), None)
                save_profile(root, prior['id'] if prior else uuid.uuid4().hex, prior['name'] if prior else '之前的 ChatGPT', old)
            record.update(status='committing', targetRevision=revision(auth))
            atomic(operation(root, op), record)
            if (read(root / 'auth.json') if (root / 'auth.json').exists() else None) != old:
                raise ValueError('外部凭据已变化，账号未切换')
            atomic(root / 'auth.json', auth)
            record.update(status='succeeded', message='账号已切换，请重新打开 Codex 并恢复会话')
            atomic(operation(root, op), record)
            return record
        if method == 'profiles.remove':
            import shutil
            p = profile(root, params['profileId'])
            if active(root) == p['identity']['key']:
                raise ValueError('不能移除日常使用中的账号，请先切换')
            shutil.rmtree(safe(profile_path(root, p['id'])))
            runtime = safe(root / '.aieyes/codex' / p['id'])
            if runtime.exists():
                shutil.rmtree(runtime)
            return dict(removed=True)
        raise ValueError('未知账号操作')



@contextlib.contextmanager
def with_credentials(loc, profile_id):
    root = safe(pathlib.Path(os.path.expanduser(loc['path']))).resolve()
    with Lock(root):
        require_file(root)
        recover(root)
        p = profile(root, profile_id)
        is_current = active(root) == p['identity']['key']
        if is_current and running(root):
            raise ValueError('日常 Codex 正在使用此账号，已跳过唤醒')
        auth = current_auth(root, p)
        managed = layout(root, profile_id, 'managed')
        atomic(managed / 'auth.json', auth)
        atomic(root / '.aieyes/state/refresh.json', dict(profileId=profile_id, current=is_current, revision=revision(auth)))
        try:
            yield managed
        finally:
            recover(root)

if __name__ == '__main__':
    try:
        output = dict(version=VERSION, result=call(json.loads(sys.argv[1])))
    except Exception as error:
        output = dict(version=VERSION, error=public_error(error))
    print(json.dumps(output, separators=(',', ':'), ensure_ascii=False))
