"""Read agy's public account summary on the login machine (also used over SSH).

No credentials are written, passed in argv, or returned by this helper. The CLI's
built-in /usage may renew its own login. curl reads authorization through stdin.
"""
import base64
import datetime
import hashlib
import hmac
import json
import os
import pathlib
import re
import sqlite3
import shutil
import subprocess
import sys
import tempfile
import time

LIMIT = 1024 * 1024
LICENSE_URL = 'https://aicode.googleapis.com/v1:fetchLicenses'
USERINFO_URL = 'https://www.googleapis.com/oauth2/v2/userinfo'
ASSIST_URL = 'https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist'
QUOTA_URL = 'https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary'
USER_AGENT = 'antigravity (aieyes read-only quota)'
RELOGIN = 'Antigravity 登录已过期，请在 agy 中重新登录'
RELOGIN_IDE = 'Antigravity IDE 登录已过期，请在 IDE 中重新登录'
IDE_INVALID = 'Antigravity IDE 登录凭据无法读取，请在 IDE 中重新登录'


class PublicError(ValueError):
    """Only fixed, credential-free diagnostics may be returned to the UI."""


def public_error(error, fallback):
    return str(error) if isinstance(error, PublicError) else fallback



def proto_fields(raw):
    """Bounded protobuf wire reader for the IDE's saved public schema only."""
    if len(raw) > LIMIT:
        raise PublicError(IDE_INVALID)
    index, out = 0, []

    def varint():
        nonlocal index
        number = 0
        for shift in range(0, 70, 7):
            if index >= len(raw):
                raise PublicError(IDE_INVALID)
            byte = raw[index]
            index += 1
            number |= (byte & 127) << shift
            if byte < 128:
                return number
        raise PublicError(IDE_INVALID)

    while index < len(raw):
        tag = varint()
        field, wire = tag >> 3, tag & 7
        if field == 0 or len(out) >= 4096:
            raise PublicError(IDE_INVALID)
        if wire == 0:
            value = varint()
        elif wire in (1, 2, 5):
            size = varint() if wire == 2 else (8 if wire == 1 else 4)
            if size > len(raw) - index:
                raise PublicError(IDE_INVALID)
            value = raw[index:index + size]
            index += size
        else:
            raise PublicError(IDE_INVALID)
        out.append((field, wire, value))
    return out


def proto_one(fields, field, wire, default=b''):
    matches = [(w, v) for f, w, v in fields if f == field]
    if len(matches) > 1 or matches and matches[0][0] != wire:
        raise PublicError(IDE_INVALID)
    return matches[0][1] if matches else default


def proto_base64(raw):
    if not isinstance(raw, (str, bytes)) or len(raw) > LIMIT:
        raise PublicError(IDE_INVALID)
    if isinstance(raw, str):
        raw = raw.encode('ascii')
    return proto_fields(base64.b64decode(raw + b'=' * (-len(raw) % 4), validate=True))


def ide_token(raw):
    # IDE 2.6.0: Topic.data[oauthTokenInfoSentinelKey].value is base64
    # OAuthTokenInfo. Its one-time legacy migration also writes JSON here.
    matches = []
    for field, wire, entry in proto_base64(raw):
        if field != 1:
            continue
        if wire != 2:
            raise PublicError(IDE_INVALID)
        entry = proto_fields(entry)
        if proto_one(entry, 1, 2) != b'oauthTokenInfoSentinelKey':
            continue
        row = proto_fields(proto_one(entry, 2, 2))
        matches.append(proto_one(row, 1, 2))
    if not matches:
        return None
    if len(matches) != 1:
        raise PublicError(IDE_INVALID)
    raw = matches[0]
    if raw.lstrip().startswith(b'{'):
        value = json.loads(raw)
        access, kind, expires = (value.get('accessToken'), value.get('tokenType'),
                                 value.get('expiryDateSeconds'))
        enterprise = value.get('isGcpTos', False)
    else:
        fields = proto_base64(raw)
        access = proto_one(fields, 1, 2).decode('utf-8')
        kind = proto_one(fields, 2, 2).decode('utf-8')
        # Field 3 is the refresh token; it is never decoded or projected.
        expiry = proto_fields(proto_one(fields, 4, 2))
        expires = proto_one(expiry, 1, 0, 0)
        enterprise = bool(proto_one(fields, 6, 0, 0))
    if not isinstance(access, str) or not access or not isinstance(expires, (int, float)):
        raise PublicError(IDE_INVALID)
    expiry = datetime.datetime.fromtimestamp(expires, datetime.timezone.utc).isoformat()
    return {'_origin': 'ide', 'auth_method': 'enterprise' if enterprise else 'consumer',
            'token': {'access_token': access, 'token_type': kind, 'expiry': expiry}}


