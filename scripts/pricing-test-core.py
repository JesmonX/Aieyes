#!/usr/bin/env python3
"""Deterministic engine fixture: an unrelated request blocks its own IPC stream."""
import json
import sys
import time

for line in sys.stdin:
    request = json.loads(line)
    if request['method'] == 'test.block':
        print(json.dumps({'method': 'operations.progress', 'params': {'stage': 'blocked'}}), flush=True)
        time.sleep(1.5)
        result = {'id': 'completed'}
    elif request['method'] == 'prices.list':
        result = [{'id': 'local/fixture', 'name': 'Fixture', 'fetchedAt': 0}]
    else:
        raise AssertionError(request['method'])
    print(json.dumps({'id': request['id'], 'result': result}), flush=True)
