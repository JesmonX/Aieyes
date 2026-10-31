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
import queue
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
RECIPES = ('pi', 'omp', 'codebuddy', 'kimi', 'deepseek-harness', 'opencode', 'grok', 'claude', 'qoder', 'cline', 'kilo', 'trae', 'copilot')
NEW_PROVIDERS = ('workbuddy', 'copilot', 'cline', 'kilo', 'roo', 'kiro', 'trae', 'trae-cn')
RESUME_UNVERIFIED = {'cline': 'Cline 3.0.74 --id forces interactive mode and clears the prompt; JSON mode rejects it. This script has no verified interactive resume recipe',
                    'deepseek-harness': 'This isolated CLI recipe has no native resume command',
                    'trae': 'This isolated CLI recipe has no native resume command'}
DEFAULT_PROVIDERS = tuple(dict.fromkeys(('pi', 'omp', 'codebuddy', 'kimi', 'deepseek-harness', 'opencode', 'grok', 'claude') + NEW_PROVIDERS))
PROGRAMS = {'deepseek-harness': ('dsh',), 'qoder': ('qodercli', 'qoder'), 'kiro': ('kiro-cli', 'kiro'), 'trae': ('trae-cli',), 'trae-cn': ()}
APPLICATIONS = {'workbuddy': ('WorkBuddy',), 'copilot': (), 'cline': (), 'kilo': (), 'roo': (), 'kiro': ('Kiro',), 'trae': ('Trae',), 'trae-cn': ('Trae CN',)}
EXTENSIONS = {'cline': ('saoudrizwan.claude-dev',), 'kilo': ('kilocode.kilo-code',),
              'roo': ('rooveterinaryinc.roo-cline',), 'copilot': ('github.copilot', 'github.copilot-chat')}


def stop_process(process, group=False):
    if process.poll() is not None:
        return
    def stop(force):
        try:
            if group:
                os.killpg(process.pid, signal.SIGKILL if force else signal.SIGTERM)
            elif force:
                process.kill()
            else:
                process.terminate()
        except ProcessLookupError:
            pass
    stop(False)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        stop(True)
        process.wait(timeout=5)


