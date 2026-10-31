"""Read-only Trae CLI trajectories and IDE process status. Never emit prompts."""
import datetime
import json
import os
import pathlib
import re
import time

LIMIT = 64 * 1024 * 1024


def files(root):
    root = pathlib.Path(root)
    root = root if root.name == 'trajectories' else root / 'trajectories'
    return sorted(p for p in root.glob('*.json') if not p.name.startswith('.') and p.is_file())


def read(path):
    with open(path, 'rb') as stream:
        raw = stream.read(LIMIT + 1)
    if len(raw) > LIMIT:
        raise ValueError('Trae trajectory exceeds 64 MiB')
    try:
        value = json.loads(raw)
        if not isinstance(value, dict) or not value.get('start_time'):
            raise ValueError()
        instant(value['start_time'])
        end = value.get('end_time', '')
        if end != '':
            instant(end)
        if 'success' in value and type(value['success']) is not bool:
            raise ValueError()
        if not isinstance(value.get('llm_interactions'), list):
            raise ValueError()
    except (ValueError, TypeError):
        raise ValueError('Invalid Trae trajectory metadata') from None
    return value, len(raw)


def instant(value):
    # Naive timestamps are local to the host writing the trajectory, not the desktop.
    if not isinstance(value, str) or not re.fullmatch(r'\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?', value):
        raise ValueError('Invalid Trae timestamp')
    return datetime.datetime.fromisoformat(value.replace('Z', '+00:00')).timestamp()


def timestamp(value):
    return int(instant(value))


def history(value):
    task = value['start_time']
    def count(value):
        return value if type(value) is int and value >= 0 else 0
    for call in value['llm_interactions']:
        if not isinstance(call, dict):
            continue
        model = call.get('model') or value.get('model')
        at = call.get('timestamp')
        if not isinstance(model, str) or not model or len(model) > 256 or not isinstance(at, str):
            continue
        response = call.get('response')
        usage = response.get('usage') if isinstance(response, dict) else None
        if not isinstance(usage, dict):
            usage = {}
        try:
            at_seconds = timestamp(at)
        except (ValueError, OverflowError):
            continue
        yield dict(id='call:' + at, sessionId='trae:' + task, timestamp=at_seconds, model=model,
                   tokens={k: count(usage.get(v)) for k, v in [('input', 'input_tokens'), ('output', 'output_tokens'), ('cacheRead', 'cache_read_input_tokens'), ('cacheWrite', 'cache_creation_input_tokens'), ('reasoning', 'reasoning_tokens')]})


def processes():
    """Linux SSH process metadata; only classification flags leave this helper."""
    output = []
    try:
        boot = next(int(line.split()[1]) for line in pathlib.Path('/proc/stat').read_text().splitlines() if line.startswith('btime '))
        ticks = os.sysconf('SC_CLK_TCK')
        directories = list(pathlib.Path('/proc').iterdir())
    except (OSError, ValueError, StopIteration):
        return output
    for directory in directories:
        if not directory.name.isdecimal():
            continue
        try:
            raw = (directory / 'stat').read_text()
            fields = raw[raw.rfind(')') + 2:].split()
            argv = [a.decode('utf-8', 'replace') for a in (directory / 'cmdline').read_bytes().split(b'\0') if a]
            try:
                values = (directory / 'environ').read_bytes().split(b'\0')
            except OSError:
                values = []
            cwd = pathlib.Path(os.readlink(directory / 'cwd'))
            # Trae initializes the trajectory before chdir(--working-dir).
            initial = next((v[4:].decode('utf-8', 'replace') for v in values if v.startswith(b'PWD=/')), None)
            output.append((boot + int(fields[19]) // ticks, argv, pathlib.Path(initial) if initial else cwd,
                           any(v.startswith(b'AIEYES_INTERNAL=') for v in values)))
        except (OSError, ValueError, IndexError, ZeroDivisionError):
            continue
    return output


