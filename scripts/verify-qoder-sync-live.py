#!/usr/bin/env python3
"""Opt-in Qoder history and quota audit using an isolated Aieyes store.

Reads the existing Aieyes settings with SQLite mode=ro. Native history and login
stay in place; only read-only collectors and the CLI usage protocol are called.
The report contains counts/timings, never credentials, identities or transcripts.
"""
import argparse
import collections
import copy
import json
import os
import pathlib
import re
import selectors
import sqlite3
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]


class Core:
    def __init__(self, binary, directory):
        self.process = subprocess.Popen([str(binary), '--data-dir', str(directory)], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.pending, self.sequence = b'', 0

    def rpc(self, method, params=None, timeout=120):
        self.sequence += 1
        self.process.stdin.write((json.dumps(dict(jsonrpc='2.0', id=self.sequence, method=method, params=params or {})) + '\n').encode())
        self.process.stdin.flush()
        deadline = time.monotonic() + timeout
        while True:
            while b'\n' not in self.pending:
                if not self.selector.select(max(0, deadline - time.monotonic())):
                    raise RuntimeError(method + ': timed out')
                chunk = os.read(self.process.stdout.fileno(), 65536)
                if not chunk:
                    raise RuntimeError(method + ': core exited')
                self.pending += chunk
            line, self.pending = self.pending.split(b'\n', 1)
            response = json.loads(line)
            if response.get('id') != self.sequence:
                continue
            if response.get('error'):
                # Do not copy arbitrary backend diagnostics into a public report.
                raise RuntimeError(method + ': RPC error')
            return response['result']

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=10)
        self.selector.close()


def issue_codes(value):
    if isinstance(value, dict):
        if isinstance(value.get('code'), str):
            yield value['code']
        for item in value.values():
            yield from issue_codes(item)
    elif isinstance(value, list):
        for item in value:
            yield from issue_codes(item)


def failed_state_paths(value):
    if isinstance(value, dict):
        if value.get('code') == 'qoderStateDecrypt':
            match = re.match(r'(.*/state\.json):', value.get('message', ''))
            if match:
                yield match[1]
        for item in value.values():
            yield from failed_state_paths(item)
    elif isinstance(value, list):
        for item in value:
            yield from failed_state_paths(item)


