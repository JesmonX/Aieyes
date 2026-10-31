"""Bounded, read-only native config transport. stdout is private core input.

Secrets are never sent to the UI or persisted by this helper. Parsing uses the
same Rust adapters as local discovery; no TOML/YAML dependencies on SSH hosts.
"""
import base64
import contextlib
import glob
import hashlib
import json
import math
import os
import pathlib
import sqlite3
import stat
import subprocess
import sys

LIMIT = 1024 * 1024
MAX_FILES = 128
TOTAL = 8 * 1024 * 1024

# ZCode's Node credential codec. The owning machine supplies its native
# home/user/secret; only the selected values cross this private transport.
ZCODE_PROJECT = r"""
const crypto=require('node:crypto'), os=require('node:os');
const allowed=k=>['oauth:active_provider','zcodejwttoken','oauth:zai:user_info','oauth:bigmodel:user_info'].includes(k)
 || k.startsWith('account-provider:') && (k.endsWith(':identity') || k.endsWith(':auth-source')
 || k.startsWith('account-provider:coding-plan:') && k.endsWith(':api-key'));
let input='';
process.stdin.setEncoding('utf8');
process.stdin.on('data', c=>{ input+=c; if(input.length>1048576) process.exit(1); });
process.stdin.on('end', ()=>{
 try {
  let user='unknown'; try { user=os.userInfo().username; } catch {}
  const secret=process.env.ZCODE_CREDENTIAL_SECRET || 'zcode-credential-fallback:'+os.platform()+':'+os.homedir()+':'+user;
  const key=crypto.createHash('sha256').update(secret).digest();
  const out={};
  for(const [k,v] of Object.entries(JSON.parse(input))) {
   if(!allowed(k)) continue;
   if(typeof v!=='string') throw Error();
   if(v.startsWith('enc:v1:')) {
    const p=v.slice(7).split('.');
    if(p.length!==3) throw Error();
    const [iv,tag,data]=p.map(x=>Buffer.from(x,'base64url'));
    if(iv.length!==12 || tag.length!==16 || data.length>32768) throw Error();
    const d=crypto.createDecipheriv('aes-256-gcm',key,iv); d.setAuthTag(tag);
    out[k]=Buffer.concat([d.update(data),d.final()]).toString('utf8');
   } else {
    if(v.startsWith('enc:')) throw Error();
    out[k]=v;
   }
  }
  process.stdout.write(JSON.stringify(out));
 } catch { process.exitCode=1; }
});
"""


def zcode_projection(raw):
    value = json.loads(raw)
    if not isinstance(value, dict):
        raise ValueError('ZCode 登录凭据格式不支持')
    # Older versions already used a plain object; retain that shape.
    if not any(k.startswith(('oauth:', 'account-provider:')) or k == 'zcodejwttoken' for k in value):
        return raw
    allowed = lambda k: k in ('oauth:active_provider', 'zcodejwttoken', 'oauth:zai:user_info', 'oauth:bigmodel:user_info') or (
        k.startswith('account-provider:') and (k.endswith((':identity', ':auth-source')) or (
            k.startswith('account-provider:coding-plan:') and k.endswith(':api-key'))))
    selected = {k: v for k, v in value.items() if allowed(k)}
    if all(isinstance(v, str) and not v.startswith('enc:') for v in selected.values()):
        return json.dumps(selected).encode()
    try:
        result = subprocess.run(['node', '-e', ZCODE_PROJECT], input=raw, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, timeout=5, check=False)
    except (OSError, subprocess.TimeoutExpired):
        raise ValueError('ZCode 登录凭据无法解密，请检查 Node.js 与原生凭据')
    if result.returncode or len(result.stdout) > LIMIT:
        raise ValueError('ZCode 登录凭据无法解密，请检查 Node.js 与原生凭据')
    return result.stdout