def read_ide_credential():
    home = pathlib.Path.home()
    if sys.platform == 'darwin':
        base = home / 'Library' / 'Application Support'
    elif sys.platform == 'win32':
        base = pathlib.Path(os.environ.get('APPDATA', str(home / 'AppData' / 'Roaming')))
    else:
        base = pathlib.Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config')))
    if not base.is_absolute():
        raise PublicError(IDE_INVALID)
    # The current product's application store takes precedence over legacy IDE
    # storage, even when signed out. Custom --user-data-dir is not guessed.
    for name in ('Antigravity IDE', 'Antigravity'):
        path = base / name / 'User' / 'globalStorage' / 'state.vscdb'
        if not path.is_file():
            continue
        db = None
        try:
            db = sqlite3.connect(path.as_uri() + '?mode=ro&immutable=0', uri=True, timeout=0.5)
            db.execute('PRAGMA query_only=ON')
            row = db.execute('SELECT length(value), substr(value, 1, ?) FROM ItemTable WHERE key=?',
                             (LIMIT + 1, 'antigravityUnifiedStateSync.oauthToken')).fetchone()
            if row is None:
                return None
            if not isinstance(row[0], int) or row[0] > LIMIT:
                raise PublicError(IDE_INVALID)
            value = ide_token(row[1])
            if value is not None:
                value['_store_path'] = str(path)
            return value
        except (sqlite3.Error, ValueError, TypeError, OverflowError, OSError):
            raise PublicError(IDE_INVALID)
        finally:
            if db is not None:
                db.close()
    return None


def decode(raw):
    if len(raw) > LIMIT:
        raise PublicError('凭据格式无法识别')
    raw = raw.strip()
    if raw.startswith(b'go-keyring-base64:'):
        raw = base64.b64decode(raw.split(b':', 1)[1], validate=True)
    value = json.loads(raw)
    if not isinstance(value, dict):
        raise PublicError('凭据格式无法识别')
    return value


def identity(credential):
    if credential.get('auth_method') not in (None, '', 'consumer'):
        raise PublicError('此登录方式暂不支持账户身份识别')
    token = credential.get('id_token', '')
    parts = token.split('.') if isinstance(token, str) else []
    if len(parts) != 3:
        raise PublicError('现有登录未提供 Google 账户身份')
    claims = json.loads(base64.urlsafe_b64decode(parts[1] + '=' * (-len(parts[1]) % 4)))
    sub, email = claims.get('sub'), claims.get('email')
    # This is an observation of the trusted CLI store, not JWT authentication.
    # Expired ID tokens still identify a CLI credential refreshed via OAuth.
    if (claims.get('iss') not in ('accounts.google.com', 'https://accounts.google.com')
            or claims.get('email_verified') not in (True, 'true')
            or not isinstance(sub, str) or not sub or len(sub) > 255
            or not isinstance(email, str) or not re.fullmatch(r'[^\s@\x00-\x1f]{1,254}@[^\s@\x00-\x1f]{1,254}', email)):
        raise PublicError('现有登录未提供可靠的 Google 账户身份')
    return {'key': 'google:' + hashlib.sha256(sub.encode()).hexdigest(),
            'email': email, 'checkedAt': int(time.time()), 'stale': False}


