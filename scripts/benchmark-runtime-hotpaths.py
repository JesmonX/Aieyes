#!/usr/bin/env python3
"""Compare estimate projection and unchanged session reads in disposable stores."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import statistics
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('performance', ROOT / 'scripts/benchmark-performance.py')
performance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(performance)


def summarize(runs):
    return dict(coldMs=round(runs[0][1], 3), warmMedianMs=round(statistics.median(r[1] for r in runs[1:]), 3), replyBytes=runs[-1][2])


def estimates(binary, count, mode, directory):
    core = performance.Core(directory, binary)
    core.call('hello')
    db = sqlite3.connect(directory / 'aieyes.sqlite')
    rows = []
    for i in range(count):
        payload = dict(id=f'q{i}', accountKey='codex:fixture', status='completed', startedAt=i, checkpointAt=i, weeklyValue=10+i, prices=[dict(id='frozen-model-'+str(p), input=1e-6, output=3e-6) for p in range(8)])
        rows.append(('quota', payload['id'], 'codex:fixture', 'completed', json.dumps(payload)))
    db.executemany('INSERT INTO estimates VALUES(?,?,?,?,?)', rows)
    db.commit()
    db.close()
    try:
        runs = [core.call('dashboard.summary', {'estimateHistory':mode}) for _ in range(6)]
        result = summarize(runs)
        result['rows'] = len(runs[-1][0]['quotaEstimates'])
        return result
    finally:
        core.close()


def sessions(binary, directory):
    codex = directory / 'codex'
    agy = directory / 'agy'
    codex.mkdir()
    (agy / 'conversations').mkdir(parents=True)
    (agy / '.aieyes').mkdir()
    (agy / '.aieyes/session-hooks-disabled').write_text('disabled')
    old = time.time() - 172800
    for i in range(10000):
        path = codex / f'old-{i}.jsonl'
        path.write_text('{}\n')
        os.utime(path, (old, old))
    (codex / 'live.jsonl').write_text('{"type":"event_msg","payload":{"type":"task_started"}}\n')
    for i in range(80):
        db = sqlite3.connect(agy / f'conversations/{i}.db')
        db.executescript(f"CREATE TABLE trajectory_meta(cascade_id TEXT); INSERT INTO trajectory_meta VALUES('fixture-{i}'); CREATE TABLE steps(idx INTEGER,step_type INTEGER,status INTEGER,step_payload TEXT); INSERT INTO steps VALUES(0,14,3,'');")
        db.close()
    core = performance.Core(directory / 'store', binary)
    try:
        settings = core.call('settings.get')[0]
        settings.update(accounts=[], hosts=[], sources=[dict(id='codex',name='Fixture',provider='codex',path=str(codex),enabled=True),dict(id='agy',name='Fixture AGY',provider='antigravity',path=str(agy),enabled=True)])
        settings['localMonitor']['enabled'] = False
        core.call('settings.save', settings)
        first = core.call('sessions.list')
        runs = []
        # Cross the old five-second directory discovery interval.
        for _ in range(9):
            time.sleep(1)
            runs.append(core.call('sessions.list'))
        return dict(firstMs=round(first[1],3), unchangedMs=[round(r[1],3) for r in runs], unchangedMedianMs=round(statistics.median(r[1] for r in runs),3), sessions=len(runs[-1][0]['sessions']))
    finally:
        core.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, default=ROOT/'target/debug/aieyes-core')
    parser.add_argument('--output', type=Path, default=ROOT/'.local/runtime-performance-implementation/hotpaths.json')
    args = parser.parse_args()
    report = {}
    with tempfile.TemporaryDirectory(prefix='aieyes-runtime-hotpaths-') as temporary:
        root = Path(temporary)
        for label, binary, mode in [('before',args.before.resolve(),'full'),('after',args.after.resolve(),'display')]:
            result = dict(binary=str(binary), estimates={})
            for count in (1000,10000):
                path=root/f'{label}-{count}'; path.mkdir()
                result['estimates'][str(count)]=estimates(binary,count,mode,path)
            path=root/f'{label}-sessions'; path.mkdir()
            result['sessions']=sessions(binary,path)
            report[label]=result
            args.output.parent.mkdir(parents=True,exist_ok=True)
            args.output.write_text(json.dumps(report,indent=2))
            print(json.dumps({label:result}),flush=True)


if __name__ == '__main__':
    main()