def kiro_credentials(path):
    """Project only Kiro's active auth metadata from its read-only SQLite DB."""
    import sqlite3
    with sqlite3.connect(pathlib.Path(path).resolve().as_uri() + '?mode=ro', uri=True, timeout=2) as db:
        db.execute('PRAGMA query_only=1')
        db.execute('BEGIN')
        tables = {r[0] for r in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        def read(table, key):
            if table not in tables:
                return {}
            row = db.execute('SELECT CAST(value AS TEXT) FROM ' + table + ' WHERE key=?', (key,)).fetchone()
            if not row or len(row[0]) > LIMIT:
                return {}
            value = json.loads(row[0])
            return value if isinstance(value, dict) else {}
        result = {}
        for method in ('odic', 'social'):
            value = read('auth_kv', 'kirocli:' + method + ':token')
            if value:
                result[method] = {k: v for k, v in value.items() if 'refresh' not in k.lower()}
        result['profileArn'] = read('state', 'api.codewhisperer.profile').get('arn')
        return json.dumps({'kiro': result}).encode('utf-8')

def workbuddy_identity(path):
    """Same projection as Rust `workbuddy_identity`: only `sessions.user_id` of the
    latest visible session, and only for clients without `storage/skeleton`."""
    if (pathlib.Path(path).parent / 'storage' / 'skeleton').is_dir():
        return {}
    db = sqlite3.connect(pathlib.Path(path).resolve().as_uri() + '?mode=ro', uri=True, timeout=.5)
    try:
        db.execute('PRAGMA query_only=ON')
        if db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='sessions'").fetchone() is None:
            return {}
        row = db.execute("SELECT user_id FROM sessions WHERE deleted_at IS NULL AND user_id<>'' ORDER BY COALESCE(last_activity_at,updated_at) DESC LIMIT 1").fetchone()
        return {'workbuddy': {'userId': row[0] if row else None}}
    finally:
        db.close()


def omp_credentials(db):
    """OMP's enabled OAuth pool, projected like account_scan/omp.rs.

    Filter providers in SQL before fetching their credential data. Row ids are
    stable references only; the upstream server supplies account identity.
    """
    result, size = {}, 0
    def invalid_number(_):
        raise ValueError('invalid JSON number')
    def finite_float(raw):
        number = float(raw)
        if not math.isfinite(number):
            invalid_number(raw)
        return number
    def finite_int(raw):
        # Match serde_json's finite numeric range without rounding normal
        # integers, especially the native millisecond expiry/fingerprint.
        finite_float(raw)
        return int(raw)
    if not db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='auth_credentials'").fetchone():
        return result
    rows = db.execute("""SELECT id,provider,CASE WHEN length(data)<=? THEN data END
        FROM auth_credentials
        WHERE disabled_cause IS NULL AND credential_type='oauth' AND id>0
          AND provider IN ('anthropic','openai','openai-codex','github-copilot','google-gemini-cli')
        ORDER BY id ASC LIMIT 257""", (LIMIT,))
    for index, (row_id, provider, raw) in enumerate(rows):
        if index >= 256 or not isinstance(raw, str):
            raise ValueError('原生配置超过扫描大小限制')
        size += len(raw.encode('utf-8'))
        if size > LIMIT:
            raise ValueError('原生配置超过扫描大小限制')
        try:
            value = json.loads(raw, parse_constant=invalid_number,
                               parse_float=finite_float, parse_int=finite_int)
        except (ValueError, TypeError):
            continue
        if not isinstance(value, dict):
            continue
        selected = {k: value[k] for k in ('access', 'expires', 'projectId', 'email',
                    'accountId', 'enterpriseUrl') if k in value}
        selected['type'] = 'oauth'
        if provider == 'github-copilot' and 'refresh' in value:
            selected['refresh'] = value['refresh']
        if isinstance(value.get('metadata'), dict):
            selected['metadata'] = {k: value['metadata'][k] for k in
                ('accountID', 'accountId', 'enterpriseUrl') if k in value['metadata']}
        result[provider + '#' + str(row_id)] = selected
    return result


def sqlite_document(path):
    """Read only the same bounded credential projection as the local core.

    Never transport a whole DB, open a writable connection, or checkpoint WAL.
    """
    with contextlib.closing(sqlite3.connect(pathlib.Path(path).resolve().as_uri() + '?mode=ro&immutable=0',
                         uri=True, timeout=.5)) as db:
        db.execute('PRAGMA query_only=ON')
        if pathlib.Path(path).name == 'agent.db':
            result = omp_credentials(db)
        elif pathlib.Path(path).name == 'state.vscdb':
            auth = {}
            rows = db.execute("SELECT key,value FROM ItemTable WHERE key IN ('cursorAuth/accessToken','cursorAuth/cachedEmail','cursorAuth/stripeMembershipType')")
            for key, value in rows:
                if isinstance(value, bytes):
                    value = value.decode('utf-8', errors='replace')
                if not isinstance(value, str):
                    continue
                value = value.strip().strip('"')
                if key == 'cursorAuth/accessToken' and value.startswith('cursor-safe-storage:'):
                    auth['encrypted'] = True
                elif value and len(value) <= 32768:
                    auth[key.removeprefix('cursorAuth/')] = value
            result = {'cursorAuth': auth}
        else:
            result = {}
            if db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='credential'").fetchone():
                rows = db.execute("SELECT integration_id,value FROM credential WHERE integration_id IS NOT NULL AND integration_id<>'' ORDER BY integration_id,active DESC,time_created DESC,id DESC LIMIT 256")
                for name, raw in rows:
                    if name in result or len(result) >= 64:
                        continue
                    try:
                        value = json.loads(raw)
                        if not isinstance(value, dict):
                            continue
                    except (ValueError, TypeError):
                        if not isinstance(raw, str) or not raw.strip():
                            continue
                        value = {'type': 'api', 'key': raw.strip()}
                    result[name] = value
    raw = json.dumps(result, ensure_ascii=True).encode()
    if len(raw) > LIMIT:
        raise ValueError('native projection too large')
    return raw


COPILOT_ENV = ('COPILOT_GITHUB_TOKEN', 'GH_TOKEN', 'GITHUB_TOKEN')


def copilot_environment(root):
    """The owning host's first token; generic GitHub env needs a Copilot home."""
    selected = next(((name, os.environ[name].strip()) for name in COPILOT_ENV
                     if os.environ.get(name, '').strip()), None)
    if not selected:
        return None
    name, token = selected
    if name != 'COPILOT_GITHUB_TOKEN' and not pathlib.Path(root).is_dir():
        return None
    if len(token) > 32768 or any(c.isspace() or ord(c) < 32 or 127 <= ord(c) <= 159 for c in token):
        return None
    return 'env:' + name, token


def copilot_projection(token):
    """No environment secret crosses SSH, including during first discovery."""
    prefix = next((p for p in ('ghp_', 'gho_', 'ghu_', 'github_pat_') if token.startswith(p)), '')
    return {'token': prefix + 'aieyes_remote_identity.e30.not_a_credential',
            'aieyesTokenFingerprint': hashlib.sha256(token.encode('utf-8')).hexdigest()}


def collect(request):
    provider = request['provider']
    home = pathlib.Path.home()
    roots = list(request['roots'])
    variable = request.get('env', '')
    override = os.environ.get(variable) if variable else None
    if provider == 'cline' and 'pathsOnly' not in request and not override and os.environ.get('CLINE_DIR'):
        override = str(pathlib.Path(os.environ['CLINE_DIR']) / 'data')
    if override:
        if not pathlib.Path(override).is_absolute():
            return {'roots': [], 'documents': [], 'errors': ['远端原生目录环境变量应为绝对路径']}
        roots.append(str(pathlib.Path(override) / provider if provider in ('opencode', 'kilo') else pathlib.Path(override)))
    if provider == 'omp':
        config = pathlib.Path(os.environ.get('PI_CONFIG_DIR', '.omp'))
        profile = os.environ.get('OMP_PROFILE', os.environ.get('PI_PROFILE', '')).strip()
        if config.is_absolute() or '..' in config.parts or '/' in profile or '\\' in profile or profile in ('.', '..'):
            return {'roots': [], 'documents': [], 'errors': ['远端 OMP Profile 配置无效']}
        root = home / config
        if profile and profile != 'default':
            root = root / 'profiles' / profile
        roots.append(str(root / 'agent'))
        # Native OMP stores agent.db in the data directory when the selected
        # profile uses its default agent root and an XDG data root exists.
        if sys.platform in ('linux', 'darwin') and (not override or override == str(root / 'agent') or profile not in ('', 'default')):
            xdg = os.environ.get('XDG_DATA_HOME')
            if xdg:
                if not pathlib.Path(xdg).is_absolute():
                    return {'roots': [], 'documents': [], 'errors': ['远端原生目录环境变量应为绝对路径']}
                data = pathlib.Path(xdg) / 'omp'
                if profile and profile != 'default':
                    data = data / 'profiles' / profile
                if data.is_dir():
                    roots.append(str(data))
    work_configs = []
    codebuddy = None
    if provider == 'codebuddy':
        source = request.get('codebuddyIdeScript')
        if source is None:
            source = pathlib.Path(__file__).with_name('codebuddy_ide_remote.py').read_text(encoding='utf-8')
        codebuddy = {}
        exec(compile(source, '<aieyes-codebuddy-ide>', 'exec'), codebuddy)
        if 'pathsOnly' not in request:
            base = home / ('Library/Application Support/CodeBuddyExtension' if sys.platform == 'darwin' else 'AppData/Local/CodeBuddyExtension' if os.name == 'nt' else '.local/share/CodeBuddyExtension')
            roots.extend(str(root) for root in codebuddy['history_roots'](base))
    if provider == 'kimi' and 'pathsOnly' not in request:
        config = home / 'Library/Application Support' if sys.platform == 'darwin' else pathlib.Path(os.environ.get('APPDATA', str(home / 'AppData/Roaming'))) if os.name == 'nt' else pathlib.Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config')))
        data = config / 'Kimi'
        share = data / 'daimon-share'
        try:
            with (data / 'daimon-storage.json').open('rb') as stream:
                pointer = stream.read(65537)
            if len(pointer) <= 65536:
                decoded = json.loads(pointer)
                selected = decoded.get('shareDir', '') if isinstance(decoded, dict) else ''
                if selected and pathlib.Path(selected).is_absolute():
                    share = pathlib.Path(selected)
        except (OSError, ValueError, TypeError):
            pass
        shares = {share, home / '.kimi'}
        if os.environ.get('KIMI_SHARE_DIR') and pathlib.Path(os.environ['KIMI_SHARE_DIR']).is_absolute():
            shares.add(pathlib.Path(os.environ['KIMI_SHARE_DIR']))
        for share in shares:
            work = share / 'daimon/runtime/kimi-code/home'
            native = share / 'daimon/config.json'
            if work.is_dir() or native.is_file():
                roots.append(str(work))
                work_configs.append((str(work.resolve()), str(native)))
    aliases = {r: str(pathlib.Path(os.path.expanduser(r)).resolve()) for r in roots}
    roots = sorted(set(aliases.values()))
    paths = set(work_configs)
    for root in roots:
        if codebuddy is not None and codebuddy['user_id'](root):
            paths.add((root, root))
        patterns = [] if provider == 'cline' and 'pathsOnly' not in request and os.environ.get('CLINE_PROVIDER_SETTINGS_PATH') else request['patterns']
        for pattern in patterns:
            for path in sorted(glob.glob(str(pathlib.Path(root) / pattern)))[:MAX_FILES]:
                paths.add((root, path))
    primary = override or request.get('primary', str(home))
    if override and provider in ('opencode', 'kilo'):
        primary = str(pathlib.Path(override) / provider)
    anchor = str(pathlib.Path(os.path.expanduser(primary)).resolve())
    if 'pathsOnly' in request:
        paths = {(anchor, p) for p in request['pathsOnly']}
    elif provider == 'claude':
        paths.add((str((home / '.claude').resolve()), str(home / '.claude.json')))
    if 'pathsOnly' not in request and provider == 'antigravity':
        paths.add((anchor, str(home / '.gemini/antigravity-oauth-token')))
    if 'pathsOnly' not in request and provider == 'opencode':
        for root in roots:
            paths.add((root, str(pathlib.Path(root) / 'opencode.db')))
        root = pathlib.Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config'))) / 'opencode'
        for name in ('opencode.json', 'opencode.jsonc'):
            paths.add((anchor, str(root / name)))
    if 'pathsOnly' not in request and provider == 'cursor':
        config = home / 'Library/Application Support' if sys.platform == 'darwin' else pathlib.Path(os.environ.get('APPDATA', str(home / 'AppData/Roaming'))) if os.name == 'nt' else pathlib.Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config')))
        paths.add((anchor, str(config / 'Cursor/User/globalStorage/state.vscdb')))
    if 'pathsOnly' not in request and provider == 'cline' and os.environ.get('CLINE_PROVIDER_SETTINGS_PATH'):
        path = pathlib.Path(os.environ['CLINE_PROVIDER_SETTINGS_PATH'])
        if not path.is_absolute():
            return {'roots': roots, 'documents': [], 'errors': ['远端原生目录环境变量应为绝对路径']}
        paths.add((anchor, str(path)))
    if 'pathsOnly' not in request and provider == 'kilo':
        root = pathlib.Path(os.environ.get('KILO_CONFIG_DIR') or str(pathlib.Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config'))) / 'kilo'))
        for name in ('config.json', 'kilo.json', 'kilo.jsonc', 'opencode.json', 'opencode.jsonc'):
            paths.add((anchor, str(root / name)))
        if os.environ.get('KILO_CONFIG'):
            paths.add((anchor, os.environ['KILO_CONFIG']))
    if 'pathsOnly' not in request and provider == 'zcode':
        base = pathlib.Path(os.environ.get('ZCODE_DATA_BASE_DIR', str(home))) / '.zcode/v2'
        path = os.environ.get('ZCODE_PERSONAL_PROVIDER_CONFIG_FILE') or str(base / 'provider_config.json')
        paths.add((anchor, path))
        # OAuth JWT and login profile; parsed by the same Rust adapter as local files.
        paths.add((anchor, str(base / 'credentials.json')))
    if 'pathsOnly' not in request and provider == 'copilot':
        # Plain-text GitHub files only; no gh command or third-party keychain.
        config = pathlib.Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config')))
        for name in ('github-copilot/apps.json', 'github-copilot/hosts.json'):
            paths.add((anchor, str(config / name)))
        paths.add((anchor, str(pathlib.Path(os.environ.get('GH_CONFIG_DIR', str(config / 'gh'))) / 'hosts.yml')))
    copilot_env = copilot_environment(anchor) if provider == 'copilot' else None
    if copilot_env and 'pathsOnly' in request and copilot_env[0] not in request['pathsOnly']:
        copilot_env = None
    # Virtual references are handled above and never opened as relative files.
    paths = {(root, path) for root, path in paths if not path.startswith(('env:', 'aieyes:'))}
    if 'pathsOnly' not in request and provider == 'kiro':
        data = home / 'Library/Application Support' if sys.platform == 'darwin' else pathlib.Path(os.environ.get('XDG_DATA_HOME', str(home / '.local/share')))
        paths.add((anchor, str(data / 'kiro-cli/data.sqlite3')))
        paths.add((anchor, str(home / '.aws/sso/cache/kiro-auth-token.json')))
    if request.get('fingerprint'):
        watched = {path for _, path in sorted(paths)[:MAX_FILES]}
        if provider == 'antigravity' and 'pathsOnly' not in request:
            if sys.platform == 'darwin':
                base = home / 'Library/Application Support'
            elif sys.platform == 'win32':
                base = pathlib.Path(os.environ.get('APPDATA', str(home / 'AppData/Roaming')))
            else:
                base = pathlib.Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config')))
            # IDE auth stays in its dedicated reader; poll only store metadata.
            if base.is_absolute():
                for name in ('Antigravity IDE', 'Antigravity'):
                    watched.add(str(base / name / 'User/globalStorage/state.vscdb'))
        for path in list(watched):
            if pathlib.Path(path).name in ('opencode.db', 'agent.db', 'state.vscdb', 'workbuddy.db', 'data.sqlite3'):
                watched.add(path + '-wal')
        signatures = []
        for path in sorted(watched):
            try:
                metadata = os.stat(path)
                signatures.append((path, metadata.st_mtime_ns, metadata.st_size))
            except OSError:
                signatures.append((path, None, None))
        if copilot_env:
            signatures.append((copilot_env[0], copilot_projection(copilot_env[1])['aieyesTokenFingerprint'], None))
        digest = hashlib.sha256(json.dumps([roots, len(paths), signatures], ensure_ascii=True).encode()).hexdigest()
        return {'fingerprint': digest}
    documents, errors, size = [], [], 0
    if copilot_env:
        raw = json.dumps(copilot_projection(copilot_env[1])).encode('utf-8')
        documents.append({'root': anchor, 'path': copilot_env[0], 'data': base64.b64encode(raw).decode('ascii')})
        size += len(raw)
    for root, path in sorted(paths)[:MAX_FILES - len(documents)]:
        if provider == 'workbuddy' and os.path.basename(path) == 'workbuddy.db':
            # SQLite is never shipped; send the identity projection the Rust reader makes.
            try:
                if os.path.isfile(path):
                    raw = json.dumps(workbuddy_identity(path)).encode()
                    documents.append({'root': root, 'path': path, 'data': base64.b64encode(raw).decode('ascii')})
            except (OSError, sqlite3.Error):
                errors.append(path + '：原生配置无法读取')
            continue
        try:
            metadata = os.stat(path)
            if provider == 'kiro' and pathlib.Path(path).name == 'data.sqlite3' and stat.S_ISREG(metadata.st_mode):
                raw = kiro_credentials(path)
                size += len(raw)
                if len(raw) > LIMIT or size > TOTAL:
                    errors.append('原生配置超过扫描大小限制')
                    break
                documents.append({'root': root, 'path': path, 'data': base64.b64encode(raw).decode('ascii')})
                continue
            database = pathlib.Path(path).name in ('opencode.db', 'agent.db', 'state.vscdb')
            history_user = codebuddy['user_id'](path) if codebuddy is not None and stat.S_ISDIR(metadata.st_mode) else None
            if not history_user and (not stat.S_ISREG(metadata.st_mode) or not database and metadata.st_size > LIMIT):
                errors.append(path + '：原生配置不是普通文件或超过 1 MiB')
                continue
            if history_user:
                raw = json.dumps({'codebuddyHistoryUserId': history_user}).encode()
            elif database:
                raw = sqlite_document(path)
            else:
                with open(path, 'rb') as stream:
                    raw = stream.read(LIMIT + 1)
            if not database and not history_user and len(raw) != metadata.st_size:
                # Same as the Rust reader: a size change means it is still being written.
                errors.append(path + '：原生配置正在写入')
                continue
            size += len(raw)
            if len(raw) > LIMIT or size > TOTAL:
                errors.append('原生配置超过扫描大小限制')
                break
            if provider == 'zcode' and pathlib.Path(path).name == 'credentials.json' and pathlib.Path(path).parent.name == 'v2':
                raw = zcode_projection(raw)
            documents.append({'root': root, 'path': path, 'data': base64.b64encode(raw).decode('ascii')})
        except FileNotFoundError:
            continue
        except (OSError, sqlite3.Error):
            errors.append(path + '：原生配置无法读取')
        except (ValueError, subprocess.TimeoutExpired):
            errors.append(path + '：原生凭据格式不支持或无法解密')
        except Exception:
            errors.append(path + '：原生配置格式不支持')
    if len(paths) > MAX_FILES:
        errors.append('原生配置数量超过 128，请分别配置目录后扫描')
    return {'roots': [r for r in roots if os.path.isdir(r)], 'aliases': aliases, 'documents': documents, 'errors': errors}


def run_request(request):
    if request.get('action') == 'fingerprints':
        result = {}
        for target in request.get('targets', [])[:64]:
            result[target['provider']] = collect(dict(target, fingerprint=True))
        return {'fingerprints': result}
    return collect(request)


if __name__ == '__main__':
    try:
        request = json.loads(base64.b64decode(sys.argv[1], validate=True))
        result = run_request(request)
    except Exception:
        result = {'roots': [], 'documents': [], 'errors': ['远端扫描请求无效或原生配置不可读']}
    print(json.dumps(result, ensure_ascii=True))