def read_file(path):
    with path.open('rb') as stream:
        return decode(stream.read(LIMIT + 1))


def same_owner(left, right):
    try:
        return identity(left)['key'] == identity(right)['key']
    except Exception:
        # New CLI stores id_token_email, not an ID token. An email alone is
        # not an account ID; an identical OAuth grant still proves one owner.
        # The access token rotates on every refresh, so a stale file-backend
        # copy only shares the refresh token with the current keychain entry.
        # Tokens are compared locally and never projected.
        def same(name):
            mine, theirs = left.get('token', {}).get(name), right.get('token', {}).get(name)
            return (isinstance(mine, str) and isinstance(theirs, str) and bool(mine)
                    and hmac.compare_digest(mine.encode(), theirs.encode()))
        return same('access_token') or same('refresh_token')


def freshest(credentials):
    # Call only after every store has been checked for the same owner. An old
    # file-backend copy must not mask a newer grant in another CLI store.
    def expiry(value):
        try:
            access_token(value)
            raw = value.get('token', {}).get('expiry')
            return (True, datetime.datetime.fromisoformat(raw.replace('Z', '+00:00')).timestamp() if raw else 0)
        except (ValueError, TypeError, AttributeError):
            return (False, 0)
    return max(credentials, key=expiry)


def read_credential():
    if any(os.environ.get(k) for k in ('GEMINI_API_KEY', 'AGY_ADC_AUTH', 'AGY_LLM_GATEWAY_API_KEY')):
        raise PublicError('此登录方式暂不支持账户身份识别')
    # Auth belongs to the CLI's home, never the history source directory.
    root = pathlib.Path.home() / '.gemini'
    candidates = [root / 'antigravity-oauth-token',
                  root / 'antigravity-cli' / 'antigravity-oauth-token']
    files = [p for p in candidates if p.is_file()]
    # SSH/headless installations use the CLI file backend. If multiple CLI
    # generations left different accounts behind, do not guess which is active.
    file_values = [read_file(p) for p in files]
    if file_values and any(not same_owner(file_values[0], v) for v in file_values[1:]):
        raise PublicError('发现多个不同的 CLI 登录身份，请在 agy 中确认当前账户')
    bypass = bool(os.environ.get('SSH_CONNECTION') or os.environ.get('SSH_TTY'))
    if bypass and file_values:
        return freshest(file_values)
    command = (['/usr/bin/security', 'find-generic-password', '-s', 'gemini', '-a', 'antigravity', '-w']
               if sys.platform == 'darwin' else
               ['secret-tool', 'lookup', 'service', 'gemini', 'username', 'antigravity'])
    try:
        result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, timeout=4, check=False)
        if result.returncode == 0:
            if not result.stdout.strip():
                raise PublicError('无法读取 agy 登录身份，请检查凭据库或 CLI 登录状态')
            value = decode(result.stdout)
            # A recent keyring failure can make the CLI use its file backend.
            # Avoid claiming identity when those stores disagree.
            if file_values and not same_owner(value, file_values[0]):
                raise PublicError('系统凭据与 CLI 文件身份不同，请在 agy 中确认当前账户')
            return freshest([value, *file_values])
    except (OSError, subprocess.TimeoutExpired):
        pass
    if file_values:
        return freshest(file_values)
    # A present CLI store, including expired/empty/corrupt credentials, never
    # silently switches the query to an unrelated IDE account.
    value = read_ide_credential()
    if value is not None:
        return value
    raise PublicError('无法读取 agy 登录身份，请检查凭据库或 CLI 登录状态')