def history_totals(directory, source_id):
    with sqlite3.connect((directory / 'aieyes.sqlite').as_uri() + '?mode=ro', uri=True) as db:
        rows = [json.loads(r[0]) for r in db.execute('SELECT e.payload FROM events e JOIN event_sources s ON s.event_id=e.id WHERE s.source_id=?', (source_id,))]
        states = {r[0] for r in db.execute('SELECT path FROM qoder_state WHERE source_id=?', (source_id,))}
        return dict(events=len(rows), tokens=sum(e['tokens'].get(k, 0) for e in rows for k in ('input', 'output', 'cacheRead', 'cacheWrite')), states=len(states)), states


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--settings-db', type=pathlib.Path, required=True)
    parser.add_argument('--ssh', default='144', help='Existing SSH alias to audit alongside local Qoder')
    parser.add_argument('--core', type=pathlib.Path, default=ROOT / 'target/debug/aieyes-core')
    parser.add_argument('--output', type=pathlib.Path, default=ROOT / '.local/qoder-sync-live')
    parser.add_argument('--repeats', type=int, default=3)
    args = parser.parse_args()
    if not 2 <= args.repeats <= 10:
        parser.error('--repeats must be between 2 and 10')
    output = args.output.resolve() / str(time.time_ns())
    output.mkdir(parents=True, mode=0o700)
    directory = output / 'app'
    selected, previous_failures = [], {}
    with sqlite3.connect(args.settings_db.resolve().as_uri() + '?mode=ro', uri=True) as db:
        saved = json.loads(db.execute("SELECT value FROM kv WHERE key='settings'").fetchone()[0])
        hosts = {h['id']: h for h in saved.get('hosts', [])}
        for source in saved.get('sources', []):
            if source.get('provider') != 'qoder' or not source.get('path') or source.get('subscription') or not source.get('enabled', True):
                continue
            location = hosts.get(source.get('hostId'), {}).get('target', 'local')
            if location not in ('local', args.ssh) or any(label == location for label, _ in selected):
                continue
            selected.append((location, copy.deepcopy(source)))
            status = db.execute('SELECT payload FROM source_status WHERE source_id=?', (source['id'],)).fetchone()
            previous_failures[location] = set(failed_state_paths(json.loads(status[0]))) if status else set()
    if {label for label, _ in selected} != {'local', args.ssh}:
        raise RuntimeError('Both local and requested SSH Qoder history sources must already exist')
    core = Core(args.core.resolve(), directory)
    report = dict(startedAt=int(time.time()), isolated=True, scope=['state_decryption', 'repeat_sync', 'quota_reads'], locations=[], passed=False)
    try:
        settings = core.rpc('settings.get')
        settings.update(sources=[], accounts=[], nativeConnections=[], proxy=saved.get('proxy', {}), preciseCounting=True)
        active_hosts = {source.get('hostId') for _, source in selected if source.get('hostId')}
        settings['hosts'] = [dict(copy.deepcopy(hosts[key]), monitoringEnabled=False) for key in active_hosts]
        for agent in settings['agents']:
            agent.update(enabled=agent['provider'] == 'qoder', machineIds=['local', *sorted(active_hosts)])
        for index, (location, original) in enumerate(selected):
            source = dict(original, id='history-' + str(index), name='Qoder history ' + str(index), accountId='')
            account_id = 'audit-' + str(index)
            quota = dict(original, id='quota-' + str(index), name='Qoder quota ' + str(index), accountId=account_id,
                         path='', subscription=dict(mode='cli', nativeConfigDir=original['path']))
            settings['sources'].extend([source, quota])
            settings['accounts'].append(dict(id=account_id, provider='qoder', name='Qoder audit ' + str(index), quotaEnabled=True))
        core.rpc('settings.save', settings)
        for index, (location, _) in enumerate(selected):
            source_id, quota_id, account_key = 'history-' + str(index), 'quota-' + str(index), 'qoder:audit-' + str(index)
            result = dict(location=location, previousDecryptFailures=len(previous_failures[location]), scans=[], quotas=[])
            report['locations'].append(result)
            for attempt in range(2):
                started = time.monotonic()
                scan = core.rpc('sources.scan', dict(sourceId=source_id), timeout=180)
                totals, paths = history_totals(directory, source_id)
                details = [row.get('result', row) for row in scan]
                result['scans'].append(dict(seconds=round(time.monotonic()-started, 3),
                                            partial=any(row.get('partial') is True for row in details),
                                            invalidRecords=sum(row.get('invalidRecords', 0) for row in details),
                                            issueCodes=dict(collections.Counter(issue_codes(details))), **totals))
            result['recoveredPreviousFailures'] = len(previous_failures[location] & paths)
            result['repeatStable'] = all(result['scans'][0][key] == result['scans'][1][key] for key in ('events', 'tokens', 'states'))
            observation = core.rpc('accounts.status.refresh', dict(accountKey=account_key, sourceId=quota_id), timeout=60)
            result['identityConfirmed'] = observation.get('identityConfirmed') is True and bool(observation.get('identity', {}).get('key'))
            if result['identityConfirmed']:
                settings = core.rpc('settings.get')
                next(a for a in settings['accounts'] if 'qoder:' + a['id'] == account_key)['identityKey'] = observation['identity']['key']
                core.rpc('settings.save', settings)
                for attempt in range(args.repeats):
                    started = time.monotonic()
                    response = core.rpc('quotas.refresh', dict(accountKey=account_key, sourceId=quota_id), timeout=60)
                    rows = response if isinstance(response, list) else response.get('quotas', response.get('rows', []))
                    ok = bool(rows) and all(not row.get('error') and row.get('updatedAt', 0) > 0 for row in rows)
                    result['quotas'].append(dict(seconds=round(time.monotonic()-started, 3), success=ok,
                                                 windows=sum(len(row.get('windows', [])) for row in rows),
                                                 updatedAt=max((row.get('updatedAt', 0) for row in rows), default=0)))
            print(json.dumps(result, ensure_ascii=False), flush=True)
        report['passed'] = all(r['repeatStable'] and r['identityConfirmed'] and len(r['quotas']) == args.repeats
                               and all(q['success'] for q in r['quotas'])
                               and all(not s['issueCodes'].get('qoderStateDecrypt') for s in r['scans'])
                               and r['previousDecryptFailures'] == r['recoveredPreviousFailures'] for r in report['locations'])
        report['historyComplete'] = all(not scan['partial'] for row in report['locations'] for scan in row['scans'])
    finally:
        core.close()
        (output / 'report.json').write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n')
        print('Report:', output / 'report.json', flush=True)
    if not report['passed']:
        raise SystemExit('Live verification has unresolved checks; see the redacted report.')


if __name__ == '__main__':
    main()
