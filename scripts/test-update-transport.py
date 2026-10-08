#!/usr/bin/env python3
"""Exercise the real child-process bridge with a recording proxy; no external traffic."""
import json
import os
import pathlib
import queue
import socketserver
import sqlite3
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
CORE = ROOT / 'target/debug/aieyes-core'
TOKEN = 'a' * 32
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))

class Proxy(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

class Reject(socketserver.StreamRequestHandler):
    def handle(self):
        self.connection.settimeout(3)
        line = self.rfile.readline().decode().strip()
        self.server.requests.put(line)
        while self.rfile.readline() not in (b'\r\n', b''):
            pass
        if self.server.hang:
            self.server.release.wait(20)
        else:
            self.wfile.write(b'HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n')

def config(root, proxy):
    with sqlite3.connect(root / 'aieyes.sqlite') as db:
        db.execute('CREATE TABLE IF NOT EXISTS kv(key TEXT PRIMARY KEY,value TEXT NOT NULL)')
        db.execute('INSERT OR REPLACE INTO kv VALUES (?,?)', ('settings', json.dumps({'proxy': proxy})))

def start(root):
    # Bad environment proxy confirms that a custom choice overrides inherited env.
    env = dict(os.environ, HTTPS_PROXY='http://127.0.0.1:1', ALL_PROXY='http://127.0.0.1:1', NO_PROXY='*')
    p = subprocess.Popen([str(CORE), '--data-dir', str(root), '--update-transport'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
    p.stdin.write(json.dumps({'feed': 'https://example.invalid/feed.xml', 'token': TOKEN}) + '\n'); p.stdin.flush()
    lines = queue.Queue()
    def read():
        for line in p.stdout:
            lines.put(json.loads(line))
    threading.Thread(target=read, daemon=True).start()
    endpoint = lines.get(timeout=6)['endpoint']
    return p, endpoint, lines

def request(url, status):
    try:
        with HTTP.open(url, timeout=23) as response:
            assert response.status == status
    except urllib.error.HTTPError as error:
        assert error.code == status, error.code

with Proxy(('127.0.0.1', 0), Reject) as proxy, tempfile.TemporaryDirectory(prefix='aieyes-update-transport-') as temporary:
    proxy.requests = queue.Queue(); proxy.hang = False; proxy.release = threading.Event()
    threading.Thread(target=proxy.serve_forever, daemon=True).start()
    root = pathlib.Path(temporary)
    config(root, {'mode': 'custom', 'url': f'http://127.0.0.1:{proxy.server_address[1]}'})
    p, endpoint, lines = start(root)
    try:
        request(endpoint.replace(TOKEN, 'wrong') + '/feed.xml', 403)
        assert proxy.requests.empty(), 'Invalid token must not reach the network'
        request(endpoint + '/feed.xml', 502)
        assert [proxy.requests.get(timeout=1) for _ in range(2)] == ['CONNECT example.invalid:443 HTTP/1.1'] * 2
        # A changed saved proxy is applied next cycle, not halfway through one.
        config(root, {'mode': 'direct', 'url': ''})
        request(endpoint + '/archive/update.dmg?' + urllib.parse.urlencode({'url': 'https://example.invalid/update.dmg'}), 502)
        assert [proxy.requests.get(timeout=1) for _ in range(2)] == ['CONNECT example.invalid:443 HTTP/1.1'] * 2
        request(endpoint + '/archive/update.dmg?' + urllib.parse.urlencode({'url': 'file:///tmp/update.dmg'}), 403)
        p.stdin.close(); assert p.wait(timeout=3) == 0
    finally:
        if p.poll() is None: p.kill(); p.wait()
    p, endpoint, lines = start(root)
    try:
        request(endpoint + '/feed.xml', 502)
        assert proxy.requests.empty(), 'Direct mode must ignore inherited proxies'
        p.stdin.close(); assert p.wait(timeout=3) == 0
    finally:
        if p.poll() is None: p.kill(); p.wait()
    config(root, {'mode': 'custom', 'url': f'http://127.0.0.1:{proxy.server_address[1]}'})
    proxy.hang = True
    p, endpoint, lines = start(root)
    def blocked_request():
        try: request(endpoint + '/feed.xml', 502)
        except Exception: pass
    thread = threading.Thread(target=blocked_request, daemon=True); thread.start()
    proxy.requests.get(timeout=4)
    started = time.monotonic(); p.stdin.close(); assert p.wait(timeout=3) == 0
    assert time.monotonic() - started < 3, 'Cancellation must interrupt blocked proxy requests'
    proxy.release.set(); proxy.shutdown()
print('Bridge: feed/archive use saved custom proxy; direct ignores environment proxy; bounded retry, token checks, cycle snapshots and in-flight cancellation passed')
