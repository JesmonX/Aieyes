"""Read a selected native credential and use it on its owning SSH host.

The protocol carries a file reference, a public identity projection and bounded
HTTP response data. Access/refresh tokens never leave this process or enter an
argv. Rust owns provider request construction and quota/identity parsing. Every
request reopens the selected entry and checks the pin returned by inspection.
Only Python stdlib is needed; the payload includes the existing Tomli/SQLite
readers and Codex app-server adapter for older Python installations.
"""
import base64
import hashlib
import json
import math
import os
import pathlib
import re
import stat
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

LIMIT = 1024 * 1024
RESPONSE_LIMIT = 2 * 1024 * 1024
MARKER = '${AIEYES_NATIVE_TOKEN}'
TRAE_LOGIN = 'iCubeAuthInfo://icube.cloudide'


class QuotaError(Exception):
    """Only fixed protocol error codes, never exception/HTTP body text."""


def compact(value):
    # serde_json::Map's default ordering, used by the native scanner's hashes.
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=False,
                      allow_nan=False)


def digest(value):
    return hashlib.sha256(value.encode('utf-8')).hexdigest()


def text(value):
    return value if isinstance(value, str) else ''


def obj(value):
    return value if isinstance(value, dict) else {}


def first(value, keys):
    return next((value[k] for k in keys if isinstance(value.get(k), str) and value[k]), '')


def public_scalar(value):
    if isinstance(value, str):
        return value[:1024] if not any(ord(c) < 32 for c in value) else None
    if value is None or isinstance(value, (bool, int)):
        return value
    if isinstance(value, float) and math.isfinite(value):
        return value
    return None


def fields(value, names):
    return {k: public_scalar(value[k]) for k in names if k in value}


def public_url(value):
    try:
        parsed = urllib.parse.urlsplit(value)
        if parsed.scheme in ('http', 'https') and parsed.hostname and not (parsed.username or parsed.password or parsed.query or parsed.fragment):
            return value
    except ValueError:
        pass
    return ''


def claims(token):
    try:
        raw = token.split('.')[1]
        value = json.loads(base64.urlsafe_b64decode(raw + '=' * (-len(raw) % 4)))
        return obj(value)
    except (IndexError, ValueError, UnicodeError):
        return {}


def claim_projection(token):
    value = claims(token)
    result = fields(value, ('sub', 'iss', 'email', 'organization_id', 'exp', 'iat'))
    auth = obj(value.get('https://api.openai.com/auth'))
    if auth:
        result['https://api.openai.com/auth'] = fields(auth, (
            'chatgpt_account_id', 'chatgpt_user_id', 'chatgpt_plan_type'))
    profile = obj(value.get('https://api.openai.com/profile'))
    if profile:
        result['https://api.openai.com/profile'] = fields(profile, ('email',))
    return result


def stub(token, timing_only=False):
    """Unsigned, visibly synthetic identity carrier for the shared Rust parser."""
    public = fields(claims(token), ('exp', 'iat')) if timing_only else claim_projection(token)
    payload = base64.urlsafe_b64encode(compact(public).encode()).decode().rstrip('=')
    prefix = next((p for p in ('ghp_', 'gho_', 'ghu_', 'github_pat_', 'xai-')
                   if token.startswith(p)), '')
    if token.lower().startswith('workos:'):
        prefix = token[:7]
    return prefix + 'aieyes_remote_identity.' + payload + '.not_a_credential'


def login_projection(value, token, token_key=None):
    result = fields(value, ('email', 'accountId', 'account_id', 'userId', 'user_id', 'organizationId',
                            'expires_at', 'expiresAt', 'expires', 'type', 'last_used', 'lastUsed',
                            'last_used_at', 'updated_at', 'created_at'))
    key = token_key or next((k for k in ('access_token', 'accessToken', 'access', 'token',
                                       'key', 'refresh_token', 'refreshToken', 'refresh')
                            if value.get(k) == token), 'access_token')
    # A refresh-only entry is identified exactly as the existing native scanner;
    # the transport never refreshes or serializes its real value.
    result[key] = stub(token)
    if isinstance(value.get('id_token'), str):
        result['id_token'] = stub(value['id_token'])
    return result


def pointer(value, selector):
    if not selector:
        return value
    if not selector.startswith('/'):
        raise QuotaError('reference')
    for part in selector[1:].split('/'):
        part = part.replace('~1', '/').replace('~0', '~')
        try:
            value = value[int(part)] if isinstance(value, list) else value[part]
        except (KeyError, TypeError, ValueError, IndexError):
            raise QuotaError('removed') from None
    return value


def at_pointer(selector, value):
    if not selector:
        return value
    result = value
    for part in reversed(selector[1:].split('/')):
        result = {part.replace('~1', '/').replace('~0', '~'): result}
    return result


