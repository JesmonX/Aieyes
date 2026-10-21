"""Bounded, read-only native config transport. stdout is private core input.

Secrets are never sent to the UI or persisted by this helper. Parsing uses the
same Rust adapters as local discovery; no TOML/YAML dependencies on SSH hosts.
"""
import base64
import glob
import json
import os
import pathlib
import stat
import sys

LIMIT = 1024 * 1024
MAX_FILES = 128
TOTAL = 8 * 1024 * 1024


def collect(request):
    provider = request['provider']
    home = pathlib.Path.home()
    roots = list(request['roots'])
    variable = request.get('env', '')
    override = os.environ.get(variable) if variable else None
    if override:
        if not pathlib.Path(override).is_absolute():
            return {'roots': [], 'documents': [], 'errors': ['远端原生目录环境变量应为绝对路径']}
        roots.append(str(pathlib.Path(override) / 'opencode' if provider == 'opencode' else pathlib.Path(override)))
    if provider == 'omp':
        config = pathlib.Path(os.environ.get('PI_CONFIG_DIR', '.omp'))
        profile = os.environ.get('OMP_PROFILE', os.environ.get('PI_PROFILE', ''))
        if config.is_absolute() or '..' in config.parts or '/' in profile or '\\' in profile or profile in ('.', '..'):
            return {'roots': [], 'documents': [], 'errors': ['远端 OMP Profile 配置无效']}
        root = home / config
        if profile and profile != 'default':
            root = root / 'profiles' / profile
        roots.append(str(root / 'agent'))
    aliases = {r: str(pathlib.Path(os.path.expanduser(r)).resolve()) for r in roots}
    roots = sorted(set(aliases.values()))
    paths = set()
    for root in roots:
        for pattern in request['patterns']:
            for path in sorted(glob.glob(str(pathlib.Path(root) / pattern)))[:MAX_FILES]:
                paths.add((root, path))
    primary = override or request.get('primary', str(home))
    if override and provider == 'opencode':
        primary = str(pathlib.Path(override) / 'opencode')
    anchor = str(pathlib.Path(os.path.expanduser(primary)).resolve())
    if 'pathsOnly' in request:
        paths = {(anchor, p) for p in request['pathsOnly']}
    elif provider == 'claude':
        paths.add((str((home / '.claude').resolve()), str(home / '.claude.json')))
    if 'pathsOnly' not in request and provider == 'antigravity':
        paths.add((anchor, str(home / '.gemini/antigravity-oauth-token')))
    if 'pathsOnly' not in request and provider == 'opencode':
        root = pathlib.Path(os.environ.get('XDG_CONFIG_HOME', str(home / '.config'))) / 'opencode'
        for name in ('opencode.json', 'opencode.jsonc'):
            paths.add((anchor, str(root / name)))
    if 'pathsOnly' not in request and provider == 'zcode':
        path = os.environ.get('ZCODE_PERSONAL_PROVIDER_CONFIG_FILE') or str(pathlib.Path(os.environ.get('ZCODE_DATA_BASE_DIR', str(home))) / '.zcode/v2/provider_config.json')
        paths.add((anchor, path))
    documents, errors, size = [], [], 0
    for root, path in sorted(paths)[:MAX_FILES]:
        try:
            metadata = os.stat(path)
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > LIMIT:
                errors.append(path + '：原生配置不是普通文件或超过 1 MiB')
                continue
            with open(path, 'rb') as stream:
                raw = stream.read(LIMIT + 1)
            size += len(raw)
            if len(raw) > LIMIT or size > TOTAL:
                errors.append('原生配置超过扫描大小限制')
                break
            documents.append({'root': root, 'path': path, 'data': base64.b64encode(raw).decode('ascii')})
        except FileNotFoundError:
            continue
        except OSError:
            errors.append(path + '：原生配置无法读取')
    if len(paths) > MAX_FILES:
        errors.append('原生配置数量超过 128，请分别配置目录后扫描')
    return {'roots': [r for r in roots if os.path.isdir(r)], 'aliases': aliases, 'documents': documents, 'errors': errors}


if __name__ == '__main__':
    try:
        request = json.loads(base64.b64decode(sys.argv[1], validate=True))
        result = collect(request)
    except Exception:
        result = {'roots': [], 'documents': [], 'errors': ['远端扫描请求无效或原生配置不可读']}
    print(json.dumps(result, ensure_ascii=True))
