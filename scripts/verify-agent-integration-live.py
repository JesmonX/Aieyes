#!/usr/bin/env python3
"""Opt-in native-client audit against a real, supplied model endpoint.

Creates fresh client directories and an isolated Aieyes database. Only the
proxy's request metadata and response usage are retained in the report. Model
responses are forwarded unchanged; the two-second delay makes state observable.
Existing client settings and credentials are never copied or modified.
"""
import argparse
import http.client
import http.server
import json
import os
import pathlib
import shlex
import shutil
import signal
import sqlite3
import subprocess
import threading
import time
import urllib.parse

PROJECT = pathlib.Path(__file__).resolve().parents[1]
TOKEN_KEYS = ('input', 'output', 'cacheRead', 'cacheWrite')


class Audit:
    def __init__(self, args):
        self.args = args
        self.root = args.output.resolve() / ('live-' + str(time.time_ns()))
        self.root.mkdir(parents=True)
        self.calls, self.results, self.stage, self.sources = [], {}, 'configuration', {}
        self.core = None
        self.sequence = 0
        audit = self

        class Proxy(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                self.forward()

            def do_POST(self):
                self.forward()

            def forward(self):
                provider, _, path = self.path.lstrip('/').partition('/')
                data = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                stage = audit.stage
                if self.command == 'POST' and stage != 'configuration':
                    time.sleep(2)
                endpoint = urllib.parse.urlsplit(args.endpoint)
                connection_type = http.client.HTTPSConnection if endpoint.scheme == 'https' else http.client.HTTPConnection
                connection = connection_type(endpoint.hostname, endpoint.port, timeout=120)
                try:
                    connection.request(self.command, endpoint.path.rstrip('/') + '/' + path, data,
                                       {'Content-Type': 'application/json', 'Accept-Encoding': 'identity'})
                    response = connection.getresponse()
                    body = response.read()
                    usage = {}
                    try:
                        if response.getheader('Content-Type', '').startswith('text/event-stream'):
                            for line in body.decode().splitlines():
                                if line.startswith('data: ') and line[6:] != '[DONE]':
                                    frame = json.loads(line[6:])
                                    usage.update(frame.get('message', {}).get('usage') or {})
                                    usage.update(frame.get('usage') or {})
                        else:
                            usage = json.loads(body).get('usage') or {}
                    except (ValueError, UnicodeError, AttributeError):
                        pass
                    request = json.loads(data) if data else {}
                    names = [t.get('function', t).get('name') for t in request.get('tools', [])]
                    # These controlled CLI runs always offer tools to the main
                    # model. Their title/summary calls have a separate tool
                    # signature and may not persist usage in native history.
                    auxiliary = ((provider in ('grok', 'opencode', 'deepseek-harness') and (not names or names == ['session_title']))
                                 or (provider == 'codebuddy' and stage in ('permission', 'child', 'cancel') and not names))
                    audit.calls.append(dict(provider=provider, stage=stage, method=self.command, path='/' + path,
                                            status=response.status, model=request.get('model'), usage=usage,
                                            tools=len(request.get('tools') or []),
                                            toolNames=names, auxiliary=auxiliary))
                    self.send_response(response.status)
                    self.send_header('Content-Type', response.getheader('Content-Type', 'application/json'))
                    self.send_header('Content-Length', str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass
                finally:
                    connection.close()

        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.start_core()

    def start_core(self):
        self.core = subprocess.Popen([str(self.args.core.resolve()), '--data-dir', str(self.root / 'app')],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=(self.root / 'core.log').open('a'), text=True)

    def rpc(self, method, params=None):
        self.sequence += 1
        self.core.stdin.write(json.dumps(dict(jsonrpc='2.0', id=self.sequence, method=method, params=params or {})) + '\n')
        self.core.stdin.flush()
        while True:
            line = self.core.stdout.readline()
            if not line:
                raise RuntimeError('Aieyes core exited')
            response = json.loads(line)
            if response.get('id') != self.sequence:
                continue
            if response.get('error'):
                raise RuntimeError(response['error'])
            return response['result']

    def setup(self, providers):
        settings = self.rpc('settings.get')
        settings.update(sources=[], accounts=[], hosts=[], proxy=dict(mode='direct', url=''))
        for agent in settings['agents']:
            agent.update(enabled=agent['provider'] in providers, machineIds=['local'])
        for provider in providers:
            base = self.root / provider
            home = base / ('data/opencode' if provider == 'opencode' else 'agent' if provider in ('pi', 'omp') else 'home')
            home.mkdir(parents=True)
            settings['sources'].append(dict(id=provider, provider=provider, name=provider, path=str(home), enabled=True))
            self.sources[provider] = home
        self.rpc('settings.save', settings)

    def configure(self, provider):
        home = self.sources[provider]
        path = home / ({'deepseek-harness': 'profiles/headless/cordis.patch.yml', 'kimi': 'config.toml',
                        'grok': 'config.toml', 'pi': 'models.json', 'omp': 'models.yml'}.get(provider, 'settings.json'))
        if provider == 'opencode':
            path = self.root / provider / 'config/opencode/opencode.json'
        if provider == 'deepseek-harness':
            subprocess.run([shutil.which('dsh'), '--profile', 'headless', '--dump-config'],
                           env=self.environment(provider, path), stdout=subprocess.DEVNULL,
                           stderr=(self.root / provider / 'initialize.stderr').open('w'), timeout=120, check=True)
        path.parent.mkdir(parents=True, exist_ok=True)
        self.stage = 'configuration'
        old = self.rpc('agentApi.inspect', dict(sourceId=provider, configPath=str(path)))
        params = dict(sourceId=provider, configPath=str(path), revision=old['revision'],
                      baseUrl=f'http://127.0.0.1:{self.server.server_address[1]}/{provider}/v1',
                      model=self.args.model, noKey=True, effort='low', contextWindow=261000, maxOutputTokens=1024)
        if 'nativeRevision' in old:
            params['nativeRevision'] = old['nativeRevision']
        saved = self.rpc('agentApi.save', params)
        if not saved.get('verified'):
            raise RuntimeError('Native configuration did not verify')
        if provider == 'opencode':
            value = json.loads(path.read_text())
            value.update(autoupdate=False, permission={'*': 'deny', 'bash': 'allow'})
            path.write_text(json.dumps(value))
        if provider in ('claude', 'codebuddy'):
            self.rpc('sessions.hooks.repair', dict(sourceId=provider))
        return path

    def environment(self, provider, config):
        prefixes = ('ANTHROPIC_', 'CLAUDE_', 'OPENAI_', 'KIMI_', 'CODEBUDDY_', 'QODER_', 'GROK_', 'DSH_', 'PI_', 'OMP_')
        env = {k: v for k, v in os.environ.items() if not k.upper().startswith(prefixes)
               and k.lower() not in ('http_proxy', 'https_proxy', 'all_proxy')}
        home, base = self.sources[provider], self.root / provider
        env.update(NO_PROXY='127.0.0.1,localhost,' + urllib.parse.urlsplit(self.args.endpoint).hostname)
        for name, folder in [('XDG_CONFIG_HOME', 'config'), ('XDG_DATA_HOME', 'data'), ('XDG_CACHE_HOME', 'cache'), ('XDG_STATE_HOME', 'state')]:
            env[name] = str(base / folder)
        variable = {'claude': 'CLAUDE_CONFIG_DIR', 'codebuddy': 'CODEBUDDY_CONFIG_DIR', 'qoder': 'QODER_CONFIG_DIR', 'kimi': 'KIMI_CODE_HOME',
                    'grok': 'GROK_HOME', 'deepseek-harness': 'DSH_HOME', 'pi': 'PI_CODING_AGENT_DIR', 'omp': 'PI_CODING_AGENT_DIR'}
        if provider in variable:
            env[variable[provider]] = str(home)
        env.update(DISABLE_GALILEO='1', PI_OFFLINE='1', CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1')
        if provider == 'opencode':
            env['OPENCODE_CONFIG'] = str(config)
        return env

    def command(self, provider, prompt, tool=False, resume=False):
        name = {'deepseek-harness': 'dsh', 'qoder': 'qodercli'}.get(provider, provider)
        program = shutil.which(name)
        if not program and provider in ('pi', 'omp'):
            program = str(PROJECT / '.local/agent-integration' / ('pi-current-install/node_modules/.bin/pi' if provider == 'pi' else 'omp-install/bin/omp'))
        if not program or not pathlib.Path(program).exists():
            raise RuntimeError('Native client not installed: ' + name)
        if provider in ('pi', 'omp'):
            args = [program, '-p', prompt, '--mode', 'json', '--no-extensions', '--no-skills', '--system-prompt', 'Follow the user request.']
            args += ['--tools', 'bash'] if tool else ['--no-tools']
            args += ['--offline', '--no-context-files', '--no-mcp', '--no-prompt-templates', '--no-themes'] if provider == 'pi' else ['--no-title', '--no-lsp', '--no-rules']
        elif provider in ('claude', 'codebuddy'):
            args = [program, '-p', prompt, '--output-format', 'json', '--tools', 'Bash' if tool else '', '--max-turns', '4', '--setting-sources', 'user']
            if tool:
                args += ['--allowedTools', 'Bash']
        elif provider == 'kimi':
            args = [program, '-p', prompt, '--output-format', 'stream-json']
        elif provider == 'qoder':
            args = [program, '--config-dir', str(self.sources[provider]), '--output-format', 'json',
                    '--setting-sources', 'user', '--tools', 'Bash' if tool else '', '-p', prompt]
            if tool:
                args += ['--allowed-tools', 'Bash']
        elif provider == 'deepseek-harness':
            args = [program, 'headless', prompt]
        elif provider == 'opencode':
            args = [program, 'run', '--standalone', '--format', 'json', prompt]
        elif provider == 'grok':
            args = [program, '-p', prompt, '--output-format', 'json', '--tools', 'Bash' if tool else 'read_file',
                    '--no-subagents', '--disable-web-search', '--max-turns', '4']
            if tool:
                args += ['--allow', 'Bash(sleep:*)', '--allow', 'Bash(cat:*)']
        if resume:
            args += ['--continue']
        return args

    def totals(self, provider):
        with sqlite3.connect(self.root / 'app/aieyes.sqlite') as db:
            rows = [json.loads(r[0]) for r in db.execute('SELECT payload FROM events WHERE provider=?', (provider,))]
        return dict(events=len(rows), total=sum(e['tokens'].get(k, 0) for e in rows for k in TOKEN_KEYS),
                    models=sorted({e['model'] for e in rows}))

    @staticmethod
    def wire_total(calls):
        total = 0
        for call in calls:
            u = call.get('usage') or {}
            if 'total_tokens' in u:
                total += u['total_tokens']
            elif 'input_tokens' in u:
                total += sum(u.get(k, 0) for k in ('input_tokens', 'output_tokens', 'cache_read_input_tokens', 'cache_creation_input_tokens'))
        return total

    def run_case(self, provider, config, stage, prompt, tool=False, resume=False):
        workspace = self.root / provider / 'workspace'
        workspace.mkdir(exist_ok=True)
        marker = 'AIEYES_' + provider.upper().replace('-', '_') + '_OK'
        (workspace / 'marker.txt').write_text(marker)
        prompt = (prompt.replace('{marker}', marker)
                  .replace('{marker_file}', shlex.quote(str(workspace / 'marker.txt')))
                  .replace('{workspace}', str(workspace)))
        prior = {s['id']: s['phase'] for s in self.rpc('sessions.list')['sessions'] if s['provider'] == provider}
        before = self.totals(provider)
        self.stage = stage
        observations = []
        with (workspace / (stage + '.stdout')).open('w') as out, (workspace / (stage + '.stderr')).open('w') as err:
            client = subprocess.Popen(self.command(provider, prompt, tool, resume), cwd=workspace,
                                      env=self.environment(provider, config), stdin=subprocess.DEVNULL, stdout=out, stderr=err,
                                      start_new_session=True)
            deadline, finished = time.monotonic() + 180, None
            try:
                while time.monotonic() < deadline:
                    snap = self.rpc('sessions.list')
                    for session in snap['sessions']:
                        if session['provider'] == provider and prior.get(session['id']) != session['phase']:
                            observations.append(dict(id=session['id'], phase=session['phase']))
                            prior[session['id']] = session['phase']
                    if client.poll() is not None:
                        finished = finished or time.monotonic()
                        if time.monotonic() - finished > 3:
                            break
                    time.sleep(.25)
                if client.poll() is None:
                    raise TimeoutError('Native client exceeded 180 seconds')
            finally:
                if client.poll() is None:
                    os.killpg(client.pid, signal.SIGTERM)
                    client.wait(timeout=10)
        scans = self.rpc('sources.scan', dict(sourceId=provider))
        after = self.totals(provider)
        calls = [c for c in self.calls if c['provider'] == provider and c['stage'] == stage and c['method'] == 'POST']
        imported, wire = after['total'] - before['total'], self.wire_total(calls)
        main_wire = self.wire_total([c for c in calls if not c.get('auxiliary')])
        text = (workspace / (stage + '.stdout')).read_text(errors='replace')
        return dict(exitCode=client.returncode, marker=marker in text, observations=observations, calls=calls,
                    importedTokens=imported, httpTokens=wire, mainHttpTokens=main_wire, auxiliaryHttpTokens=wire - main_wire,
                    usageMatches=imported == main_wire, scans=scans)

    def verify_replay(self, provider):
        before = self.totals(provider)
        repeat = self.rpc('sources.scan', dict(sourceId=provider))
        self.core.terminate()
        self.core.wait(timeout=10)
        self.start_core()
        restarted = self.rpc('sources.scan', dict(sourceId=provider))
        rescan = self.rpc('sources.rescan', dict(sourceId=provider))
        after = self.totals(provider)
        return dict(before=before, after=after, stable=before == after, repeat=repeat, restarted=restarted, rescan=rescan)

    def save(self):
        report = dict(endpoint=self.args.endpoint, model=self.args.model, root=str(self.root), results=self.results)
        (self.root / 'report.json').write_text(json.dumps(report, indent=2))
        print(json.dumps({p: dict(error=r.get('error'), stable=r.get('replay', {}).get('stable'),
                                 tokens={k: (v['importedTokens'], v['httpTokens']) for k, v in r.items()
                                         if k in ('normal', 'tool', 'multiTurn')})
                          for p, r in self.results.items()}), flush=True)

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        if self.core is not None:
            self.core.terminate()
            self.core.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--endpoint', required=True, help='Supplied OpenAI-compatible origin, without /v1')
    parser.add_argument('--model', required=True)
    parser.add_argument('--providers', default='pi,omp,codebuddy,kimi,deepseek-harness,opencode,grok,claude')
    parser.add_argument('--core', type=pathlib.Path, default=PROJECT / 'target/debug/aieyes-core')
    parser.add_argument('--output', type=pathlib.Path, default=PROJECT / '.local/agent-audit')
    args = parser.parse_args()
    audit = Audit(args)
    print('Live audit: ' + str(audit.root), flush=True)
    try:
        providers = args.providers.split(',')
        audit.setup(providers)
        for provider in providers:
            result = audit.results[provider] = {}
            try:
                config = audit.configure(provider)
                result['normal'] = audit.run_case(provider, config, 'normal', 'Reply exactly {marker}. Do not use tools or agents.')
                result['tool'] = audit.run_case(provider, config, 'tool', 'Use the shell tool once to run exactly `sleep 3; cat {marker_file}` with working directory {workspace}. Then reply with the file content only. Do not use other tools or agents.', tool=True)
                if provider not in ('deepseek-harness',):
                    result['multiTurn'] = audit.run_case(provider, config, 'multiTurn', 'Reply exactly {marker} again. Do not use tools or agents.', tool=provider in ('pi', 'omp'), resume=True)
                result['replay'] = audit.verify_replay(provider)
            except Exception as error:
                result['error'] = str(error)
            audit.save()
    finally:
        audit.close()
    failed = any(r.get('error') or not r.get('replay', {}).get('stable') or any(
        c.get('exitCode') or not c.get('marker') or not c.get('usageMatches') or not c.get('httpTokens')
        or any(call['status'] != 200 for call in c['calls'])
        or 'complete' not in {o['phase'] for o in c['observations']}
        for name, c in r.items() if name in ('normal', 'tool', 'multiTurn')) for r in audit.results.values())
    raise SystemExit(1 if failed else 0)


if __name__ == '__main__':
    main()
