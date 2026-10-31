"""Read agy's public account summary on the login machine (also used over SSH).

No credentials are written, refreshed, passed in argv, or returned. Only the CLI
may refresh its credential. curl reads its authorization header through stdin.
"""
import base64
import datetime
import hashlib
import json
import os
import pathlib
import re
import sqlite3
import subprocess
import sys
import time

LIMIT = 1024 * 1024
LICENSE_URL = 'https://aicode.googleapis.com/v1:fetchLicenses'
USERINFO_URL = 'https://www.googleapis.com/oauth2/v2/userinfo'
ASSIST_URL = 'https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist'
QUOTA_URL = 'https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary'
RELOGIN = 'Antigravity 登录已过期，请在 agy 中重新登录'
RELOGIN_IDE = 'Antigravity IDE 登录已过期，请在 IDE 中重新登录'
IDE_INVALID = 'Antigravity IDE 登录凭据无法读取，请在 IDE 中重新登录'


def proto_fields(raw):
    """Bounded protobuf wire reader for the IDE's saved public schema only."""
    if len(raw) > LIMIT:
        raise ValueError(IDE_INVALID)
    index, out = 0, []

    def varint():
        nonlocal index
        number = 0
        for shift in range(0, 70, 7):
            if index >= len(raw):
                raise ValueError(IDE_INVALID)
            byte = raw[index]
            index += 1
            number |= (byte & 127) << shift
            if byte < 128:
                return number
        raise ValueError(IDE_INVALID)

    while index < len(raw):
        tag = varint()
        field, wire = tag >> 3, tag & 7
        if field == 0 or len(out) >= 4096:
            raise ValueError(IDE_INVALID)
        if wire == 0:
            value = varint()
        elif wire in (1, 2, 5):
            size = varint() if wire == 2 else (8 if wire == 1 else 4)
            if size > len(raw) - index:
                raise ValueError(IDE_INVALID)
            value = raw[index:index + size]
            index += size
        else:
            raise ValueError(IDE_INVALID)
        out.append((field, wire, value))
    return out


def proto_one(fields, field, wire, default=b''):
    matches = [(w, v) for f, w, v in fields if f == field]
    if len(matches) > 1 or matches and matches[0][0] != wire:
        raise ValueError(IDE_INVALID)
    return matches[0][1] if matches else default


def proto_base64(raw):
    if not isinstance(raw, (str, bytes)) or len(raw) > LIMIT:
        raise ValueError(IDE_INVALID)
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
            raise ValueError(IDE_INVALID)
        entry = proto_fields(entry)
        if proto_one(entry, 1, 2) != b'oauthTokenInfoSentinelKey':
            continue
        row = proto_fields(proto_one(entry, 2, 2))
        matches.append(proto_one(row, 1, 2))
    if not matches:
        return None
    if len(matches) != 1:
        raise ValueError(IDE_INVALID)
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
        raise ValueError(IDE_INVALID)
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
        raise ValueError(IDE_INVALID)
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
                raise ValueError(IDE_INVALID)
            value = ide_token(row[1])
            if value is not None:
                value['_store_path'] = str(path)
            return value
        except (sqlite3.Error, ValueError, TypeError, OverflowError, OSError):
            raise ValueError(IDE_INVALID)
        finally:
            if db is not None:
                db.close()
    return None


def decode(raw):
    if len(raw) > LIMIT:
        raise ValueError('凭据格式无法识别')
    raw = raw.strip()
    if raw.startswith(b'go-keyring-base64:'):
        raw = base64.b64decode(raw.split(b':', 1)[1], validate=True)
    value = json.loads(raw)
    if not isinstance(value, dict):
        raise ValueError('凭据格式无法识别')
    return value