def subscription(value):
    licenses = value.get('licenses')
    if not isinstance(licenses, list) or not licenses or not isinstance(licenses[0], dict):
        return None
    first = licenses[0]
    # Free tier display names vary; the tier id keeps one label for them.
    if first.get('userTier') in ('free-tier', 'g1-free-tier'):
        return 'Free'
    label = first.get('tierDisplayName')
    if isinstance(label, str) and label.strip() and len(label) <= 120 and not any(ord(c) < 32 for c in label):
        return label.strip()
    return {'g1-pro-tier': 'Google AI Pro', 'g1-ultra-tier': 'Google AI Ultra'}.get(first.get('userTier'))


def access_token(credential):
    if credential.get('auth_method') not in (None, '', 'consumer'):
        raise PublicError('此登录方式暂不支持只读额度查询')
    expiry = credential.get('token', {}).get('expiry')
    if expiry:
        try:
            expires = datetime.datetime.fromisoformat(expiry.replace('Z', '+00:00')).timestamp()
        except (ValueError, TypeError, AttributeError):
            raise PublicError('Antigravity 登录有效期无法确认')
        if expires <= time.time() + 60:
            raise PublicError(RELOGIN_IDE if credential.get('_origin') == 'ide' else RELOGIN)
    token = credential.get('token', {}).get('access_token', '')
    if not isinstance(token, str) or not re.fullmatch(r'[A-Za-z0-9._~+/=-]+', token):
        raise PublicError('订阅暂不可用：现有登录缺少访问凭据')
    return token


def request_json(credential, url, body=None):
    if url not in (LICENSE_URL, USERINFO_URL, ASSIST_URL, QUOTA_URL):
        raise PublicError('Antigravity 查询地址不支持')
    token = access_token(credential)
    config = 'header = "Authorization: Bearer ' + token + '"\n'
    # Code Assist rejects unknown clients (curl/x.y) as UNSUPPORTED_CLIENT: no
    # tier or project, and the quota summary answers 403 "no valid license".
    config += 'user-agent = "' + USER_AGENT + '"\n'
    if body is not None:
        config += 'header = "Content-Type: application/json"\n'
        config += 'data = ' + json.dumps(json.dumps(body, separators=(',', ':'))) + '\n'
    try:
        result = subprocess.run(['curl', '-q', '--silent', '--fail', '--proto', '=https',
                             '--connect-timeout', '5', '--max-time', '10',
                             '--max-filesize', str(LIMIT), '--write-out', '%{stderr}%{http_code}', '--config', '-', url],
                                input=config.encode(), stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=12, check=False)
    except FileNotFoundError:
        raise PublicError('Antigravity 查询需要 curl，请检查安装与 PATH')
    except subprocess.TimeoutExpired:
        raise PublicError('Antigravity 查询超时，请检查网络与代理')
    if result.returncode:
        status = (result.stderr or b'').strip()
        if result.returncode == 22 and status == b'401':
            raise PublicError(RELOGIN_IDE if credential.get('_origin') == 'ide' else RELOGIN)
        if result.returncode == 22 and status == b'403':
            raise PublicError('Antigravity 拒绝访问，请检查当前账户的服务权限')
        if result.returncode == 22 and status == b'429':
            raise PublicError('Antigravity 查询过于频繁，请稍后重试')
        if result.returncode == 28:
            raise PublicError('Antigravity 查询超时，请检查网络与代理')
        if result.returncode in (5, 6, 7, 35, 60):
            raise PublicError('Antigravity 网络连接失败，请检查代理、DNS 与证书')
        raise PublicError('Antigravity 服务暂不可用，请稍后重试')
    if len(result.stdout) > LIMIT:
        raise PublicError('Antigravity 响应格式不支持')
    try:
        value = json.loads(result.stdout)
    except (ValueError, UnicodeError):
        raise PublicError('Antigravity 响应格式不支持')
    if not isinstance(value, dict):
        raise PublicError('Antigravity 响应格式不支持')
    return value


def fetch_license(credential):
    return subscription(request_json(credential, LICENSE_URL))


