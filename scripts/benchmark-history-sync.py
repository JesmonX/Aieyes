#!/usr/bin/env python3
"""Measure local import visibility while a disposable fake SSH source is slow."""
import argparse
import concurrent.futures
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT/'target/release/aieyes-core')
    parser.add_argument('--output', type=Path, default=ROOT/'.local/performance-ui/history-sync.json')
    args = parser.parse_args()
    os.environ['AIEYES_AUTO_SCAN'] = 'off'
    spec = importlib.util.spec_from_file_location('streaming', ROOT/'scripts/test-streaming-core.py')
    streaming = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(streaming)
    streaming.BINARY = args.binary.resolve()
    report = dict(binary=str(streaming.BINARY), samples=[], note='1,000 synthetic Codex records. Fake SSH returns an empty valid stream after two seconds; no credentials, accounts or network are used.')
    with tempfile.TemporaryDirectory(prefix='aieyes-sync-latency-') as temporary:
        # macOS /var is a symlink; managed source roots require canonical paths.
        base = Path(temporary).resolve()
        bin_dir = base/'bin'
        bin_dir.mkdir()
        ssh = bin_dir/'ssh'
        ssh.write_text('#!/usr/bin/env python3\nimport sys,time,json\nsys.stdin.read()\ntime.sleep(2)\nprint(json.dumps(dict(protocol=2,kind="done",files=0,events=0,readBytes=0,failedFiles=0)),flush=True)\n')
        ssh.chmod(0o700)
        for remote in [False, True]:
            folder = base/str(remote)
            history = folder/'history'
            history.mkdir(parents=True)
            records = [dict(type='session_meta', payload=dict(id='fixture')),
                       dict(type='turn_context', payload=dict(model='fixture'))]
            records += [dict(type='event_msg', timestamp=int(time.time())-1000+i,
                             payload=dict(type='token_count', info=dict(total_token_usage=dict(input_tokens=(i+1)*100))))
                        for i in range(1000)]
            (history/'fixture.jsonl').write_text(''.join(json.dumps(row)+'\n' for row in records))
            core = streaming.Core(folder/'data', bin_dir)
            try:
                settings = core.call('settings.get')
                settings.update(accounts=[], sources=[dict(id='local', provider='codex', accountId='', path=str(history), name='Synthetic local', enabled=True)], hosts=[])
                if remote:
                    settings['sources'].append(dict(id='remote', provider='codex', accountId='', path='/fixture', name='Synthetic remote', enabled=True, hostId='fixture'))
                    settings['hosts'] = [dict(id='fixture', target='synthetic.invalid', enabled=True)]
                settings['agents'] = [dict(provider='codex', enabled=True, machineIds=['local','fixture'])]
                settings['localMonitor']['enabled'] = False
                core.call('settings.save', settings)
                started = time.monotonic()
                first = None
                with concurrent.futures.ThreadPoolExecutor(1) as executor:
                    work = executor.submit(core.call, 'sources.scan')
                    with sqlite3.connect(folder/'data/aieyes.sqlite', timeout=1) as db:
                        while not work.done():
                            if first is None and db.execute('SELECT count(*) FROM event_sources WHERE source_id=?', ('local',)).fetchone()[0]:
                                first = (time.monotonic()-started)*1000
                            time.sleep(.01)
                        result = work.result()
                        complete = (time.monotonic()-started)*1000
                        assert db.execute('SELECT count(*) FROM event_sources WHERE source_id=?', ('local',)).fetchone()[0] == 1000
                assert all('error' not in row for row in result), result
                started = time.monotonic()
                unchanged = core.call('sources.scan')
                warm = (time.monotonic()-started)*1000
                assert all('error' not in row for row in unchanged), unchanged
                assert sum(row.get('result', {}).get('newEvents', 0) for row in unchanged) == 0
                sample = dict(remoteDelayMs=2000 if remote else 0, events=1000,
                              firstLocalVisibleMs=round(first if first is not None else complete, 2),
                              completeMs=round(complete, 2), unchangedMs=round(warm, 2))
                report['samples'].append(sample)
                print(json.dumps(sample), flush=True)
            finally:
                core.close()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2)+'\n')


if __name__ == '__main__':
    main()
