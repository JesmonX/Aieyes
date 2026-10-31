#!/usr/bin/env python3
"""Opt-in ACP permission, child-task and cancellation checks with real clients.

Uses the isolated setup and unmodified HTTP forwarding from the companion live
audit. Permission requests are held for three seconds, then approved for the
temporary test workspace. Cancellation uses the client's native ACP message.
"""
import argparse
import importlib.util
import json
import pathlib
import queue
import sqlite3
import subprocess
import threading
import time

spec = importlib.util.spec_from_file_location('native_audit', pathlib.Path(__file__).with_name('verify-agent-integration-live.py'))
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class UnverifiedLiveCase(RuntimeError):
    pass


class Acp:
    def __init__(self, audit, provider, config):
        self.audit, self.provider, self.sequence = audit, provider, 0
        self.workspace = audit.workspace(provider)
        (self.workspace / 'marker.txt').write_text('AIEYES_ACP_OK')
        program = audit.program(provider)
        if provider in ('kimi', 'kilo'):
            command = [program, 'acp']
            if provider == 'kilo':
                value = json.loads(config.read_text())
                value['permission'] = {'*': 'ask', 'bash': 'ask', 'read': 'allow', 'glob': 'allow', 'grep': 'allow'}
                config.write_text(json.dumps(value))
        elif provider == 'cline':
            command = [program, '--acp', '-P', 'openai-compatible', '-m', audit.args.model, '--auto-approve', 'false']
        else:
            command = [program, '--acp', '--setting-sources', 'user', '--agents',
                       json.dumps({'audit': {'description': 'Read the audit marker in a child task', 'prompt': 'Read marker.txt and return its exact content.', 'tools': ['Read']}})]
        self.process = subprocess.Popen(command, cwd=self.workspace, env=audit.environment(provider, config),
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=(self.workspace / 'acp.stderr').open('w'), text=True, start_new_session=True)
        self.messages = queue.Queue()
        self.permissions = []
        self.observations = []
        self.phases = {}
        self.max_visible = 0
        self.protocol = []
        self.raw = (self.workspace / 'acp.jsonl').open('w')

        def read():
            for line in self.process.stdout:
                self.raw.write(line)
                self.raw.flush()
                try:
                    self.messages.put(json.loads(line))
                except ValueError:
                    pass
        self.reader = threading.Thread(target=read, daemon=True)
        self.reader.start()

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
        tool_running = False
        while time.monotonic() < deadline:
            self.snapshot()
            # ACP is headless and must stay out of sessions.list. The protocol's
            # tool lifecycle, or our permission response, triggers cancellation.
            if cancel and not cancelled and (tool_running or approved_at is not None and time.monotonic() - approved_at > 2):
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
            if method == 'session/update':
                update = (message.get('params') or {}).get('update') or {}
                kind, status = update.get('sessionUpdate'), update.get('status')
                if kind in ('tool_call', 'tool_call_update'):
                    self.protocol.append(dict(update=kind, status=status, tool=update.get('kind')))
                    if status == 'in_progress':
                        tool_running = True
            elif method == 'session/request_permission':
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
                    if cancelled and message['error'].get('code') == -32800:
                        return dict(stopReason='cancelled', cancellationCode=-32800), cancelled
                    if session is None and message['error'].get('code') == -32000 and str(message['error'].get('message', '')).startswith('Authentication required:'):
                        raise UnverifiedLiveCase('ACP requires native account authentication; the isolated BYOK configuration cannot start a session')
                    raise RuntimeError(message['error'])
                return message.get('result'), cancelled
        raise UnverifiedLiveCase('ACP request exceeded 180 seconds without a terminal response')

    def run(self):
        initialized, _ = self.receive(self.request('initialize', dict(protocolVersion=1, clientCapabilities={}, clientInfo=dict(name='Aieyes audit', version='1'))))
        self.audit.results[self.provider]['authMethods'] = [dict(id=str(row.get('id', ''))[:100], name=str(row.get('name', ''))[:160]) for row in initialized.get('authMethods', [])]
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
            self.observations, self.permissions, self.max_visible, self.protocol = [], [], 0, []
            self.snapshot()
            identity = self.request('session/prompt', dict(sessionId=session, prompt=[dict(type='text', text=prompt)]))
            response, cancelled, error = {}, False, None
            try:
                response, cancelled = self.receive(identity, session, stage == 'cancel')
            except UnverifiedLiveCase as failure:
                error = str(failure)
                native.stop_process(self.process, group=True)
            until = time.monotonic() + 3
            while time.monotonic() < until:
                self.snapshot()
                time.sleep(.2)
            scans = self.audit.rpc('sources.scan', dict(sourceId=self.provider))
            after = self.audit.totals(self.provider)
            calls = [c for c in self.audit.calls if c['provider'] == self.provider and c['stage'] == stage and c['method'] == 'POST']
            result[stage] = dict(stopReason=response.get('stopReason'), cancelled=cancelled, permissions=self.permissions,
                                 protocol=self.protocol, observations=self.observations, maxVisibleSessions=self.max_visible, calls=calls,
                                 importedTokens=after['total'] - before['total'], httpTokens=self.audit.wire_total(calls),
                                 mainHttpTokens=self.audit.wire_total([c for c in calls if not c.get('auxiliary')]),
                                 projectRoot=str(self.workspace), scans=scans)
            if error:
                result[stage]['error'] = error
            if self.provider == 'kilo' and stage == 'cancel':
                result[stage]['nativeUsage'] = self.kilo_native_usage(session)
            if stage == 'child':
                result[stage]['nativeChildren'] = self.native_children()
            status, reason = case_status(stage, result[stage])
            result[stage].update(status=status, liveVerified=status == 'passed')
            if reason:
                result[stage]['reason'] = reason
            self.audit.results[self.provider].update(result)
            self.audit.save()
            if error:
                for remaining, _ in cases:
                    if remaining in self.audit.args.cases.split(',') and remaining not in result:
                        result[remaining] = dict(status='unverified', liveVerified=False,
                                                 reason='Previous ACP request did not finish; the isolated client was stopped')
                break
        result['replay'] = self.audit.verify_replay(self.provider)
        return result

    def kilo_native_usage(self, session):
        path = self.audit.sources[self.provider] / 'kilo.db'
        with sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True) as db:
            rows = [json.loads(row[0]) for row in db.execute('SELECT data FROM message WHERE session_id=?', (session,))]
        last = max((row for row in rows if row.get('role') == 'assistant'),
                   key=lambda row: (row.get('time') or {}).get('created', 0), default={})
        tokens = last.get('tokens') or {}
        cache = tokens.get('cache') or {}
        return dict(lastAssistantTokens=sum(tokens.get(key, 0) for key in ('input', 'output')) + sum(cache.get(key, 0) for key in ('read', 'write')),
                    lastAssistantError=(last.get('error') or {}).get('name'))

    def native_children(self):
        home = self.audit.sources[self.provider]
        if self.provider in ('cline', 'kilo'):
            path = home / ('db/sessions.db' if self.provider == 'cline' else 'kilo.db')
            table, parent = ('sessions', 'parent_session_id') if self.provider == 'cline' else ('session', 'parent_id')
            with sqlite3.connect(path.resolve().as_uri() + '?mode=ro', uri=True) as db:
                return db.execute(f'SELECT count(*) FROM {table} WHERE {parent} IS NOT NULL AND {parent}<>?', ('',)).fetchone()[0]
        if self.provider == 'codebuddy':
            return len(list(home.glob('projects/*/*/subagents/agent-*.jsonl')))
        return len([p for p in home.glob('sessions/*/*/agents/*/wire.jsonl') if p.parent.name != 'main'])

    def close(self):
        native.stop_process(self.process, group=True)
        self.reader.join(timeout=1)
        self.raw.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    native.add_arguments(parser, lifecycle=True)
    parser.add_argument('--cases', default='permission,child,cancel')
    args = parser.parse_args()
    if not args.cases or set(args.cases.split(',')) - {'permission', 'child', 'cancel'}:
        parser.error('Unknown or empty lifecycle case selection')
    results, providers = native.prepare(args, parser, lifecycle=True)
    if args.list:
        return
    if not providers:
        print(json.dumps(dict(results=results, liveVerified=False), indent=2))
        raise SystemExit(1 if args.require_live else 0)
    audit = native.Audit(args)
    audit.results = results
    print('Lifecycle audit: ' + str(audit.root), flush=True)
    try:
        audit.setup(providers)
        for provider in providers:
            client = None
            try:
                config = audit.configure(provider)
                client = Acp(audit, provider, config)
                result = audit.results[provider]
                result.update(client.run())
                verified = [stage for stage in args.cases.split(',') if result[stage]['status'] == 'passed']
                unverified = [stage for stage in args.cases.split(',') if result[stage]['status'] == 'unverified']
                failed = not result['replay']['stable'] or any(result[stage]['status'] == 'failed' for stage in args.cases.split(','))
                status = 'failed' if failed else ('partial' if verified else 'unverified') if unverified else 'passed'
                result.update(status=status, liveVerified=status == 'passed', verifiedCases=verified, unverifiedCases=unverified)
            except UnverifiedLiveCase as error:
                audit.results[provider].update(status='unverified', liveVerified=False, reason=str(error),
                                               verifiedCases=[], unverifiedCases=args.cases.split(','))
                for stage in args.cases.split(','):
                    audit.results[provider].setdefault(stage, dict(status='unverified', liveVerified=False, reason=str(error)))
            except Exception as error:
                audit.results[provider].update(status='failed', liveVerified=False, error=str(error))
            finally:
                if client:
                    try:
                        client.close()
                    except Exception as error:
                        audit.results[provider].update(status='failed', liveVerified=False, cleanupError=str(error))
            audit.save()
    finally:
        audit.close()
    failed = any(result['status'] == 'failed' or args.require_live and result['status'] != 'passed' for result in audit.results.values())
    raise SystemExit(1 if failed else 0)