def jsonc(raw):
    """Strip comments/trailing commas without touching JSON string contents."""
    output, i, quoted = [], 0, False
    while i < len(raw):
        c = raw[i]
        if quoted:
            output.append(c)
            if c == '\\' and i + 1 < len(raw):
                i += 1
                output.append(raw[i])
            elif c == '"':
                quoted = False
        elif c == '"':
            quoted = True
            output.append(c)
        elif raw[i:i + 2] == '//':
            i = raw.find('\n', i)
            if i < 0:
                break
            output.append('\n')
        elif raw[i:i + 2] == '/*':
            end = raw.find('*/', i + 2)
            if end < 0:
                raise QuotaError('format')
            output.append(' ')
            i = end + 1
        elif c == ',':
            output.append(c)
        else:
            output.append(c)
        i += 1
    # Comments have gone; use a second string-aware pass for trailing commas.
    raw, output, i, quoted = ''.join(output), [], 0, False
    while i < len(raw):
        c = raw[i]
        if quoted:
            output.append(c)
            if c == '\\' and i + 1 < len(raw):
                i += 1
                output.append(raw[i])
            elif c == '"':
                quoted = False
        elif c == '"':
            quoted = True
            output.append(c)
        elif c != ',' or not raw[i + 1:].lstrip().startswith(('}', ']')):
            output.append(c)
        i += 1
    return ''.join(output)


def yaml_plain(raw):
    """The plain scalar/map/list subset written by dsh and gh; no constructors.

    Unsupported anchors, tags, flow syntax or multiline scalars fail closed.
    A tagged credential is ignored just as the Rust scanner ignores YAML tags.
    """
    def split(line, delimiter):
        quote, escaped = '', False
        for i, c in enumerate(line):
            if escaped:
                escaped = False
            elif c == '\\' and quote == '"':
                escaped = True
            elif quote and c == quote:
                quote = ''
            elif not quote and c in ('"', "'"):
                quote = c
            elif not quote and c == delimiter and (
                    (delimiter == '#' and (i == 0 or line[i - 1].isspace())) or
                    (delimiter != '#' and (i + 1 == len(line) or line[i + 1].isspace()))):
                return line[:i], line[i + 1:]
        return None

    def scalar(value):
        value = value.strip()
        if not value:
            return None
        if value.startswith('!'):
            return None
        if value.startswith(('&', '*', '|', '>')):
            raise QuotaError('format')
        if value.startswith("'") and value.endswith("'"):
            return value[1:-1].replace("''", "'")
        if value.startswith(('"', '[', '{')):
            try:
                return json.loads(value)
            except ValueError:
                raise QuotaError('format') from None
        if value in ('null', 'Null', 'NULL', '~'):
            return None
        if value in ('true', 'True', 'TRUE', 'false', 'False', 'FALSE'):
            return value.lower() == 'true'
        if re.fullmatch(r'-?[0-9]+', value):
            return int(value)
        return value

    lines = []
    for original in raw.splitlines():
        if '\t' in original[:len(original) - len(original.lstrip())]:
            raise QuotaError('format')
        comment = split(original, '#')
        line = (comment[0] if comment else original).rstrip()
        if not line.strip() or line.lstrip().startswith('#') or line.strip() in ('---', '...'):
            continue
        lines.append((len(line) - len(line.lstrip()), line.lstrip()))

    def block(index, indent):
        sequence = lines[index][1].startswith('- ')
        result = [] if sequence else {}
        while index < len(lines) and lines[index][0] == indent:
            line = lines[index][1]
            if sequence:
                if not line.startswith('- '):
                    raise QuotaError('format')
                item = line[2:]
                pair = split(item, ':')
                if pair:
                    # Reuse the mapping parser at the item's normal indentation.
                    lines[index] = (indent + 2, item)
                    entry, index = block(index, indent + 2)
                elif item:
                    entry, index = scalar(item), index + 1
                else:
                    index += 1
                    entry, index = block(index, lines[index][0])
                result.append(entry)
                continue
            pair = split(line, ':')
            if not pair:
                raise QuotaError('format')
            key, value = scalar(pair[0]), pair[1].strip()
            if not isinstance(key, str) or key in result:
                raise QuotaError('format')
            index += 1
            if not value and index < len(lines) and lines[index][0] > indent:
                result[key], index = block(index, lines[index][0])
            else:
                result[key] = scalar(value)
        return result, index
    if not lines:
        return {}
    result, consumed = block(0, lines[0][0])
    if consumed != len(lines):
        raise QuotaError('format')
    return result


def dotenv(raw):
    """Node parseEnv's quoting/comments as used by the shared Rust scanner."""
    result, lines = {}, iter(raw.splitlines())
    for line in lines:
        line = line.strip()
        if line.startswith('export '):
            line = line[7:]
        if '=' not in line:
            continue
        key, value = line.split('=', 1)
        key, value = key.strip(), value.lstrip()
        if not re.fullmatch(r'[A-Za-z_][A-Za-z0-9_]*', key):
            continue
        if value.startswith(('"', "'", '`')):
            quote, rest = value[0], value[1:]
            while quote not in rest:
                try:
                    rest += '\n' + next(lines)
                except StopIteration:
                    break
            if quote not in rest:
                continue
            value = rest.split(quote, 1)[0]
            if quote == '"':
                value = value.replace('\\n', '\n').replace('\\r', '\r')
        else:
            value = value.split('#', 1)[0].strip()
        result[key] = value
    return result