def isolated_environment(base):
    """Do not inherit any credential, native-directory or Aieyes override."""
    base = pathlib.Path(base)
    env = {key: value for key, value in os.environ.items() if key in (
        'PATH', 'TMPDIR', 'TEMP', 'TMP', 'TERM', 'LANG', 'LC_ALL', 'SystemRoot', 'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'PATHEXT')}
    home = pathlib.Path(base) / 'user-home'
    home.mkdir(parents=True, exist_ok=True)
    env.update(HOME=str(home), USERPROFILE=str(home), APPDATA=str(base / 'config'), LOCALAPPDATA=str(base / 'local'),
               XDG_CONFIG_HOME=str(base / 'config'), XDG_DATA_HOME=str(base / 'data'),
               XDG_CACHE_HOME=str(base / 'cache'), XDG_STATE_HOME=str(base / 'state'),
               AIEYES_AUTO_SCAN='off', DO_NOT_TRACK='1')
    return env


def binary_overrides(values):
    result = {}
    for value in values:
        provider, separator, path = value.partition('=')
        if not separator or provider not in set(RECIPES + NEW_PROVIDERS) or not path:
            raise ValueError('--binary must be PROVIDER=/absolute/path/to/program')
        program = pathlib.Path(path).expanduser()
        if not program.is_absolute() or not program.is_file() or not os.access(program, os.X_OK):
            raise ValueError('Explicit native program is not executable: ' + provider)
        result[provider] = str(program)
    return result


def discover(provider, overrides=None, home=None):
    overrides, home = overrides or {}, home or pathlib.Path.home()
    program = overrides.get(provider)
    for name in PROGRAMS.get(provider, (provider,)):
        program = program or shutil.which(name)
    if not program and provider in ('pi', 'omp'):
        candidate = PROJECT / '.local/agent-integration' / ('pi-current-install/node_modules/.bin/pi' if provider == 'pi' else 'omp-install/bin/omp')
        if candidate.is_file() and os.access(candidate, os.X_OK):
            program = str(candidate)
    surfaces = []
    for application in APPLICATIONS.get(provider, ()):
        for root in (pathlib.Path('/Applications'), home / 'Applications'):
            path = root / (application + '.app')
            if path.is_dir():
                surfaces.append(str(path))
    for extension in EXTENSIONS.get(provider, ()):
        for directory in ('.vscode', '.vscode-insiders', '.cursor', '.windsurf', '.vscode-server'):
            surfaces.extend(str(path) for path in sorted((home / directory / 'extensions').glob(extension + '-*')) if path.is_dir())
    return dict(installed=bool(program or surfaces), program=program, surfaces=surfaces)


def availability(provider, installed, lifecycle=False):
    if not installed['installed']:
        return dict(status='skipped', reason='notInstalled', liveVerified=False, discovery=installed)
    supported = ('codebuddy', 'kimi', 'cline', 'kilo') if lifecycle else RECIPES
    if provider not in supported or not installed['program']:
        return dict(status='unverified', reason='No isolated %s recipe for the detected authenticated or desktop-only surface' % ('ACP lifecycle' if lifecycle else 'BYOK'),
                    liveVerified=False, discovery=installed)
    return dict(status='ready', liveVerified=False, discovery=installed)


def provider_list(value):
    providers = list(DEFAULT_PROVIDERS) if value in ('auto', 'all') else list(dict.fromkeys(p.strip() for p in value.split(',') if p.strip()))
    unknown = set(providers) - set(RECIPES + NEW_PROVIDERS)
    if unknown or not providers:
        raise ValueError('Unknown or empty provider selection: ' + ','.join(sorted(unknown)))
    return providers


def add_arguments(parser, lifecycle=False):
    parser.add_argument('--endpoint', help='Supplied OpenAI-compatible origin, without /v1')
    parser.add_argument('--model')
    parser.add_argument('--providers', default='auto', help='Comma-separated providers, or auto (detect installed clients)')
    parser.add_argument('--binary', action='append', default=[], metavar='PROVIDER=PATH', help='Explicit isolated CLI installation; repeat as needed')
    parser.add_argument('--list', action='store_true', help='Print installation and recipe coverage without starting clients or the core')
    parser.add_argument('--require-live', action='store_true', help='Fail if any requested provider or case remains skipped or unverified')
    parser.add_argument('--core', type=pathlib.Path, default=PROJECT / 'target/debug/aieyes-core')
    parser.add_argument('--output', type=pathlib.Path, default=PROJECT / '.local/agent-audit' / ('lifecycle' if lifecycle else 'integration'))


def prepare(args, parser, lifecycle=False):
    try:
        providers, overrides = provider_list(args.providers), binary_overrides(args.binary)
    except ValueError as error:
        parser.error(str(error))
    results = {provider: availability(provider, discover(provider, overrides), lifecycle) for provider in providers}
    args.programs = {provider: result['discovery']['program'] for provider, result in results.items()}
    if args.list:
        print(json.dumps(results, indent=2))
        return results, []
    runnable = [provider for provider, result in results.items() if result['status'] == 'ready']
    if runnable and (not args.endpoint or not args.model):
        parser.error('--endpoint and --model are required to run installed native clients')
    if runnable and not args.core.is_file():
        parser.error('Build aieyes-core or supply --core before running native clients')
    if args.endpoint:
        endpoint = urllib.parse.urlsplit(args.endpoint)
        if endpoint.scheme not in ('http', 'https') or not endpoint.hostname or endpoint.username or endpoint.password:
            parser.error('--endpoint must be an HTTP(S) origin without credentials')
    return results, runnable


class Audit:
    def __init__(self, args):
        self.args = args
        self.root = args.output.resolve() / ('live-' + str(time.time_ns()))
        self.root.mkdir(parents=True, mode=0o700)
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
                    auxiliary = ((provider in ('grok', 'opencode', 'kilo', 'deepseek-harness') and (not names or names == ['session_title']))
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
        env = isolated_environment(self.root / 'core')
        self.core = subprocess.Popen([str(self.args.core.resolve()), '--data-dir', str(self.root / 'app')],
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=(self.root / 'core.log').open('a'), text=True, env=env,
                                     cwd=self.root)
        replies, process = queue.Queue(), self.core
        self.replies = replies
        def read():
            for line in process.stdout:
                try:
                    replies.put(json.loads(line))
                except ValueError:
                    continue
            replies.put(None)
        threading.Thread(target=read, daemon=True).start()

    def rpc(self, method, params=None):
        self.sequence += 1
        self.core.stdin.write(json.dumps(dict(jsonrpc='2.0', id=self.sequence, method=method, params=params or {})) + '\n')
        self.core.stdin.flush()
        while True:
            try:
                response = self.replies.get(timeout=120)
            except queue.Empty as error:
                raise TimeoutError('Aieyes RPC timed out: ' + method) from error
            if response is None:
                raise RuntimeError('Aieyes core exited')
            if response.get('id') != self.sequence:
                continue
            if response.get('error'):
                raise RuntimeError(response['error'])
            return response['result']

    def setup(self, providers):
        settings = self.rpc('settings.get')
        settings.update(sources=[], accounts=[], hosts=[], nativeConnections=[], proxy=dict(mode='direct', url=''))
        for agent in settings['agents']:
            agent.update(enabled=agent['provider'] in providers, machineIds=['local'])
        for provider in providers:
            base = self.root / provider
            home = base / ('data/' + provider if provider in ('opencode', 'kilo', 'cline') else 'agent' if provider in ('pi', 'omp') else 'home')
            home.mkdir(parents=True)
            settings['sources'].append(dict(id=provider, provider=provider, name=provider, path=str(home), enabled=True))
            self.sources[provider] = home
        self.rpc('settings.save', settings)

    def configure(self, provider):
        home = self.sources[provider]
        url = f'http://127.0.0.1:{self.server.server_address[1]}/{provider}/v1'
        if provider == 'copilot':
            path = home / 'config.json'
            path.write_text('{}')
            return path
        if provider == 'cline':
            path = home / 'settings/providers.json'
            subprocess.run([self.program(provider), 'auth', '-p', 'openai-compatible', '-k', 'not-required', '-b', url, '-m', self.args.model],
                           cwd=self.root / provider, env=self.environment(provider, path), stdout=subprocess.DEVNULL,
                           stderr=(self.root / provider / 'configure.stderr').open('w'), timeout=60, check=True)
            return path
        if provider == 'kilo':
            path = self.root / provider / 'config/kilo/kilo.json'
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(dict(autoupdate=False, model='audit/' + self.args.model,
                permission={'*': 'deny', 'bash': 'allow'}, provider={'audit': dict(npm='@ai-sdk/openai-compatible', name='Isolated BYOK audit',
                    options=dict(baseURL=url, apiKey='not-required'), models={self.args.model: dict(name=self.args.model, limit=dict(context=100000, output=1024))})})))
            return path
        if provider == 'trae':
            path = self.root / provider / 'config.yaml'
            path.write_text(json.dumps(dict(model_providers={'openai': dict(provider='openai', api_key='not-required', base_url=url)},
                models={'audit': dict(model=self.args.model, model_provider='openai', temperature=0, top_p=1, top_k=0,
                                     parallel_tool_calls=False, max_retries=1, max_tokens=1024)},
                agents={'trae_agent': dict(model='audit', max_steps=4, tools=['bash', 'task_done'], enable_lakeview=False)})))
            return path
        path = home / ({'deepseek-harness': 'profiles/headless/cordis.patch.yml', 'kimi': 'config.toml',
                        'grok': 'config.toml', 'pi': 'models.json', 'omp': 'models.yml'}.get(provider, 'settings.json'))
        if provider == 'opencode':
            path = self.root / provider / 'config/opencode/opencode.json'
        if provider == 'deepseek-harness':
            subprocess.run([self.program(provider), '--profile', 'headless', '--dump-config'],
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
        home, base = self.sources[provider], self.root / provider
        env = isolated_environment(base)
        env.update(NO_PROXY='127.0.0.1,localhost,' + urllib.parse.urlsplit(self.args.endpoint).hostname)
        for name, folder in [('XDG_CONFIG_HOME', 'config'), ('XDG_DATA_HOME', 'data'), ('XDG_CACHE_HOME', 'cache'), ('XDG_STATE_HOME', 'state')]:
            env[name] = str(base / folder)
        variable = {'claude': 'CLAUDE_CONFIG_DIR', 'codebuddy': 'CODEBUDDY_CONFIG_DIR', 'qoder': 'QODER_CONFIG_DIR', 'kimi': 'KIMI_CODE_HOME', 'copilot': 'COPILOT_HOME',
                    'grok': 'GROK_HOME', 'deepseek-harness': 'DSH_HOME', 'pi': 'PI_CODING_AGENT_DIR', 'omp': 'PI_CODING_AGENT_DIR'}
        if provider in variable:
            env[variable[provider]] = str(home)
        env.update(DISABLE_GALILEO='1', PI_OFFLINE='1', CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1')
        if provider == 'opencode':
            env['OPENCODE_CONFIG'] = str(config)
        if provider == 'cline':
            env.update(CLINE_DIR=str(base / 'user-home/.cline'), CLINE_DATA_DIR=str(home),
                       CLINE_SESSION_DATA_DIR=str(home / 'sessions'), CLINE_DB_DATA_DIR=str(home / 'db'))
        if provider == 'kilo':
            env.update(KILO_CONFIG=str(config), KILO_DISABLE_AUTOUPDATE='1', KILO_DISABLE_MODELS_FETCH='1',
                       KILO_DISABLE_DEFAULT_PLUGINS='1', KILO_DISABLE_EXTERNAL_SKILLS='1',
                       KILO_DISABLE_CLAUDE_CODE='1', KILO_DISABLE_LSP_DOWNLOAD='1')
        if provider == 'copilot':
            env.update(COPILOT_PROVIDER_TYPE='openai', COPILOT_PROVIDER_WIRE_API='completions',
                       COPILOT_PROVIDER_BASE_URL=f'http://127.0.0.1:{self.server.server_address[1]}/copilot/v1',
                       COPILOT_PROVIDER_API_KEY='not-required', COPILOT_MODEL=self.args.model,
                       COPILOT_PROVIDER_MAX_PROMPT_TOKENS='100000', COPILOT_PROVIDER_MAX_OUTPUT_TOKENS='1024',
                       COPILOT_OFFLINE='true', COPILOT_AUTO_UPDATE='false', USE_TGREP='false')
        return env

    def program(self, provider):
        program = getattr(self.args, 'programs', {}).get(provider) or discover(provider)['program']
        if not program:
            raise RuntimeError('Native client not installed: ' + provider)
        return program

    def workspace(self, provider):
        workspace = self.root / provider / 'workspace'
        workspace.mkdir(exist_ok=True)
        if not (workspace / '.git').is_dir():
            env = isolated_environment(self.root / provider)
            env.update(GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull)
            subprocess.run(['git', '-c', 'init.templateDir=', 'init', '--quiet', str(workspace)],
                           env=env, cwd=workspace, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                           stderr=subprocess.PIPE, timeout=15, check=True)
        return workspace

    def command(self, provider, prompt, tool=False, resume=False):
        program = self.program(provider)
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
        elif provider == 'cline':
            args = [program, '--json', '-P', 'openai-compatible', '-m', self.args.model, '--timeout', '150', '--retries', '1', prompt]
            if resume:
                raise RuntimeError(RESUME_UNVERIFIED[provider])
        elif provider == 'kilo':
            args = [program, 'run', '--format', 'json', '-m', 'audit/' + self.args.model, prompt]
        elif provider == 'trae':
            path = self.sources[provider] / 'trajectories' / (self.stage + '.json')
            path.parent.mkdir(parents=True, exist_ok=True)
            args = [program, 'run', prompt, '--provider', 'openai', '--model', self.args.model,
                    '--model-base-url', f'http://127.0.0.1:{self.server.server_address[1]}/trae/v1', '--api-key', 'not-required',
                    '--config-file', str(self.root / provider / 'config.yaml'),
                    '--max-steps', '4', '--working-dir', str(self.root / provider / 'workspace'), '--trajectory-file', str(path), '--console-type', 'simple']
        elif provider == 'copilot':
            args = [program, '-p', prompt, '--model', self.args.model, '--output-format', 'json', '--stream', 'off',
                    '--allow-all-tools', '--available-tools=bash', '--no-auto-update',
                    '--disable-client-telemetry', '--disable-builtin-mcps', '--no-custom-instructions',
                    '--no-ask-user', '--no-bash-env', '--sync-session=off']
            if not resume:
                args.append('--trust-folder')
        else:
            raise RuntimeError('No isolated run recipe: ' + provider)
        if resume and provider != 'cline':
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
        workspace = self.workspace(provider)
        marker = 'AIEYES_' + provider.upper().replace('-', '_') + '_OK'
        (workspace / 'marker.txt').write_text(marker)
        prompt = (prompt.replace('{marker}', marker)
                  .replace('{marker_file}', shlex.quote(str(workspace / 'marker.txt')))
                  .replace('{workspace}', str(workspace)))
        if provider == 'trae':
            prompt = prompt.replace('Do not use tools or agents.', 'Do not use shell tools. Finish with task_done and include the exact marker in the conclusion.')
            prompt = prompt.replace('Do not use other tools or agents.', 'Finish with task_done after the requested shell command.')
        rows = [s for s in self.rpc('sessions.list')['sessions'] if s['provider'] == provider]
        prior = {s['id']: s['phase'] for s in rows}
        max_visible = len(rows)
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
                    max_visible = max(max_visible, sum(session['provider'] == provider for session in snap['sessions']))
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
                    stop_process(client, group=True)
        scans = self.rpc('sources.scan', dict(sourceId=provider))
        after = self.totals(provider)
        calls = [c for c in self.calls if c['provider'] == provider and c['stage'] == stage and c['method'] == 'POST']
        imported, wire = after['total'] - before['total'], self.wire_total(calls)
        main_wire = self.wire_total([c for c in calls if not c.get('auxiliary')])
        text = (workspace / (stage + '.stdout')).read_text(errors='replace')
        return dict(exitCode=client.returncode, marker=marker in text, observations=observations, calls=calls,
                    importedTokens=imported, httpTokens=wire, mainHttpTokens=main_wire, auxiliaryHttpTokens=wire - main_wire,
                    usageMatches=imported == main_wire, headlessExcluded=not max_visible,
                    maxVisibleSessions=max_visible, projectRoot=str(workspace), scans=scans)

    def verify_replay(self, provider):
        before = self.totals(provider)
        repeat = self.rpc('sources.scan', dict(sourceId=provider))
        stop_process(self.core)
        self.start_core()
        restarted = self.rpc('sources.scan', dict(sourceId=provider))
        rescan = self.rpc('sources.rescan', dict(sourceId=provider))
        after = self.totals(provider)
        return dict(before=before, after=after, stable=before == after, repeat=repeat, restarted=restarted, rescan=rescan)

    def save(self):
        report = dict(endpoint=self.args.endpoint, model=self.args.model, root=str(self.root),
                      isolatedHome=True, isolatedProject=True, automaticAccountScan=False, results=self.results)
        (self.root / 'report.json').write_text(json.dumps(report, indent=2))
        cases = ('normal', 'tool', 'multiTurnSeed', 'multiTurn', 'permission', 'child', 'cancel')
        print(json.dumps({p: dict(status=r.get('status'), reason=r.get('reason'), error=r.get('error'), stable=r.get('replay', {}).get('stable'),
                                 tokens={k: dict(importedTokens=v['importedTokens'], mainHttpTokens=v['mainHttpTokens'],
                                                 auxiliaryHttpTokens=v['httpTokens'] - v['mainHttpTokens']) for k, v in r.items()
                                         if k in cases and 'importedTokens' in v},
                                 caseReasons={k: v.get('reason') or v.get('error') for k, v in r.items()
                                              if k in cases and (v.get('reason') or v.get('error'))})
                          for p, r in self.results.items()}), flush=True)

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        if self.core is not None and self.core.poll() is None:
            stop_process(self.core)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    add_arguments(parser)
    parser.add_argument('--cases', default='normal,tool,multiTurn', help='Select normal, tool, and/or multiTurn')
    args = parser.parse_args()
    cases = args.cases.split(',')
    if not cases or set(cases) - {'normal', 'tool', 'multiTurn'}:
        parser.error('Unknown or empty case selection')
    results, providers = prepare(args, parser)
    if args.list:
        return
    if not providers:
        print(json.dumps(dict(results=results, liveVerified=False), indent=2))
        raise SystemExit(1 if args.require_live else 0)
    audit = Audit(args)
    audit.results = results
    print('Live audit: ' + str(audit.root), flush=True)
    try:
        audit.setup(providers)
        for provider in providers:
            result = audit.results[provider]
            try:
                config = audit.configure(provider)
                if 'normal' in cases:
                    result['normal'] = audit.run_case(provider, config, 'normal', 'Reply exactly {marker}. Do not use tools or agents.')
                if 'tool' in cases:
                    result['tool'] = audit.run_case(provider, config, 'tool', 'Use the shell tool once to run exactly `sleep 3; cat {marker_file}` with working directory {workspace}. Then reply with the file content only. Do not use other tools or agents.', tool=True)
                if 'multiTurn' in cases and provider not in RESUME_UNVERIFIED:
                    if not any(stage in cases for stage in ('normal', 'tool')):
                        result['multiTurnSeed'] = audit.run_case(provider, config, 'multiTurnSeed', 'Reply exactly {marker}. Do not use tools or agents.')
                        if not case_passed(result['multiTurnSeed']):
                            raise RuntimeError('The isolated seed session for the multi-turn case did not pass')
                    result['multiTurn'] = audit.run_case(provider, config, 'multiTurn', 'Reply exactly {marker} again. Do not use tools or agents.', tool=provider in ('pi', 'omp'), resume=True)
                elif 'multiTurn' in cases:
                    result['multiTurn'] = dict(status='unverified', reason=RESUME_UNVERIFIED[provider])
                result['replay'] = audit.verify_replay(provider)
                tested = [result[name] for name in cases if 'exitCode' in result.get(name, {})]
                valid = bool(tested) and result['replay']['stable'] and all(case_passed(case) for case in tested)
                unverified = [name for name in cases if result.get(name, {}).get('status') == 'unverified']
                result.update(status=('partial' if unverified else 'passed') if valid else ('failed' if tested else 'unverified'),
                              liveVerified=valid and not unverified, unverifiedCases=unverified,
                              verifiedCases=[name for name in cases if 'exitCode' in result.get(name, {}) and case_passed(result[name])])
            except Exception as error:
                result.update(status='failed', error=str(error), liveVerified=False)
            audit.save()
    finally:
        audit.close()
    failed = any(r['status'] == 'failed' or args.require_live and (r['status'] != 'passed' or r.get('unverifiedCases')) for r in audit.results.values())
    raise SystemExit(1 if failed else 0)


def case_passed(case):
    return (case.get('exitCode') == 0 and case.get('marker') and case.get('usageMatches')
            and case.get('httpTokens', 0) > 0 and case.get('headlessExcluded')
            and not case.get('maxVisibleSessions', 0) and not case.get('observations')
            and all(call['status'] == 200 for call in case.get('calls', [])))


if __name__ == '__main__':
    main()