def native_quota(credential):
    """Delegate quota and renewal to the installed CLI, never a model prompt."""
    if credential.get('_origin') == 'ide':
        return None
    if credential.get('auth_method') not in (None, '', 'consumer'):
        raise PublicError('此登录方式暂不支持只读额度查询')
    binary = os.environ.get('AIEYES_AGY_BINARY') or shutil.which('agy')
    if not binary:
        installed = pathlib.Path.home() / '.local' / 'bin' / 'agy'
        binary = str(installed) if installed.is_file() else None
    if not binary:
        return None
    # The fixed built-in slash command returns a command result without asking
    # a model to run tools. Use an empty workspace and suppress CLI logs/updates.
    with tempfile.TemporaryDirectory(prefix='aieyes-agy-quota-') as directory:
        try:
            result = subprocess.run([binary, '--log-file', os.devnull, '--print', '/usage',
                                     '--output-format', 'json'], cwd=directory,
                                    env=dict(os.environ, AGY_CLI_DISABLE_AUTO_UPDATE='true', AIEYES_INTERNAL='1'),
                                    stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                    stderr=subprocess.DEVNULL, timeout=20, check=False)
        except subprocess.TimeoutExpired:
            raise PublicError('Antigravity 查询超时，请检查网络与代理')
        except OSError:
            raise PublicError('无法运行 agy，请检查 CLI 路径与安装')
    if result.returncode or len(result.stdout) > LIMIT:
        raise PublicError('agy 额度查询失败，请在 agy 中运行 /usage 检查登录状态')
    try:
        value = json.loads(result.stdout)
        command = value.get('command', {})
        data = command.get('data', {})
        if value.get('status') != 'SUCCESS' or command.get('name') != 'usage' or not isinstance(data.get('groups'), list):
            raise ValueError()
    except (ValueError, TypeError, AttributeError):
        raise PublicError('agy 未返回模型组额度，请更新 CLI 后重试')
    return data


def identity_credential(credential):
    try:
        access_token(credential)
    except PublicError as error:
        if str(error) != RELOGIN or native_quota(credential) is None:
            raise
        current = read_credential()
        if not same_owner(credential, current):
            raise PublicError('登录账户已变化，请重新检查')
        access_token(current)
        return current
    return credential


def resolve_identity(credential):
    if credential.get('id_token'):
        return identity(credential)
    # Verified Google userinfo supplies the stable ID omitted by newer agy.
    user = request_json(credential, USERINFO_URL)
    subject, email = user.get('id'), user.get('email')
    if (not isinstance(subject, str) or not subject or len(subject) > 255
            or user.get('verified_email') is not True
            or not isinstance(email, str)
            or not re.fullmatch(r'[^\s@\x00-\x1f]{1,254}@[^\s@\x00-\x1f]{1,254}', email)):
        raise PublicError('现有登录未提供可靠的 Google 账户身份')
    return {'key': 'google:' + hashlib.sha256(subject.encode()).hexdigest(),
            'email': email, 'checkedAt': int(time.time()), 'stale': False}