_modules = {}


def helpers(name):
    if name not in _modules:
        bundled = globals().get('SCRIPTS', {})
        filename = {'scan': 'remote_account_scan.py', 'codex': 'remote_codex_auth.py'}[name]
        source = bundled.get(name)
        if source is None:
            source = pathlib.Path(__file__).with_name(filename).read_text(encoding='utf-8')
        scope = {'__name__': 'aieyes_native_' + name, '__file__': globals().get('__file__', filename),
                 '_aieyes_tomli_sources': bundled.get('tomli', {}),
                 '_aieyes_client_version': globals().get('CLIENT_VERSION', 'development')}
        exec(compile(source, '<aieyes-native-' + name + '>', 'exec'), scope)
        _modules[name] = scope
    return _modules[name]


def read_document(path, provider):
    path = pathlib.Path(os.path.expanduser(path))
    if not path.is_absolute():
        raise QuotaError('reference')
    try:
        metadata = path.stat()
        if not stat.S_ISREG(metadata.st_mode):
            raise QuotaError('format')
        if path.name in ('opencode.db', 'state.vscdb') or provider == 'omp' and path.name == 'agent.db':
            return json.loads(helpers('scan')['sqlite_document'](str(path)))
        if provider == 'kiro' and path.name == 'data.sqlite3':
            return json.loads(helpers('scan')['kiro_credentials'](str(path)))
        if metadata.st_size > LIMIT:
            raise QuotaError('large_file')
        with path.open('rb') as stream:
            raw = stream.read(LIMIT + 1)
        if len(raw) > LIMIT or len(raw) != metadata.st_size:
            raise QuotaError('changing')
        raw = raw.decode('utf-8')
        if provider == 'zcode' and path.name == 'credentials.json' and path.parent.name == 'v2':
            # Reuse the owning host's native AES-GCM codec, but retain its
            # plaintext only in this process before the public projection.
            return json.loads(helpers('scan')['zcode_projection'](raw.encode('utf-8')))
        if path.suffix == '.toml':
            return helpers('codex')['parse_toml'](raw)
        if path.suffix in ('.yml', '.yaml'):
            try:
                return json.loads(raw)
            except ValueError:
                return yaml_plain(raw)
        if path.name == '.env':
            return dotenv(raw)
        return json.loads(jsonc(raw) if path.suffix == '.jsonc' or provider == 'copilot' else raw)
    except QuotaError:
        raise
    except FileNotFoundError:
        raise QuotaError('removed') from None
    except Exception:
        raise QuotaError('format') from None


def epoch(value):
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return int(value / 1000 if value > 10_000_000_000 else value)
    if isinstance(value, str):
        try:
            import datetime
            return int(datetime.datetime.fromisoformat(value.replace('Z', '+00:00')).timestamp())
        except ValueError:
            pass
    return None


def kiro_kind(value, entry):
    provider = text(value.get('provider')).lower()
    start = first(value, ('start_url', 'startUrl')).rstrip('/')
    if provider in ('google', 'github'):
        return 'social:' + provider
    return ({'builderid': 'builderid', 'internal': 'internal', 'enterprise': 'idc',
             'externalidp': 'external'}.get(provider) or
            ('builderid' if start == 'https://view.awsapps.com/start' else 'idc' if start else entry))


def kimi_base(root):
    try:
        value = read_document(str(pathlib.Path(root) / 'config.toml'), 'kimi')
        managed = obj(value.get('providers')).get('managed:kimi-code')
        if isinstance(managed, dict):
            oauth = obj(managed.get('oauth'))
            global_region = any(urllib.parse.urlsplit(text(endpoint)).hostname in ('api.kimi.ai', 'auth.kimi.ai')
                                for endpoint in (managed.get('base_url'), managed.get('baseUrl'),
                                                 oauth.get('oauth_host'), oauth.get('oauthHost')))
            return 'https://api.kimi.ai/coding/v1' if global_region else 'https://api.kimi.com/coding/v1'
    except QuotaError:
        pass
    try:
        with (pathlib.Path(root) / 'region').open() as stream:
            region = stream.read(128).strip().lower()
        if region == 'global':
            return 'https://api.kimi.ai/coding/v1'
    except OSError:
        pass
    return 'https://api.kimi.com/coding/v1'


