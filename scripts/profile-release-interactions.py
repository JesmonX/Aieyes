#!/usr/bin/env python3
"""Local macOS interaction audit. Private snapshots stay in ignored .local/ directories.

prepare: online SQLite backup + preferences, then benchmark read-only RPCs on copies.
bridge: fixed reply replay or an allowlisted real core, with all external work stubbed.
build: compile the same diagnostic driver with installed-version or workspace sources.
run: drive real NSHostingViews and report process CPU, latency, visibility and RPCs.
"""
import argparse
import ctypes
from contextlib import closing
import hashlib
import json
import math
import os
from pathlib import Path
import plistlib
import selectors
import signal
import shutil
import sqlite3
import statistics
import subprocess
import sys
import tarfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]
READS = {'hello', 'settings.get', 'prices.list', 'dashboard', 'dashboard.summary',
         'dashboard.status', 'runtime.revisions', 'quotaEstimates.list', 'creditEstimates.list',
         'accounts.status.get', 'wakeups.list', 'pets.current'}


def write_json(path, value):
    path = Path(path)
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
    temporary.replace(path)


def database_copy(source, target):
    target.parent.mkdir(parents=True, exist_ok=True)
    with closing(sqlite3.connect(source.resolve().as_uri() + '?mode=ro', uri=True)) as src:
        with closing(sqlite3.connect(target)) as dst:
            src.backup(dst)
    target.chmod(0o600)


def percentile(values, fraction=.95):
    return sorted(values)[max(0, math.ceil(len(values) * fraction) - 1)]


