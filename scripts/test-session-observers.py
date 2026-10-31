import contextlib
import importlib.util,json,os,pathlib,sqlite3,subprocess,sys,tempfile,unittest
from unittest.mock import patch
ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('sessions',ROOT/'scripts/session_remote.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);m.SCRIPT=(ROOT/'scripts/session_remote.py').read_text()
class ObserverTests(unittest.TestCase):
    def test_kiro_cli_and_ide_metadata_keep_parent_and_headless_classification(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            def write(path, value):
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(json.dumps(value))
            cli = root / 'sessions/cli/work.json'
            write(cli, dict(session_id='work', cwd='/work', private='PRIVATE', session_state=dict(conversation_metadata=dict(user_turn_metadatas=[dict(end_reason='UserTurnEnd')]))))
            write(cli.with_suffix('.lock'), dict(pid=123))
            write(root / 'sessions/space/sess_child/session.json', dict(id='child', status='completed', createdReason='subagent', parentSessionId='work', title='PRIVATE'))
            write(root / 'extensions/ignored/session.json', dict(id='ignored'))
            with patch.object(m, 'kiro_process_classification', return_value='headless'):
                rows, overflow = m.collect_kiro(root, {})
            self.assertEqual(overflow, 0)
            self.assertEqual(len(rows), 2)
            work = next(row for row in rows if row['identity'] == 'work')
            child = next(row for row in rows if row['identity'] == 'child')
            self.assertEqual(work['events'][-1]['kind'], 'complete')
            self.assertEqual(work['events'][0]['classification'], 'headless')
            self.assertEqual(child['events'][0]['parentSession'], 'work')
            self.assertNotIn('PRIVATE', json.dumps(rows))
            cli.with_suffix('.lock').unlink()
            rows, _ = m.collect_kiro(root, {work['key']: work})
            self.assertEqual(next(r for r in rows if r['identity'] == 'work')['events'][0]['classification'], 'headless')
            self.assertEqual(m.collect_kiro(root / 'empty', {}), ([], 0))

    def test_trae_remote_sessions_match_projects_and_keep_task_verdicts(self):
        helper = m.trae_helper()
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory); trajectories = root / 'trajectories'; trajectories.mkdir()
            path = trajectories / 'result.json'
            value = dict(task='PRIVATE_PROMPT', start_time='2026-10-31T01:00:00.000001', end_time='', llm_interactions=[], success=False)
            path.write_text(json.dumps(value))
            source = dict(id='trae', provider='trae', path=directory)
            wrong = (1, ['trae-cli', 'run', '-t', '/another-project/trajectories/result.json'], root, False)
            with patch.dict(helper, processes=lambda: [wrong]):
                first = m.collect(source, {})
            row = first['files'][0]
            self.assertEqual(row['events'][-1]['classification'], 'unknown')
            self.assertEqual(row['events'][-1]['kind'], 'start')
            self.assertNotIn('PRIVATE', json.dumps(first))
            process = (1, ['trae-cli', 'run', '-t', './trajectories/result.json'], root, False)
            with patch.dict(helper, processes=lambda: [process]):
                headless = m.collect(source, {})['files'][0]
            self.assertEqual(headless['events'][0]['classification'], 'headless')
            with patch.dict(helper, processes=lambda: []):
                repeat = m.collect(source, {str(path): headless})['files'][0]
                self.assertEqual(repeat['continuation']['classification'], 'headless')
                value['start_time'] = '2026-10-31T02:00:00.000001'
                path.write_text(json.dumps(value))
                next_task = m.collect(source, {str(path): repeat})['files'][0]
                self.assertEqual(next_task['events'][0]['classification'], 'unknown')
                path.write_text('{}')
                invalid = m.collect(source, {})
                self.assertEqual(invalid['files'], [])
                self.assertIn('metadata', invalid['error'])
                fallback = m.collect(source, {str(path): next_task})
                self.assertEqual(fallback['files'][0]['events'], [])
                self.assertFalse(fallback['files'][0]['reset'])

    def test_trae_ide_process_status_has_no_synthetic_conversations(self):
        helper = m.trae_helper()
        for provider, executable in [('trae', '/Applications/Trae.app/Contents/MacOS/Electron'), ('trae-cn', '/Applications/Trae CN.app/Contents/MacOS/Electron'), ('trae', r'C:\Apps\Trae\Trae.exe'), ('trae-cn', '/opt/trae-cn/trae-cn')]:
            self.assertTrue(helper['ide_process'](provider, executable, []))
            self.assertFalse(helper['ide_process'](provider, executable, ['--type=renderer']))
            self.assertFalse(helper['ide_process']('trae-cn' if provider == 'trae' else 'trae', executable, []))
        with tempfile.TemporaryDirectory() as directory:
            (pathlib.Path(directory) / 'chat.db').write_bytes(b'PRIVATE_ENCRYPTED')
            for running in (False, True):
                with patch.dict(helper, ide_running=lambda _: running):
                    result = m.collect(dict(id='trae', provider='trae-cn', path=directory), {})
                self.assertEqual(result['files'], [])
                self.assertEqual(result['processRunning'], running)
                self.assertIsNone(result['error'])

    def test_trae_default_writer_requires_the_initial_project_and_internal_runs_exclude(self):
        import datetime
        helper = m.trae_helper()
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            path = root / 'trajectories/trajectory_20261031_012437.json'
            at = int(datetime.datetime(2026, 10, 31, 1, 24, 37).timestamp())
            process = (at, ['python3', '-m', 'trae_agent.cli', 'interactive'], root, False)
            self.assertEqual(helper['writer'](path, [process], m.headless_argv), 'interactive')
            wrong = (*process[:2], root / 'other-project', False)
            self.assertIsNone(helper['writer'](path, [wrong], m.headless_argv))
            self.assertIsNone(helper['writer'](path, [process, process], m.headless_argv))
            self.assertEqual(helper['writer'](path, [(*process[:3], True)], m.headless_argv), 'internal')

    def test_claude_sources_survive_log_tail_incremental_cache_and_hook_environment(self):
        for entrypoint in ('cli','sdk-cli','sdk-ts','sdk-py'):
            with self.subTest(entrypoint=entrypoint), tempfile.TemporaryDirectory() as directory:
                root=pathlib.Path(directory);path=root/'session.jsonl'
                header=dict(type='user',sessionId='session',entrypoint=entrypoint,message=dict(content='PRIVATE'))
                path.write_text(json.dumps(header)+'\n'+json.dumps(dict(type='ignored',text='PRIVATE_'*50000))+'\n'+json.dumps(dict(type='assistant',sessionId='session',message=dict(stop_reason='end_turn')))+'\n')
                first=m.read_file(path,'claude',False,{})
                self.assertEqual(first['events'][0]['sessionSource'],entrypoint)
                self.assertEqual(first['events'][-1]['kind'],'complete')
                self.assertNotIn('PRIVATE',json.dumps(first))
                with path.open('a') as f:f.write(json.dumps(dict(type='user',sessionId='session',message=dict(content='PRIVATE')))+'\n')
                second=m.read_file(path,'claude',False,first)
                self.assertFalse(second['reset']);self.assertEqual(second['events'][0]['kind'],'start')
                self.assertEqual(m.read_file(path,'claude',False,second)['events'],[])
                if os.name=='nt':continue  # SSH hook helper uses POSIX file locks.
                (root/'.aieyes/session-events').mkdir(parents=True)
                helper=root/'observe.py';helper.write_text(m.SCRIPT)
                env=dict(os.environ,CLAUDE_CODE_ENTRYPOINT=entrypoint)
                subprocess.run([sys.executable,str(helper),m.MARKER,'claude','UserPromptSubmit',directory],input=json.dumps(dict(session_id='hook',entrypoint='cli',prompt='PRIVATE')),env=env,capture_output=True,text=True,check=True,timeout=3)
                event=json.loads(next((root/'.aieyes/session-events').glob('*.jsonl')).read_text())
                self.assertEqual(event['sessionSource'],entrypoint)
                self.assertNotIn('PRIVATE',json.dumps(event))

    def test_codebuddy_nested_child_identity_and_intermediate_commentary(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);path=root/'parent/subagents/agent-worker.jsonl';path.parent.mkdir(parents=True)
            row=dict(type='message',role='assistant',status='completed',sessionId='independent-native-child',id='commentary',timestamp=1234,providerData=dict(messageId='step'),content='PRIVATE')
            path.write_text(json.dumps(row)+'\n')
            result=m.read_file(path,'codebuddy',False,{})
            self.assertEqual(result['events'][0]['parentSessionPath'],str(root/'parent.jsonl'))
            self.assertEqual(result['events'][-1]['kind'],'thinking')
            self.assertEqual(result['events'][-1]['session'],'independent-native-child')
            self.assertNotIn('PRIVATE',json.dumps(result))
            row['providerData']['usage']=dict(inputTokens=1,outputTokens=1)
            self.assertEqual(m.native_metadata(row,'codebuddy')[0]['kind'],'complete')
            row.update(status='incomplete',providerData=dict(skipRun=True,error=dict(message='Interrupted by user',isRetryable=False)))
            self.assertEqual(m.native_metadata(row,'codebuddy')[0]['kind'],'interrupted')
            row['type']='function_call_result'
            self.assertEqual(m.native_metadata(row,'codebuddy')[0]['kind'],'interrupted')
            row['type']='message'
            row['providerData']['error']['message']='PRIVATE_ERROR'
            self.assertEqual(m.native_metadata(row,'codebuddy')[0]['kind'],'failed')
            self.assertNotIn('PRIVATE',json.dumps(m.native_metadata(row,'codebuddy')))

    def test_native_wait_ids_grok_tools_and_parent_metadata_are_projected(self):
        for kind,expected in [('interaction.request','Elicitation'),('interaction.resolved','ElicitationResult')]:
            event=m.native_metadata(dict(type=kind,id='approval',time=1234,payload='PRIVATE'),'kimi')[0]
            self.assertEqual((event['kind'],event['call'],event['at']),(expected,'approval',1234))
            self.assertNotIn('PRIVATE',json.dumps(event))
        for kind,expected in [('tool_started','tool'),('tool_completed','output'),('permission_requested','PermissionRequest'),('permission_resolved','ElicitationResult')]:
            event=m.native_metadata(dict(type=kind,tool_name='read',tool_call_id='call',request_id='permission',arguments='PRIVATE'),'grok')[0]
            self.assertEqual(event['kind'],expected)
            self.assertNotIn('PRIVATE',json.dumps(event))
        event=m.metadata(dict(type='session_meta',payload=dict(id='child',source=dict(subagent=dict(thread_spawn=dict(parent_thread_id='parent',prompt='PRIVATE'))))),'codex',False)[0]
        self.assertEqual(event['parentSession'],'parent');self.assertNotIn('PRIVATE',json.dumps(event))

    def test_large_complete_and_split_session_records_survive_both_boundaries(self):
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'session.jsonl'
            header=dict(type='session',id='large')
            answer=dict(type='message',id='answer',timestamp='2026-10-22T00:00:00Z',message=dict(role='assistant',stopReason='stop',content=[dict(type='text',text='PRIVATE_'*45000)]))
            path.write_text(json.dumps(header)+'\n'+json.dumps(answer)+'\n')
            first=m.read_file(path,'pi',False,{})
            self.assertEqual(first['events'][-1]['kind'],'complete');self.assertNotIn('PRIVATE_',json.dumps(first))
            row=json.dumps(dict(type='message',id='prompt',message=dict(role='user',content='x'*(600*1024))))+'\n'
            with path.open('a') as f:f.write(row[:400*1024])
            partial=m.read_file(path,'pi',False,first)
            self.assertEqual(partial['events'],[]);self.assertEqual(partial['offset'],first['offset'])
            with path.open('a') as f:f.write(row[400*1024:])
            complete=m.read_file(path,'pi',False,partial)
            self.assertEqual(complete['events'][-1]['kind'],'start');self.assertEqual(complete['offset'],complete['size'])
            with path.open('a') as f:f.write('x'*(16*1024*1024+1))
            with self.assertRaisesRegex(ValueError,'16 MiB'):m.read_file(path,'pi',False,complete)

    @unittest.skipIf(os.name=='nt','SSH observer installs on POSIX machines; local Windows uses Rust')
    def test_codebuddy_observer_install_preserves_custom_hooks_and_disabling(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);path=root/'settings.json'
            original=dict(theme='light',hooks=dict(Stop=[dict(matcher='custom',hooks=[dict(type='command',command='my-observer')])]))
            path.write_text(json.dumps(original));source=dict(id='cb',provider='codebuddy',path=directory,hostId='ssh')
            self.assertEqual(m.manage(source,'ensure')['state'],'installed')
            saved=json.loads(path.read_text());self.assertEqual(saved['hooks']['Stop'][0],original['hooks']['Stop'][0])
            self.assertIn('PermissionDenied',saved['hooks']);self.assertIn('SubagentStop',saved['hooks'])
            saved['disableAllHooks']=True;path.write_text(json.dumps(saved));before=path.read_bytes()
            self.assertEqual(m.manage(source,'ensure')['state'],'disabled');self.assertEqual(path.read_bytes(),before)

    def tearDown(self):
        for discovery in m._DISCOVERY.values(): discovery.watch.close()
        m._DISCOVERY.clear()

    def test_codex_creation_source_projection_uses_exact_metadata_and_keeps_exec_tools(self):
        cases=json.loads((ROOT/'scripts/fixtures/codex-session-sources.json').read_text())
        for case in cases:
            with self.subTest(case=case['name']):
                event=m.metadata(dict(type='session_meta',payload=case['metadata']),'codex',False)[0]
                self.assertEqual(event['sessionSource'],case['source'])
                self.assertEqual(event.get('originator',''),(case['metadata'].get('originator') or '') if isinstance(case['metadata'].get('originator'),str) else '')
                self.assertEqual(event['kind'],'identity')
                self.assertNotIn('PRIVATE_',json.dumps(event))
                self.assertTrue(set(event)<=m.EVENT_KEYS)
        event=m.metadata(dict(type='response_item',payload=dict(type='function_call',name='functions.exec_command',call_id='c',arguments='PRIVATE_COMMAND')),'codex',False)[0]
        self.assertEqual(event['kind'],'tool');self.assertEqual(event['tool'],'functions.exec_command')
        self.assertNotIn('sessionSource',event);self.assertNotIn('PRIVATE_',json.dumps(event))

    def test_codex_large_log_header_incremental_cache_and_replacement(self):
        def header(source):return json.dumps(dict(type='session_meta',payload=dict(id='session',source=source)))+'\n'
        body=json.dumps(dict(type='event_msg',payload=dict(type='task_started')))+'\n'
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'session.jsonl'
            path.write_text(header('vscode')+json.dumps(dict(type='ignored',text='PRIVATE_'*50000))+'\n'+body)
            first=m.read_file(path,'codex',False,{})
            self.assertTrue(first['reset']);self.assertEqual(first['events'][0]['sessionSource'],'vscode')
            self.assertEqual(first['events'][-1]['kind'],'start');self.assertNotIn('PRIVATE_',json.dumps(first))
            with path.open('a') as stream:stream.write(json.dumps(dict(type='response_item',payload=dict(type='function_call',name='exec_command',call_id='normal')))+'\n')
            second=m.read_file(path,'codex',False,first)
            self.assertFalse(second['reset']);self.assertEqual(second['events'][0]['kind'],'tool')
            self.assertEqual(m.read_file(path,'codex',False,second)['events'],[])
            previous=second
            for index,source in enumerate(('exec',None,'cli')):
                path.rename(path.with_name('old%d.jsonl'%index))
                path.write_text((header(source) if source else '')+body)
                current=m.read_file(path,'codex',False,previous)
                self.assertTrue(current['reset'])
                sources=[e.get('sessionSource') for e in current['events'] if e['kind']=='identity']
                self.assertEqual(sources,[source,source] if source else [])
                previous=current

    def test_confirmed_eof_does_not_open_and_partial_line_is_retried(self):
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'session.jsonl'
            path.write_text(json.dumps(dict(type='event_msg',payload=dict(type='task_started')))+'\n')
            first=m.read_file(path,'codex',False,{})
            with patch.object(pathlib.Path,'open',side_effect=AssertionError('unchanged file opened')):
                cached=m.read_file(path,'codex',False,first)
            self.assertEqual(cached['events'],[]);self.assertEqual(cached['offset'],first['offset'])
            with path.open('a') as stream:stream.write(json.dumps(dict(type='event_msg',payload=dict(type='task_complete'))))
            half=m.read_file(path,'codex',False,cached)
            self.assertEqual(half['events'],[]);self.assertLess(half['offset'],half['size'])
            with path.open('a') as stream:stream.write('\n')
            complete=m.read_file(path,'codex',False,half)
            self.assertEqual(complete['events'][0]['kind'],'complete')

    def test_discovery_caches_old_files_and_falls_back_when_watching_is_unavailable(self):
        import time
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory)/'sessions';root.mkdir()
            for index in range(1000):
                path=root/('%04d.jsonl'%index);path.write_text('{}\n')
                if index<990:os.utime(path,(time.time()-172800,)*2)
            source=dict(id='s',provider='codex',path=directory)
            with patch.object(m.FileWatch,'LIMIT',32):
                cold=m.collect(source,{},processes=[])
            self.assertEqual(len(cold['files']),10)
            cursors={row['key']:row for row in cold['files']}
            calls=[];stat=pathlib.Path.stat
            def tracked(path,*args,**kwargs):calls.append(path);return stat(path,*args,**kwargs)
            with patch.object(pathlib.Path,'stat',tracked),patch.object(pathlib.Path,'open',side_effect=AssertionError('unchanged file opened')):
                warm=m.collect(source,cursors,processes=[])
            self.assertEqual(len(warm['files']),10);self.assertLess(len(calls),25)
            discovery=next(iter(m._DISCOVERY.values()));discovery.watch.close()
            added=root/'new.jsonl';added.write_text('{}\n')
            with patch.object(m.time,'monotonic',return_value=discovery.scanned+5.1):
                fallback=m.collect(source,cursors,processes=[])
            self.assertEqual(len(fallback['files']),11)

    def test_known_noninteractive_logs_do_not_use_the_recent_session_limit(self):
        import time
        with tempfile.TemporaryDirectory() as directory:
            project=pathlib.Path(directory)/'projects'/'home';project.mkdir(parents=True);now=time.time()
            def write(name,entrypoint,age):
                path=project/(name+'.jsonl');path.write_text(json.dumps(dict(type='user',sessionId=name,entrypoint=entrypoint,message=dict(content='PRIVATE_PROMPT')))+'\n')
                os.utime(path,(now-age,)*2);return path
            for index in range(300):write('sdk-%03d'%index,'sdk-cli',60+index)
            interactive=write('cli','cli',900);source=dict(id='s',provider='claude',path=directory)
            cold=m.collect(source,{})
            self.assertEqual((len(cold['files']),cold['ignored'],cold['overflow'],cold['error']),(256,[],45,None))
            # The Rust reader flags classified cursors; unchanged ones skip the limit.
            cursors={row['key']:dict(row,nonInteractive=True) for row in cold['files']}
            warm=m.collect(source,cursors)
            self.assertEqual((len(warm['files']),len(warm['ignored']),warm['overflow'],warm['error']),(45,256,0,None))
            self.assertIn(str(interactive),{row['key'] for row in warm['files']})
            changed=str(project/'sdk-000.jsonl');cursors[changed]['modified']-=1
            self.assertIn(changed,{row['key'] for row in m.collect(source,cursors)['files']})

    def test_watched_append_new_file_and_rotation_refresh_cached_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory)/'sessions';root.mkdir();path=root/'one.jsonl';path.write_text('{}\n')
            source=dict(id='s',provider='codex',path=directory)
            m.collect(source,{})
            discovery=next(iter(m._DISCOVERY.values()))
            if not discovery.watch.active:self.skipTest('native watcher unavailable; fallback covered separately')
            added=root/'two.jsonl';added.write_text('{}\n')
            paths={row['key'] for row in m.collect(source,{})['files']};self.assertIn(str(added),paths)
            path.rename(root/'renamed.jsonl');path.write_text('{}\n')
            paths={row['key'] for row in m.collect(source,{})['files']}
            self.assertEqual(paths,{str(path),str(added),str(root/'renamed.jsonl')})

    def test_retained_protocol_uses_one_process_and_exits_on_eof(self):
        import queue
        import threading
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory)/'sessions';root.mkdir();path=root/'one.jsonl'
            path.write_text(json.dumps(dict(type='event_msg',payload=dict(type='task_started')))+'\n')
            loader="import json,pathlib,sys;s=pathlib.Path(sys.argv[1]).read_text();n={'__name__':'aieyes_observer','SCRIPT':s};exec(compile(s,'observer','exec'),n);n['serve'](json.loads(sys.stdin.readline()))"
            child=subprocess.Popen([sys.executable,'-u','-c',loader,str(ROOT/'scripts/session_remote.py')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
            responses=queue.Queue()
            def read_responses():
                # Windows select() supports sockets only, so read the pipe on
                # a thread and keep the response timeout on the test thread.
                for line in child.stdout:responses.put(line)
                responses.put('')
            reader=threading.Thread(target=read_responses,daemon=True)
            reader.start()
            request=dict(action='collect',sources=[dict(id='s',provider='codex',path=directory)],cursors={})
            try:
                for index in range(3):
                    child.stdin.write(json.dumps(request)+'\n');child.stdin.flush()
                    try:line=responses.get(timeout=5)
                    except queue.Empty:self.fail('observer response timed out')
                    self.assertTrue(line,'observer exited before responding')
                    response=json.loads(line);self.assertEqual(response['aieyesSession'],1)
                    files=response['result']['polls'][0]['files'];self.assertEqual(len(files),1)
                    self.assertEqual(len(files[0]['events']),1 if index==0 else 0)
                    request['cursors']={row['key']:row for row in files}
                    self.assertIsNone(child.poll())
                child.stdin.close();child.wait(timeout=3);self.assertEqual(child.returncode,0)
            finally:
                if child.poll() is None:child.kill();child.wait()
                reader.join(timeout=3)
                child.stdin.close();child.stdout.close();child.stderr.close()

    def test_pi_and_omp_lifecycle_projection_and_mutable_title_header(self):
        import time
        for provider in ('pi','omp'):
            with self.subTest(provider=provider),tempfile.TemporaryDirectory() as directory:
                path=pathlib.Path(directory)/'sessions'/'project'/'session.jsonl';path.parent.mkdir(parents=True)
                stamp='2026-10-18T22:00:00Z'
                rows=[dict(type='session',id='native-session',timestamp=stamp),dict(type='message',id='prompt',timestamp=stamp,message=dict(role='user',content='PRIVATE_PROMPT'))]
                title=(json.dumps(dict(type='title',title='PRIVATE_TITLE')).ljust(255)+'\n') if provider=='omp' else ''
                path.write_bytes((title+''.join(json.dumps(r)+'\n' for r in rows)).encode())
                first=m.read_file(path,provider,False,{})
                self.assertTrue(any(e.get('session')=='native-session' for e in first['events']))
                self.assertEqual(first['events'][-1]['kind'],'start')
                self.assertNotIn('PRIVATE_',json.dumps(first))
                appended=dict(type='message',id='model',parentId='prompt',timestamp=stamp,message=dict(role='assistant',stopReason='toolUse',content=[dict(type='toolCall',id='call',name='bash',arguments=dict(secret='PRIVATE_ARGS'))]))
                with path.open('a') as stream:stream.write(json.dumps(appended)+'\n')
                second=m.read_file(path,provider,False,first)
                self.assertFalse(second['reset']);self.assertEqual(second['events'][0]['kind'],'tool')
                self.assertNotIn('PRIVATE_',json.dumps(second))
                terminal=m.metadata(dict(type='message',id='end',timestamp=stamp,message=dict(role='assistant',stopReason='aborted')),provider,False)
                self.assertEqual(terminal[0]['kind'],'interrupted')

    @unittest.skipIf(os.name=='nt','Remote observer is POSIX')
    def test_zcode_native_process_hooks_preserve_config_and_project_only_state(self):
        import time
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);cli=root/'cli';cli.mkdir()
            source=dict(id='z',provider='zcode',path=directory,hostId='ssh')
            foreign=dict(hooks=[dict(type='process',command='other',args=['keep'])])
            config=cli/'config.json';config.write_text(json.dumps(dict(mcp=dict(keep=True),hooks=dict(events=dict(PreToolUse=[foreign])))))
            self.assertEqual(m.manage(source,'ensure')['state'],'installed')
            installed=config.read_bytes();self.assertEqual(m.manage(source,'ensure')['state'],'installed');self.assertEqual(config.read_bytes(),installed)
            self.assertEqual(m.manage(source,'status')['state'],'installed')
            data=json.loads(installed);handler=data['hooks']['events']['UserPromptSubmit'][0]['hooks'][0]
            self.assertEqual(handler['type'],'process');self.assertNotIn('matcher',data['hooks']['events']['UserPromptSubmit'][0])
            result=subprocess.run([handler['command'],*handler['args']],input=json.dumps(dict(session_id='native',turn_id='turn',prompt='PRIVATE_PROMPT',tool_input=dict(secret='PRIVATE_KEY'))),capture_output=True,text=True,timeout=5)
            self.assertEqual(result.returncode,0);self.assertEqual(result.stdout,'')
            dbpath=cli/'db/db.sqlite';dbpath.parent.mkdir()
            with contextlib.closing(sqlite3.connect(dbpath)) as db, db:db.executescript('CREATE TABLE turn_usage(session_id TEXT,turn_id TEXT,status TEXT,started_at INTEGER,completed_at INTEGER);CREATE TABLE model_usage(session_id TEXT,turn_id TEXT,status TEXT);CREATE TABLE tool_usage(session_id TEXT,turn_id TEXT,status TEXT);')
            poll=m.collect(source,{})
            self.assertIsNone(poll['error']);self.assertEqual([f for f in poll['files'] if f['events']][0]['events'][-1]['kind'],'UserPromptSubmit');self.assertNotIn('PRIVATE',json.dumps(poll))
            data['hooks']['enabled']=False;config.write_text(json.dumps(data));before=config.read_bytes()
            self.assertEqual(m.manage(source,'ensure')['state'],'disabled');self.assertEqual(config.read_bytes(),before)
            self.assertEqual(m.manage(source,'remove')['state'],'disabled');data=json.loads(config.read_text());self.assertEqual(data['hooks']['events']['PreToolUse'],[foreign]);self.assertEqual(data['mcp'],dict(keep=True));self.assertNotIn(m.MARKER,json.dumps(data))

    def test_codebuddy_legacy_parent_identity_is_projected_without_content(self):
        user=dict(id='user',type='message',role='user',timestamp=100,content='PRIVATE_PROMPT')
        reasoning=dict(id='reason',parentId='user',type='reasoning',timestamp=101,providerData=dict(conversationRequestId='request'),content='PRIVATE_THOUGHT')
        answer=dict(id='answer',parentId='reason',type='message',role='assistant',status='completed',timestamp=102,providerData=dict(conversationRequestId='request'),content='PRIVATE_REPLY')
        projected=[m.native_metadata(row,'codebuddy')[0] for row in [user,reasoning,answer]]
        self.assertEqual([r['kind'] for r in projected],['start','thinking','complete'])
        self.assertEqual(projected[0]['turn'],'user')
        self.assertEqual(projected[1]['parent'],'user')
        self.assertEqual(projected[2]['parent'],'reason')
        self.assertNotIn('PRIVATE',json.dumps(projected))
        self.assertTrue(all(set(r)<=m.EVENT_KEYS for r in projected))

    @unittest.skipIf(os.name=='nt','Remote observer installation requires POSIX; Windows uses the Rust helper')
    def test_cursor_native_hooks_are_flat_silent_and_keep_only_lifecycle_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory)/'.cursor';root.mkdir();source=dict(id='cursor',provider='cursor',path=str(root))
            original=dict(version=1,extra='keep',hooks=dict(preToolUse=[dict(command='other-guard')]))
            (root/'hooks.json').write_text(json.dumps(original))
            self.assertEqual(m.manage(source,'ensure')['state'],'installed')
            config=json.loads((root/'hooks.json').read_text());self.assertEqual(config['hooks']['preToolUse'][0],original['hooks']['preToolUse'][0]);self.assertIn('command',config['hooks']['preToolUse'][1]);self.assertNotIn('hooks',config['hooks']['preToolUse'][1])
            before=(root/'hooks.json').read_bytes();self.assertEqual(m.manage(source,'ensure')['state'],'installed');self.assertEqual((root/'hooks.json').read_bytes(),before)
            helper=root/'.aieyes/observe.py';payload=dict(conversation_id='c',generation_id='g',tool_call_id='call',tool_name='Shell',prompt='PRIVATE_PROMPT',tool_input='PRIVATE_ARGUMENTS',tool_output='PRIVATE_RESPONSE',user_email='PRIVATE_EMAIL',transcript_path='PRIVATE_PATH')
            for kind,status in [('beforeSubmitPrompt',None),('preToolUse',None),('postToolUse',None),('stop','error')]:
                p=subprocess.run([sys.executable,str(helper),m.MARKER,'cursor',kind,str(root)],input=json.dumps(dict(payload,status=status)),capture_output=True,text=True,check=True,timeout=3)
                self.assertEqual((p.stdout,p.stderr),('',''))
            logs=list((root/'.aieyes/session-events').glob('*.jsonl'));self.assertEqual(len(logs),1);self.assertNotIn('PRIVATE_',logs[0].read_text())
            poll=m.collect(source,{});events=poll['files'][0]['events'];self.assertEqual([e['kind'] for e in events],['start','PreToolUse','PostToolUse','failed']);self.assertEqual(events[-1]['session'],'c');self.assertEqual(events[-1]['turn'],'g')
            self.assertEqual(m.manage(source,'remove')['state'],'disabled');self.assertEqual(m.manage(source,'ensure')['state'],'disabled')
            removed=json.loads((root/'hooks.json').read_text());self.assertEqual(removed['hooks']['preToolUse'],original['hooks']['preToolUse']);self.assertEqual(removed['extra'],'keep')
            self.assertEqual(m.manage(source,'repair')['state'],'installed')
            future=dict(version=2,hooks={});(root/'hooks.json').write_text(json.dumps(future));before=(root/'hooks.json').read_bytes();self.assertEqual(m.manage(source,'ensure')['state'],'error');self.assertEqual((root/'hooks.json').read_bytes(),before)

    def test_cursor_state_collection_never_reads_native_transcripts(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);folder=root/'projects/project/agent-transcripts';folder.mkdir(parents=True);(folder/'private.jsonl').write_text('{"kind":"start","session":"PRIVATE_SESSION","content":"PRIVATE_PROMPT"}\n')
            private=root/'chats/workspace/session';private.mkdir(parents=True);(private/'store.db').write_bytes(b'PRIVATE_DATABASE')
            poll=m.collect(dict(id='cursor',provider='cursor',path=directory),{})
            self.assertEqual(poll['files'],[]);self.assertNotIn('PRIVATE',json.dumps(poll))

    def test_opencode_native_database_handles_tool_step_and_final_answer(self):
        import time
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'opencode.db';stamp=int(time.time()*1000)
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                db.executescript('CREATE TABLE message(id TEXT,session_id TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);CREATE TABLE session_message(id TEXT,session_id TEXT,type TEXT,time_created INTEGER,time_updated INTEGER,data TEXT);CREATE TABLE part(message_id TEXT,data TEXT);')
                data=dict(role='assistant',finish='tool-calls',time=dict(completed=stamp),content='PRIVATE_PROMPT')
                db.execute('INSERT INTO message VALUES(?,?,?,?,?)',('m','s',stamp,stamp,json.dumps(data)))
                db.execute('INSERT INTO part VALUES(?,?)',('m',json.dumps(dict(type='tool',tool='question',state=dict(status='running',input='PRIVATE_INPUT')))))
            source=dict(id='s',provider='opencode',path=directory)
            first=m.collect(source,{})
            self.assertIsNone(first['error']);self.assertEqual(first['files'][0]['events'][-1]['kind'],'PermissionRequest')
            self.assertNotIn('PRIVATE_',json.dumps(first))
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                data['finish']='stop';db.execute('UPDATE message SET data=?',(json.dumps(data),))
            self.assertEqual(m.collect(source,{})['files'][0]['events'][-1]['kind'],'complete')

    def test_workbuddy_database_is_found_under_its_data_directory(self):
        fixture=pathlib.Path(__file__).resolve().parents[1]/'crates/core/tests/fixtures/workbuddy/workbuddy.sql'
        with tempfile.TemporaryDirectory() as directory:
            with contextlib.closing(sqlite3.connect(pathlib.Path(directory)/'workbuddy.db')) as db, db:
                db.executescript(fixture.read_text())
            (pathlib.Path(directory)/'logs').mkdir();(pathlib.Path(directory)/'logs/startup.jsonl').write_text('{"PRIVATE":1}\n')
            poll=m.collect(dict(id='w',provider='workbuddy',path=directory),{})
            self.assertIsNone(poll['error'])
            sessions=sorted(f['key'].rsplit('#',1)[-1] for f in poll['files'] if '#' in f['key'])
            self.assertEqual(sessions,['earlier-user','user-chat','user-live','user-waiting'])
            self.assertNotIn('PRIVATE',json.dumps(poll))

    def test_zcode_sqlite_status_uses_turn_and_request_tables_without_messages(self):
        import time
        with tempfile.TemporaryDirectory() as directory:
            path=pathlib.Path(directory)/'cli/db/db.sqlite';path.parent.mkdir(parents=True)
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                db.executescript('CREATE TABLE turn_usage(session_id TEXT,turn_id TEXT,status TEXT,started_at INTEGER,completed_at INTEGER);CREATE TABLE model_usage(session_id TEXT,turn_id TEXT,status TEXT);CREATE TABLE tool_usage(session_id TEXT,turn_id TEXT,status TEXT);CREATE TABLE message(content TEXT);INSERT INTO message VALUES(\'PRIVATE_RESPONSE\');')
                db.execute("INSERT INTO turn_usage VALUES('s','t','running',?,NULL)",(int(time.time()*1000),))
                db.execute("INSERT INTO model_usage VALUES('s','t','running')")
            source=dict(id='s',provider='zcode',path=directory)
            first=m.collect(source,{})
            # Request tables settle at turn end in the real CLI. Exact live
            # tool/model transitions come from the native process observer.
            self.assertIsNone(first['error']);self.assertEqual(first['files'][0]['events'][-1]['kind'],'start')
            self.assertNotIn('PRIVATE_RESPONSE',json.dumps(first))
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                db.execute("UPDATE turn_usage SET status='completed',completed_at=?",(int(time.time()*1000),))
            self.assertEqual(m.collect(source,{})['files'][0]['events'][-1]['kind'],'complete')
            with contextlib.closing(sqlite3.connect(path)) as db, db:
                db.execute("UPDATE turn_usage SET status='future-status'")
            self.assertEqual(m.collect(source,{})['files'][0]['events'][-1]['kind'],'unknown')

    def test_native_metadata_matches_lifecycle_without_reading_conversation_content(self):
        fixtures=ROOT/'crates/core/tests/fixtures/native-agents'
        for provider,name,nested in [('kimi','kimi.jsonl','sessions/s/agents/main/wire.jsonl'),('codebuddy','codebuddy.jsonl','projects/p/s.jsonl'),('grok','grok-events.jsonl','sessions/p/s/events.jsonl')]:
            with self.subTest(provider=provider),tempfile.TemporaryDirectory() as directory:
                root=pathlib.Path(directory);path=root/nested;path.parent.mkdir(parents=True)
                rows=[json.loads(line) for line in (fixtures/name).read_text().splitlines()]
                for row in rows:row['content']='PRIVATE_PROMPT'
                path.write_text(''.join(json.dumps(row)+'\n' for row in rows))
                first=m.collect(dict(id='s',provider=provider,path=directory),{})
                self.assertIsNone(first['error'])
                self.assertEqual(first['files'][0]['events'][-1]['kind'],'complete')
                self.assertNotIn('PRIVATE_PROMPT',json.dumps(first))

    def test_copilot_session_state_matches_rust_lifecycle_and_prompt_mode_is_headless(self):
        # Same fixture and expectations as native_agents.rs (Copilot CLI 1.0.105 `copilot -p`).
        fixture=ROOT/'crates/core/tests/fixtures/native-agents/copilot-events.jsonl'
        rows=[json.loads(line) for line in fixture.read_text().splitlines()]
        for row in rows:
            if row['type']=='user.message':row['data']['content']='PRIVATE_PROMPT'
            if row['type']=='assistant.message':row['data']['content']='PRIVATE_ANSWER'
        with tempfile.TemporaryDirectory() as directory:
            session=pathlib.Path(directory)/'session-state'/'9b009a56-0bf5-4830-8d65-8da8fe5fc299'
            session.mkdir(parents=True)
            (session/'events.jsonl').write_text(''.join(json.dumps(row)+'\n' for row in rows))
            (session/'workspace.yaml').write_text('cwd: PRIVATE_CWD\n')
            result=m.collect(dict(id='s',provider='copilot',path=directory),{})
            self.assertIsNone(result['error'])
            self.assertEqual([pathlib.Path(f['key']).name for f in result['files']],['events.jsonl'])
            events=result['files'][0]['events']
            # The reader re-reads the header for identity on a fresh file, so identity rows may repeat.
            self.assertEqual([e['kind'] for e in events if e['kind']!='identity'],['start','thinking','complete'])
            self.assertEqual(events[0]['session'],'9b009a56-0bf5-4830-8d65-8da8fe5fc299')
            self.assertEqual([e['kind'] for e in events if e.get('classification')=='headless'],['identity'])
            self.assertNotIn('PRIVATE_',json.dumps(result))
            self.assertNotIn('non-interactive mode',json.dumps(result))
        tool=m.native_metadata({'type':'tool.execution_start','timestamp':'2026-10-29T22:56:40.000Z','data':{'toolCallId':'call-1','toolName':'bash','arguments':'PRIVATE'}},'copilot')[0]
        self.assertEqual((tool['kind'],tool['call'],tool['tool']),('tool','call-1','bash'))
        self.assertEqual(m.native_metadata({'type':'assistant.message','data':{'toolRequests':[{'toolCallId':'call-1'}]}},'copilot'),[])

    @unittest.skipIf(os.name=='nt','SSH observer installs on POSIX machines; local Windows uses Rust')
    def test_merge_preserves_unrelated_config_and_explicit_disables(self):
        with tempfile.TemporaryDirectory() as temp:
            root=pathlib.Path(temp);source=dict(id='s',provider='claude',path=temp,hostId='ssh')
            original={'env':{'KEEP':'yes'},'hooks':{'PreToolUse':[{'matcher':'Bash','hooks':[{'type':'command','command':'other'}]}]}}
            (root/'settings.json').write_text(json.dumps(original))
            self.assertEqual(m.manage(source,'ensure')['state'],'installed')
            installed=(root/'settings.json').read_text();m.manage(source,'ensure');self.assertEqual((root/'settings.json').read_text(),installed)
            self.assertEqual(m.manage(source,'remove')['state'],'disabled');self.assertEqual(m.manage(source,'ensure')['state'],'disabled')
            data=json.loads((root/'settings.json').read_text());self.assertEqual(data['env'],original['env']);self.assertEqual(data['hooks']['PreToolUse'],original['hooks']['PreToolUse']);self.assertNotIn(m.MARKER,json.dumps(data))
            self.assertEqual(m.manage(source,'repair')['state'],'installed')
            data=json.loads((root/'settings.json').read_text());data['disableAllHooks']=True;(root/'settings.json').write_text(json.dumps(data))
            self.assertEqual(m.manage(source,'ensure')['state'],'disabled')
    @unittest.skipIf(os.name=='nt','Remote POSIX observer')
    def test_explicitly_disabled_agy_handler_requires_repair(self):
        with tempfile.TemporaryDirectory() as temp:
            root=pathlib.Path(temp);source=dict(id='s',provider='antigravity',path=temp,hostId='ssh')
            self.assertEqual(m.manage(source,'ensure')['state'],'installed')
            config=root/'settings.json';data=json.loads(config.read_text())
            data['hooks']['aieyes']['Stop'][0]['enabled']=False
            config.write_text(json.dumps(data));original=config.read_text()
            self.assertEqual(m.manage(source,'status')['state'],'disabled')
            self.assertEqual(m.manage(source,'ensure')['state'],'disabled')
            self.assertEqual(config.read_text(),original)
            self.assertEqual(m.manage(source,'repair')['state'],'installed')
            self.assertEqual(m.manage(source,'status')['state'],'installed')
    @unittest.skipIf(os.name=='nt','Remote POSIX observer')
    def test_agy_observer_has_no_permission_hook_or_content(self):
        with tempfile.TemporaryDirectory() as temp:
            source=dict(id='s',provider='antigravity',path=temp,hostId='ssh')
            self.assertEqual(m.manage(source,'ensure')['state'],'installed')
            root=pathlib.Path(temp);cfg=json.loads((root/'settings.json').read_text())
            self.assertNotIn('PreToolUse',cfg['hooks']['aieyes'])
            payload=dict(conversationId='a',fullyIdle=False,prompt='SECRET',toolCall={'name':'run_command','args':{'command':'SECRET'}})
            helper=root/'.aieyes/observe.py'
            p=subprocess.run([sys.executable,str(helper),m.MARKER,'antigravity','Stop',temp],input=json.dumps(payload),capture_output=True,text=True,timeout=3,check=True)
            self.assertEqual(json.loads(p.stdout),{'decision':'allow'});self.assertEqual(p.stderr,'')
            records=list((root/'.aieyes/session-events').glob('*.jsonl'));self.assertEqual(len(records),1)
            event=json.loads(records[0].read_text());self.assertFalse(event['fullyIdle']);self.assertNotIn('SECRET',records[0].read_text())
    def test_collector_sends_metadata_only_and_tracks_incremental_offsets(self):
        with tempfile.TemporaryDirectory() as temp:
            root=pathlib.Path(temp);p=root/'s.jsonl'
            p.write_text(json.dumps({'type':'session_meta','payload':{'id':'s','cwd':'PRIVATE'}})+'\n'+json.dumps({'type':'event_msg','payload':{'type':'task_started','turn_id':'turn','text':'SECRET'}})+'\n')
            source=dict(id='s',provider='codex',path=temp)
            first=m.collect(source,{});row=first['files'][0];self.assertNotIn('SECRET',json.dumps(first));self.assertNotIn('PRIVATE',json.dumps(first))
            self.assertEqual(row['events'][-1]['kind'],'start');self.assertEqual(m.collect(source,{str(p):row})['files'][0]['events'],[])
            with p.open('a') as f:f.write(json.dumps({'type':'response_item','payload':{'type':'function_call','name':'request_user_input','call_id':'ask','arguments':'SECRET'}})+'\n')
            self.assertEqual(m.collect(source,{str(p):row})['files'][0]['events'][0]['call'],'ask')

    def test_qoder_api_error_is_terminal_without_forwarding_response_or_stack(self):
        row=dict(type='assistant',sessionId='session',timestamp='2026-10-18T06:17:32Z',isApiErrorMessage=True,error='PRIVATE_ERROR',errorDetails='PRIVATE_STACK',message=dict(stop_reason='stop_sequence',content=[dict(type='text',text='PRIVATE_BODY')]))
        events=m.metadata(row,'qoder',False)
        self.assertEqual([event['kind'] for event in events],['failed'])
        self.assertNotIn('PRIVATE_',json.dumps(events))
        row['isApiErrorMessage']=False
        self.assertEqual(m.metadata(row,'qoder',False),[])
    def test_sqlite_wal_tools_do_not_leak_payload(self):
        with tempfile.TemporaryDirectory() as temp:
            p=pathlib.Path(temp)/'s.db';db=sqlite3.connect(p)
            try:
                db.executescript("PRAGMA journal_mode=WAL;CREATE TABLE trajectory_meta(cascade_id TEXT);INSERT INTO trajectory_meta VALUES('s');CREATE TABLE steps(idx INTEGER,step_type INTEGER,status INTEGER,payload TEXT);INSERT INTO steps VALUES(0,14,3,'SECRET'),(1,138,9,'SECRET');");db.commit()
                reader=sqlite3.connect(p.resolve().as_uri()+'?mode=ro',uri=True)
                with patch.object(sqlite3,'connect',return_value=reader):row=m.read_db(p)
                self.assertEqual(row['steps'],[(0,14,3),(1,138,9)]);self.assertNotIn('SECRET',json.dumps(row))
                with self.assertRaises(sqlite3.ProgrammingError):reader.execute('SELECT 1')
            finally:db.close()
    def test_database_cache_skips_sqlite_and_invalidates_wal_and_replacements(self):
        with tempfile.TemporaryDirectory() as temp:
            path=pathlib.Path(temp)/'s.db';db=sqlite3.connect(path)
            db.executescript("PRAGMA journal_mode=WAL;CREATE TABLE trajectory_meta(cascade_id TEXT);INSERT INTO trajectory_meta VALUES('s');CREATE TABLE steps(idx INTEGER,step_type INTEGER,status INTEGER);INSERT INTO steps VALUES(0,14,3);");db.commit()
            first=m.read_db(path)
            with patch.object(sqlite3,'connect',side_effect=AssertionError('Unchanged database reopened')):
                cached=m.read_db(path,first)
            self.assertIsNone(cached['steps']);self.assertTrue(cached['database']);self.assertFalse(cached['reset'])
            db.execute('UPDATE steps SET status=9');db.commit();changed=m.read_db(path,cached)
            self.assertEqual(changed['steps'][0][2],9);self.assertNotEqual(changed['signature'],cached['signature'])
            db.close();replacement=pathlib.Path(temp)/'new.db';db=sqlite3.connect(replacement)
            db.executescript("CREATE TABLE trajectory_meta(cascade_id TEXT);INSERT INTO trajectory_meta VALUES('replacement');CREATE TABLE steps(idx INTEGER,step_type INTEGER,status INTEGER);");db.commit();db.close();os.replace(replacement,path)
            self.assertEqual(m.read_db(path,changed)['session'],'replacement')
            # Old callers omit the capability and still receive complete steps each time.
            source=dict(id='s',provider='antigravity',path=temp)
            result=m.collect(source,{str(path):m.read_db(path)})
            self.assertIsNotNone(result['files'][0]['steps'])
    def test_agy_database_activity_comes_from_metadata_not_file_touches(self):
        with tempfile.TemporaryDirectory() as temp:
            path=pathlib.Path(temp)/'s.db';db=sqlite3.connect(path)
            db.executescript("CREATE TABLE trajectory_meta(cascade_id TEXT);INSERT INTO trajectory_meta VALUES('s');CREATE TABLE steps(idx INTEGER,step_type INTEGER,status INTEGER,metadata BLOB);")
            # created_at.seconds=9999; completed_at.seconds=10000.
            raw=b'\x0a\x03\x08\x8f\x4e\x42\x03\x08\x90\x4e'
            db.execute('INSERT INTO steps VALUES(0,15,3,?)',(raw,));db.commit();db.close()
            row=m.read_db(path)
            self.assertEqual(row['activityAt'],10000)
            self.assertGreater(row['modified'],row['activityAt'])
            self.assertNotIn('metadata',row)
            self.assertEqual(m.agy_activity_at(b'\x0a\xff'),0)
    def test_failed_cached_database_read_preserves_a_degraded_placeholder(self):
        with tempfile.TemporaryDirectory() as temp:
            path=pathlib.Path(temp)/'s.db';path.write_bytes(b'invalid sqlite')
            cursor=dict(signature='db:previous',identity='old',modified=int(m.time.time()))
            result=m.collect(dict(id='s',provider='antigravity',path=temp),{str(path):cursor},True)
            self.assertTrue(result['error']);self.assertEqual(len(result['files']),1)
            self.assertIsNone(result['files'][0]['steps']);self.assertFalse(result['files'][0]['reset'])
    def test_invalid_database_closes_reader(self):
        with tempfile.TemporaryDirectory() as temp:
            p=pathlib.Path(temp)/'invalid.db';sqlite3.connect(p).close()
            reader=sqlite3.connect(p.resolve().as_uri()+'?mode=ro',uri=True)
            with patch.object(sqlite3,'connect',return_value=reader),self.assertRaises(sqlite3.OperationalError):m.read_db(p)
            with self.assertRaises(sqlite3.ProgrammingError):reader.execute('SELECT 1')
    def test_headless_argv_table_matches_the_shared_fixture(self):
        for case in json.loads((ROOT/'scripts/fixtures/headless-argv.json').read_text()):
            with self.subTest(**case):self.assertEqual(m.headless_argv(case['provider'],case['argv']),case['expected'])
    def test_hook_records_argv_verdict_and_aieyes_internal_runs(self):
        if os.name=='nt':return  # SSH hook helper uses POSIX file locks.
        def run(env):
            with tempfile.TemporaryDirectory() as directory:
                root=pathlib.Path(directory);(root/'.aieyes/session-events').mkdir(parents=True)
                helper=root/'observe.py';helper.write_text(m.SCRIPT)
                subprocess.run([sys.executable,str(helper),m.MARKER,'kimi','UserPromptSubmit',directory],input=json.dumps(dict(session_id='s',prompt='PRIVATE')),env=env,capture_output=True,text=True,check=True,timeout=3)
                return json.loads(next((root/'.aieyes/session-events').glob('*.jsonl')).read_text())
        base={k:v for k,v in os.environ.items() if k!='AIEYES_INTERNAL'}
        self.assertNotIn('classification',run(base))
        internal=run(dict(base,AIEYES_INTERNAL='1'))
        self.assertEqual((internal['classification'],internal['sessionSource']),('internal','aieyes-internal'))
        self.assertNotIn('PRIVATE',json.dumps(internal))
        with patch.object(m,'process_info',side_effect=lambda pid:('kimi',0,1) if pid==7 else None),patch.object(m,'cmdline',return_value=['kimi','-p','hi']),patch.object(m.os,'getppid',return_value=7):
            self.assertEqual(m.ancestor('kimi'),(0,0,'headless'))
        with patch.object(m,'process_info',return_value=('sh',0,1)),patch.object(m,'cmdline',return_value=['sh','-c','observe.py '+m.MARKER+' kimi']),patch.object(m.os,'getppid',return_value=7):
            self.assertEqual(m.ancestor('kimi'),(0,0,None))
        with patch.object(m,'process_info',side_effect=lambda pid:('node',0,1) if pid==7 else None),patch.object(m,'cmdline',return_value=['cursor-agent','-p']),patch.object(m.os,'getppid',return_value=7):
            self.assertEqual(m.ancestor('cursor'),(0,0,'headless'))