def identity(credential):
    if credential.get('auth_method') not in (None, '', 'consumer'):
        raise ValueError('此登录方式暂不支持账户身份识别')
    token = credential.get('id_token', '')
    parts = token.split('.') if isinstance(token, str) else []
    if len(parts) != 3:
        raise ValueError('现有登录未提供 Google 账户身份')
    claims = json.loads(base64.urlsafe_b64decode(parts[1] + '=' * (-len(parts[1]) % 4)))
    sub, email = claims.get('sub'), claims.get('email')
    # This is an observation of the trusted CLI store, not JWT authentication.
    # Expired ID tokens still identify a CLI credential refreshed via OAuth.
    if (claims.get('iss') not in ('accounts.google.com', 'https://accounts.google.com')
            or claims.get('email_verified') not in (True, 'true')
            or not isinstance(sub, str) or not sub or len(sub) > 255
            or not isinstance(email, str) or not re.fullmatch(r'[^\s@\x00-\x1f]{1,254}@[^\s@\x00-\x1f]{1,254}', email)):
        raise ValueError('现有登录未提供可靠的 Google 账户身份')
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
        # not an account ID; identical access tokens still prove the same store.
        token = left.get('token', {}).get('access_token')
        return bool(token and token == right.get('token', {}).get('access_token'))


def read_credential():
    if any(os.environ.get(k) for k in ('GEMINI_API_KEY', 'AGY_ADC_AUTH', 'AGY_LLM_GATEWAY_API_KEY')):
        raise ValueError('此登录方式暂不支持账户身份识别')
    # Auth belongs to the CLI's home, never the history source directory.
    root = pathlib.Path.home() / '.gemini'
    candidates = [root / 'antigravity-oauth-token',
                  root / 'antigravity-cli' / 'antigravity-oauth-token']
    files = [p for p in candidates if p.is_file()]
    # SSH/headless installations use the CLI file backend. If multiple CLI
    # generations left different accounts behind, do not guess which is active.
    file_values = [read_file(p) for p in files]
    if file_values and any(not same_owner(file_values[0], v) for v in file_values[1:]):
        raise ValueError('发现多个不同的 CLI 登录身份，请在 agy 中确认当前账户')
    bypass = bool(os.environ.get('SSH_CONNECTION') or os.environ.get('SSH_TTY'))
    if bypass and file_values:
        return file_values[0]
    command = (['/usr/bin/security', 'find-generic-password', '-s', 'gemini', '-a', 'antigravity', '-w']
               if sys.platform == 'darwin' else
               ['secret-tool', 'lookup', 'service', 'gemini', 'username', 'antigravity'])
    try:
        result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, timeout=4, check=False)
        if result.returncode == 0:
            if not result.stdout.strip():
                raise ValueError('无法读取 agy 登录身份，请检查凭据库或 CLI 登录状态')
            value = decode(result.stdout)
            # A recent keyring failure can make the CLI use its file backend.
            # Avoid claiming identity when those stores disagree.
            if file_values and not same_owner(value, file_values[0]):
                raise ValueError('系统凭据与 CLI 文件身份不同，请在 agy 中确认当前账户')
            return value
    except (OSError, subprocess.TimeoutExpired):
        pass
    if file_values:
        return file_values[0]
    # A present CLI store, including expired/empty/corrupt credentials, never
    # silently switches the query to an unrelated IDE account.
    value = read_ide_credential()
    if value is not None:
        return value
    raise ValueError('无法读取 agy 登录身份，请检查凭据库或 CLI 登录状态')


def subscription(value):
    licenses = value.get('licenses')
    if not isinstance(licenses, list) or not licenses or not isinstance(licenses[0], dict):
        return None
    first = licenses[0]
    label = first.get('tierDisplayName')
    if isinstance(label, str) and label.strip() and len(label) <= 120 and not any(ord(c) < 32 for c in label):
        return label.strip()
    return {'g1-pro-tier': 'Google AI Pro', 'g1-ultra-tier': 'Google AI Ultra',
            'free-tier': 'Free', 'g1-free-tier': 'Free'}.get(first.get('userTier'))


