#!/usr/bin/python3
"""Transparent development IPC recorder. Logs sizes/timings, never RPC payloads."""
import json
import os
import pathlib
import signal
import subprocess
import sys
import threading
import time

child = subprocess.Popen([os.environ['AIEYES_AUDIT_REAL_CORE'], *sys.argv[1:]],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
log = open(pathlib.Path(os.environ['AIEYES_AUDIT_LOG_DIR']) / (str(os.getpid()) + '.jsonl'), 'a', buffering=1)
pending = {}
guard = threading.Lock()


def stop(*_):
    if child.poll() is None:
        child.terminate()
        try: child.wait(timeout=3)
        except subprocess.TimeoutExpired: child.kill(); child.wait()
    if _: sys.exit(0)


signal.signal(signal.SIGTERM, stop)
signal.signal(signal.SIGINT, stop)


def forward():
    try:
        for line in sys.stdin.buffer:
            value = json.loads(line)
            with guard: pending[value.get('id')] = (value.get('method'), time.monotonic())
            child.stdin.write(line); child.stdin.flush()
    except (BrokenPipeError, ValueError): pass
    finally:
        try: child.stdin.close()
        except OSError: pass


threading.Thread(target=forward, daemon=True).start()
try:
    for line in child.stdout:
        value = json.loads(line)
        record = dict(at=time.time(), bytes=len(line), corePid=child.pid)
        if 'id' in value:
            with guard: method, start = pending.pop(value['id'], ('unknown', time.monotonic()))
            record.update(method=method, ms=round((time.monotonic()-start)*1000, 3), error='error' in value)
        else: record.update(notification=value.get('method', 'unknown'))
        log.write(json.dumps(record)+'\n')
        try: sys.stdout.buffer.write(line); sys.stdout.buffer.flush()
        except BrokenPipeError: break
finally: stop()