def resolve(reference):
    provider, selector, kind = (text(reference.get(k)) for k in ('provider', 'selector', 'kind'))
    path, root = (text(reference.get(k)) for k in ('configPath', 'root'))
    if not provider or not path or len(path) > 8192 or len(selector) > 4096:
        raise QuotaError('reference')
    environment = path.startswith('env:')
    if environment:
        if provider != 'copilot' or kind != 'login' or selector or not pathlib.Path(root).is_absolute():
            raise QuotaError('reference')
        selected = helpers('scan')['copilot_environment'](root)
        if not selected:
            raise QuotaError('removed')
        if selected[0] != path:
            raise QuotaError('changed')
        value = {'token': selected[1]}
    else:
        value = read_document(path, provider)
    if provider == 'deepseek-harness' and pathlib.Path(path).name == '.credentials.yaml' and value.get('version') != 1:
        raise QuotaError('format')
    entry = pointer(value, selector) if not (provider == 'kiro' or selector in ('/zcodeJwtToken', '/zcodejwttoken')) else None
    projection, token, fingerprint, secret, base = {}, '', '', '', ''
    companions, auth = [], None
    upstream = (selector[1:] if provider in ('opencode', 'pi', 'omp') and kind == 'login' else '')
    if provider == 'omp' and '#' in upstream:
        upstream, row_id = upstream.split('#', 1)
        if not re.fullmatch(r'[1-9][0-9]*', row_id) or len(row_id) > 19 or int(row_id) > 2**63 - 1:
            raise QuotaError('reference')
    service = {'anthropic': 'claude', 'openai': 'codex', 'openai-codex': 'codex',
               'github-copilot': 'copilot'}.get(upstream)
    if upstream == 'google-gemini-cli' and provider in ('pi', 'omp'):
        service = 'gemini'
    if provider == 'kiro':
        k = obj(value.get('kiro'))
        if k:
            candidates = [(name, obj(k.get(name))) for name in ('odic', 'social')]
            candidates = [(name, item) for name, item in candidates if first(item, ('access_token', 'accessToken'))]
            def rank(item):
                item = item[1]
                expiry = epoch(item.get('expires_at')) or epoch(item.get('expiresAt'))
                updated = next((epoch(item.get(n)) for n in ('updated_at', 'updatedAt', 'issued_at', 'issuedAt')
                                if epoch(item.get(n)) is not None), expiry or 0)
                return expiry is None or expiry > time.time(), updated
            if not candidates:
                raise QuotaError('removed')
            # Rust Iterator::max_by_key keeps the last equal maximum; the
            # native scan visits OIDC then social in this exact order.
            _, (name, entry) = max(enumerate(candidates), key=lambda item: (rank(item[1]), item[0]))
            if selector != '/auth_kv/' + name:
                raise QuotaError('changed')
            arn = first(k, ('profileArn',)) or first(entry, ('profile_arn', 'profileArn'))
        else:
            if selector != '/kiro-auth-token':
                raise QuotaError('changed')
            name, entry, arn = 'ide', obj(value), text(value.get('profileArn'))
        token = first(entry, ('access_token', 'accessToken'))
        expiry = epoch(entry.get('expires_at')) or epoch(entry.get('expiresAt'))
        login = fields(entry, ('expires_at', 'expiresAt', 'updated_at', 'updatedAt', 'issued_at',
                               'issuedAt', 'provider', 'start_url', 'startUrl', 'profile_arn', 'profileArn'))
        login['accessToken'] = stub(token)
        projection = {'kiro': {name: login, 'profileArn': arn}} if k else dict(login, profileArn=arn)
        fingerprint = digest(compact([selector, arn, kiro_kind(entry, name)]))
        secret = compact({'accessToken': token, 'profileArn': arn, 'expiresAt': expiry})
    elif provider in ('trae', 'trae-cn'):
        entry = json.loads(entry) if isinstance(entry, str) else obj(entry)
        token = text(entry.get('token'))
        host = first(entry, ('host', 'apiHost', 'api_host'))
        user = next((str(entry[k]) for k in ('userId', 'user_id') if isinstance(entry.get(k), (str, int)) and str(entry[k])), '')
        login = fields(entry, ('host', 'apiHost', 'api_host', 'userId', 'user_id', 'expiredAt', 'expired_at'))
        login.update(token=stub(token), account=fields(obj(entry.get('account')), ('email',)))
        projection = {TRAE_LOGIN: login}
        secret = compact({'token': token, 'host': host})
        fingerprint = digest(compact([selector, user])) if user else digest(secret)
    elif provider == 'copilot':
        if environment:
            token = value['token']
            projection = helpers('scan')['copilot_projection'](token)
        elif pathlib.Path(path).name == 'config.json':
            selected = []
            for key in ('copilot_tokens', 'copilotTokens'):
                for name in obj(value.get(key)):
                    login = name[len('https://github.com:'):] if name.startswith('https://github.com:') else name[len('github.com:'):] if name.startswith('github.com:') else None
                    if login is not None:
                        selected.append(('/' + key + '/' + name.replace('~', '~0').replace('/', '~1'), login))
            last = obj(value.get('last_logged_in_user')).get('login')
            if not isinstance(last, str):
                last = text(obj(value.get('lastLoggedInUser')).get('login'))
            if len(selected) > 1 and last:
                selected = [(path, login) for path, login in selected if login.lower() == last.lower()]
            if selector not in {path for path, _ in selected}:
                raise QuotaError('changed')
        if environment:
            pass
        elif isinstance(entry, str):
            token = entry
            projection = at_pointer(selector, stub(token))
            for name in ('last_logged_in_user', 'lastLoggedInUser'):
                if name in value:
                    projection[name] = fields(obj(value[name]), ('login',))
        else:
            token = text(obj(entry).get('oauth_token'))
            projection = at_pointer(selector, dict(fields(obj(entry), ('user',)), oauth_token=stub(token)))
    elif provider == 'cursor' and pathlib.Path(path).name == 'state.vscdb':
        entry = obj(entry)
        token = text(entry.get('accessToken'))
        projection = {'cursorAuth': dict(fields(entry, ('cachedEmail', 'stripeMembershipType')),
                                        accessToken=stub(token))}
    elif provider == 'zcode' and selector in ('/zcodeJwtToken', '/zcodejwttoken'):
        token = text(value.get(selector[1:]))
        def info_projection(info):
            result = fields(obj(info), ('userId', 'user_id', 'id', 'email', 'provider', 'loginProvider'))
            if isinstance(obj(info).get('rawProfile'), dict):
                result['rawProfile'] = fields(info['rawProfile'], ('user_id', 'email'))
            return result
        if selector == '/zcodejwttoken':
            active = text(value.get('oauth:active_provider'))
            if active not in ('zai', 'bigmodel'):
                raise QuotaError('changed')
            key = 'oauth:' + active + ':user_info'
            try:
                info = json.loads(text(value.get(key)))
            except ValueError:
                info = {}
            projection = {'oauth:active_provider': active, 'zcodejwttoken': stub(token),
                          key: compact(info_projection(info))}
        else:
            projection = dict(fields(value, ('provider', 'loginProvider')), zcodeJwtToken=stub(token),
                              zaiUserInfo=info_projection(value.get('zaiUserInfo')))
    elif service:
        entry = obj(entry)
        token = text(entry.get('access')).strip()
        gemini_fields = {}
        if service == 'gemini':
            if entry.get('type') != 'oauth' or any(c.isspace() or ord(c) < 32 or 127 <= ord(c) <= 159 for c in token):
                raise QuotaError('credential')
            # Never replace malformed routing/expiry data with null: that can
            # select a different project or silently drop a native expiry.
            expiry, project = entry.get('expires'), entry.get('projectId')
            if expiry is not None:
                if not isinstance(expiry, int) or isinstance(expiry, bool) or not -2**63 <= expiry < 2**63:
                    raise QuotaError('credential')
                gemini_fields['expires'] = expiry
            if project is not None:
                if not isinstance(project, str) or not project.strip() or len(project.strip().encode('utf-8')) > 256 or any(c.isspace() or ord(c) < 32 or 127 <= ord(c) <= 159 for c in project.strip()):
                    raise QuotaError('credential')
                gemini_fields['projectId'] = project
        if service == 'copilot':
            metadata = obj(entry.get('metadata'))
            enterprise = (text(entry.get('enterpriseUrl')) or text(metadata.get('enterpriseUrl'))).strip()
            if enterprise.removeprefix('https://').rstrip('/') not in ('', 'github.com'):
                raise QuotaError('unsupported')
            token = next((text(entry.get(k)).strip() for k in ('refresh', 'access')
                          if text(entry.get(k)).strip().startswith(('gho_', 'ghu_', 'ghp_', 'github_pat_'))), '')
        projection_entry = login_projection(entry, token, 'access')
        projection_entry.update(gemini_fields)
        if service == 'copilot':
            projection_entry['refresh'] = stub(token)
        metadata = fields(obj(entry.get('metadata')), ('accountID', 'accountId', 'enterpriseUrl'))
        if metadata:
            projection_entry['metadata'] = metadata
        if 'enterpriseUrl' in entry:
            projection_entry['enterpriseUrl'] = public_scalar(entry['enterpriseUrl'])
        projection = at_pointer(selector, projection_entry)
        auth = {'access': token}
        auth.update(gemini_fields)
        if service == 'codex':
            tokens = {'access_token': token}
            account = first(entry, ('accountId',)) or first(obj(entry.get('metadata')), ('accountID', 'accountId'))
            if account:
                tokens['account_id'] = account
            auth = {'tokens': tokens}
        fingerprint = digest(compact(auth))
    elif provider == 'codex' and kind == 'login':
        tokens = obj(value.get('tokens'))
        token = text(tokens.get('access_token'))
        auth = {'tokens': {k: tokens[k] for k in ('access_token', 'id_token', 'account_id') if k in tokens}}
        projected_tokens = fields(tokens, ('account_id',))
        projected_tokens['access_token'] = stub(token)
        if isinstance(tokens.get('id_token'), str):
            projected_tokens['id_token'] = stub(tokens['id_token'])
        projection = {'tokens': projected_tokens}
    elif provider == 'cline' and isinstance(value.get('providers'), dict):
        parts = selector.split('/')
        if len(parts) not in (4, 5) or parts[1] != 'providers' or parts[3] != 'settings' or len(parts) == 5 and parts[4] != 'auth':
            raise QuotaError('changed')
        settings_selector = '/'.join(parts[:4])
        settings = obj(pointer(value, settings_selector))
        native_provider = settings.get('provider')
        if not isinstance(native_provider, str):
            native_provider = parts[2].replace('~1', '/').replace('~0', '~')
        native_auth = obj(settings.get('auth'))
        if native_provider == 'cline':
            selected = settings_selector + ('/auth' if isinstance(native_auth.get('accessToken'), str) else '')
            if kind != 'login' or selector != selected:
                raise QuotaError('changed')
            token = text(native_auth.get('accessToken')) or next((item for item in (
                settings.get('apiKey'), native_auth.get('apiKey')) if isinstance(item, str)), '')
            public_settings = fields(settings, ('provider',))
            public_settings['auth'] = fields(native_auth, ('accountId', 'expiresAt'))
            if isinstance(native_auth.get('accessToken'), str):
                public_settings['auth']['accessToken'] = stub(token)
            elif isinstance(settings.get('apiKey'), str):
                public_settings['apiKey'] = stub(token)
            else:
                public_settings['auth']['apiKey'] = stub(token)
            projection = at_pointer(settings_selector, public_settings)
        else:
            if kind != 'api' or selector != settings_selector:
                raise QuotaError('changed')
            token = text(settings.get('apiKey')) or text(native_auth.get('apiKey'))
            base = text(settings.get('baseUrl'))
            if native_provider == 'openrouter' and not base:
                base = 'https://openrouter.ai/api/v1'
            projection = {'nativeApi': {'baseUrl': public_url(base)}}
    elif kind == 'api':
        if isinstance(entry, str):
            token = entry
        else:
            entry = obj(entry)
            route = obj(entry.get('options')) or entry
            token = first(route, ('key', 'apiKey', 'api_key'))
            base = first(route, ('baseURL', 'baseUrl', 'base_url'))
            if provider == 'zcode' and selector.startswith('/config/providerConfigRules/'):
                config = obj(entry.get('config'))
                token = text(obj(config.get('access')).get('apiKey'))
                base = text(obj(config.get('api')).get('baseUrl'))
            elif provider == 'deepseek-harness' and not token:
                token = text(obj(route.get('headers')).get('Authorization')).removeprefix('Bearer ')
        if provider == 'deepseek-harness' and selector in ('/refs/DEEPSEEK_API_KEY', '/DEEPSEEK_API_KEY'):
            base = text(value.get('DEEPSEEK_BASE_URL')) or 'https://api.deepseek.com'
        parts = selector.split('/')
        if provider == 'kilo' and len(parts) == 3 and parts[1] in ('providers', 'provider', 'model', 'model_providers') and parts[2] == 'openrouter' and not base:
            base = 'https://openrouter.ai/api/v1'
        if provider == 'zcode' and selector.startswith('/account-provider:coding-plan:'):
            family = 'zai' if ':account:zai-individual-coding-plan:' in selector else 'bigmodel' if ':account:bigmodel-individual-coding-plan:' in selector else ''
            provider_key = 'account:' + family + '-individual-coding-plan'
            identity = text(value.get('account-provider:' + provider_key + ':identity'))
            selected = 'account-provider:coding-plan:' + provider_key + ':account:' + identity + ':api-key'
            if not family or not identity.startswith('key-') or not re.fullmatch(r'[A-Za-z0-9-]+', identity) or value.get('account-provider:' + provider_key + ':auth-source') != 'manual' or selector != '/' + selected:
                raise QuotaError('changed')
            base = 'https://api.z.ai/api/coding/paas/v4' if family == 'zai' else 'https://open.bigmodel.cn/api/coding/paas/v4'
        # API rows have no account identity; the Rust verifier uses the exact
        # credential fingerprint plus the freshly read route, not a made-up JWT.
        projection = {'nativeApi': {'baseUrl': public_url(base)}}
    else:
        entry = obj(entry)
        if provider == 'deepseek-harness' and pathlib.Path(path).name == '.credentials.yaml':
            payload = obj(entry.get('payload'))
            if value.get('version') != 1 or selector != '/records/deepseek-account-platform~1default' or entry.get('kind') != 'grant' or payload.get('version') != 1:
                raise QuotaError('format')
            token = text(payload.get('token'))
            selected = login_projection(payload, token)
            selected['version'] = 1
            selected['issuer'] = public_url(text(payload.get('issuer')))
            projection = at_pointer(selector, {'kind': 'grant', 'payload': selected})
            projection['version'] = 1
            base = text(payload.get('issuer'))
        else:
            token = first(entry, ('access_token', 'accessToken', 'access', 'token', 'key',
                                  'refresh_token', 'refreshToken', 'refresh'))
            projection = at_pointer(selector, login_projection(entry, token))
    if not token:
        raise QuotaError('removed')
    if len(token) > 32768 or any(c in token for c in ('\r', '\n', '\0')):
        raise QuotaError('credential')
    secret = secret or token
    fingerprint = fingerprint or digest(secret)
    if provider == 'claude':
        identity_paths = [pathlib.Path(root) / '.claude.json']
        if pathlib.Path(root).name == '.claude':
            identity_paths.append(pathlib.Path(root + '.json'))
        for companion in identity_paths:
            try:
                data = read_document(str(companion), provider)
                oauth = fields(obj(data.get('oauthAccount')), ('accountUuid', 'organizationUuid', 'emailAddress'))
                if oauth.get('accountUuid'):
                    companions.append({'path': str(companion), 'value': {'oauthAccount': oauth}})
                    break
            except QuotaError:
                pass
    if provider == 'kimi' and kind == 'login':
        base = kimi_base(root)
    if provider == 'grok' and pathlib.Path(path).name == 'auth.json':
        # Keep only the timing evidence of other entries so Rust makes the
        # same active-entry choice, without exporting those entries' identities.
        selected_name = selector[1:].replace('~1', '/').replace('~0', '~')
        for scope, other in obj(value).items():
            if scope != selected_name and (scope.startswith('https://auth.x.ai::') or scope == 'https://accounts.x.ai/sign-in') and text(obj(other).get('key')):
                projection[scope] = dict(fields(obj(other), ('expires_at', 'last_used', 'lastUsed', 'last_used_at', 'updated_at', 'created_at')),
                                         key=stub(text(obj(other).get('key')), timing_only=True))
    public = {'fingerprint': fingerprint, 'value': projection, 'companions': companions,
              'baseUrl': public_url(base), 'tokenStub': projection['token'] if environment else stub(token)}
    if token in compact(public):
        raise QuotaError('response_private')
    public['pin'] = digest(compact([path, selector, token, public]))
    return {'token': token, 'secret': secret, 'auth': auth, 'public': public}


