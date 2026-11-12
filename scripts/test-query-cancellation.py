#!/usr/bin/env python3
"""Cancel a real cold aggregation through line RPC, without touching user data."""
import json
import pathlib
import tempfile
import time
from script_loader import load_script

ROOT = pathlib.Path(__file__).resolve().parents[1]
benchmark = load_script('benchmark', 'benchmark-performance.py')
binary = ROOT / 'target/release/aieyes-core'
with tempfile.TemporaryDirectory(prefix='aieyes-query-cancel-') as temporary:
    directory = pathlib.Path(temporary)
    benchmark.seed(directory, 100000, binary)
    core = benchmark.Core(directory, binary)
    try:
        core.call('hello')
        def send(identifier, method, params):
            core.process.stdin.write(json.dumps(dict(jsonrpc='2.0', id=identifier, method=method, params=params))+'\n')
            core.process.stdin.flush()
        send(100, 'dashboard', dict(days=365, operationId='cancel-me'))
        started = time.monotonic()
        progress = None
        while progress is None:
            value = json.loads(core.process.stdout.readline())
            if value.get('params', {}).get('kind') == 'dashboard.progress':
                progress = value['params']
            assert value.get('id') != 100, 'Cold query must report progress before completing'
            assert time.monotonic() - started < 30
        assert progress['operationId'] == 'cancel-me'
        cancel_at = time.monotonic()
        send(101, 'dashboard.cancel', dict(operationId='cancel-me'))
        replies = {}
        while len(replies) < 2:
            value = json.loads(core.process.stdout.readline())
            if value.get('id') in (100, 101): replies[value['id']] = value
        assert replies[101]['result']['cancelled'] == 1, replies
        assert 'error' in replies[100], replies
        elapsed = (time.monotonic() - cancel_at) * 1000
        assert elapsed < 2000, elapsed
        core.call('hello')
        print(json.dumps(dict(events=100000, progress=progress, cancelMs=round(elapsed, 2), reusable=True)))
    finally:
        core.close()
