#!/usr/bin/env python3
"""Full autostart macOS app/real core soak, with disposable data and bundle identity."""
import argparse
import ctypes
import hashlib
import importlib.util
import json
import math
import os
import pathlib
import plistlib
import shutil
import signal
import sqlite3
import statistics
import subprocess
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]


class Usage(ctypes.Structure):
    _fields_ = [('uuid', ctypes.c_ubyte * 16)] + [(name, ctypes.c_uint64) for name in (
        'user_time', 'system_time', 'pkg_idle_wkups', 'interrupt_wkups', 'pageins', 'wired_size',
        'resident_size', 'phys_footprint', 'proc_start_abstime', 'proc_exit_abstime',
        'child_user_time', 'child_system_time', 'child_pkg_idle_wkups', 'child_interrupt_wkups',
        'child_pageins', 'child_elapsed_abstime', 'diskio_bytesread', 'diskio_byteswritten')]


LIBPROC = ctypes.CDLL('/usr/lib/libproc.dylib')
LIBPROC.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]


def processes(parent):
    rows = {}
    for line in subprocess.check_output(['ps', '-axo', 'pid=,ppid=,rss=,time=,comm='], text=True).splitlines():
        values = line.strip().split(None, 4)
        if len(values) == 5:
            pid, ppid, rss, cpu, name = values
            parts = cpu.replace('-', ':').split(':'); seconds = 0
            for part in parts: seconds = seconds * 60 + float(part)
            rows[int(pid)] = dict(parent=int(ppid), rssKiB=int(rss), cpuSeconds=seconds, name=pathlib.Path(name).name)
    selected = {parent}
    while True:
        children = {pid for pid, row in rows.items() if row['parent'] in selected}
        if children <= selected: break
        selected |= children
    return {pid: rows[pid] for pid in selected if pid in rows}


def summary_rpc(directory):
    methods = {}; notifications = {}
    for path in directory.glob('*.jsonl'):
        for line in path.read_text().splitlines():
            try: row = json.loads(line)
            except ValueError: continue
            if 'method' in row: methods.setdefault(row['method'], []).append(row)
            else: notifications[row['notification']] = notifications.get(row['notification'], 0) + 1
    return {method: dict(count=len(rows), errors=sum(r['error'] for r in rows),
        medianMs=round(statistics.median(r['ms'] for r in rows), 3), maxMs=max(r['ms'] for r in rows),
        replyBytes=sum(r['bytes'] for r in rows)) for method, rows in methods.items()}, notifications