def allowed_url(url):
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != 'https' or parsed.username or parsed.password or parsed.fragment or parsed.port not in (None, 443):
        return False
    host, path = parsed.hostname, parsed.path
    fixed = {
        'api.anthropic.com': {'/api/oauth/profile', '/api/oauth/usage'},
        'api.github.com': {'/user', '/copilot_internal/user'},
        'www.googleapis.com': {'/oauth2/v2/userinfo'},
        'cloudcode-pa.googleapis.com': {'/v1internal:loadCodeAssist', '/v1internal:retrieveUserQuota'},
        'api.z.ai': {'/api/monitor/usage/quota/limit'},
        'open.bigmodel.cn': {'/api/monitor/usage/quota/limit'},
        'api2.cursor.sh': {'/aiserver.v1.DashboardService/GetCurrentPeriodUsage'},
        'api.moonshot.cn': {'/v1/users/me/balance'},
        'api.moonshot.ai': {'/v1/users/me/balance'},
        'api.kimi.com': {'/coding/v1/me', '/coding/v1/usages'},
        'api.kimi.ai': {'/coding/v1/me', '/coding/v1/usages'},
        'api.deepseek.com': {'/user/balance'},
        'platform.deepseek.com': {'/api/v0/users/get_user_summary'},
        'cli-chat-proxy.grok.com': {'/v1/billing'},
        'opencode.ai': {'/zen/go/v1/usage', '/console/api/go/status', '/console/api/billing/status'},
        'openrouter.ai': {'/api/v1/key'},
        'api.kilo.ai': {'/api/profile', '/api/profile/balance'},
        'codewhisperer.us-east-1.amazonaws.com': {'/'},
        'q.eu-central-1.amazonaws.com': {'/'},
    }
    if host in fixed:
        return path in fixed[host] and (not parsed.query or host == 'cli-chat-proxy.grok.com' and parsed.query == 'format=credits')
    if host == 'api.cline.bot' and not parsed.query:
        return path == '/api/v1/users/me' or bool(re.fullmatch(
            r'/api/v1/(users|organizations)/[A-Za-z0-9_-][A-Za-z0-9_.:-]{0,127}/balance', path))
    if host and any(host.endswith(domain) for domain in ('.trae.ai', '.traeapi.us', '.trae.cn', '.trae.com.cn')):
        return not parsed.query and path in ('/trae/api/v1/pay/ide_user_ent_usage', '/trae/api/v2/pay/ide_user_ent_usage')
    return False


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def response_projection(value, token):
    """Drop credential-shaped fields and reject a server echo of our token."""
    blocked = {'token', 'accesstoken', 'refreshtoken', 'idtoken', 'apikey', 'key', 'authorization',
               'cookie', 'setcookie', 'password', 'secret', 'clientsecret', 'credential', 'credentials'}
    def walk(item, depth=0):
        if depth > 64:
            raise QuotaError('response_format')
        if isinstance(item, dict):
            if any(token in key for key in item):
                raise QuotaError('response_private')
            return {k: walk(v, depth + 1) for k, v in item.items()
                    if re.sub('[^a-z]', '', k.lower()) not in blocked}
        if isinstance(item, list):
            return [walk(v, depth + 1) for v in item]
        if isinstance(item, str) and token in item:
            raise QuotaError('response_private')
        if isinstance(item, float) and not math.isfinite(item):
            raise QuotaError('response_format')
        return item
    output = walk(value)
    if token in compact(output):
        raise QuotaError('response_private')
    return output


