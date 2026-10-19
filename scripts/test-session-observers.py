import contextlib
import importlib.util,json,os,pathlib,sqlite3,subprocess,sys,tempfile,unittest
from unittest.mock import patch
ROOT=pathlib.Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('sessions',ROOT/'scripts/session_remote.py')
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);m.SCRIPT=(ROOT/'scripts/session_remote.py').read_text()
class ObserverTests(unittest.TestCase):
    def tearDown(self):
        for discovery in m._DISCOVERY.values(): discovery.watch.close()
        m._DISCOVERY.clear()

    def test_codex_creation_source_projection_uses_exact_metadata_and_keeps_exec_tools(self):
        cases=json.loads((ROOT/'scripts/fixtures/codex-session-sources.json').read_text())
        for case in cases:
            with self.subTest(case=case['name']):
                event=m.metadata(dict(type='session_meta',payload=case['metadata']),'codex',False)[0]
                self.assertEqual(event['sessionSource'],case['source'])
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
                cold=m.collect(source,{})
            self.assertEqual(len(cold['files']),10)
            cursors={row['key']:row for row in cold['files']}
            calls=[];stat=pathlib.Path.stat
            def tracked(path,*args,**kwargs):calls.append(path);return stat(path,*args,**kwargs)
            with patch.object(pathlib.Path,'stat',tracked),patch.object(pathlib.Path,'open',side_effect=AssertionError('unchanged file opened')):
                warm=m.collect(source,cursors)
            self.assertEqual(len(warm['files']),10);self.assertLess(len(calls),25)
            discovery=next(iter(m._DISCOVERY.values()));discovery.watch.close()
            added=root/'new.jsonl';added.write_text('{}\n')
            with patch.object(m.time,'monotonic',return_value=discovery.scanned+5.1):
                fallback=m.collect(source,cursors)
            self.assertEqual(len(fallback['files']),11)

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
if __name__=='__main__':unittest.main()
