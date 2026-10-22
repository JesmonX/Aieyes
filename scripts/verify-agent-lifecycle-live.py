#!/usr/bin/env python3
"""Opt-in ACP permission, child-task and cancellation checks with real clients.

Uses the isolated setup and unmodified HTTP forwarding from the companion live
audit. Permission requests are held for three seconds, then approved for the
temporary test workspace. Cancellation uses the client's native ACP message.
"""
import argparse
import importlib.util
import json
import os
import pathlib
import queue
import shutil
import signal
import subprocess
import threading
import time

spec = importlib.util.spec_from_file_location('native_audit', pathlib.Path(__file__).with_name('verify-agent-integration-live.py'))
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class Acp:
    def __init__(self, audit, provider, config):
        self.audit, self.provider, self.sequence = audit, provider, 0
        self.workspace = audit.root / provider / 'workspace'
        self.workspace.mkdir(exist_ok=True)
        (self.workspace / 'marker.txt').write_text('AIEYES_ACP_OK')
        command = [shutil.which(provider), 'acp'] if provider == 'kimi' else [shutil.which(provider), '--acp', '--setting-sources', 'user',
                   '--agents', json.dumps({'audit': {'description': 'Read the audit marker in a child task', 'prompt': 'Read marker.txt and return its exact content.', 'tools': ['Read']}})]
        self.process = subprocess.Popen(command, cwd=self.workspace, env=audit.environment(provider, config),
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=(self.workspace / 'acp.stderr').open('w'), text=True, start_new_session=True)
        self.messages = queue.Queue()
        self.permissions = []
        self.observations = []
        self.phases = {}
        self.max_visible = 0
        self.raw = (self.workspace / 'acp.jsonl').open('w')

        def read():
            for line in self.process.stdout:
                self.raw.write(line)
                self.raw.flush()
                try:
                    self.messages.put(json.loads(line))
                except ValueError:
                    pass
        threading.Thread(target=read, daemon=True).start()

    def send(self, value):
        self.process.stdin.write(json.dumps(value) + '\n')
        self.process.stdin.flush()

    def request(self, method, params):
        self.sequence += 1
        identity = 'audit-' + str(self.sequence)
        self.send(dict(jsonrpc='2.0', id=identity, method=method, params=params))
        return identity

    def snapshot(self):
        rows = [s for s in self.audit.rpc('sessions.list')['sessions'] if s['provider'] == self.provider]
        self.max_visible = max(self.max_visible, len(rows))
        for row in rows:
            if self.phases.get(row['id']) != row['phase']:
                self.phases[row['id']] = row['phase']
                self.observations.append(dict(id=row['id'], phase=row['phase']))
        return rows

    def receive(self, identity, session=None, cancel=False):
        deadline, pending, cancelled, approved_at = time.monotonic() + 180, [], False, None
        while time.monotonic() < deadline:
            rows = self.snapshot()
            if cancel and not cancelled and (any(r['phase'] == 'tool' for r in rows)
                                             or approved_at is not None and time.monotonic() - approved_at > 2):
                self.send(dict(jsonrpc='2.0', method='session/cancel', params=dict(sessionId=session)))
                cancelled = True
            for item in pending[:]:
                if time.monotonic() >= item[0]:
                    self.send(item[1])
                    approved_at = time.monotonic()
                    pending.remove(item)
            try:
                message = self.messages.get(timeout=.15)
            except queue.Empty:
                if self.process.poll() is not None:
                    raise RuntimeError('ACP client exited: ' + str(self.process.returncode))
                continue
            method = message.get('method')
            if method == 'session/request_permission':
                params = message.get('params') or {}
                options = params.get('options', [])
                choice = next((o for o in options if o.get('kind') == 'allow_once'), None)
                if choice is None:
                    choice = next((o for o in options if o.get('kind') == 'allow_always'), None)
                if choice is None:
                    raise RuntimeError('ACP permission has no supported allow option')
                self.permissions.append(dict(tool=params.get('toolCall', {}).get('kind'), option=choice['kind']))
                pending.append((time.monotonic() + 3, dict(jsonrpc='2.0', id=message['id'], result=dict(outcome=dict(outcome='selected', optionId=choice['optionId'])))))
            elif method and 'id' in message:
                self.send(dict(jsonrpc='2.0', id=message['id'], error=dict(code=-32601, message='Unsupported audit client method')))
            elif message.get('id') == identity:
                if 'error' in message:
                    raise RuntimeError(message['error'])
                return message.get('result'), cancelled
        raise TimeoutError('ACP request exceeded 180 seconds')

    def run(self):
        self.receive(self.request('initialize', dict(protocolVersion=1, clientCapabilities={}, clientInfo=dict(name='Aieyes audit', version='1'))))
        response, _ = self.receive(self.request('session/new', dict(cwd=str(self.workspace), mcpServers=[])))
        session = response['sessionId']
        result = {}
        cases = [
            ('permission', 'Use the shell tool once to run exactly: sleep 3; cat marker.txt . Then reply with the file content. Do not use agents.'),
            ('child', 'Delegate reading marker.txt to one child agent, wait for its result, then reply with the marker. For CodeBuddy use the audit subagent. Do not read the file in the main agent.'),
            ('cancel', 'Use the shell tool to run exactly: sleep 25; cat marker.txt . Do not use other tools.'),
        ]
        for stage, prompt in cases:
            if stage not in self.audit.args.cases.split(','):
                continue
            before = self.audit.totals(self.provider)
            self.audit.stage = stage
            self.observations, self.permissions, self.max_visible = [], [], 0
            self.snapshot()
            identity = self.request('session/prompt', dict(sessionId=session, prompt=[dict(type='text', text=prompt)]))
            response, cancelled = self.receive(identity, session, stage == 'cancel')
            until = time.monotonic() + 3
            while time.monotonic() < until:
                self.snapshot()
                time.sleep(.2)
            scans = self.audit.rpc('sources.scan', dict(sourceId=self.provider))
            after = self.audit.totals(self.provider)
            calls = [c for c in self.audit.calls if c['provider'] == self.provider and c['stage'] == stage and c['method'] == 'POST']
            result[stage] = dict(stopReason=response.get('stopReason'), cancelled=cancelled, permissions=self.permissions,
                                 observations=self.observations, maxVisibleSessions=self.max_visible, calls=calls,
                                 importedTokens=after['total'] - before['total'], httpTokens=self.audit.wire_total(calls),
                                 mainHttpTokens=self.audit.wire_total([c for c in calls if not c.get('auxiliary')]), scans=scans)
            if stage == 'child':
                home = self.audit.sources[self.provider]
                if self.provider == 'codebuddy':
                    children = list(home.glob('projects/*/*/subagents/agent-*.jsonl'))
                else:
                    children = [p for p in home.glob('sessions/*/*/agents/*/wire.jsonl') if p.parent.name != 'main']
                result[stage]['nativeChildren'] = len(children)
            self.audit.results[self.provider] = result
            self.audit.save()
        result['replay'] = self.audit.verify_replay(self.provider)
        return result

    def close(self):
        if self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGTERM)
            self.process.wait(timeout=10)
        self.raw.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--endpoint', required=True)
    parser.add_argument('--model', required=True)
    parser.add_argument('--providers', default='codebuddy,kimi')
    parser.add_argument('--cases', default='permission,child,cancel')
    parser.add_argument('--core', type=pathlib.Path, default=native.PROJECT / 'target/debug/aieyes-core')
    parser.add_argument('--output', type=pathlib.Path, default=native.PROJECT / '.local/agent-audit/lifecycle')
    args = parser.parse_args()
    audit = native.Audit(args)
    print('Lifecycle audit: ' + str(audit.root), flush=True)
    try:
        providers = args.providers.split(',')
        audit.setup(providers)
        for provider in providers:
            client = None
            try:
                config = audit.configure(provider)
                client = Acp(audit, provider, config)
                audit.results[provider] = client.run()
            except Exception as error:
                audit.results.setdefault(provider, {})['error'] = str(error)
            finally:
                if client:
                    client.close()
            audit.save()
    finally:
        audit.close()
    failed = False
    for result in audit.results.values():
        failed |= bool(result.get('error')) or not result.get('replay', {}).get('stable')
        for stage, case in result.items():
            if stage not in ('permission', 'child', 'cancel'):
                continue
            phases = {o['phase'] for o in case['observations']}
            failed |= case['importedTokens'] != case['mainHttpTokens']
            if stage == 'cancel':
                failed |= not case['cancelled'] or 'interrupted' not in phases
            else:
                failed |= case['stopReason'] != 'end_turn' or 'complete' not in phases
                if stage == 'permission':
                    failed |= not case['permissions'] or 'needsInput' not in phases
                if stage == 'child':
                    failed |= case['maxVisibleSessions'] != 1 or not case['nativeChildren']
    raise SystemExit(1 if failed else 0)


if __name__ == '__main__':
    main()