def quota():
    """CLI quota is authoritative; IDE-only installs use the native REST schema."""
    try:
        credential = read_credential()
        result = native_quota(credential)
        if result is None:
            access_token(credential)
        current = read_credential() if result is not None else credential
        if not same_owner(credential, current):
            raise PublicError('登录账户已变化，请重新检查')
        credential = current
        assist, metadata_error = {}, None
        try:
            assist = request_json(credential, ASSIST_URL, {'metadata': {'ideType': 'ANTIGRAVITY'}})
        except Exception:
            metadata_error = 'AI Credits 暂不可用；模型组额度仍可查询'
        project = assist.get('cloudaicompanionProject', assist.get('cloudaicompanion_project'))
        body = {'project': project} if isinstance(project, str) and project else {}
        if result is None:
            result = request_json(credential, QUOTA_URL, body)
        if not isinstance(result.get('groups'), list):
            raise PublicError('Antigravity 未返回模型组额度')
        # Only public quota/tier fields may leave the credential machine.
        groups = []
        for group in result['groups']:
            if not isinstance(group, dict) or not isinstance(group.get('buckets'), list):
                raise PublicError('Antigravity 模型组格式不支持')
            name = group.get('displayName', group.get('display_name', group.get('name', '')))
            buckets = []
            for bucket in group['buckets']:
                if not isinstance(bucket, dict):
                    raise PublicError('Antigravity 模型组格式不支持')
                buckets.append({
                    'id': bucket.get('bucketId', bucket.get('bucket_id', bucket.get('id'))),
                    'name': bucket.get('displayName', bucket.get('display_name', bucket.get('name'))),
                    'window': bucket.get('window'),
                    'remaining_fraction': bucket.get('remainingFraction', bucket.get('remaining_fraction')),
                    'remaining_amount': bucket.get('remainingAmount', bucket.get('remaining_amount')),
                    'disabled': bucket.get('disabled', False),
                    'reset_time': bucket.get('resetTime', bucket.get('reset_time')),
                })
            groups.append({'id': name, 'name': name, 'buckets': buckets})
        tiers = {}
        for camel, snake in [('paidTier', 'paid_tier'), ('currentTier', 'current_tier')]:
            tier = assist.get(camel, assist.get(snake))
            if isinstance(tier, dict):
                credits = tier.get('availableCredits', tier.get('available_credits'))
                tiers[camel] = {k: tier[k] for k in ('id', 'name') if isinstance(tier.get(k), str)}
                if isinstance(credits, list):
                    tiers[camel]['availableCredits'] = [{
                        'creditType': c.get('creditType', c.get('credit_type')),
                        'creditAmount': c.get('creditAmount', c.get('credit_amount')),
                    } for c in credits if isinstance(c, dict)]
        if not same_owner(credential, read_credential()):
            raise PublicError('登录账户已变化，请重新检查')
        return {'status': 'SUCCESS', 'command': {'name': 'usage', 'data': {
            'groups': groups, 'loadCodeAssist': tiers}}, 'metadataError': metadata_error}
    except Exception as error:
        return {'error': public_error(error, 'Antigravity 只读额度查询失败，请检查客户端登录与网络')}


def observe(identity_only=False):
    try:
        credential = identity_credential(read_credential())
        summary = resolve_identity(credential)
    except Exception as error:
        message = public_error(error, '无法确认账户身份，请检查 Antigravity 登录方式与凭据库')
        return {'identity': None, 'metadataError': message}
    if identity_only:
        return {'identity': summary, 'metadataError': None}
    error = None
    try:
        summary['subscription'] = fetch_license(credential)
        summary['subscriptionCheckedAt'] = int(time.time())
        if not summary['subscription']:
            error = '订阅类型未知'
    except Exception:
        error = '订阅查询暂不可用，请稍后刷新'
        summary['stale'] = True
    # Do not attach a response obtained while the credential was changing.
    try:
        if not same_owner(credential, read_credential()):
            return {'identity': None, 'metadataError': '登录账户已变化，请重新检查'}
    except Exception:
        return {'identity': None, 'metadataError': '登录状态已变化，请重新检查'}
    return {'identity': summary, 'metadataError': error}


def probe_ide():
    """Discover an IDE-only store without keychain, CLI or HTTP requests."""
    try:
        credential = read_ide_credential()
        if credential is None:
            return {'root': None}
        access_token(credential)
        return {'root': str(pathlib.Path(credential['_store_path']).parents[2])}
    except Exception:
        return {'root': None}


if __name__ == '__main__':
    result = probe_ide() if '--probe-ide' in sys.argv else quota() if '--quota' in sys.argv else observe('--identity-only' in sys.argv)
    print(json.dumps(result, ensure_ascii=False, separators=(',', ':')))