def http(spec, token, proxy, url_guard=allowed_url):
    url = text(spec.get('url'))
    if not url_guard(url) or MARKER in url:
        raise QuotaError('endpoint')
    method = spec.get('method', 'GET')
    if method not in ('GET', 'POST'):
        raise QuotaError('request')
    headers = {}
    for name, value in obj(spec.get('headers')).items():
        if not re.fullmatch(r'[A-Za-z0-9-]{1,80}', name) or not isinstance(value, str):
            raise QuotaError('request')
        if MARKER in value and name.lower() not in ('authorization', 'x-dsh-auth-token', 'cookie'):
            raise QuotaError('request')
        value = value.replace(MARKER, token)
        if any(c in value for c in ('\r', '\n', '\0')) or len(value) > 65536:
            raise QuotaError('request')
        headers[name] = value
    body = spec.get('body')
    if body is not None and (not isinstance(body, str) or len(body) > 65536 or MARKER in body):
        raise QuotaError('request')
    handlers = [NoRedirect()]
    mode = proxy.get('mode', 'system')
    if mode == 'direct':
        handlers.append(urllib.request.ProxyHandler({}))
    elif mode == 'custom':
        parsed = urllib.parse.urlsplit(text(proxy.get('url')))
        if parsed.scheme not in ('http', 'https') or not parsed.hostname:
            raise QuotaError('proxy')
        handlers.append(urllib.request.ProxyHandler({'http': proxy['url'], 'https': proxy['url']}))
    opener = urllib.request.build_opener(*handlers)
    request = urllib.request.Request(url, data=body.encode() if body is not None else None,
                                     headers=headers, method=method)
    try:
        with opener.open(request, timeout=max(1, min(int(spec.get('timeoutSeconds', 25)), 30))) as response:
            status = response.status
            raw = response.read(RESPONSE_LIMIT + 1)
        if len(raw) > RESPONSE_LIMIT:
            raise QuotaError('response_large')
        try:
            data = json.loads(raw)
        except (ValueError, UnicodeError):
            raise QuotaError('response_format') from None
        return {'status': status, 'data': response_projection(data, token)}
    except urllib.error.HTTPError as error:
        # No error body or Location is exposed; either may echo credentials.
        status = error.code
        error.close()
        return {'status': status}
    except QuotaError:
        raise
    except Exception:
        raise QuotaError('connection') from None