def case_status(stage, case):
    if case['maxVisibleSessions'] or case['observations']:
        return 'failed', 'ACP session appeared in the visible session list'
    if any(call['status'] != 200 for call in case['calls']):
        return 'failed', 'The model endpoint returned an unsuccessful response'
    if case.get('error'):
        return 'unverified', case['error']
    if case['importedTokens'] != case['mainHttpTokens']:
        native_usage = case.get('nativeUsage') or {}
        if (stage == 'cancel' and native_usage.get('lastAssistantError') == 'MessageAbortedError'
                and native_usage.get('lastAssistantTokens') == 0):
            return 'unverified', 'Kilo did not persist usage for the cancelled assistant; HTTP/native reconciliation remains unverified'
        return 'failed', 'Imported native usage did not match the recorded model usage'
    if not case['mainHttpTokens']:
        return 'unverified', 'No positive model usage was observed'
    if stage == 'cancel':
        if case['cancelled'] and case['stopReason'] in ('cancelled', 'canceled'):
            return 'passed', None
        return 'unverified', 'Native cancellation acknowledgement was not observed'
    if case['stopReason'] != 'end_turn':
        return 'unverified', 'Native completion acknowledgement was not observed'
    if stage == 'permission' and not case['permissions']:
        return 'unverified', 'The native client did not request permission in this case'
    if stage == 'child' and not case['nativeChildren']:
        return 'unverified', 'A persisted native child session was not observed'
    return 'passed', None


def case_passed(stage, case):
    return case_status(stage, case)[0] == 'passed'


if __name__ == '__main__':
    main()
