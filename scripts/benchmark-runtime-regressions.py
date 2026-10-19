#!/usr/bin/env python3
"""Real IPC regressions with disposable data, synthetic SSH, and no network access."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import sqlite3
import statistics
import subprocess
import tempfile
import threading
import time
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


class Core:
    def __init__(self, binary, directory, path=None):
        env = dict(os.environ, AIEYES_DATA_DIR=str(directory))
        if path:
            env['PATH'] = str(path) + os.pathsep + env.get('PATH', '')
        self.child = subprocess.Popen([str(binary)], env=env, stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.changed = threading.Condition()
        self.sequence = 0
        self.replies = {}
        self.events = []
        self.closed = False
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def read(self):
        try:
            for line in self.child.stdout:
                value = json.loads(line)
                with self.changed:
                    entry = time.monotonic(), value
                    if 'id' in value:
                        self.replies[value['id']] = entry
                    else:
                        self.events.append(entry)
                    self.changed.notify_all()
        finally:
            with self.changed:
                self.closed = True
                self.changed.notify_all()

    def send(self, method, params=None):
        self.sequence += 1
        ticket = self.sequence, time.monotonic()
        self.child.stdin.write(json.dumps(dict(id=ticket[0], method=method, params=params or {})) + '\n')
        self.child.stdin.flush()
        return ticket

    def reply(self, ticket, timeout=20):
        with self.changed:
            assert self.changed.wait_for(lambda: ticket[0] in self.replies or self.closed, timeout), 'IPC response timed out'
            assert ticket[0] in self.replies, 'Core exited before its response'
            at, value = self.replies.pop(ticket[0])
        assert 'error' not in value, value
        return value['result'], (at - ticket[1]) * 1000

    def call(self, method, params=None):
        return self.reply(self.send(method, params))

    def event(self, predicate, since=0, timeout=10):
        deadline = time.monotonic() + timeout
        with self.changed:
            while True:
                for at, value in self.events:
                    if at >= since and predicate(value):
                        return at, value
                remaining = deadline - time.monotonic()
                assert remaining > 0 and not self.closed, 'Core event timed out'
                self.changed.wait(remaining)

    def close(self):
        self.child.stdin.close()
        try:
            self.child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.child.kill()
            self.child.wait()
        self.reader.join(timeout=2)
        self.child.stdout.close()
        self.child.stderr.close()


def settings(core, local=False):
    value = core.call('settings.get')[0]
    value.update(accounts=[], sources=[], hosts=[], proxy=dict(mode='direct', url=''),
        proxyTestUrls=['http://127.0.0.1:9'])
    value['localMonitor'].update(enabled=local, metrics=['memory'])
    return value


def host_benchmark(binary, directory, monitor):
    executables = directory / 'bin'
    executables.mkdir(parents=True)
    fake = executables / 'ssh'
    fake.write_text('''#!/usr/bin/env python3
import json,sys,time
target=next((arg for arg in sys.argv[1:] if arg in ('fixture-fast','fixture-slow')),None)
assert target, 'Only synthetic targets are allowed'
sys.stdin.buffer.read()
time.sleep(6 if target=='fixture-slow' else .08)
print(json.dumps(dict(timestamp=int(time.time()),monotonic=time.monotonic(),memory=dict(total=1024,available=512))),flush=True)
''')
    fake.chmod(0o700)
    core = Core(binary, directory / 'data', executables)
    try:
        value = settings(core, local=True)
        value['hosts'] = [dict(id=name, name=name, target='fixture-' + name,
            enabled=True, metrics=['memory']) for name in ('fast', 'slow')]
        core.call('settings.save', value)
        if not monitor:
            batch = core.send('hosts.sample', dict(stream=True))
            at, _ = core.event(lambda v: v.get('method') == 'operations.progress'
                and v['params'].get('row', {}).get('id') == 'fast', since=batch[1])
            queued = core.send('hosts.sample', dict(hostId='fast'))
            _, queued_ms = core.reply(queued)
            core.reply(batch)
            assert queued_ms > 4000, 'Synthetic slow peer should expose the old lane blockage'
            return dict(firstFastMs=round((at-batch[1])*1000, 3), queuedFastMs=round(queued_ms, 3))
        started = time.monotonic()
        _, configure_ms = core.call('hosts.monitor.configure', dict(intervalSeconds=60))
        def fast_after(v, version=0):
            return v.get('method') == 'hosts.changed' and any(row.get('id') == 'fast'
                and row.get('sampleVersion', 0) > version and row.get('sample') for row in v['params']['rows'])
        at, event = core.event(fast_after, since=started)
        first_ms = (at-started)*1000
        runs = []
        for _ in range(3):
            version = next(row['sampleVersion'] for row in event['params']['rows'] if row['id'] == 'fast')
            requested = time.monotonic()
            _, acknowledged_ms = core.call('hosts.monitor.refresh', dict(hostId='fast'))
            at, event = core.event(lambda v: fast_after(v, version), since=requested)
            assert 'slow' in event['params']['inFlight'], 'Slow peer must still be sampling'
            assert any(row.get('id') == 'local' and row.get('sample') for row in event['params']['rows'])
            runs.append(dict(acknowledgedMs=round(acknowledged_ms, 3), sampledMs=round((at-requested)*1000, 3)))
        requested = time.monotonic()
        _, paused_ms = core.call('hosts.monitor.configure', dict(intervalSeconds=60, paused=True))
        at, event = core.event(lambda v: v.get('method') == 'hosts.changed'
            and v['params']['paused'] and not v['params']['inFlight'], since=requested)
        assert not any(row.get('error') for row in event['params']['rows']), 'Cancellation is not a host failure'
        assert configure_ms < 1000 and max(r['sampledMs'] for r in runs) < 1500
        return dict(configureMs=round(configure_ms, 3), firstFastMs=round(first_ms, 3),
            repeatedFast=runs, pauseMs=round(paused_ms, 3), cancelledMs=round((at-requested)*1000, 3))
    finally:
        core.close()


def estimate_benchmark(binary, directory, lightweight):
    core = Core(binary, directory)
    db = None
    try:
        config = settings(core)
        quota = directory/'quota.json'
        quota.write_text(json.dumps(dict(windows=[dict(id='five', name='5h', usedPercent=25)])))
        config['accounts'] = [dict(id='fixture', provider='codex', name='Fixture', quotaEnabled=True, quotaSourceId='fixture')]
        config['sources'] = [dict(id='fixture', provider='codex', name='Fixture', accountId='fixture',
            path=str(directory/'empty'), quotaCommand='cat '+shlex.quote(str(quota)), enabled=True)]
        core.call('settings.save', config)
        db = sqlite3.connect(directory / 'aieyes.sqlite')
        rows = []
        for index in range(10000):
            payload = dict(id=f'q{index}', accountKey='codex:fixture', status='completed',
                startedAt=index, checkpointAt=index, weeklyValue=10+index,
                prices=[dict(id=f'frozen-model-{p}', input=1e-6, output=3e-6) for p in range(8)])
            rows.append(('quota', payload['id'], payload['accountKey'], 'completed', json.dumps(payload)))
        db.executemany('INSERT INTO estimates VALUES(?,?,?,?,?)', rows)
        db.commit()
        core.call('dashboard.summary', dict(estimateHistory='display'))
        methods = ['dashboard.summary', 'quotas.refresh']
        if lightweight:
            methods.append('dashboard.status')
        result = {}
        for method in methods:
            runs = []
            for index in range(5):
                db.execute("UPDATE estimates SET payload=json_set(payload,'$.weeklyValue',?) WHERE id='q9999'", (20000+index,))
                db.commit()
                value, elapsed = core.call(method, dict(estimateHistory='display'))
                if isinstance(value, dict) and 'quotaEstimates' in value:
                    assert len(value['quotaEstimates']) == 1, 'Display must retain the latest completed value'
                if method == 'quotas.refresh':
                    assert len(value) == 1 and not value[0].get('error'), value
                runs.append(round(elapsed, 3))
            result[method] = dict(afterEachChangeMs=runs, medianMs=round(statistics.median(runs), 3),
                replyBytes=len(json.dumps(value).encode()))
        return result
    finally:
        if db:
            db.close()
        core.close()


def observer_benchmark(source, root, name):
    module = dict(__name__=name, __file__=str(ROOT/'scripts/session_remote.py'))
    exec(compile(source, name, 'exec'), module)
    config = dict(id='fixture', provider='codex', path=str(root))
    try:
        first = module['collect'](config, {})
        assert first.get('error') is None, first.get('error')
        cursors = {row['key']: row for row in first['files']}
        assert len(cursors) == 256
        runs = []
        real_stat, real_open = Path.stat, Path.open
        for _ in range(5):
            counts = dict(stat=0, open=0)
            def counted_stat(path, *args, **kwargs):
                counts['stat'] += 1
                return real_stat(path, *args, **kwargs)
            def counted_open(path, *args, **kwargs):
                counts['open'] += 1
                return real_open(path, *args, **kwargs)
            with patch.object(Path, 'stat', counted_stat), patch.object(Path, 'open', counted_open):
                started = time.monotonic()
                value = module['collect'](config, cursors)
                elapsed = (time.monotonic()-started)*1000
            assert value.get('error') is None, value.get('error')
            assert len(value['files']) == 256 and all(not row['events'] for row in value['files'])
            runs.append(dict(ms=round(elapsed, 3), **counts))
        return dict(totalFiles=10000, recentFiles=256, runs=runs,
            medianMs=round(statistics.median(r['ms'] for r in runs), 3),
            note='Path.stat/Path.open calls, not all kernel syscalls; collector only, excluding SSH startup')
    finally:
        for discovery in module.get('_DISCOVERY', {}).values():
            discovery.watch.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, default=ROOT/'target/debug/aieyes-core')
    parser.add_argument('--before-ref', default='HEAD')
    parser.add_argument('--output', type=Path, default=ROOT/'.local/runtime-regressions.json')
    args = parser.parse_args()
    before_source = subprocess.check_output(['git', 'show', args.before_ref+':scripts/session_remote.py'], cwd=ROOT, text=True)
    current_source = (ROOT/'scripts/session_remote.py').read_text()
    report = dict(fixture='Disposable SQLite, fake ssh only, synthetic session files', finished=False,
        beforeObserverRef=args.before_ref, beforeObserverSha256=hashlib.sha256(before_source.encode()).hexdigest(),
        afterObserverSha256=hashlib.sha256(current_source.encode()).hexdigest(), slowHostSeconds=6)
    with tempfile.TemporaryDirectory(prefix='aieyes-runtime-regressions-') as temporary:
        root = Path(temporary)
        sessions = root/'sessions-source'/'sessions'
        sessions.mkdir(parents=True)
        old = time.time()-172800
        for index in range(10000):
            path = sessions/f'{index:05d}.jsonl'
            path.write_text('{"type":"session_meta","payload":{"id":"fixture"}}\n')
            if index < 9744:
                os.utime(path, (old, old))
        for name, binary, source in [('before', args.before.resolve(), before_source), ('after', args.after.resolve(), current_source)]:
            directory = root/name
            directory.mkdir()
            data = directory/'estimates'
            data.mkdir()
            result = dict(binary=str(binary), sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
            result['hosts'] = host_benchmark(binary, directory/'hosts', name == 'after')
            result['estimates10000'] = estimate_benchmark(binary, data, name == 'after')
            result['remoteUnchanged'] = observer_benchmark(source, sessions.parent, name)
            report[name] = result
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(report, indent=2)+'\n')
            print(json.dumps({name: result}), flush=True)
    report['finished'] = True
    args.output.write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
