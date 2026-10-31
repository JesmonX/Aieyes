"""Native session metadata -> SSH process association; no model/account calls."""
import contextlib
import importlib.util
import json
import pathlib
import tempfile
import time
import unittest
from unittest.mock import patch
ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('sessions', ROOT/'scripts/session_remote.py')
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

class ProcessTests(unittest.TestCase):
    def tearDown(self):
        for discovery in m._DISCOVERY.values(): discovery.watch.close()
        m._DISCOVERY.clear(); m._PROCESS_CONTEXTS.clear(); m._NATIVE_PROCESS_CACHE.clear()

    def test_linux_proc_sampling_keeps_normalized_title_and_exit_evidence_private(self):
        import os, shutil
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory).resolve(); proc=root/'proc'; pid=proc/'4321'; pid.mkdir(parents=True)
            data=root/'data';data.mkdir();workspace=root/'project';workspace.mkdir()
            fields=['0']*20;fields[0]='S';fields[1]='1';fields[19]='10000'
            (pid/'stat').write_text('4321 (pi) '+' '.join(fields));(proc/'stat').write_text('btime 0\n')
            (pid/'cmdline').write_bytes(b'pi\0-p\0PRIVATE_PROMPT\0')
            (pid/'environ').write_bytes(('PI_CODING_AGENT_DIR='+str(data)+'\0AIEYES_INTERNAL=1\0PRIVATE_TOKEN=PRIVATE_SECRET\0').encode())
            real_path=type(root)
            def translated(value):
                value=str(value)
                return str(proc)+value[len('/proc'):] if value=='/proc' or value.startswith('/proc/') else value
            with patch.object(m.pathlib,'Path',side_effect=lambda value:real_path(translated(value))),patch.object(m.os,'readlink',return_value=str(workspace)),patch.object(m.os,'sysconf',return_value=100,create=True),patch.object(m.time,'monotonic',return_value=10) as clock:
                first=m.native_processes({'pi'})
                self.assertEqual((first[0]['classification'],first[0]['internal'],first[0]['root']),('headless',True,str(data)))
                (pid/'cmdline').write_bytes(b'pi\0PRIVATE_TOKEN=PRIVATE_SECRET\0');(pid/'environ').write_bytes(b'')
                clock.return_value=11
                rewritten=m.native_processes({'pi'})
                self.assertEqual((rewritten[0]['classification'],rewritten[0]['root'],rewritten[0]['internal']),('headless',str(data),True))
                self.assertNotIn('PRIVATE_',json.dumps({key:value for key,value in rewritten[0].items() if key!='files'}))
                shutil.rmtree(pid)
                retired=m.native_processes({'pi'})
                self.assertEqual((retired[0]['classification'],retired[0]['alive']),('headless',False))
                clock.return_value=27
                self.assertEqual(m.native_processes({'pi'}),[])

    def test_process_title_rewrite_retains_only_normalized_launch_evidence_until_exit(self):
        launched = dict(provider='pi',pid=10,start=100,classification='headless',selectedSession='session-a',
                        root='/data/pi',rootObserved=True,workspace='/project',internal=True)
        rewritten = dict(launched,classification=None,selectedSession=None,root='/default',rootObserved=False,internal=False)
        m.retain_launch_evidence(rewritten,launched)
        self.assertEqual(rewritten,launched)
        resumed = dict(rewritten,start=101,classification='interactive',internal=False)
        m.retain_launch_evidence(resumed,launched)
        self.assertEqual((resumed['classification'],resumed['internal']),('interactive',False))
        self.assertNotIn('argv',rewritten)

    def test_harness_lock_belongs_only_to_its_exact_native_session(self):
        path='/data/dsh/session-a/session.v4.jsonl.zstd'
        context=dict(path=path,session='a',workspace=None,created=0)
        process=dict(provider='deepseek-harness',pid=10,start=100,classification='headless',selectedSession=None,
                     root='/data/dsh',workspace='/workspace',files={'/data/dsh/session-a/session.lock'},internal=False,alive=False)
        a=(path,context,{})
        other='/data/dsh/session-b/session.v4.jsonl.zstd';b=(other,dict(context,path=other,session='b'),{})
        result=m.associate_native_processes('deepseek-harness',[a,b],[process])
        self.assertEqual(set(result),{path})
        self.assertEqual(result[path]['classification'],'headless')

    def test_first_hook_directory_is_discovered_without_waiting_for_reconciliation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory).resolve(); (root/'projects').mkdir()
            source = dict(provider='codebuddy',id='cb',path=str(root))
            self.assertEqual(m.collect(source,{},processes=[])['files'],[])
            discovery = next(iter(m._DISCOVERY.values()))
            if not discovery.watch.active: self.skipTest('native watcher unavailable')
            hook = root/'.aieyes/session-events/first.jsonl'; hook.parent.mkdir(parents=True)
            hook.write_text(json.dumps(dict(kind='UserPromptSubmit',session='same-session',classification='headless',at=int(time.time()*1000)))+'\n')
            deadline = time.monotonic()+2
            while True:
                rows = m.collect(source,{},processes=[])['files']
                if rows: break
                self.assertLess(time.monotonic(),deadline,'first hook was not discovered by its parent watcher')
                time.sleep(.02)
            self.assertEqual([(row['hook'],row['events'][0]['classification']) for row in rows],[(True,'headless')])

    def test_native_pipeline_keeps_interactive_and_unknown_projects_separate(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory).resolve(); logs = root/'data/sessions'; logs.mkdir(parents=True)
            stamp = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
            started = int(time.time())
            for identity in ('headless','interactive','internal','unknown'):
                workspace = root/identity; workspace.mkdir()
                (logs/(identity+'.jsonl')).write_text(json.dumps(dict(type='session',id=identity,cwd=str(workspace),timestamp=stamp))+'\n'+json.dumps(dict(type='message',id='turn-'+identity,timestamp=stamp,message=dict(role='user',content='PRIVATE_TRANSCRIPT')))+'\n')
            def process(identity, argv, internal=False):
                return dict(pid=len(identity),start=started,provider='pi',argv=argv,workspace=str(root/identity),root=str(root/'data'),internal=internal,files=set())
            processes = [process('headless',['pi','-p','PRIVATE_ARGV']),process('interactive',['pi']),process('internal',['pi'],True)]
            source = dict(provider='pi',id='pi',path=str(root/'data'))
            result = m.collect(source,{},processes=processes)
            verdicts = {pathlib.Path(row['key']).stem:next((e['classification'] for e in reversed(row['events']) if e.get('classification')),None) for row in result['files']}
            self.assertEqual(verdicts,dict(headless='headless',interactive='interactive',internal='internal',unknown=None))
            self.assertNotIn('PRIVATE_',json.dumps(result))
            # A second data directory with identical workspace names is not the
            # source belonging to these processes.
            for p in processes: p['root'] = str(root/'other-data')
            other = m.collect(source,{},processes=processes)
            self.assertFalse(any(e.get('classification') for row in other['files'] for e in row['events']))

    def test_ambiguous_workspace_is_unknown_but_exact_session_and_pid_are_safe(self):
        activity = dict(phase='working',turnStartedAt=101000)
        def session(identity):
            return ('/data/pi/'+identity+'.jsonl',dict(path='/data/pi/'+identity+'.jsonl',session=identity,workspace='/workspace',created=101),dict(activity,session=identity))
        process = dict(pid=10,start=100,provider='pi',argv=['pi','-p','fixture'],workspace='/workspace',root='/data/pi',internal=False,files=set())
        sessions = [session('a'),session('b')]
        self.assertEqual(m.associate_native_processes('pi',sessions,[process]),{})
        process['argv'] = ['pi','--session-id','a','-p','fixture']
        self.assertEqual(list(m.associate_native_processes('pi',sessions,[process])),[sessions[0][0]])
        key, context, state = session('old')
        context.update(workspace='/elsewhere',created=1);state.update(pid=10,processStart=99,turnStartedAt=1000)
        self.assertEqual(m.associate_native_processes('pi',[(key,context,state)],[process]),{})
        state['processStart']=100;process['internal']=True
        self.assertEqual(m.associate_native_processes('pi',[(key,context,state)],[process])[key]['classification'],'internal')

    def test_kimi_state_and_opencode_v2_provide_exact_workspace_metadata(self):
        import sqlite3
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory).resolve();directory=str(root);session=root/'sessions/wd/session_abc';path=session/'agents/main/wire.jsonl';path.parent.mkdir(parents=True)
            path.write_text('{}\n');(session/'state.json').write_text(json.dumps(dict(id='session_abc',cwd=directory,createdAt=101000000000)))
            context=m._session_context('kimi',str(path),'session_abc:main')
            self.assertEqual((context['session'],context['workspace'],context['created']),('session_abc',directory,101000000))
            database=root/'opencode.db'
            with contextlib.closing(sqlite3.connect(database)) as db, db:
                db.execute('CREATE TABLE session_v2(id TEXT,directory TEXT,time_created INTEGER)')
                db.execute('INSERT INTO session_v2 VALUES(?,?,?)',('abc',directory,101000000000))
            context=m._session_context('opencode',str(database)+'#abc','abc')
            self.assertEqual((context['session'],context['workspace'],context['created']),('abc',directory,101000000))

if __name__ == '__main__': unittest.main()