class WorkForkTests(unittest.TestCase):
    def tearDown(self):
        for discovery in m._DISCOVERY.values(): discovery.watch.close()
        m._DISCOVERY.clear(); m._PROCESS_CONTEXTS.clear()

    def test_embedded_ssh_forks_inherit_work_without_changing_explicit_origin_or_source(self):
        import contextlib, io
        scripts = pathlib.Path(__file__).resolve().parent
        scope = dict(__name__='aieyes_embedded',HISTORY_SCRIPT=(scripts/'remote_history.py').read_text(),HARNESS_SCRIPT=(scripts/'harness_remote.py').read_text())
        exec(compile((scripts/'session_remote.py').read_text(),'<aieyes-embedded-sessions>','exec'),scope)
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory).resolve()
            def header(identity,origin='',source='cli',base=None):
                payload=dict(id=identity,source=source)
                if origin:payload['originator']=origin
                if base:payload['history_base']=dict(thread_id=base[0],end_byte_offset=base[1])
                return json.dumps(dict(type='session_meta',payload=payload))+'\n'
            parent=header('parent','codex_work_desktop','vscode')
            middle=header('middle',base=('parent',len(parent.encode())))
            rows=dict(parent=parent,middle=middle,child=header('child',base=('middle',len(middle.encode()))),explicit=header('explicit','codex_cli_rs',base=('parent',len(parent.encode()))),headless=header('headless',source='exec',base=('parent',len(parent.encode()))))
            for identity,value in rows.items():(root/(identity+'.jsonl')).write_text(value+json.dumps(dict(type='event_msg',payload=dict(type='task_started',private='PRIVATE_PROMPT')))+'\n')
            with contextlib.redirect_stdout(io.StringIO()) as output:
                result=scope['collect'](dict(provider='codex',id='codex',path=str(root)),{},processes=[])
            self.assertEqual(output.getvalue(),'','loading the history resolver must not execute its CLI')
            origins={pathlib.Path(row['key']).stem:next(event for event in row['events'] if event['kind']=='identity') for row in result['files']}
            self.assertEqual({key:value.get('originator') for key,value in origins.items()},dict(parent='codex_work_desktop',middle='codex_work_desktop',child='codex_work_desktop',explicit='codex_cli_rs',headless='codex_work_desktop'))
            self.assertEqual(origins['headless']['sessionSource'],'exec')
            self.assertNotIn('PRIVATE_',json.dumps(result))
        for discovery in scope['_DISCOVERY'].values():discovery.watch.close()

if __name__=='__main__':unittest.main()
