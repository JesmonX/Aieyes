"""Read agy's public account summary on the login machine (also used over SSH).

No credentials are written, refreshed, passed in argv, or returned. Only the CLI
may refresh its credential. curl reads its authorization header through stdin.
"""
import base64
import hashlib
import json
import os
import pathlib
import re
import subprocess
import sys
import time

LIMIT = 1024 * 1024
LICENSE_URL = 'https://aicode.googleapis.com/v1:fetchLicenses'


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
    if len(file_values) > 1 and len({identity(v)['key'] for v in file_values}) != 1:
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
        if result.returncode == 0 and result.stdout.strip():
            value = decode(result.stdout)
            # A recent keyring failure can make the CLI use its file backend.
            # Avoid claiming identity when those stores disagree.
            if file_values and identity(value)['key'] != identity(file_values[0])['key']:
                raise ValueError('系统凭据与 CLI 文件身份不同，请在 agy 中确认当前账户')
            return value
    except (OSError, subprocess.TimeoutExpired):
        pass
    if file_values:
        return file_values[0]
    # Legacy standalone credentials are not silently attributed to the CLI.
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


def fetch_license(credential):
    token = credential.get('token', {}).get('access_token', '')
    if not isinstance(token, str) or not re.fullmatch(r'[A-Za-z0-9._~+/=-]+', token):
        raise ValueError('订阅暂不可用：现有登录缺少访问凭据')
    config = 'header = "Authorization: Bearer ' + token + '"\n'
    result = subprocess.run(['curl', '-q', '--silent', '--fail', '--proto', '=https',
                             '--connect-timeout', '5', '--max-time', '10',
                             '--max-filesize', str(LIMIT), '--config', '-', LICENSE_URL],
                            input=config.encode(), stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL, timeout=12, check=False)
    if result.returncode or len(result.stdout) > LIMIT:
        raise ValueError('订阅查询暂不可用，请稍后刷新')
    return subscription(json.loads(result.stdout))


def observe(identity_only=False):
    try:
        credential = read_credential()
        summary = identity(credential)
    except Exception:
        return {'identity': None, 'metadataError': '无法确认账户身份，请检查 agy 登录方式与凭据库'}
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
        if identity(read_credential())['key'] != summary['key']:
            return {'identity': None, 'metadataError': '登录账户已变化，请重新检查'}
    except Exception:
        return {'identity': None, 'metadataError': '登录状态已变化，请重新检查'}
    return {'identity': summary, 'metadataError': error}


if __name__ == '__main__':
    print(json.dumps(observe('--identity-only' in sys.argv), ensure_ascii=False, separators=(',', ':')))