class Core:
    def __init__(self, binary, data):
        self.allowed=frozenset(READS)
        self.process = subprocess.Popen([str(binary), '--data-dir', str(data)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.buffer = b''
        self.sequence = 0

    def call(self, method, params=None):
        if method not in self.allowed:
            raise ValueError('Not an allowlisted local read: ' + method)
        self.sequence += 1
        started = time.monotonic()
        self.process.stdin.write(json.dumps(dict(jsonrpc='2.0', id=self.sequence,
            method=method, params=params or {})).encode() + b'\n')
        self.process.stdin.flush()
        deadline = started + 90
        while True:
            while b'\n' not in self.buffer:
                if not self.selector.select(max(0, deadline-time.monotonic())):
                    raise TimeoutError(method)
                data = os.read(self.process.stdout.fileno(), 65536)
                if not data:
                    raise RuntimeError('Core exited during ' + method)
                self.buffer += data
            line, self.buffer = self.buffer.split(b'\n', 1)
            row = json.loads(line)
            if row.get('id') != self.sequence:
                continue
            if 'error' in row:
                raise RuntimeError(method + ': ' + json.dumps(row['error'], ensure_ascii=False))
            return row['result'], (time.monotonic()-started)*1000, len(line)+1

    def close(self):
        self.selector.close()
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.process.stdout.close()


def dashboard_key(params):
    # UI tests use these exact day/provider scopes; unsupported filters fail closed.
    if params.get('sourceId') or params.get('model') or params.get('accountId') or params.get('dateRange'):
        raise ValueError('This replay does not record source/model/account/custom-date scopes')
    return str(params.get('days', 30)) + ':' + params.get('provider', 'all')


def validate_fixture(fixture):
    # dashboard.status always returns the production display projection. Compare
    # identities so a full-history summary cannot silently inflate the UI replay.
    status=fixture.get('dashboard.status',{})
    summary=fixture.get('dashboard.summary',{})
    for key in ['quotaEstimates','creditEstimates']:
        if key in status and {r['id'] for r in status[key]}!={r['id'] for r in summary.get(key,[])}:
            raise ValueError('Menu replay must match the display projection: '+key)


def prepare(args):
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True, mode=0o700)
    if (output/'snapshot').exists():
        raise FileExistsError('Refusing to replace an existing snapshot')
    source = Path.home()/'Library/Application Support/Aieyes'
    database_copy(source/'aieyes.sqlite', output/'snapshot/aieyes.sqlite')
    prefs_path = Path.home()/'Library/Preferences/app.aieyes.desktop.plist'
    prefs = plistlib.loads(prefs_path.read_bytes())
    prefixes = ('panel.', 'detail.', 'quota.expanded.', 'server.', 'dashboard.filters.',
                'main.', 'aieyes.pet.', 'menu.', 'appearance.', 'view.scroll.v2.')
    prefs = {k: v for k, v in prefs.items() if k.startswith(prefixes) or k.startswith('NSWindow Frame Aieyes')}
    (output/'preferences.plist').write_bytes(plistlib.dumps(prefs))
    (output/'preferences.plist').chmod(0o600)
    info = plistlib.loads((args.app/'Contents/Info.plist').read_bytes())
    installed = args.app/'Contents/Resources/aieyes-core'
    database = output/'snapshot/aieyes.sqlite'
    with sqlite3.connect(database) as db:
        settings = json.loads(db.execute("select value from kv where key='settings'").fetchone()[0])
        tables = {row[0] for row in db.execute("select name from sqlite_master where type='table'")}
        inventory = dict(version=info['CFBundleShortVersionString'], build=info['CFBundleVersion'],
            capturedAt=time.time(), databaseBytes=database.stat().st_size,
            counts={table: db.execute('select count(*) from "'+table+'"').fetchone()[0]
                    for table in ['events','event_sources','files','quotas','quota_history','estimates',
                                  'price_history','capacity_samples'] if table in tables},
            configuration={key: len(settings.get(key) or []) for key in ['accounts','sources','hosts','agents']},
            enabledSources=sum(s.get('enabled', True) for s in settings['sources']),
            enabledHosts=sum(h.get('enabled', True) for h in settings['hosts']),
            eventsByProvider=dict(db.execute('select provider,count(*) from events group by provider')),
            uiPage=prefs.get('detail.page'),
            installedUIHash=hashlib.sha256((args.app/'Contents/MacOS/Aieyes').read_bytes()).hexdigest(),
            installedCoreHash=hashlib.sha256(installed.read_bytes()).hexdigest(),
            workspaceHead=subprocess.check_output(['git','rev-parse','HEAD'], cwd=ROOT, text=True).strip())
    write_json(output/'inventory.json', inventory)
    # Copy only pet image resources, never auth directories or external source roots.
    (output/'assets').mkdir()
    for key in list(prefs):
        if key in ['aieyes.pet.position']:
            # Place the test pet on the measured display rather than a stale desktop.
            prefs[key] = '{100, 100}'
    (output/'preferences.plist').write_bytes(plistlib.dumps(prefs))
    report = {}
    for label, binary in [('installed', installed), ('workspace', args.core.resolve())]:
        data = output/(label+'-data')
        database_copy(database, data/'aieyes.sqlite')
        core = Core(binary, data)
        try:
            hello, initialization, _ = core.call('hello')
            rows = []
            for method, params in [('settings.get', {}), ('prices.list', {}),
                    ('dashboard.summary', {'estimateHistory':'display'}),
                    ('dashboard.status', {}),
                    *[('dashboard', {'days': days, 'estimateHistory':'display', **scope})
                      for scope in [{}, {'provider':'qoder'}] for days in [1,7,30,365]],
                    ('quotaEstimates.list', {}), ('creditEstimates.list', {})]:
                samples = [core.call(method, params) for _ in range(args.repetitions)]
                durations = [r[1] for r in samples]
                rows.append(dict(method=method, params=params, firstMs=durations[0],
                    hotMedianMs=statistics.median(durations[1:]), hotP95Ms=percentile(durations[1:]),
                    replyBytes=samples[-1][2], samplesMs=durations))
            # The same real history is read after a single local usage mutation.
            invalidations = []
            with sqlite3.connect(data/'aieyes.sqlite') as db:
                event = db.execute('select id from events order by stamp desc limit 1').fetchone()[0]
                for _ in range(args.repetitions):
                    db.execute('update events set cost=cost+0.000001 where id=?', (event,))
                    db.commit()
                    invalidations.append(core.call('dashboard', {'days':7,'estimateHistory':'display'})[1])
            rows.append(dict(method='dashboard.afterUsageChange', samplesMs=invalidations,
                hotMedianMs=statistics.median(invalidations), hotP95Ms=percentile(invalidations)))
            report[label] = dict(binaryHash=hashlib.sha256(binary.read_bytes()).hexdigest(),
                initializationMs=initialization, queries=rows)
            if label == 'installed':
                fixture = {method: core.call(method, {'estimateHistory':'display'} if method=='dashboard.summary' else {})[0] for method in [
                    'settings.get','prices.list','runtime.revisions','dashboard.summary',
                    'dashboard.status','quotaEstimates.list','creditEstimates.list',
                    'accounts.status.get','wakeups.list']}
                fixture['hello'] = hello
                fixture['dashboards'] = {}
                providers = {'all', 'qoder', *(s['provider'] for s in settings['sources'])}
                for provider in sorted(providers):
                    for days in [1,7,30,365]:
                        params = dict(days=days, estimateHistory='display')
                        if provider != 'all': params['provider'] = provider
                        fixture['dashboards'][dashboard_key(params)] = core.call('dashboard', params)[0]
                # A selected pet can require a cached built-in installation; it stays in the copy.
                pet = core.call('pets.current')[0]
                asset = pet.get('pet')
                if asset and Path(asset['spritePath']).is_file():
                    image = output/'assets/spritesheet.png'
                    shutil.copyfile(asset['spritePath'], image)
                    asset['spritePath'] = str(image)
                fixture['pets.current'] = pet
                fixture['hosts.sample'] = [dict(id=h['id'], name=h['name'],
                    sample=dict(timestamp=time.time(), uptime=10000, load=[.5,.5,.5], errors={},
                        cpu=[dict(id='cpu',utilization=20)]+[dict(id='cpu'+str(i),utilization=20)
                            for i in range(os.cpu_count() or 1)],
                        memory=dict(total=32*1024**3,available=16*1024**3)), error=None)
                    for h in settings['hosts'] if h.get('enabled', True)]
                if settings.get('localMonitor',{}).get('enabled', True):
                    fixture['hosts.sample'].insert(0, dict(id='local',name='本机',
                        sample=fixture['hosts.sample'][0]['sample'] if fixture['hosts.sample'] else
                            dict(timestamp=time.time(),load=[],errors={},cpu=[]),error=None))
                validate_fixture(fixture)
                write_json(output/'fixture.json', fixture)
        finally:
            core.close()
        write_json(output/'core-report.json', report)
    print(json.dumps(dict(output=str(output), inventory=inventory, coreReport='core-report.json'), ensure_ascii=False))


def bridge():
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    output = Path(os.environ['AIEYES_PROFILE_ROOT'])
    fixture = json.loads((output/'fixture.json').read_text())
    validate_fixture(fixture)
    mode = os.environ.get('AIEYES_PROFILE_MODE','replay')
    core = None
    output_lock = threading.Lock()
    monitor_lock = threading.Lock()
    monitor_stop = threading.Event()
    monitor = dict(version=0, interval=30, paused=True, configured=False)
    log = open(Path(os.environ['AIEYES_PROFILE_RPC'])/(str(os.getpid())+'.jsonl'), 'a', buffering=1)
    # All UI workers share a tiny isolated revision store, independent of the history DB.
    control = sqlite3.connect(Path(os.environ['AIEYES_PROFILE_RPC'])/'control.sqlite', timeout=10)
    control.execute('create table if not exists revisions(domain text primary key, value integer)')
    control.executemany('insert or ignore into revisions values(?,?)', fixture['runtime.revisions'].items())
    control.commit()
    def host_snapshot(advance=False):
        with monitor_lock:
            if advance: monitor['version'] += 1
            rows=json.loads(json.dumps(fixture['hosts.sample']))
            for row in rows:
                row.update(sampleSession='profile',sampleVersion=monitor['version'])
                row['sample']['timestamp']=time.time()
            return dict(rows=rows,inFlight=[],pending=[],sampleSession='profile',
                        version=monitor['version'],paused=monitor['paused'])
    def publish_hosts():
        while not monitor_stop.wait(.1):
            with monitor_lock:
                due=monitor.get('due', float('inf')); paused=monitor['paused']
                if not paused and time.monotonic()>=due:
                    monitor['due']=time.monotonic()+monitor['interval']
                else: continue
            with output_lock:
                print(json.dumps(dict(jsonrpc='2.0',method='hosts.changed',params=host_snapshot(True))),flush=True)
    threading.Thread(target=publish_hosts,daemon=True).start()
    try:
        for line in sys.stdin:
            request = json.loads(line); method=request['method']; params=request.get('params', {})
            started = time.monotonic(); error = None
            try:
                if method == 'runtime.revisions':
                    result = dict(control.execute('select * from revisions'))
                elif method in READS and method != 'pets.current' and mode == 'core':
                    if core is None:
                        core = Core(Path(os.environ['AIEYES_PROFILE_CORE']), Path(os.environ['AIEYES_DATA_DIR']))
                    result = core.call(method, params)[0]
                elif method == 'dashboard':
                    result = fixture['dashboards'][dashboard_key(params)]
                elif method in ['quotaEstimates.list','creditEstimates.list']:
                    result = fixture[method]
                    if params.get('accountKey'): result = [r for r in result if r['accountKey']==params['accountKey']]
                    if params.get('provider'): result = [r for r in result if r['accountKey'].startswith(params['provider']+':')]
                elif method in fixture:
                    result = fixture[method]
                elif method in ['sources.scan', 'sources.rescan', 'quotas.refresh', 'prices.sync']:
                    domain = {'sources.scan':'usage','sources.rescan':'usage','quotas.refresh':'quotas','prices.sync':'prices'}[method]
                    control.execute('update revisions set value=value+1 where domain=?', (domain,)); control.commit()
                    result = (fixture['dashboard.summary']['sources'] if method.startswith('sources.') else
                              fixture['dashboard.summary']['quotas'] if method=='quotas.refresh' else {})
                elif method == 'quotas.schedule': result = {'nextDueAt':None}
                elif method == 'accounts.status.refreshBatch': result = fixture['accounts.status.get']
                elif method in ['hosts.monitor.configure','hosts.monitor.refresh']:
                    with monitor_lock:
                        if method.endswith('configure'):
                            monitor.update(interval=params.get('intervalSeconds',30),paused=params.get('paused',False) or os.environ.get('AIEYES_PROFILE_STATIC')=='1')
                        monitor['due']=time.monotonic()+monitor['interval']
                    result=host_snapshot(True)
                elif method == 'accounts.deployments.sync': result = {'failedTasks':[]}
                elif method == 'accounts.cleanup.list': result = []
                elif method == 'wakeups.status': result = {'installed':False,'enabled':False,'timezone':'Asia/Shanghai','history':[]}
                elif method == 'pets.list': result = {'pets':[fixture['pets.current']['pet']] if fixture['pets.current'].get('pet') else [],'warnings':[]}
                elif method == 'autoEstimates.tick': result = {'changed':False,'enabled':False}
                elif method == 'quotaResetNotifications.take': result = []
                elif method.startswith('operations.'): result = {'preparing':False,'protected':[],'active':0}
                elif method == 'sessions.list':
                    result = {'sessions':[], 'sources':[], 'unavailable':False,
                        'pet':{'behavior':'idle','label':'空闲','row':0,'frames':8,'frameMs':150,'key':'profile'}}
                elif method in ['network.status','network.test']:
                    result = {'mode':'direct','averageMs':0,'sites':[],'status':'ok','testedAt':time.time()}
                else: raise ValueError('Unrecorded RPC blocked: '+method)
                if method == 'hello':
                    result = dict(result, contractHash=os.environ.get('AIEYES_PROFILE_CONTRACT',result.get('contractHash')),
                                  hostMonitoring=True)
                response = dict(jsonrpc='2.0',id=request['id'],result=result)
            except Exception as exc:
                error = str(exc)
                response = dict(jsonrpc='2.0',id=request['id'],error=dict(code=-32000,message=error))
            encoded = json.dumps(response, ensure_ascii=False, separators=(',',':'))
            try: phase=(Path(os.environ['AIEYES_PROFILE_RPC']).parent/'phase.txt').read_text()
            except FileNotFoundError: phase='bootstrap'
            log.write(json.dumps(dict(method=method, phase=phase,at=time.time(), ms=(time.monotonic()-started)*1000,
                bytes=len(encoded.encode())+1, error=error))+'\n')
            with output_lock: print(encoded, flush=True)
    finally:
        monitor_stop.set()
        if core: core.close()
        control.close(); log.close()


def build(args):
    output = args.output.resolve(); cache=ROOT/'.build/swift-module-cache'
    framework = ROOT/'.build/swift/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64'
    if args.ref == 'workspace':
        source = output/'source-workspace/apps/macos/Sources/Aieyes'
        if '--reuse-source' in args.driver_args:
            if not source.is_dir(): raise FileNotFoundError('No captured workspace source to reuse')
        else:
            shutil.copytree(ROOT/'apps/macos/Sources/Aieyes',source,dirs_exist_ok=True)
            shutil.copytree(ROOT/'apps/macos/Resources',output/'source-workspace/apps/macos/Resources',dirs_exist_ok=True)
    else:
        checkout = output/('source-'+args.ref)
        checkout.mkdir(parents=True,exist_ok=True)
        archive = subprocess.check_output(['git','archive',args.ref,'apps/macos/Sources/Aieyes','apps/macos/Resources'],cwd=ROOT)
        import io
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(checkout, filter='data')
        source = checkout/'apps/macos/Sources/Aieyes'
    engine = source/'EngineClient.swift'
    original = engine.read_text()
    # The only source instrumentation: a control that pauses recurring work while
    # retaining production visibility/query selection and all manual handlers.
    if 'func profilePauseBackground()' not in original:
        original = original.replace('private func scheduleRefreshes() {',
            'private func scheduleRefreshes() {\n        if ProcessInfo.processInfo.environment["AIEYES_PROFILE_STATIC"] == "1" { return }')
        engine.write_text(original + '''
extension AppModel {
    func profilePauseBackground() {
        timer?.invalidate(); timer = nil
        automaticTask?.cancel(); runtimeTask?.cancel(); runtimeRefresh?.cancel(); sessionTask?.cancel()
    }
}
''')
    binary = output/('driver-'+args.ref)
    env = dict(os.environ, CLANG_MODULE_CACHE_PATH=str(ROOT/'.build/clang-cache'))
    sources=[p for p in sorted(source.glob('*.swift')) if p.name!='AieyesApp.swift']
    module=output/('module-'+args.ref); module.mkdir(exist_ok=True)
    object_path=module/'production.o'
    fingerprint=hashlib.sha256(b''.join(p.read_bytes() for p in sources)).hexdigest()
    fingerprint_path=module/'fingerprint.txt'
    active_source=output/('AuditActiveWindow-'+args.ref+'.swift')
    active=(source/'MainWindowView.swift').read_text().replace('struct MainWindowView:', 'struct AuditActiveWindowView:')
    active=active.replace('ForEach(monitoring, id: \\.0) { key, _, _ in',
        'ForEach(monitoring.filter { $0.0 == model.detailPage }, id: \\.0) { key, _, _ in')
    active=active.replace('if visitedSettings || model.detailPage == "settings" {', 'if model.detailPage == "settings" {')
    active_source.write_text('@testable import Aieyes\n'+active)
    flags=['-O','-whole-module-optimization','-g','-swift-version','5','-parse-as-library','-module-cache-path',str(cache),
           '-F',str(framework),'-framework','Sparkle']
    with (output/('build-'+args.ref+'.log')).open('w') as log:
        if not object_path.exists() or not fingerprint_path.exists() or fingerprint_path.read_text()!=fingerprint:
            subprocess.run(['swiftc',*flags,'-module-name','Aieyes','-enable-testing','-emit-module','-emit-object',
                '-emit-module-path',str(module/'Aieyes.swiftmodule'),*[str(p) for p in sources],'-o',str(object_path)],
                env=env,stdout=log,stderr=log,check=True)
            fingerprint_path.write_text(fingerprint)
        subprocess.run(['swiftc',*flags,'-D','AIEYES_PERFORMANCE_MODULE','-I',str(module),
            '-Xlinker','-rpath','-Xlinker',str(framework),str(active_source),str(ROOT/'scripts/verify-release-interactions.swift'),
            str(object_path),'-o',str(binary)],env=env,stdout=log,stderr=log,check=True)
    print(binary)


def run(args):
    output = args.output.resolve(); label = args.run_name or args.ref+'-'+args.mode
    run_dir = output/label
    run_dir.mkdir(exist_ok=False)
    logs = run_dir/'rpc'; logs.mkdir()
    database_copy(output/'snapshot/aieyes.sqlite', run_dir/'data/aieyes.sqlite')
    # The executable bridge is this script via a tiny fixed shell wrapper.
    launcher = run_dir/'bridge'
    import shlex
    launcher.write_text('#!/bin/sh\nexec '+shlex.quote(sys.executable)+' '+shlex.quote(str(Path(__file__).resolve()))+' bridge\n')
    launcher.chmod(0o700)
    core = args.core.resolve() if args.ref=='workspace' else args.app/'Contents/Resources/aieyes-core'
    env = dict(os.environ,AIEYES_PROFILE_ROOT=str(output),AIEYES_PROFILE_RPC=str(logs),
        AIEYES_PROFILE_MODE=args.mode,AIEYES_PROFILE_CORE=str(core), AIEYES_DATA_DIR=str(run_dir/'data'),
        AIEYES_CORE_PATH=str(launcher),AIEYES_PROFILE_REPETITIONS=str(args.repetitions),
        AIEYES_PROFILE_STATIC='1' if '--static-input' in args.driver_args else '0')
    bundle='app.aieyes.interactionaudit.'+str(time.time_ns())
    app=run_dir/'Interaction Audit.app'; executable=app/'Contents/MacOS/driver'
    executable.parent.mkdir(parents=True); shutil.copy2(output/('driver-'+args.ref),executable)
    (app/'Contents/Info.plist').write_bytes(plistlib.dumps(dict(CFBundleIdentifier=bundle,
        CFBundleExecutable='driver',CFBundleName='Aieyes Interaction Audit',LSUIElement=True)))
    subprocess.run(['defaults','import',bundle,str(output/'preferences.plist')],check=True,capture_output=True)
    write_json(run_dir/'run-config.json',dict(ref=args.ref,mode=args.mode,driverArgs=args.driver_args,
        preferenceStorage='disposable writable bundle domain',
        fixtureHash=hashlib.sha256((output/'fixture.json').read_bytes()).hexdigest(),
        coreHash=hashlib.sha256(core.read_bytes()).hexdigest(),
        driverHash=hashlib.sha256(executable.read_bytes()).hexdigest()))
    process=None
    sampling_stop=threading.Event()
    sampling_thread=None
    try:
        with (run_dir/'driver.log').open('w') as log:
            process=subprocess.Popen([str(executable),str(run_dir),str(output/'fixture.json'),
                str(output/'preferences.plist'),*args.driver_args],env=env,stdout=log,stderr=log,start_new_session=True)
            def sample_phases():
                sampled=set(); manifest=[]
                while not sampling_stop.wait(.05):
                    try: phase=(run_dir/'phase.txt').read_text()
                    except FileNotFoundError: continue
                    group=next((name for name in ['navigate','refresh','details','analysis.scroll','panel.scroll'] if phase.startswith(name)),None)
                    if not group or group in sampled: continue
                    sampled.add(group)
                    started=time.time(); target=run_dir/('sample-'+group+'.txt')
                    result=subprocess.run(['sample',str(process.pid),'5','10','-file',str(target)],
                        capture_output=True,timeout=20)
                    manifest.append(dict(group=group,trigger=phase,startedAt=started,
                        endedAt=time.time(),returncode=result.returncode))
                    write_json(run_dir/'sample-manifest.json',manifest)
            sampling_thread=threading.Thread(target=sample_phases,daemon=True); sampling_thread.start()
            if process.wait(timeout=7200): raise RuntimeError('Driver failed; see '+str(run_dir/'driver.log'))
    finally:
        sampling_stop.set()
        if sampling_thread: sampling_thread.join(timeout=20)
        if process:
            try: os.killpg(process.pid,signal.SIGTERM)
            except ProcessLookupError: pass
        subprocess.run(['defaults','delete',bundle],capture_output=True)
        # Reports and the immutable input snapshot reproduce a run. Avoid retaining
        # another full database and app bundle for every isolated scenario.
        shutil.rmtree(run_dir/'data',ignore_errors=True)
        shutil.rmtree(app,ignore_errors=True)
    print(json.dumps(dict(run=str(run_dir), complete=True)))


def probe(args):
    """Check the installed app's actual executable, apart from the instrumented driver."""
    import importlib.util
    spec=importlib.util.spec_from_file_location('soak_metrics',ROOT/'scripts/soak-runtime-ipc.py')
    metrics=importlib.util.module_from_spec(spec);spec.loader.exec_module(metrics)
    output=args.output.resolve();run_dir=output/(args.run_name or 'installed-executable')
    run_dir.mkdir(exist_ok=False);logs=run_dir/'rpc';logs.mkdir()
    database_copy(output/'snapshot/aieyes.sqlite',run_dir/'data/aieyes.sqlite')
    app=run_dir/'Installed Release Audit.app';shutil.copytree(args.app,app,symlinks=True)
    bundle='app.aieyes.interactionaudit.installed.'+str(time.time_ns())
    info_path=app/'Contents/Info.plist';info=plistlib.loads(info_path.read_bytes())
    info.update(CFBundleIdentifier=bundle,AieyesDevelopmentDataDirectory=str(run_dir/'data'),
                SUEnableAutomaticChecks=False,SUAllowsAutomaticUpdates=False)
    info.pop('SUPublicEDKey',None);info_path.write_bytes(plistlib.dumps(info))
    subprocess.run(['codesign','--force','--sign','-',str(app)],check=True,capture_output=True)
    prefs=plistlib.loads((output/'preferences.plist').read_bytes())
    prefs.update({'updates.automatic':False,'setup.seen.v1':True})
    prefs_path=run_dir/'prefs.plist';prefs_path.write_bytes(plistlib.dumps(prefs))
    subprocess.run(['defaults','import',bundle,str(prefs_path)],check=True,capture_output=True)
    import shlex
    bridge_path=run_dir/'bridge'
    bridge_path.write_text('#!/bin/sh\nexec '+shlex.quote(sys.executable)+' '+shlex.quote(str(Path(__file__).resolve()))+' bridge\n')
    bridge_path.chmod(0o700)
    env=dict(os.environ,AIEYES_PROFILE_ROOT=str(output),AIEYES_PROFILE_RPC=str(logs),
        AIEYES_PROFILE_MODE='replay',AIEYES_DATA_DIR=str(run_dir/'data'),AIEYES_CORE_PATH=str(bridge_path))
    samples=[];owned={};process=None
    try:
        with (run_dir/'app.log').open('w') as log:
            process=subprocess.Popen([str(app/'Contents/MacOS/Aieyes'),'--show-detail'],env=env,stdout=log,stderr=log)
            start=time.monotonic();previous=None
            while time.monotonic()-start<45:
                if process.poll() is not None:raise RuntimeError('Installed app exited early')
                usage=metrics.Usage()
                if metrics.LIBPROC.proc_pid_rusage(process.pid,2,ctypes.byref(usage))!=0:
                    raise RuntimeError('Could not measure installed app')
                now=time.monotonic();cpu=(usage.user_time+usage.system_time)/1e9
                row=dict(elapsed=now-start,cpuSeconds=cpu,rssBytes=usage.resident_size,
                         physicalBytes=usage.phys_footprint,afterWarmup=now-start>=15)
                if previous:row['cpuPercent']=(cpu-previous[1])/(now-previous[0])*100
                previous=(now,cpu);samples.append(row)
                write_json(run_dir/'report.json',dict(pid=process.pid,complete=False,samples=samples,
                    note='Copied installed app, original production entrypoint, re-signed for a disposable bundle identity. Cached RPC replay; no external services. Window requested visible; occlusion not measured.'))
                time.sleep(1)
            subprocess.run(['sample',str(process.pid),'5','10','-file',str(run_dir/'sample.txt')],capture_output=True,timeout=20)
    finally:
        if process:
            owned=metrics.processes(process.pid)
            process.terminate()
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.kill();process.wait()
            for pid in owned:
                if pid!=process.pid:
                    try:os.kill(pid,signal.SIGTERM)
                    except ProcessLookupError:pass
        subprocess.run(['defaults','delete',bundle],capture_output=True)
    write_json(run_dir/'report.json',dict(complete=True,samples=samples,
        note='Original installed release entrypoint on an isolated snapshot and RPC replay; requested visible settings workspace. UI binary re-signed only; no source edits.'))
    print(run_dir)


def summarize(args):
    output=args.output.resolve();result={}
    fixture_hash=hashlib.sha256((output/'fixture.json').read_bytes()).hexdigest()
    for path in sorted(output.glob('*/report.json')):
        report=json.loads(path.read_text());config=path.parent/'run-config.json'
        if not config.exists() or report.get('appearanceTheme')!='dark':continue
        if json.loads(config.read_text()).get('fixtureHash')!=fixture_hash:continue
        groups={}
        for row in report.get('samples',[]):groups.setdefault(row['scenario'],[]).append(row)
        scenarios={}
        for name,rows in groups.items():
            valid=[row for row in rows if not row.get('unsupportedNativeAction')]
            item=dict(attempted=len(rows),valid=len(valid))
            for metric in ['feedbackMs','completionMs','cpuMs','cpuPercent','maxEventLoopLagMs',
                           'scrollFrameP95Ms','scrollInputResponseP95Ms','publications','livePublications',
                           'hoverDispatchMaxMs','refreshQueueMs','refreshCompletionMs','rssBytes','scrollPositionRange']:
                values=[row[metric] for row in valid if metric in row]
                if values:item[metric]=dict(median=statistics.median(values),p95=percentile(values),max=max(values))
            for metric in ['scrollFramesOver33ms','scrollFrames','hoverEvents','clipViewFallbackSteps','panelTintUpdatesDuringScroll']:
                values=[row[metric] for row in valid if metric in row]
                if values:item[metric]=sum(values)
            item['occludedSamples']=sum(row.get('workspaceOccluded',False) for row in valid if row.get('workspaceVisible'))
            if any('scrollViewRetained' in row for row in valid):item['retainedSamples']=sum(row.get('scrollViewRetained',False) for row in valid)
            scenarios[name]=item
        rpc={};errors=[]
        for log in (path.parent/'rpc').glob('*.jsonl'):
            for line in log.read_text().splitlines():
                row=json.loads(line);rpc.setdefault(row['method'],[]).append(row)
                if row['error']:errors.append(dict(method=row['method'],error=row['error']))
        result[path.parent.name]=dict(config=json.loads(config.read_text()),runComplete=report.get('complete',False),scenarios=scenarios,
            suppressedScrollWrites=report.get('suppressedScrollWrites',0),
            theme=report.get('appearanceTheme'),
            rpc={method:dict(count=len(rows),medianMs=statistics.median(r['ms'] for r in rows),
                            maxMs=max(r['ms'] for r in rows),bytes=sum(r['bytes'] for r in rows)) for method,rows in rpc.items()},
            rpcErrors=errors)
    write_json(output/'summary.json',result)
    print(json.dumps(dict(runs=len(result),completeRuns=sum(r['runComplete'] for r in result.values()),
        validSamples=sum(s['valid'] for r in result.values() for s in r['scenarios'].values()),
        rpcErrors=sum(len(r['rpcErrors']) for r in result.values()))))


def scan_local(args):
    """Copy registered history files only; run import against remapped disposable roots."""
    output=args.output.resolve();target=output/'local-history-scan';target.mkdir(exist_ok=False)
    fixture=json.loads((output/'fixture.json').read_text());settings=fixture['settings.get']
    data=target/'data';database_copy(output/'snapshot/aieyes.sqlite',data/'aieyes.sqlite')
    stats=[];copies={}
    with sqlite3.connect(data/'aieyes.sqlite') as db:
        for index,source in enumerate(settings['sources']):
            original=Path(source['path']).expanduser().resolve()
            folder=target/'sources'/str(index);folder.mkdir(parents=True)
            local=not source.get('hostId');copied=0;size=0;missing=0;outside=0
            source_was_file=original.is_file()
            if local:
                for name, in db.execute('select path from files where source_id=?',(source['id'],)).fetchall():
                    path=Path(name).expanduser().resolve()
                    if not path.is_file():missing+=1;continue
                    try:relative=Path(path.name) if source_was_file and path==original else path.relative_to(original)
                    except ValueError:outside+=1;continue
                    if path.suffix not in ['.jsonl','.json','.db','.sqlite','.vscdb','.zstd']:
                        outside+=1;continue
                    destination=folder/relative;destination.parent.mkdir(parents=True,exist_ok=True)
                    if path in copies:
                        # Hard links are confined to files already copied inside this audit.
                        os.link(copies[path],destination)
                    elif path.suffix in ['.db','.sqlite','.vscdb']:
                        database_copy(path,destination);copies[path]=destination
                    else:
                        shutil.copy2(path,destination);copies[path]=destination
                    db.execute('update files set path=? where source_id=? and path=?',
                               (str(destination),source['id'],name))
                    copied+=1;size+=destination.stat().st_size
            source['path']=str(folder/original.name if source_was_file else folder)
            source['enabled']=source.get('enabled',True) and local and copied>0
            source['codexHomeId']=None
            for key in ['quotaCommand','quotaPreCommand','codexBinary','agyBinary']:source[key]=''
            if isinstance(source.get('autoValuation'),dict):source['autoValuation']['enabled']=False
            stats.append(dict(provider=source['provider'],local=local,copiedFiles=copied,bytes=size,
                              missingRegisteredFiles=missing,outsideCurrentRoot=outside,enabled=source['enabled']))
        for account in settings['accounts']:account['quotaEnabled']=False
        settings['codexApis']=[]
        settings['hosts']=[]
        settings['proxy']={'mode':'custom','url':'http://127.0.0.1:9'}
        db.execute("update kv set value=? where key='settings'",(json.dumps(settings),));db.commit()
    report=dict(sources=stats,uniqueFiles=len(copies),
        note='Registered local history only, captured after the database snapshot. Missing/remote files are not reconstructed; no auth directories or live SSH. Paths and cursors remapped into this private copy; database backups may change native DB fingerprints.')
    for label,binary in [('installed',args.app/'Contents/Resources/aieyes-core'),('workspace',args.core.resolve())]:
        lane=target/label;database_copy(data/'aieyes.sqlite',lane/'aieyes.sqlite')
        core=Core(binary,lane);core.allowed=core.allowed|{'sources.scan'}
        try:
            core.call('hello');runs=[];issues=0;partial=0;failed_files=0;invalid_records=0;issue_codes={}
            for _ in range(args.repetitions):
                value,ms,size=core.call('sources.scan')
                runs.append(ms);issues+=sum(bool(row.get('error')) for row in value)
                partial+=sum(bool(row.get('partial')) for row in value)
                failed_files+=sum(row.get('result',row).get('failedFiles',0) or 0 for row in value)
                invalid_records+=sum(row.get('result',row).get('invalidRecords',0) or 0 for row in value)
                for row in value:
                    for issue in row.get('issues',[]) or []:
                        code=issue.get('code','unknown');issue_codes[code]=issue_codes.get(code,0)+1
            report[label]=dict(firstMs=runs[0],hotMedianMs=statistics.median(runs[1:]),hotP95Ms=percentile(runs[1:]),
                               samplesMs=runs,sourceErrors=issues,partialSourceResults=partial,
                               failedFiles=failed_files,invalidRecords=invalid_records,issueCodes=issue_codes,replyBytes=size)
        finally:core.close()
        write_json(target/'report.json',report)
        shutil.rmtree(lane)
    shutil.rmtree(data)
    print(json.dumps({k:v for k,v in report.items() if k in ['uniqueFiles','installed','workspace']}))


def matrix(args):
    """Resume per-scenario processes; preserve cumulative-stress failures as evidence."""
    output=args.output.resolve();script=Path(__file__).resolve()
    baseline='v'+json.loads((output/'fixture.json').read_text())['hello']['version']
    status_path=output/'isolated-matrix-status.json'
    fixture_hash=hashlib.sha256((output/'fixture.json').read_bytes()).hexdigest()
    status=json.loads(status_path.read_text()) if status_path.exists() else []
    for row in status:
        if 'returncode' in row:continue
        directory=output/row['name'];report=directory/'report.json'
        if not report.exists() or not json.loads(report.read_text()).get('complete'):continue
        # A cancelled tool session may lose its Python supervisor while the
        # detached UI driver finishes. Reconcile only its completed evidence.
        for app in directory.glob('*.app'):
            info=plistlib.loads((app/'Contents/Info.plist').read_bytes())
            bundle=info.get('CFBundleIdentifier','')
            if not bundle.startswith('app.aieyes.interactionaudit.'):
                raise ValueError('Unexpected audit bundle identity')
            subprocess.run(['defaults','delete',bundle],capture_output=True)
            shutil.rmtree(app)
        shutil.rmtree(directory/'data',ignore_errors=True)
        row.update(complete=True,returncode=0,endedAt=time.time(),supervisorRecovered=True)
    write_json(status_path,status)
    scenarios=['idle.hidden','idle.settings','navigate.overview','navigate.analysis','navigate.servers',
        *['navigate.settings.'+s for s in ['accounts','machines','prices','wakeups','appearance','desktop','network','refresh','notifications','updates']],
        'panel.open',*['filter.analysis.'+s for s in ['1d','7d','30d','365d','all-agents']],
        *['refresh.'+s for s in ['history','quotas','prices','servers']],
        'reload.unchanged-analysis','reload.unchanged-with-estimate','details.estimate.open',
        'click.estimate.calculation','details.estimate.scroll','analysis.scroll','prices.scroll','prices.catalog-scroll',
        'servers.scroll','panel.scroll','panel.scroll-with-feedback','window.reopen',
        'burst.refresh-and-navigate','burst.repeat-refresh']
    def covered(ref,name):
        for path in output.glob('*/report.json'):
            config=path.parent/'run-config.json'
            if not config.exists():continue
            metadata=json.loads(config.read_text());report=json.loads(path.read_text())
            if metadata.get('fixtureHash')!=fixture_hash:continue
            if not report.get('complete'):continue
            if metadata['ref']!=ref or metadata['mode']!='replay' or report.get('appearanceTheme')!='dark':continue
            if any(flag in metadata['driverArgs'] for flag in ['--no-scroll-persistence','--only-active-page','--static-input','--plain-surfaces']):continue
            rows=[r for r in report.get('samples',[]) if r['scenario']==name]
            if len(rows)>=(1 if name.startswith('idle.') else args.repetitions):return True
        return False
    def execute(label,ref,mode,extra):
        prior=next((r for r in status if r['name']==label and 'returncode'in r),None)
        if prior:return
        if (output/label).exists():raise FileExistsError('Inspect existing unfinished run: '+label)
        row=dict(name=label,startedAt=time.time(),complete=False);status.append(row);write_json(status_path,status)
        print('Starting '+label,flush=True)
        result=subprocess.run([sys.executable,str(script),'run','--output',str(output),'--run-name',label,
            '--ref',ref,'--mode',mode,'--repetitions',str(args.repetitions),*extra])
        row.update(complete=result.returncode==0,returncode=result.returncode,endedAt=time.time());write_json(status_path,status)
        print('Finished '+label+' '+str(result.returncode),flush=True)
        summarize(args)
    execute('isolated-control-no-scroll','workspace','replay',['--only','scroll','--no-scroll-persistence'])
    execute('isolated-control-active','workspace','replay',['--only','=navigate.analysis','--only-active-page'])
    # Put the user's scrolling/response concern first, then fill the other cases.
    ordered=sorted(scenarios,key=lambda s:(0 if 'scroll' in s else 1,scenarios.index(s)))
    for ref in ['workspace',baseline]:
        for name in ordered:
            if not covered(ref,name):execute('isolated-'+ref+'-'+name,ref,'replay',['--only','='+name])
    execute('isolated-workspace-real-core','workspace','core',['--only','refresh'])
    if not (output/'local-history-scan/report.json').exists():scan_local(args)
    summarize(args)


def main():
    os.umask(0o077)
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action',choices=['prepare','bridge','build','run','probe','summarize','scan-local','matrix'])
    parser.add_argument('--output',type=Path)
    parser.add_argument('--app',type=Path,default=Path('/Applications/Aieyes.app'))
    parser.add_argument('--core',type=Path,default=ROOT/'target/release/aieyes-core')
    parser.add_argument('--ref',default='workspace')
    parser.add_argument('--mode',choices=['replay','core'],default='replay')
    parser.add_argument('--run-name')
    parser.add_argument('--repetitions',type=int,default=20)
    args, args.driver_args=parser.parse_known_args()
    if args.action!='bridge' and args.output is None: parser.error('--output is required')
    if args.repetitions<2: parser.error('--repetitions must be at least 2')
    {'prepare':prepare,'bridge':lambda _:bridge(),'build':build,'run':run,'probe':probe,'summarize':summarize,'scan-local':scan_local,'matrix':matrix}[args.action](args)


if __name__=='__main__': main()