def access_token(credential):
    if credential.get('auth_method') not in (None, '', 'consumer'):
        raise ValueError('此登录方式暂不支持只读额度查询')
    expiry = credential.get('token', {}).get('expiry')
    if expiry:
        try:
            expires = datetime.datetime.fromisoformat(expiry.replace('Z', '+00:00')).timestamp()
        except (ValueError, TypeError, AttributeError):
            raise ValueError('Antigravity 登录有效期无法确认')
        if expires <= time.time() + 60:
            raise ValueError(RELOGIN_IDE if credential.get('_origin') == 'ide' else RELOGIN)
    token = credential.get('token', {}).get('access_token', '')
    if not isinstance(token, str) or not re.fullmatch(r'[A-Za-z0-9._~+/=-]+', token):
        raise ValueError('订阅暂不可用：现有登录缺少访问凭据')
    return token


def request_json(credential, url, body=None):
    if url not in (LICENSE_URL, USERINFO_URL, ASSIST_URL, QUOTA_URL):
        raise ValueError('Antigravity 查询地址不支持')
    token = access_token(credential)
    config = 'header = "Authorization: Bearer ' + token + '"\n'
    if body is not None:
        config += 'header = "Content-Type: application/json"\n'
        config += 'data = ' + json.dumps(json.dumps(body, separators=(',', ':'))) + '\n'
    result = subprocess.run(['curl', '-q', '--silent', '--fail', '--proto', '=https',
                             '--connect-timeout', '5', '--max-time', '10',
                             '--max-filesize', str(LIMIT), '--config', '-', url],
                            input=config.encode(), stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL, timeout=12, check=False)
    if result.returncode or len(result.stdout) > LIMIT:
        raise ValueError('订阅查询暂不可用，请稍后刷新')
    value = json.loads(result.stdout)
    if not isinstance(value, dict):
        raise ValueError('Antigravity 响应格式不支持')
    return value


def fetch_license(credential):
    return subscription(request_json(credential, LICENSE_URL))


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
        raise ValueError('现有登录未提供可靠的 Google 账户身份')
    return {'key': 'google:' + hashlib.sha256(subject.encode()).hexdigest(),
            'email': email, 'checkedAt': int(time.time()), 'stale': False}


def quota():
    """Native REST schemas, with the saved access token and no token endpoint."""
    try:
        credential = read_credential()
        access_token(credential)  # Expired credentials cause zero network calls.
        assist, metadata_error = {}, None
        try:
            assist = request_json(credential, ASSIST_URL, {'metadata': {'ideType': 'ANTIGRAVITY'}})
        except Exception:
            metadata_error = 'AI Credits 暂不可用；模型组额度仍可查询'
        project = assist.get('cloudaicompanionProject', assist.get('cloudaicompanion_project'))
        body = {'project': project} if isinstance(project, str) and project else {}
        result = request_json(credential, QUOTA_URL, body)
        if not isinstance(result.get('groups'), list):
            raise ValueError('Antigravity 未返回模型组额度')
        # Only public quota/tier fields may leave the credential machine.
        groups = []
        for group in result['groups']:
            if not isinstance(group, dict) or not isinstance(group.get('buckets'), list):
                raise ValueError('Antigravity 模型组格式不支持')
            name = group.get('displayName', group.get('display_name', ''))
            buckets = []
            for bucket in group['buckets']:
                if not isinstance(bucket, dict):
                    raise ValueError('Antigravity 模型组格式不支持')
                buckets.append({
                    'id': bucket.get('bucketId', bucket.get('bucket_id')),
                    'name': bucket.get('displayName', bucket.get('display_name')),
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
            raise ValueError('登录账户已变化，请重新检查')
        return {'status': 'SUCCESS', 'command': {'name': 'usage', 'data': {
            'groups': groups, 'loadCodeAssist': tiers}}, 'metadataError': metadata_error}
    except Exception as error:
        return {'error': str(error) if isinstance(error, ValueError) and str(error) in (RELOGIN, RELOGIN_IDE)
                else 'Antigravity 只读额度查询失败，请检查客户端登录与网络'}


def observe(identity_only=False):
    try:
        credential = read_credential()
        summary = resolve_identity(credential)
    except Exception as error:
        message = str(error) if isinstance(error, ValueError) and str(error) in (RELOGIN, RELOGIN_IDE, IDE_INVALID) else '无法确认账户身份，请检查 Antigravity 登录方式与凭据库'
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