def summarize_run(report, logs):
    """Derive metrics without changing the recorded samples or RPC timestamps."""
    warm = [row for row in report['samples'] if row['afterWarmup']]
    result = dict(postWarmupSamples=len(warm))
    if warm:
        def memory(key, scale):
            values = [row[key]/scale for row in warm if row.get(key) is not None]
            return dict(startMiB=round(values[0], 3), endMiB=round(values[-1], 3),
                peakMiB=round(max(values), 3), growthMiB=round(values[-1]-values[0], 3)) if values else None
        result.update(uiRss=memory('rssKiB', 1024), physicalFootprint=memory('physicalBytes', 1024*1024),
            coreRss=memory('coreRssKiB', 1024), coreCountRange=[min(r['coreCount'] for r in warm), max(r['coreCount'] for r in warm)])
        cpu = sorted(row['cpuPercent'] for row in warm[1:] if 'cpuPercent' in row)
        if cpu:
            result['uiCpuPercent'] = dict(median=round(statistics.median(cpu), 3),
                p95=cpu[math.ceil(len(cpu)*.95)-1], samples=len(cpu))
    phases = [('usage', 600, 1200), ('quotaOnly', 1200, 1800), ('idle', 1800, 2400)]
    buckets = {name: {} for name, _, _ in phases}
    for path in logs.glob('*.jsonl'):
        for line in path.read_text().splitlines():
            try: row = json.loads(line)
            except ValueError: continue
            if 'method' not in row: continue
            elapsed = row['at']-report['startedAt']
            for name, start, end in phases:
                if start <= elapsed < end:
                    bucket = buckets[name]
                    bucket[row['method']] = bucket.get(row['method'], 0)+1
    result['rpcByPhase'] = buckets
    result['phaseBoundariesSeconds'] = {name: [start, end] for name, start, end in phases}
    result['note'] = 'CPU uses process CPU-time deltas, memory starts at the first post-warmup sample; phase RPCs are bucketed by reply time. Window visibility is not inferred from these counters.'
    return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--app', type=pathlib.Path, default=ROOT/'dist/Aieyes.app')
    parser.add_argument('--output', type=pathlib.Path)
    parser.add_argument('--duration', type=int, default=2400)
    parser.add_argument('--warmup', type=int, default=600)
    parser.add_argument('--events', type=int, default=100000)
    args = parser.parse_args()
    (ROOT/'.local').mkdir(exist_ok=True)
    output = args.output.resolve() if args.output else pathlib.Path(tempfile.mkdtemp(prefix='runtime-ipc-', dir=ROOT/'.local'))
    if args.output: output.mkdir(parents=True, exist_ok=False)
    data = output/'data'; data.mkdir(); logs = output/'rpc'; logs.mkdir()
    app = output/'Aieyes Performance.app'; shutil.copytree(args.app.resolve(), app, symlinks=True)
    bundle = 'app.aieyes.performance.' + output.name.replace('_', '-').replace('.', '-')
    info_path = app/'Contents/Info.plist'; info = plistlib.loads(info_path.read_bytes())
    info.update(CFBundleIdentifier=bundle, CFBundleDisplayName='Aieyes Performance',
        AieyesDevelopmentDataDirectory=str(data), SUEnableAutomaticChecks=False, SUAllowsAutomaticUpdates=False)
    info_path.write_bytes(plistlib.dumps(info))
    subprocess.run(['codesign', '--force', '--sign', '-', str(app)], check=True, capture_output=True)
    binary = app/'Contents/Resources/aieyes-core'
    spec = importlib.util.spec_from_file_location('benchmark', ROOT/'scripts/benchmark-performance.py')
    benchmark = importlib.util.module_from_spec(spec); spec.loader.exec_module(benchmark)
    settings = benchmark.seed(data, args.events, binary)
    for source in settings['sources']:
        path = output/'empty-sources'/source['id']; path.mkdir(parents=True)
        source['path'] = str(path)
    observed = output/'observed-codex'; (observed/'sessions').mkdir(parents=True)
    session_file = observed/'sessions/live.jsonl'
    session_file.write_text(json.dumps(dict(type='session_meta', payload=dict(id='synthetic-live')))+'\n')
    settings['sources'].append(dict(id='synthetic-live', name='Isolated session', provider='codex',
        path=str(observed), enabled=True, accountId=''))
    settings.update(proxy=dict(mode='direct', url=''), proxyTestUrls=['http://127.0.0.1:9'],
        historyRefreshSeconds=180, serverRefreshSeconds=10, serverForegroundRefreshSeconds=2, menuMetric='icon')
    settings['localMonitor'].update(enabled=True, metrics=['cpu', 'memory'])
    core = benchmark.Core(data, binary)
    core.call('settings.save', settings); core.close()
    filters = json.dumps(dict(provider='all', source='all', account='all', model='all', days=30)).encode().hex()
    for key, kind, value in [('setup.seen.v1', '-bool', 'true'), ('detail.page', '-string', 'analysis'),
            ('dashboard.filters.v2.analysis', '-data', filters), ('aieyes.pet.enabled', '-bool', 'true')]:
        subprocess.run(['defaults', 'write', bundle, key, kind, value], check=True)
    proxy = output/'core-log.py'; shutil.copyfile(ROOT/'scripts/runtime-ipc-proxy.py', proxy); proxy.chmod(0o755)
    env = dict(os.environ, AIEYES_DATA_DIR=str(data), AIEYES_CORE_PATH=str(proxy),
        AIEYES_AUDIT_REAL_CORE=str(binary), AIEYES_AUDIT_LOG_DIR=str(logs))
    report = dict(version=info['CFBundleShortVersionString'], bundle=bundle, app=str(app), data=str(data),
        binarySha256=hashlib.sha256(binary.read_bytes()).hexdigest(), events=args.events, analysisDays=30,
        durationSeconds=args.duration, warmupSeconds=args.warmup, finished=False, samples=[], writes=dict(usage=0, quota=0),
        phases=['0–60s idle; 60–180s usage every 5s; 180–600s idle warmup',
                '600–1200s usage every 5s; 1200–1800s quota-only every 5s; 1800–2400s idle'],
        scenario='Actual AppModel autostart, requested 30-day analysis window, menu, pet, real local sampling and session IPC; isolated synthetic data. WindowServer occlusion may suspend detail refreshes.')
    (output/'implementation.patch').write_bytes(subprocess.check_output(['git', 'diff', '--binary'], cwd=ROOT))
    stream = open(output/'app.log', 'w')
    app_process = subprocess.Popen([str(app/'Contents/MacOS/Aieyes'), '--show-detail'], env=env, stdout=stream, stderr=stream)
    report.update(pid=app_process.pid, startedAt=time.time())
    print(json.dumps(dict(output=str(output), pid=app_process.pid, duration=args.duration)), flush=True)
    started = time.monotonic(); next_sample = 0; next_write = 60
    db = sqlite3.connect(data/'aieyes.sqlite', timeout=5)
    try:
        while time.monotonic()-started <= args.duration:
            elapsed = time.monotonic()-started
            if app_process.poll() is not None: raise RuntimeError('Development app exited early: '+str(app_process.returncode))
            if elapsed >= next_write:
                phase = 'usage' if 60 <= elapsed < 180 or 600 <= elapsed < 1200 else 'quota' if 1200 <= elapsed < 1800 else None
                if phase:
                    report['writes'][phase] += 1
                    if phase == 'usage':
                        db.execute("UPDATE events SET payload=json_set(payload,'$.tokens.input',?) WHERE id='e0'", (100+report['writes'][phase],))
                    else:
                        db.execute("INSERT OR REPLACE INTO kv VALUES('quota-attempt:synthetic-soak',?)", (json.dumps(dict(at=time.time())),))
                    db.commit()
                    if phase == 'usage':
                        with session_file.open('a') as target:
                            target.write(json.dumps(dict(type='event_msg', payload=dict(type='task_started' if report['writes'][phase]%2 else 'task_complete')))+'\n')
                next_write += 5
            if elapsed >= next_sample:
                tree = processes(app_process.pid); ui = tree[app_process.pid]
                usage = Usage(); ok = LIBPROC.proc_pid_rusage(app_process.pid, 2, ctypes.byref(usage)) == 0
                cores = [row for row in tree.values() if row['name'] == 'aieyes-core']
                sample = dict(elapsedSeconds=round(elapsed, 2), rssKiB=ui['rssKiB'], cpuSeconds=ui['cpuSeconds'],
                    physicalBytes=usage.phys_footprint if ok else None, coreCount=len(cores),
                    coreRssKiB=sum(row['rssKiB'] for row in cores), coreCpuSeconds=sum(row['cpuSeconds'] for row in cores),
                    descendantCount=len(tree)-1, afterWarmup=elapsed>=args.warmup)
                if report['samples']:
                    previous = report['samples'][-1]
                    sample['cpuPercent'] = round((sample['cpuSeconds']-previous['cpuSeconds'])/(elapsed-previous['elapsedSeconds'])*100, 3)
                report['samples'].append(sample)
                report['rpc'], report['notifications'] = summary_rpc(logs)
                if elapsed >= 30 and report['rpc'].get('hosts.monitor.configure', {}).get('count', 0) > 5:
                    raise RuntimeError('Unchanged host configuration repeatedly issued RPCs')
                (output/'report.json').write_text(json.dumps(report, indent=2))
                next_sample += 10
            time.sleep(0.25)
        report['finished'] = True
        subprocess.run(['sample', str(app_process.pid), '2', '-file', str(output/'sample-final.txt')], capture_output=True, timeout=15)
    finally:
        db.close()
        owned = processes(app_process.pid)
        app_process.terminate()
        try: app_process.wait(timeout=5)
        except subprocess.TimeoutExpired: app_process.kill(); app_process.wait()
        for pid in owned:
            if pid != app_process.pid:
                try: os.kill(pid, signal.SIGTERM)
                except ProcessLookupError: pass
        stream.close()
        report['elapsedSeconds'] = round(time.monotonic()-started, 2)
        report['rpc'], report['notifications'] = summary_rpc(logs)
        report['metrics'] = summarize_run(report, logs)
        (output/'report.json').write_text(json.dumps(report, indent=2))
        subprocess.run(['defaults', 'delete', bundle], capture_output=True)
    print(json.dumps(dict(output=str(output), complete=report['finished'])), flush=True)


if __name__ == '__main__': main()