def codex_quota(request, credential):
    auth = credential['auth']
    if not auth or 'tokens' not in auth:
        raise QuotaError('unsupported')
    api = helpers('codex')
    # SSH honors only the owning host/account's pre-command environment. The
    # desktop's proxy and legacy per-source proxy must never override it.
    location = {'binary': text(request.get('binary')) or 'codex'}
    previous_internal = os.environ.get('AIEYES_INTERNAL')
    os.environ['AIEYES_INTERNAL'] = '1'
    try:
        with tempfile.TemporaryDirectory(prefix='.aieyes-quota-') as directory:
            os.chmod(directory, 0o700)
            with api['Rpc'](location, pathlib.Path(directory)) as rpc:
                rpc.login(auth)
                result = rpc.call('account/rateLimits/read', None)
            return {'status': 200, 'data': response_projection(result, credential['token'])}
    except QuotaError:
        raise
    except Exception:
        raise QuotaError('codex') from None
    finally:
        if previous_internal is None:
            os.environ.pop('AIEYES_INTERNAL', None)
        else:
            os.environ['AIEYES_INTERNAL'] = previous_internal


def run(request, send=http):
    reference = obj(request.get('reference'))
    credential = resolve(reference)
    public = credential['public']
    action = request.get('action', 'inspect')
    expected = request.get('pin')
    if expected is not None and expected != public['pin']:
        raise QuotaError('changed')
    if action == 'inspect':
        return public
    if expected is None:
        raise QuotaError('reference')
    if action == 'http':
        result = send(obj(request.get('request')), credential['token'], {})
    elif action == 'codex':
        result = codex_quota(request, credential)
    else:
        raise QuotaError('request')
    # A client account switch during HTTP must discard the response, including
    # successful responses in a multi-step personal/organization balance read.
    if resolve(reference)['public']['pin'] != expected:
        raise QuotaError('changed')
    return result


def main(request):
    try:
        return {'version': 1, 'result': run(request)}
    except QuotaError as error:
        return {'version': 1, 'errorCode': str(error)}
    except Exception:
        return {'version': 1, 'errorCode': 'format'}


if __name__ == '__main__':
    request = globals().get('REQUEST')
    if request is None:
        try:
            request = json.load(sys.stdin)
        except Exception:
            request = {}
    print(json.dumps(main(request), separators=(',', ':'), ensure_ascii=True))