def writer(path, process_rows, classify):
    path = pathlib.Path(path).resolve()
    try:
        stamp = datetime.datetime.strptime(path.name, 'trajectory_%Y%m%d_%H%M%S.json').timestamp()
    except ValueError:
        stamp = None
    matches = []
    for started, argv, cwd, internal in process_rows:
        verdict = classify('trae', argv)
        if verdict is None:
            continue
        target = next((argv[i + 1] for i, a in enumerate(argv[:-1]) if a in ('-t', '--trajectory-file')), None)
        target = target or next((a.split('=', 1)[1] for a in argv if a.startswith('--trajectory-file=')), None)
        if target:
            target = pathlib.Path(target)
            match = (target if target.is_absolute() else cwd / target).resolve() == path
        else:
            match = stamp is not None and started - 2 <= stamp <= started + 120 and (cwd / 'trajectories').resolve() == path.parent
        if match:
            matches.append('internal' if internal else verdict)
    return matches[0] if len(matches) == 1 else None


def ide_process(provider, executable, argv):
    executable = str(executable).replace('\\', '/')
    name = 'Trae CN' if provider == 'trae-cn' else 'Trae'
    base = executable.rsplit('/', 1)[-1].lower()
    main = executable.endswith('/' + name + '.app/Contents/MacOS/Electron') or base in (name.lower() + '.exe', 'trae-cn' if provider == 'trae-cn' else 'trae')
    return main and not any(a == '--type' or a.startswith('--type=') for a in argv)


def ide_running(provider):
    if pathlib.Path('/proc').is_dir():
        try:
            for directory in pathlib.Path('/proc').iterdir():
                if not directory.name.isdecimal():
                    continue
                try:
                    argv = [a.decode('utf-8', 'replace') for a in (directory / 'cmdline').read_bytes().split(b'\0') if a]
                    if ide_process(provider, (directory / 'exe').resolve(), argv):
                        return True
                except OSError:
                    continue
            return False
        except OSError:
            return None
    # macOS SSH hosts expose the app executable through ps without reading messages.
    if hasattr(os, 'uname') and os.uname().sysname == 'Darwin':
        import subprocess
        try:
            result = subprocess.run(['/bin/ps', '-axo', 'comm='], capture_output=True, text=True, timeout=3, check=True)
            return any(ide_process(provider, line.strip(), []) for line in result.stdout.splitlines())
        except (OSError, subprocess.SubprocessError):
            return None
    return None


def sessions(root, cursors, classify):
    recent, errors = [], []
    for path in files(root):
        try:
            meta = path.stat()
            if time.time() - meta.st_mtime < 86400:
                recent.append((meta.st_mtime, path, meta.st_size, meta.st_mtime_ns))
        except OSError:
            errors.append('Trae trajectory metadata is unavailable')
    recent.sort(reverse=True)
    process_rows = processes() if recent else []
    result = []
    for stamp, path, size, modified_ns in recent[:256]:
        cursor = cursors.get(str(path), {})
        try:
            value, _ = read(path)
            signature = f'trae:{size}:{modified_ns}'
            after = path.stat()
            if (after.st_size, after.st_mtime_ns) != (size, modified_ns):
                raise ValueError('Trae trajectory changed while reading')
            continuation = cursor.get('continuation') or {}
            classification = continuation.get('classification', 'unknown')
            # A reused custom file is a new run. Only that run's verdict is sticky.
            if continuation.get('task') != value['start_time']:
                classification = 'unknown'
            if classification not in ('headless', 'internal'):
                classification = writer(path, process_rows, classify) or classification
            ended = bool(value.get('end_time')) and instant(value['end_time']) >= instant(value['start_time'])
            kind = 'complete' if ended and value.get('success') is True else 'failed' if ended else 'start'
            event = dict(session=path.stem, turn=value['start_time'], at=int(stamp * 1000), classification=classification)
            result.append(dict(key=str(path), identity=path.stem, signature=signature, modified=int(stamp), reset=True, database=True,
                               events=[dict(event, kind='start'), dict(event, kind=kind)], continuation=dict(classification=classification, task=value['start_time'])))
        except (OSError, ValueError, TypeError, OverflowError):
            errors.append('Trae trajectory metadata is unavailable')
            if cursor.get('signature', '').startswith('trae:'):
                result.append(dict(key=str(path), identity=cursor.get('identity', ''), signature=cursor['signature'], modified=cursor.get('modified', 0), reset=False, events=[], database=True, continuation=cursor.get('continuation')))
    return result, max(len(recent) - 256, 0), errors
